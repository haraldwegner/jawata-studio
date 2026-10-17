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

/// Spent on checking attributions before the stop gate answers regardless.
const CHECK_BUDGET: std::time::Duration = std::time::Duration::from_millis(1500);

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

/// Store one window. `None` for a window whose opening is not in the tail read, which has no id to
/// complete it by; `Some(Err)` when the engine could not take it.
fn append_window(
    store: &dyn Store,
    session: &str,
    client: Client,
    kind: &str,
    window: &WindowCapture,
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
    Some(match store.ask_value(request) {
        Ok(v) => Ok(v.get("appended").and_then(Value::as_str).unwrap_or("?").to_string()),
        Err(e) => Err(format!("{e:?}")),
    })
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
/// to the end; with no stamp, or one that has scrolled out of the tail, the last few.
fn unsaved<'a>(turn: &'a stop::Turn, last: Option<&str>) -> Vec<&'a WindowCapture> {
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
    let from = all.len().saturating_sub(WINDOWS_WITHOUT_A_STAMP);
    all[from..].to_vec()
}

fn capture_current_in(
    state: &Path,
    signals: Option<&Path>,
    client: Client,
    payload: &str,
    store: &dyn Store,
) {
    let Some((session, text)) = transcript_of(payload) else { return };
    let Ok(turn) = stop::read_turn(&text) else { return };
    let last = std::fs::read_to_string(stamp_file(state, &session)).ok();
    let mut resume_at: Option<String> = None;
    for window in unsaved(&turn, last.as_deref().map(str::trim)) {
        match append_window(store, &session, client, "turn", window) {
            Some(Err(e)) => {
                // Not advanced past a window the engine could not take: the next capture retries it.
                record(signals, "transcript-capture-failed", &format!("turn|{e}"));
                return;
            }
            Some(Ok(outcome)) => {
                if outcome == "disabled" {
                    record(signals, "transcript-capture-off", "the lane is switched off on this deployment");
                    return;
                }
                resume_at = window.key.clone();
            }
            None => {}
        }
    }
    if let Some(key) = resume_at {
        let _ = std::fs::create_dir_all(state);
        let _ = std::fs::write(stamp_file(state, &session), key);
    }
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
/// every window the transcript still holds and hand back the thread he was following.
///
/// The transcript file keeps what a compaction removes from the model's context, so the catch-up
/// reads it from disk after the fact rather than needing a hook that fires before.
///
/// `None` for any other kind of start, and the engine is not asked.
pub fn after_compaction(client: Client, payload: &str, store: &dyn Store) -> Option<String> {
    after_compaction_in(signal_dir().as_deref(), client, payload, store)
}

fn after_compaction_in(
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
    if let Some((_, text)) = transcript_of(payload) {
        if let Ok(turn) = stop::read_turn(&text) {
            let mut stored = 0usize;
            for window in turn.closed.iter().chain(std::iter::once(&turn.capture)) {
                match append_window(store, &session, client, "compaction", window) {
                    Some(Ok(_)) => stored += 1,
                    Some(Err(e)) => {
                        record(signals, "transcript-capture-failed", &format!("compaction|{e}"))
                    }
                    None => {}
                }
            }
            record(signals, "transcript-compaction-catchup", &format!("windows={stored}"));
        }
    }
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
    name_back_in(&state_dir()?, signal_dir().as_deref(), client, payload, store)
}

fn name_back_in(
    dir: &Path,
    signals: Option<&Path>,
    client: Client,
    payload: &str,
    store: &dyn Store,
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
    let started = std::time::Instant::now();
    let mut unfounded: Vec<String> = Vec::new();
    for quote in attributed_quotes(&turn.final_text) {
        if started.elapsed() > CHECK_BUDGET {
            break;
        }
        if already.contains(&flatten(&quote)) {
            continue;
        }
        let Ok(answer) = store.ask_value(json!({"kind": "transcript_verify", "quote": quote}))
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
        assert!(after_compaction_in(None, Client::ClaudeCode, &payload(&path, json!({"source": "startup"})), &store).is_none());
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
        let thread = after_compaction_in(None, Client::ClaudeCode, &payload(&path, json!({"source": "compact"})), &store)
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
        let first = name_back_in(&state, None, Client::ClaudeCode, &payload(&path, json!({"stop_hook_active": false})), &store);
        match first {
            Some(crate::safety::Outcome::Emitted(s)) => {
                assert!(s.contains("block"), "{s}");
                assert!(s.contains("rewrite everything in Rust"), "the sentence is named: {s}");
            }
            other => panic!("expected a block naming the quote: {other:?}"),
        }
        let again = name_back_in(&state, None, Client::ClaudeCode, &payload(&path, json!({"stop_hook_active": false})), &store);
        assert!(again.is_none(), "named once; the same sentence never holds the stop twice: {again:?}");
    }

    #[test]
    fn words_he_did_say_a_lane_that_could_not_check_and_a_retry_are_never_named() {
        let dir = scratch("nameback-quiet");
        let path = transcript_file(&dir, "He said \"we should rewrite everything in Rust tonight\" earlier.");
        let p = payload(&path, json!({"stop_hook_active": false}));

        let found = Stub { seen: RefCell::new(vec![]), verify: Some(json!({"checked": true, "found": true})), text: None };
        assert!(name_back_in(&scratch("q1"), None, Client::ClaudeCode, &p, &found).is_none(), "he did say it");

        let cannot = Stub { seen: RefCell::new(vec![]), verify: Some(json!({"checked": false, "found": false})), text: None };
        assert!(name_back_in(&scratch("q2"), None, Client::ClaudeCode, &p, &cannot).is_none(),
            "an off lane did not check, and not found would be a lie");

        let unfound = Stub { seen: RefCell::new(vec![]), verify: Some(json!({"checked": true, "found": false})), text: None };
        let retry = payload(&path, json!({"stop_hook_active": true}));
        assert!(name_back_in(&scratch("q3"), None, Client::ClaudeCode, &retry, &unfound).is_none(), "a retry is never held again");
        assert!(unfound.seen.borrow().is_empty(), "and the engine is not even asked");
    }
}
