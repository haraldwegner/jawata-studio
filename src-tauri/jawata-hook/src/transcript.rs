//! Sprint 28g S21 — the discussion lane, from the hook's side.
//!
//! The engine keeps the discussion in a lane of its own (what he typed, what the agent said to
//! him, one line per tool call). This module is the three moments the hooks touch it:
//!
//! | Moment | Role that fires it | What happens |
//! |---|---|---|
//! | a turn ends | stop | every window since the last one stored is stored, because a turn can be split into several windows by a notification or a nudge |
//! | work goes on | observer, at most once every two minutes | the window so far is stored: small dated steps, so a long unattended stretch is never one unsaved block |
//! | a session restarts after a compaction | primer | every window still readable is stored, then the last things he said come back into the new context |
//! | a message attributes words to him | stop | each quoted sentence is looked up among his words; one not found is named back, once |
//!
//! Everything here reads the client's transcript through [`crate::stop::read_turn`], the one
//! parser of that file, and talks to the engine through the same [`Store`] every other role uses.
//! A capture never changes what a hook answers: every failure is recorded and swallowed, because
//! the discussion lane exists to remember and a hook that cannot remember must still do its job.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::pipeline::{self, Store, TRANSCRIPT_TAIL_BYTES};
use crate::roles::Client;
use crate::stop::{self, WindowCapture};

/// The most often a running session is captured on the observer's clock.
const MIN_GAP_SECS: u64 = 120;

/// How many of his latest messages come back after a compaction.
const THREAD_MESSAGES: u32 = 12;

/// The most attributed quotes checked in one message.
const MAX_ATTRIBUTIONS: usize = 5;

/// A quote shorter than this is a phrase, not a sentence he could be misquoted in.
const MIN_QUOTE_WORDS: usize = 5;

/// A quote longer than this is a pasted passage, not a quotation.
const MAX_QUOTE_CHARS: usize = 400;

/// The most a check of attributions takes of the hook's time. The hook as a whole is killed
/// silently at four seconds and the decision it has already reached is printed last, so every
/// record job here is held to a share of that AND to [`crate::safety::optional_work_ends`], the
/// process-wide moment after which only the answer may still run.
const CHECK_BUDGET: std::time::Duration = std::time::Duration::from_millis(1200);

/// The most storing the windows of a turn takes, before the answer is printed.
const TURN_CAPTURE_BUDGET: std::time::Duration = std::time::Duration::from_millis(1200);

/// The most storing windows after a compaction takes, before the thread is asked for.
const COMPACTION_CAPTURE_BUDGET: std::time::Duration = std::time::Duration::from_millis(2000);

/// Less than this left, and an ask is not made: it could not be answered in time.
const SMALLEST_ASK: std::time::Duration = std::time::Duration::from_millis(150);

/// The moment a job given `share` must have stopped: its own share from now, and never later than
/// the moment the whole hook's optional work ends.
fn ends_by(share: std::time::Duration) -> std::time::Instant {
    (std::time::Instant::now() + share).min(crate::safety::optional_work_ends())
}

/// What is left before `until`, or `None` when too little is left to make an ask.
fn left_before(until: std::time::Instant) -> Option<std::time::Duration> {
    let left = until.saturating_duration_since(std::time::Instant::now());
    (left >= SMALLEST_ASK).then_some(left)
}

/// The furthest back a read goes to find where a window opened. A window can be longer than the
/// tail a hook normally reads (a long run of tool calls is megabytes), and a window whose opening
/// is not in the text read has no key to be stored under.
const REACH_LIMIT_BYTES: u64 = 64 * 1024 * 1024;

/// Words that say what follows is HIS: a quotation near one is an attribution to him.
const ATTRIBUTION_CUES: &[&str] = &[
    "he said", "he told", "he wrote", "he ruled", "he asked", "he answered", "he replied",
    "he insisted", "he put it", "as he put it", "his words", "his word", "in his words",
    "you said", "you told", "your words", "harald said", "harald wrote", "harald's words",
];

fn state_dir() -> Option<PathBuf> {
    pipeline::home_dir().map(|h| h.join(".claude").join("jawata-studio").join("transcript"))
}

fn signal_dir() -> Option<PathBuf> {
    pipeline::home_dir().map(|h| h.join(".claude").join("jawata-studio"))
}

/// One line in the outcomes log, when there is a directory to write it in. Tests pass `None` or a
/// scratch directory: a unit test must never write into the developer's real log.
fn record(signals: Option<&Path>, signal: &str, detail: &str) {
    if let Some(dir) = signals {
        crate::observer::emit_signal(dir, signal, detail);
    }
}

fn client_name(client: Client) -> &'static str {
    match client {
        Client::ClaudeCode => "claude-code",
        Client::Cursor => "cursor",
    }
}

/// The session and the tail of its transcript, from a hook payload.
fn transcript_of(payload: &str) -> Option<(String, String)> {
    let v: Value = serde_json::from_str(payload).ok()?;
    let path = v.get("transcript_path").and_then(Value::as_str)?;
    let session = pipeline::session_id_in(payload)?;
    let text = pipeline::read_tail(path, TRANSCRIPT_TAIL_BYTES).ok()?;
    Some((session, text))
}

/// The turn read from the transcript, reading further back until `wants` is satisfied: the tail
/// first, then four times as much, up to [`REACH_LIMIT_BYTES`] or the whole file. Reading the tail
/// alone loses a window that opened before it, silently and for good.
fn turn_reaching(
    payload: &str,
    wants: &dyn Fn(&stop::Turn) -> bool,
    limit: u64,
    until: std::time::Instant,
) -> Option<(String, stop::Turn)> {
    let v: Value = serde_json::from_str(payload).ok()?;
    let path = v.get("transcript_path").and_then(Value::as_str)?;
    let session = pipeline::session_id_in(payload)?;
    let file_len = std::fs::metadata(path).ok()?.len();
    let mut size = TRANSCRIPT_TAIL_BYTES;
    loop {
        let text = pipeline::read_tail(path, size).ok()?;
        let turn = stop::read_turn(&text).ok()?;
        if wants(&turn) || size >= file_len || size >= limit || std::time::Instant::now() >= until {
            return Some((session, turn));
        }
        size = size.saturating_mul(4).min(limit);
    }
}

/// Store one window. `None` for a window whose opening is not in the tail read, which has no id to
/// complete it by; `Some(Err)` when the engine could not take it.
fn append_window(
    store: &dyn Store,
    session: &str,
    client: Client,
    kind: &str,
    window: &WindowCapture,
    within: std::time::Duration,
) -> Option<Result<String, String>> {
    let key = window.key.as_deref()?;
    if window.is_empty() {
        return None;
    }
    let mut request = json!({
        "kind": "transcript_append",
        "session": session,
        "key": key,
        "client": client_name(client),
        "capture": kind,
        "his": window.his,
        "agent": window.agent_text(),
        "tools": window.tool_lines(),
    });
    if let Some(ts) = window.opened_at.as_deref() {
        request["ts"] = json!(ts);
    }
    Some(match store.ask_value_within(request, within) {
        Ok(v) => Ok(v.get("appended").and_then(Value::as_str).unwrap_or("?").to_string()),
        Err(e) => Err(format!("{e:?}")),
    })
}

/// How a capture is taken: what it is called, how far back to go with no stamp, how long it may take.
struct Capture {
    kind: &'static str,
    without_stamp: usize,
    /// The moment this capture must have stopped, whatever is left to store.
    until: std::time::Instant,
    /// The furthest back, in bytes, it will read to find where a window opened.
    reach: u64,
}

/// Store what the transcript holds that was not stored last time: every window since the last one
/// stored, and the window it ends in. Fired when a turn ends.
///
/// A turn is not always one window. A background job finishing, or the client's own nudge to an
/// answer that had no visible output, opens a new window in the middle of what he would call one
/// turn, and the first window then holds the tool calls and the second holds the answer. Storing
/// only the last window loses the first for good, so the stamp of the last window stored (a file
/// per session) says where to resume.
pub fn capture_current(client: Client, payload: &str, store: &dyn Store) {
    let Some(state) = state_dir() else { return };
    capture_current_in(&state, signal_dir().as_deref(), client, payload, store);
}

/// How many already-closed windows are stored when the stamp does not say where to resume.
const WINDOWS_WITHOUT_A_STAMP: usize = 8;

fn stamp_file(dir: &Path, session: &str) -> PathBuf {
    dir.join(format!("{}.last", pipeline::sanitize_session(session)))
}

/// The windows still to store, oldest first: from the one the stamp names (it may have grown since)
/// to the end; with no stamp, or one that has scrolled out of what was read, the last `fallback`.
fn unsaved<'a>(turn: &'a stop::Turn, last: Option<&str>, fallback: usize) -> Vec<&'a WindowCapture> {
    let all: Vec<&WindowCapture> = turn
        .closed
        .iter()
        .chain(std::iter::once(&turn.capture))
        .filter(|w| w.key.is_some())
        .collect();
    if let Some(key) = last {
        if let Some(i) = all.iter().rposition(|w| w.key.as_deref() == Some(key)) {
            return all[i..].to_vec();
        }
    }
    let from = all.len().saturating_sub(fallback);
    all[from..].to_vec()
}

fn capture_current_in(
    state: &Path,
    signals: Option<&Path>,
    client: Client,
    payload: &str,
    store: &dyn Store,
) {
    let how = Capture {
        kind: "turn",
        without_stamp: WINDOWS_WITHOUT_A_STAMP,
        until: ends_by(TURN_CAPTURE_BUDGET),
        reach: REACH_LIMIT_BYTES,
    };
    store_unsaved(state, signals, client, payload, store, &how);
}

/// Say so when what was read does not reach where the work has to start: the window being stored
/// opened before anything the read could cover, or the window the stamp names has scrolled out of
/// it. Either leaves history unstored, and a silence there would read as nothing to store.
fn reach_signals(signals: Option<&Path>, turn: &stop::Turn, last: Option<&str>, kind: &str) {
    if turn.capture.key.is_none() {
        record(signals, "transcript-window-out-of-reach", &format!("{kind}|the window opened before the furthest point read"));
    }
    if let Some(k) = last {
        let held = turn.closed.iter().chain(std::iter::once(&turn.capture)).any(|w| w.key.as_deref() == Some(k));
        if !held {
            record(signals, "transcript-stamp-out-of-reach", &format!("{kind}|the last window stored is not in what was read"));
        }
    }
}

/// Store the windows not yet stored, oldest first, and return how many the engine took. Stops at
/// `how.until`, and no ask is given longer than what is left: the stamp stays at the last window stored, so the next capture resumes where this
/// one stopped and no window is ever skipped over.
fn store_unsaved(
    state: &Path,
    signals: Option<&Path>,
    client: Client,
    payload: &str,
    store: &dyn Store,
    how: &Capture,
) -> usize {
    let Some(session) = pipeline::session_id_in(payload) else { return 0 };
    let last = std::fs::read_to_string(stamp_file(state, &session)).ok().map(|k| k.trim().to_string());
    let reached = |t: &stop::Turn| {
        if t.capture.key.is_none() {
            return false;
        }
        match last.as_deref() {
            Some(k) => t.closed.iter().chain(std::iter::once(&t.capture)).any(|w| w.key.as_deref() == Some(k)),
            None => t.closed.len() >= how.without_stamp,
        }
    };
    let Some((_, turn)) = turn_reaching(payload, &reached, how.reach, how.until) else { return 0 };
    reach_signals(signals, &turn, last.as_deref(), how.kind);
    let mut resume_at: Option<String> = None;
    let mut stored = 0usize;
    for window in unsaved(&turn, last.as_deref(), how.without_stamp) {
        let Some(within) = left_before(how.until) else {
            record(signals, "transcript-capture-slow", &format!("{}|stored={stored}", how.kind));
            break;
        };
        match append_window(store, &session, client, how.kind, window, within) {
            Some(Err(e)) => {
                // Not advanced past a window the engine could not take: the next capture retries it.
                record(signals, "transcript-capture-failed", &format!("{}|{e}", how.kind));
                break;
            }
            Some(Ok(outcome)) => {
                if outcome == "disabled" {
                    record(signals, "transcript-capture-off", "the lane is switched off on this deployment");
                    break;
                }
                resume_at = window.key.clone();
                stored += 1;
            }
            None => {}
        }
    }
    if let Some(key) = resume_at {
        let _ = std::fs::create_dir_all(state);
        let _ = std::fs::write(stamp_file(state, &session), key);
    }
    stored
}

/// Store the window so far, at most once every [`MIN_GAP_SECS`] per session. Fired by the observer
/// on each finished tool call; the clock is a file per session, so a thousand calls cost a thousand
/// file reads and one capture in two minutes.
pub fn capture_on_cadence(client: Client, payload: &str, store: &dyn Store) {
    let (Some(dir), Some(session)) = (state_dir(), pipeline::session_id_in(payload)) else { return };
    if !cadence_due_in(&dir, &session, now_secs(), MIN_GAP_SECS) {
        return;
    }
    capture_current(client, payload, store);
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Whether a capture is due for `session`, and if so, claim it: the stamp is written before the
/// capture runs, so an engine that is down is tried once per gap and not on every tool call.
fn cadence_due_in(dir: &Path, session: &str, now: u64, gap: u64) -> bool {
    let stamp = dir.join(format!("{}.stamp", pipeline::sanitize_session(session)));
    let last: u64 = std::fs::read_to_string(&stamp)
        .ok()
        .and_then(|t| t.trim().parse().ok())
        .unwrap_or(0);
    if last != 0 && now.saturating_sub(last) < gap {
        return false;
    }
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(&stamp, now.to_string());
    true
}

/// Fired when a session starts: after a COMPACTION (the start event whose source says so), store
/// the windows the transcript holds that were not stored, and hand back the thread he was following.
///
/// The transcript file keeps what a compaction removes from the model's context, so the catch-up
/// reads it from disk after the fact rather than needing a hook that fires before. It resumes from
/// the stamp the stop and observer captures keep, so it sends what is missing and not the whole
/// history again; with no stamp it sends every window still readable. It is held to a time budget
/// because the hook is killed silently at four seconds, and the hand-back is what the start is for.
///
/// `None` for any other kind of start, and the engine is not asked.
pub fn after_compaction(client: Client, payload: &str, store: &dyn Store) -> Option<String> {
    after_compaction_in(&state_dir()?, signal_dir().as_deref(), client, payload, store)
}

fn after_compaction_in(
    state: &Path,
    signals: Option<&Path>,
    client: Client,
    payload: &str,
    store: &dyn Store,
) -> Option<String> {
    let v: Value = serde_json::from_str(payload).ok()?;
    if v.get("source").and_then(Value::as_str) != Some("compact") {
        return None;
    }
    let session = pipeline::session_id_in(payload)?;
    let how = Capture {
        kind: "compaction",
        without_stamp: stop::MAX_CLOSED_WINDOWS,
        until: ends_by(COMPACTION_CAPTURE_BUDGET),
        reach: REACH_LIMIT_BYTES,
    };
    let stored = store_unsaved(state, signals, client, payload, store, &how);
    record(signals, "transcript-compaction-catchup", &format!("windows={stored}"));
    let request = json!({
        "kind": "recall",
        "format": "text",
        "lanes": crate::lanes::argument(crate::lanes::AFTER_COMPACTION),
        "recent": THREAD_MESSAGES,
        "session": session,
    });
    match store.ask(request) {
        Ok(crate::query::Answer::Text(thread)) if !thread.trim().is_empty() => Some(format!(
            "AFTER A COMPACTION — what he said most recently, from the transcript lane, newest \
             first and verbatim. The summary above this line is the model's; these are his \
             words:\n{thread}\nAsk `experience(kind=recall, lane=transcript, symptom=<his words>)` \
             for anything older."
        )),
        _ => None,
    }
}

// --- before something is attributed to him -------------------------------------------------

/// The quotations in `text` that attribute words to him: a sentence in quotation marks on a line
/// that says who said it, or a block quote straight under such a line.
pub fn attributed_quotes(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<String> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let cued_here = has_cue(line);
        let cued_above = i > 0 && has_cue(lines[i - 1]) && line.trim_start().starts_with('>');
        if !cued_here && !cued_above {
            continue;
        }
        let mut found: Vec<String> = quoted_spans(line);
        if cued_above {
            let body = line
                .trim_start()
                .trim_start_matches('>')
                .trim()
                .trim_matches(['"', '“', '”'])
                .to_string();
            found.push(body);
        }
        for q in found {
            let q = q.trim().to_string();
            if q.split_whitespace().count() >= MIN_QUOTE_WORDS
                && q.chars().count() <= MAX_QUOTE_CHARS
                && !out.contains(&q)
            {
                out.push(q);
            }
        }
    }
    out.truncate(MAX_ATTRIBUTIONS);
    out
}

fn has_cue(line: &str) -> bool {
    let lower = line.to_lowercase();
    ATTRIBUTION_CUES.iter().any(|c| lower.contains(c))
}

/// The text between quotation marks, straight or curly.
fn quoted_spans(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut open: Option<char> = None;
    let mut current = String::new();
    for ch in line.chars() {
        match (open, ch) {
            (None, '"') => open = Some('"'),
            (None, '“') => open = Some('“'),
            (Some('"'), '"') | (Some('“'), '”') => {
                out.push(std::mem::take(&mut current));
                open = None;
            }
            (Some(_), c) => current.push(c),
            (None, _) => {}
        }
    }
    out
}

fn named_file(dir: &Path, session: &str) -> PathBuf {
    dir.join(format!("{}.named", pipeline::sanitize_session(session)))
}

fn flatten(q: &str) -> String {
    q.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// The stop gate's check: words in the message that are said to be his, and that the transcript
/// lane does not hold among his words, are named back with a block. Once per sentence per session,
/// so a sentence he really did say in a way the lane cannot match costs one extra turn and no loop.
///
/// `None` when nothing needs naming, when the lane could not check (off, not installed, engine
/// down), or when this stop is already a retry.
pub fn name_back(client: Client, payload: &str, store: &dyn Store) -> Option<crate::safety::Outcome> {
    name_back_in(&state_dir()?, signal_dir().as_deref(), client, payload, store, ends_by(CHECK_BUDGET))
}

fn name_back_in(
    dir: &Path,
    signals: Option<&Path>,
    client: Client,
    payload: &str,
    store: &dyn Store,
    until: std::time::Instant,
) -> Option<crate::safety::Outcome> {
    let v: Value = serde_json::from_str(payload).ok()?;
    if v.get("stop_hook_active").and_then(Value::as_bool).unwrap_or(false) {
        return None;
    }
    let session = pipeline::session_id_in(payload)?;
    let (_, text) = transcript_of(payload)?;
    let turn = stop::read_turn(&text).ok()?;
    let already: Vec<String> = std::fs::read_to_string(named_file(dir, &session))
        .map(|t| t.lines().map(str::to_string).collect())
        .unwrap_or_default();
    let mut unfounded: Vec<String> = Vec::new();
    for quote in attributed_quotes(&turn.final_text) {
        let Some(within) = left_before(until) else { break };
        if already.contains(&flatten(&quote)) {
            continue;
        }
        let Ok(answer) =
            store.ask_value_within(json!({"kind": "transcript_verify", "quote": quote}), within)
        else {
            continue;
        };
        let checked = answer.get("checked").and_then(Value::as_bool).unwrap_or(false);
        let found = answer.get("found").and_then(Value::as_bool).unwrap_or(false);
        if checked && !found {
            unfounded.push(quote);
        }
    }
    if unfounded.is_empty() {
        return None;
    }
    let _ = std::fs::create_dir_all(dir);
    let mut named = already;
    named.extend(unfounded.iter().map(|q| flatten(q)));
    let _ = std::fs::write(named_file(dir, &session), named.join("\n"));
    record(signals, "transcript-attribution-unfounded", &format!("quotes={}", unfounded.len()));
    let list = unfounded.iter().map(|q| format!("- \"{q}\"")).collect::<Vec<_>>().join("\n");
    let reason = format!(
        "You put these words in his mouth, and the transcript lane holds no such sentence among \
         what he said:\n{list}\nFind what he actually said (`experience(kind=recall, \
         lane=transcript, symptom=<his words>)`) and quote that, or give it as your own reading \
         and not as his words. This is named once; it will not hold the stop again."
    );
    match crate::emit::render(client, &crate::emit::Emission::StopDecision { reason }) {
        Some(rendered) => Some(crate::safety::Outcome::Emitted(rendered)),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::{Answer, QueryError};
    use std::cell::RefCell;

    /// A store that records what it is asked and answers from a script.
    struct Stub {
        seen: RefCell<Vec<Value>>,
        verify: Option<Value>,
        text: Option<String>,
    }

    impl Stub {
        fn quiet() -> Self {
            Stub { seen: RefCell::new(vec![]), verify: None, text: None }
        }
    }

    impl Store for Stub {
        fn ask(&self, arguments: Value) -> Result<Answer, QueryError> {
            self.seen.borrow_mut().push(arguments);
            match &self.text {
                Some(t) => Ok(Answer::Text(t.clone())),
                None => Ok(Answer::Nothing),
            }
        }
        fn ask_value(&self, arguments: Value) -> Result<Value, QueryError> {
            let verify = arguments.get("kind").and_then(Value::as_str) == Some("transcript_verify");
            self.seen.borrow_mut().push(arguments);
            match (&self.verify, verify) {
                (Some(v), true) => Ok(v.clone()),
                _ => Ok(json!({"appended": "stored"})),
            }
        }
    }

    /// A deadline far enough away that a test of something else never meets it.
    fn soon() -> std::time::Instant {
        std::time::Instant::now() + std::time::Duration::from_secs(30)
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jawata-transcript-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A tiny transcript: two windows of his, a tool call that failed, an answer.
    fn transcript_file(dir: &Path, final_text: &str) -> String {
        let lines = [
            json!({"type": "user", "uuid": "u1", "timestamp": "2026-10-17T09:00:00Z",
                   "message": {"role": "user", "content": "first we need the switch for the works council"}}),
            json!({"type": "assistant", "uuid": "a1", "message": {"content": [
                {"type": "thinking", "thinking": "PRIVATE REASONING MARKER", "signature": "x"},
                {"type": "text", "text": "I will add the switch."},
                {"type": "tool_use", "id": "t1", "name": "Bash", "input": {"command": "mvn  test"}}]}}),
            json!({"type": "user", "uuid": "r1", "message": {"content": [
                {"type": "tool_result", "tool_use_id": "t1", "is_error": true, "content": "BIG OUTPUT"}]}}),
            json!({"type": "user", "uuid": "u2", "timestamp": "2026-10-17T09:05:00Z",
                   "message": {"role": "user", "content": "no size cap, just compression"}}),
            json!({"type": "assistant", "uuid": "a2", "message": {"content": [
                {"type": "text", "text": final_text}]}}),
        ];
        let path = dir.join("session.jsonl");
        std::fs::write(&path, lines.iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n")).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn payload(path: &str, extra: Value) -> String {
        let mut v = json!({"session_id": "s-1", "transcript_path": path});
        if let (Some(m), Some(e)) = (v.as_object_mut(), extra.as_object()) {
            for (k, val) in e {
                m.insert(k.clone(), val.clone());
            }
        }
        v.to_string()
    }

    // ---- what a window holds ---------------------------------------------------------------

    #[test]
    fn a_window_holds_his_words_the_agents_and_one_line_per_tool_call_and_no_output() {
        let dir = scratch("window");
        let path = transcript_file(&dir, "Understood.");
        let turn = stop::read_turn(&std::fs::read_to_string(&path).unwrap()).unwrap();

        assert_eq!(Some("u2".to_string()), turn.capture.key, "the window is keyed by the record that opened it");
        assert_eq!("no size cap, just compression", turn.capture.his);
        assert_eq!(vec!["Understood.".to_string()], turn.capture.agent);

        assert_eq!(1, turn.closed.len(), "the earlier window is kept in the history");
        let first = &turn.closed[0];
        assert_eq!("first we need the switch for the works council", first.his);
        assert_eq!(vec!["Bash mvn test failed".to_string()], first.tool_lines(),
            "one line per call, the key argument on one line, the failure marked");
        assert!(!format!("{first:?}").contains("BIG OUTPUT"), "tool output is never kept");
        assert!(!format!("{first:?}").contains("PRIVATE REASONING MARKER"), "reasoning is never kept");
    }

    // ---- capture ---------------------------------------------------------------------------

    #[test]
    fn a_turn_end_stores_every_unsaved_window_and_then_only_from_the_last_one_stored() {
        let dir = scratch("capture");
        let path = transcript_file(&dir, "Understood.");
        let state = scratch("capture-state");
        let store = Stub::quiet();
        capture_current_in(&state, None, Client::ClaudeCode, &payload(&path, json!({})), &store);

        {
            let seen = store.seen.borrow();
            assert_eq!(2, seen.len(), "with no stamp, the windows the tail holds are stored: {seen:?}");
            let r = &seen[1];
            assert_eq!("transcript_append", r["kind"]);
            assert_eq!("s-1", r["session"]);
            assert_eq!("u2", r["key"]);
            assert_eq!("claude-code", r["client"]);
            assert_eq!("turn", r["capture"]);
            assert_eq!("no size cap, just compression", r["his"]);
            assert_eq!("Understood.", r["agent"]);
            assert_eq!("2026-10-17T09:05:00Z", r["ts"]);
        }
        store.seen.borrow_mut().clear();
        capture_current_in(&state, None, Client::ClaudeCode, &payload(&path, json!({})), &store);
        let again = store.seen.borrow();
        assert_eq!(1, again.len(), "the second capture resumes at the last window stored: {again:?}");
        assert_eq!("u2", again[0]["key"], "which may have grown, so it is sent again");
    }

    /// The incident behind the stamp: a notification or a nudge opens a window in the middle of one
    /// turn, the tool calls are in the first, the answer in the second, and a capture of the last
    /// window alone lost the tool calls for good.
    #[test]
    fn a_turn_split_by_a_nudge_keeps_the_tool_calls_of_its_first_window() {
        let dir = scratch("split");
        let lines = [
            json!({"type": "user", "uuid": "u1", "timestamp": "2026-10-17T09:00:00Z",
                   "message": {"role": "user", "content": "read the only text file and say yes or no"}}),
            json!({"type": "assistant", "uuid": "a1", "message": {"content": [
                {"type": "tool_use", "id": "t1", "name": "Read", "input": {"file_path": "/x/zq7marker.txt"}}]}}),
            json!({"type": "user", "uuid": "n1", "timestamp": "2026-10-17T09:00:30Z",
                   "message": {"role": "user", "content": "[Your previous response had no visible output. Please answer.]"}}),
            json!({"type": "assistant", "uuid": "a2", "message": {"content": [{"type": "text", "text": "Yes."}]}}),
        ];
        let path = dir.join("split.jsonl");
        std::fs::write(&path, lines.iter().map(Value::to_string).collect::<Vec<_>>().join("\n")).unwrap();
        let store = Stub::quiet();
        capture_current_in(&scratch("split-state"), None, Client::ClaudeCode,
            &payload(&path.to_string_lossy(), json!({})), &store);

        let seen = store.seen.borrow();
        assert!(seen.iter().any(|r| r["tools"].to_string().contains("zq7marker")),
            "the Read in the first window reached the engine: {seen:?}");
    }

    #[test]
    fn the_cadence_allows_one_capture_in_a_gap_and_claims_it_before_it_runs() {
        let dir = scratch("cadence");
        assert!(cadence_due_in(&dir, "s-1", 1_000, 120), "the first call is due");
        assert!(!cadence_due_in(&dir, "s-1", 1_050, 120), "50 seconds later it is not");
        assert!(cadence_due_in(&dir, "s-1", 1_121, 120), "past the gap it is due again");
        assert!(cadence_due_in(&dir, "s-2", 1_050, 120), "another session has its own clock");
    }

    // ---- after a compaction ------------------------------------------------------------------

    #[test]
    fn a_start_that_is_not_a_compaction_asks_the_engine_nothing() {
        let dir = scratch("startup");
        let path = transcript_file(&dir, "Understood.");
        let store = Stub::quiet();
        assert!(after_compaction_in(&scratch("startup-state"), None, Client::ClaudeCode, &payload(&path, json!({"source": "startup"})), &store).is_none());
        assert!(store.seen.borrow().is_empty());
    }

    #[test]
    fn after_a_compaction_every_window_is_stored_and_his_last_words_come_back() {
        let dir = scratch("compact");
        let path = transcript_file(&dir, "Understood.");
        let store = Stub {
            seen: RefCell::new(vec![]),
            verify: None,
            text: Some("[transcript] no size cap, just compression · when he said on 2026-10-17".into()),
        };
        let thread = after_compaction_in(&scratch("compact-state"), None, Client::ClaudeCode, &payload(&path, json!({"source": "compact"})), &store)
            .expect("the thread comes back");
        assert!(thread.contains("no size cap, just compression"), "{thread}");

        let seen = store.seen.borrow();
        let appends: Vec<&Value> = seen.iter().filter(|r| r["kind"] == "transcript_append").collect();
        assert_eq!(2, appends.len(), "both windows still in the file are stored");
        assert!(appends.iter().all(|r| r["capture"] == "compaction"));
        let recall = seen.iter().find(|r| r["kind"] == "recall").expect("then the thread is asked for");
        assert_eq!(json!(["transcript"]), recall["lanes"], "the lane is named, as the engine requires");
        assert_eq!(THREAD_MESSAGES, recall["recent"]);
        assert_eq!("s-1", recall["session"]);
    }

    // ---- what the first audit of C21 found ---------------------------------------------------

    /// A window whose opening lies further back than the tail a hook reads had no key, so it was
    /// skipped without a sound. Measured on this machine: one human window in twelve is that long.
    #[test]
    fn a_window_that_opened_before_the_tail_is_still_stored_under_its_own_key() {
        let dir = scratch("long");
        let mut lines = vec![json!({"type": "user", "uuid": "u-long", "timestamp": "2026-10-17T09:00:00Z",
            "message": {"role": "user", "content": "run the whole migration and report"}})];
        let filler = "x".repeat(20_000);
        for i in 0..80 {
            lines.push(json!({"type": "assistant", "uuid": format!("a{i}"), "message": {"content": [
                {"type": "tool_use", "id": format!("t{i}"), "name": "Bash", "input": {"command": format!("step {i} {filler}")}}]}}));
        }
        lines.push(json!({"type": "assistant", "uuid": "a-final", "message": {"content": [
            {"type": "text", "text": "Migration finished."}]}}));
        let path = dir.join("long.jsonl");
        std::fs::write(&path, lines.iter().map(Value::to_string).collect::<Vec<_>>().join("\n")).unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > TRANSCRIPT_TAIL_BYTES, "the window is longer than the tail");

        let store = Stub::quiet();
        capture_current_in(&scratch("long-state"), None, Client::ClaudeCode,
            &payload(&path.to_string_lossy(), json!({})), &store);
        let seen = store.seen.borrow();
        let r = seen.iter().find(|r| r["key"] == "u-long")
            .unwrap_or_else(|| panic!("the long window was stored under the record that opened it: {seen:?}"));
        assert_eq!("run the whole migration and report", r["his"]);
        assert_eq!("Migration finished.", r["agent"]);
    }

    /// The client writes its own marker when he presses the interrupt key. It opens a window; it is
    /// not something he typed, and the thread handed back after a compaction must not be made of it.
    #[test]
    fn the_interrupt_marker_opens_a_window_and_is_not_his_words() {
        let dir = scratch("interrupt");
        let lines = [
            json!({"type": "user", "uuid": "u1", "timestamp": "2026-10-17T09:00:00Z",
                   "message": {"role": "user", "content": "start the long job please"}}),
            json!({"type": "assistant", "uuid": "a1", "message": {"content": [{"type": "text", "text": "Starting."}]}}),
            json!({"type": "user", "uuid": "i1", "timestamp": "2026-10-17T09:01:00Z",
                   "message": {"role": "user", "content": [{"type": "text", "text": "[Request interrupted by user]"}]}}),
            json!({"type": "assistant", "uuid": "a2", "message": {"content": [{"type": "text", "text": "Stopped."}]}}),
        ];
        let path = dir.join("interrupt.jsonl");
        std::fs::write(&path, lines.iter().map(Value::to_string).collect::<Vec<_>>().join("\n")).unwrap();
        let store = Stub::quiet();
        capture_current_in(&scratch("interrupt-state"), None, Client::ClaudeCode,
            &payload(&path.to_string_lossy(), json!({})), &store);
        let seen = store.seen.borrow();
        assert!(seen.iter().all(|r| !r["his"].to_string().contains("interrupted")),
            "the marker is never stored as his words: {seen:?}");
        assert!(seen.iter().any(|r| r["key"] == "u1" && r["his"] == "start the long job please"));
    }

    /// A record job that runs out of time stops, leaves the stamp at the last window it stored, and
    /// the next capture carries on from there: nothing is skipped and nothing waits past the answer.
    #[test]
    fn a_capture_that_runs_out_of_time_stops_and_the_next_one_resumes() {
        struct Slow(RefCell<Vec<String>>);
        impl Store for Slow {
            fn ask(&self, _: Value) -> Result<Answer, QueryError> {
                Ok(Answer::Nothing)
            }
            fn ask_value(&self, a: Value) -> Result<Value, QueryError> {
                std::thread::sleep(std::time::Duration::from_millis(80));
                self.0.borrow_mut().push(a["key"].as_str().unwrap_or("").to_string());
                Ok(json!({"appended": "stored"}))
            }
        }
        let dir = scratch("slow");
        let mut lines = vec![];
        for i in 0..6 {
            lines.push(json!({"type": "user", "uuid": format!("u{i}"), "timestamp": "2026-10-17T09:00:00Z",
                "message": {"role": "user", "content": format!("message number {i} of the session")}}));
            lines.push(json!({"type": "assistant", "uuid": format!("a{i}"), "message": {"content": [
                {"type": "text", "text": format!("answer {i}")}]}}));
        }
        let path = dir.join("slow.jsonl");
        std::fs::write(&path, lines.iter().map(Value::to_string).collect::<Vec<_>>().join("\n")).unwrap();
        let state = scratch("slow-state");
        let how = Capture {
            kind: "turn",
            without_stamp: 8,
            until: std::time::Instant::now() + std::time::Duration::from_millis(400),
            reach: REACH_LIMIT_BYTES,
        };
        let p = payload(&path.to_string_lossy(), json!({}));

        let first = Slow(RefCell::new(vec![]));
        let n1 = store_unsaved(&state, None, Client::ClaudeCode, &p, &first, &how);
        assert!(n1 >= 1 && n1 < 6, "stopped before the end: {n1}");
        let second = Slow(RefCell::new(vec![]));
        let how2 = Capture {
            kind: "turn",
            without_stamp: 8,
            until: std::time::Instant::now() + std::time::Duration::from_secs(5),
            reach: REACH_LIMIT_BYTES,
        };
        store_unsaved(&state, None, Client::ClaudeCode, &p, &second, &how2);
        let mut all: Vec<String> = first.0.borrow().clone();
        all.extend(second.0.borrow().iter().cloned());
        for i in 0..6 {
            assert!(all.contains(&format!("u{i}")), "window u{i} was stored by one run or the other: {all:?}");
        }
    }

    /// The catch-up after a compaction sends what is missing, not the whole history every time.
    #[test]
    fn a_compaction_catch_up_resumes_from_the_stamp_the_other_captures_keep() {
        let dir = scratch("resume");
        let path = transcript_file(&dir, "Understood.");
        let state = scratch("resume-state");
        let store = Stub::quiet();
        capture_current_in(&state, None, Client::ClaudeCode, &payload(&path, json!({})), &store);
        store.seen.borrow_mut().clear();
        after_compaction_in(&state, None, Client::ClaudeCode, &payload(&path, json!({"source": "compact"})), &store);
        let seen = store.seen.borrow();
        let appends: Vec<&Value> = seen.iter().filter(|r| r["kind"] == "transcript_append").collect();
        assert_eq!(1, appends.len(), "only the window the stamp names, which may have grown: {seen:?}");
        assert_eq!("u2", appends[0]["key"]);
    }

    // ---- the second audit: one clock for every optional job -----------------------------------

    /// A store that remembers how long it was told it could take.
    struct Limited(RefCell<Vec<std::time::Duration>>);
    impl Store for Limited {
        fn ask(&self, _: Value) -> Result<Answer, QueryError> {
            Ok(Answer::Nothing)
        }
        fn ask_value(&self, _: Value) -> Result<Value, QueryError> {
            Ok(json!({"appended": "stored", "checked": true, "found": true}))
        }
        fn ask_value_within(&self, a: Value, within: std::time::Duration) -> Result<Value, QueryError> {
            self.0.borrow_mut().push(within);
            self.ask_value(a)
        }
    }

    #[test]
    fn an_ask_is_never_given_more_time_than_is_left_before_the_deadline() {
        let dir = scratch("within");
        let path = transcript_file(&dir, "Understood.");
        let store = Limited(RefCell::new(vec![]));
        let how = Capture {
            kind: "turn",
            without_stamp: 8,
            until: std::time::Instant::now() + std::time::Duration::from_millis(600),
            reach: REACH_LIMIT_BYTES,
        };
        store_unsaved(&scratch("within-state"), None, Client::ClaudeCode, &payload(&path, json!({})), &store, &how);
        let told = store.0.borrow();
        assert!(!told.is_empty(), "the windows were stored through the limited ask");
        assert!(told.iter().all(|d| *d <= std::time::Duration::from_millis(600)),
            "no ask is told it may take longer than what was left: {told:?}");
    }

    #[test]
    fn the_check_of_attributions_stops_at_its_deadline_and_asks_no_more() {
        struct Slow(RefCell<usize>);
        impl Store for Slow {
            fn ask(&self, _: Value) -> Result<Answer, QueryError> {
                Ok(Answer::Nothing)
            }
            fn ask_value(&self, _: Value) -> Result<Value, QueryError> {
                std::thread::sleep(std::time::Duration::from_millis(120));
                *self.0.borrow_mut() += 1;
                Ok(json!({"checked": true, "found": true}))
            }
        }
        let dir = scratch("checkdeadline");
        let quotes = (1..=6)
            .map(|i| format!("He said \"quote number {i} about the lighthouse ledger rule\" there."))
            .collect::<Vec<_>>()
            .join("\n");
        let path = transcript_file(&dir, &quotes);
        let store = Slow(RefCell::new(0));
        let until = std::time::Instant::now() + std::time::Duration::from_millis(400);
        name_back_in(&scratch("checkdeadline-state"), None, Client::ClaudeCode,
            &payload(&path, json!({"stop_hook_active": false})), &store, until);
        let asked = *store.0.borrow();
        assert!(asked >= 1 && asked < 5, "the check stopped before it had asked all five: {asked}");
    }

    /// A window longer than anything the read will go back for cannot be stored; saying nothing
    /// there reads as having nothing to store.
    #[test]
    fn a_window_beyond_the_furthest_read_is_said_to_be_out_of_reach() {
        let dir = scratch("outofreach");
        let mut lines = vec![json!({"type": "user", "uuid": "u-far", "timestamp": "2026-10-17T09:00:00Z",
            "message": {"role": "user", "content": "start the very long job"}})];
        let filler = "y".repeat(20_000);
        for i in 0..70 {
            lines.push(json!({"type": "assistant", "uuid": format!("a{i}"), "message": {"content": [
                {"type": "tool_use", "id": format!("t{i}"), "name": "Bash", "input": {"command": format!("step {i} {filler}")}}]}}));
        }
        let path = dir.join("far.jsonl");
        std::fs::write(&path, lines.iter().map(Value::to_string).collect::<Vec<_>>().join("\n")).unwrap();
        let signals = scratch("outofreach-signals");
        let store = Stub::quiet();
        let how = Capture { kind: "turn", without_stamp: 8, until: soon(), reach: TRANSCRIPT_TAIL_BYTES };
        store_unsaved(&scratch("outofreach-state"), Some(&signals), Client::ClaudeCode,
            &payload(&path.to_string_lossy(), json!({})), &store, &how);
        let log = std::fs::read_to_string(signals.join("outcomes.log")).unwrap_or_default();
        assert!(log.contains("transcript-window-out-of-reach"), "the loss is recorded: {log}");
    }

    #[test]
    fn a_stamp_that_has_scrolled_out_of_what_was_read_is_said_so() {
        let dir = scratch("stampgone");
        let path = transcript_file(&dir, "Understood.");
        let state = scratch("stampgone-state");
        std::fs::write(stamp_file(&state, "s-1"), "a-record-no-longer-in-the-file").unwrap();
        let signals = scratch("stampgone-signals");
        let how = Capture { kind: "turn", without_stamp: 8, until: soon(), reach: REACH_LIMIT_BYTES };
        store_unsaved(&state, Some(&signals), Client::ClaudeCode, &payload(&path, json!({})), &Stub::quiet(), &how);
        let log = std::fs::read_to_string(signals.join("outcomes.log")).unwrap_or_default();
        assert!(log.contains("transcript-stamp-out-of-reach"), "recorded: {log}");
    }

    // ---- before something is attributed to him ---------------------------------------------

    #[test]
    fn only_a_sentence_in_quotation_marks_beside_who_said_it_is_an_attribution() {
        let said = "He said \"we need the complete transcript in the store\" and then moved on.";
        assert_eq!(vec!["we need the complete transcript in the store".to_string()], attributed_quotes(said));

        assert!(attributed_quotes("The doc calls it \"the complete transcript lane\" throughout.").is_empty(),
            "a quotation with nobody said to have spoken it is not an attribution");
        assert!(attributed_quotes("He said \"no cap\" there.").is_empty(), "a phrase is not a sentence");

        let block = "In his words:\n> \"No size cap. Just compression. Keep it local.\"\nNext.";
        assert_eq!(vec!["No size cap. Just compression. Keep it local.".to_string()], attributed_quotes(block));
    }

    #[test]
    fn words_he_never_said_are_named_back_once() {
        let dir = scratch("nameback");
        let path = transcript_file(&dir, "He said \"we should rewrite everything in Rust tonight\" earlier.");
        let state = scratch("nameback-state");
        let store = Stub {
            seen: RefCell::new(vec![]),
            verify: Some(json!({"checked": true, "found": false})),
            text: None,
        };
        let first = name_back_in(&state, None, Client::ClaudeCode, &payload(&path, json!({"stop_hook_active": false})), &store, soon());
        match first {
            Some(crate::safety::Outcome::Emitted(s)) => {
                assert!(s.contains("block"), "{s}");
                assert!(s.contains("rewrite everything in Rust"), "the sentence is named: {s}");
            }
            other => panic!("expected a block naming the quote: {other:?}"),
        }
        let again = name_back_in(&state, None, Client::ClaudeCode, &payload(&path, json!({"stop_hook_active": false})), &store, soon());
        assert!(again.is_none(), "named once; the same sentence never holds the stop twice: {again:?}");
    }

    #[test]
    fn words_he_did_say_a_lane_that_could_not_check_and_a_retry_are_never_named() {
        let dir = scratch("nameback-quiet");
        let path = transcript_file(&dir, "He said \"we should rewrite everything in Rust tonight\" earlier.");
        let p = payload(&path, json!({"stop_hook_active": false}));

        let found = Stub { seen: RefCell::new(vec![]), verify: Some(json!({"checked": true, "found": true})), text: None };
        assert!(name_back_in(&scratch("q1"), None, Client::ClaudeCode, &p, &found, soon()).is_none(), "he did say it");

        let cannot = Stub { seen: RefCell::new(vec![]), verify: Some(json!({"checked": false, "found": false})), text: None };
        assert!(name_back_in(&scratch("q2"), None, Client::ClaudeCode, &p, &cannot, soon()).is_none(),
            "an off lane did not check, and not found would be a lie");

        let unfound = Stub { seen: RefCell::new(vec![]), verify: Some(json!({"checked": true, "found": false})), text: None };
        let retry = payload(&path, json!({"stop_hook_active": true}));
        assert!(name_back_in(&scratch("q3"), None, Client::ClaudeCode, &retry, &unfound, soon()).is_none(), "a retry is never held again");
        assert!(unfound.seen.borrow().is_empty(), "and the engine is not even asked");
    }
}
