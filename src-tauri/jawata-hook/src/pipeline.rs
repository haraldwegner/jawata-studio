//! The pipeline: role → cue → query → emit, driven by the table.
//!
//! One function, taking everything it needs as arguments, so the whole flow is
//! testable without a client, a resident, or a process exit. `main` is then a
//! thin shell: arm the watchdog, gather the real inputs, call this, exit.
//!
//! Every branch that ends without emitting returns a
//! [`SilenceReason`](crate::safety::SilenceReason). That is the invariant
//! Stage 8's log depends on, and the reason the return type is not an
//! `Option`.

use crate::config::HookConfig;
use crate::emit::{self, Emission};
use crate::query::{self, Answer, Endpoint, QueryError};
use crate::roles::{Availability, Client, Role};
use crate::safety::{Outcome, SilenceReason};

/// How the store is reached. Injected so the pipeline can be driven with a
/// stub — a hook whose only test needs a live JVM is a hook nobody tests.
pub trait Store {
    fn ask(&self, arguments: serde_json::Value) -> Result<Answer, QueryError>;
}

/// The real one.
pub struct LiveStore(pub Endpoint);

impl Store for LiveStore {
    fn ask(&self, arguments: serde_json::Value) -> Result<Answer, QueryError> {
        query::ask(&self.0, arguments)
    }
}

/// Run one hook invocation.
pub fn run(role: Role, config: &HookConfig, payload: &str, store: &dyn Store) -> Outcome {
    let client = match config.client() {
        Ok(c) => c,
        Err(reason) => return Outcome::Silent(reason),
    };
    let Some(spec) = crate::roles::spec(role, client) else {
        return Outcome::Silent(SilenceReason::RoleAbsentOnClient);
    };
    if matches!(spec.availability, Availability::Absent { .. }) {
        return Outcome::Silent(SilenceReason::RoleAbsentOnClient);
    }

    match role {
        // The guard decides locally and never asks: it must answer while the
        // resident is down, and a guard that asked and failed open would leak
        // exactly the calls it exists to deny.
        Role::Guard => guard(client, payload),
        Role::Primer => primer(client, store),
        Role::UserPrompt | Role::ToolRecall => recall(role, client, payload, store),
        Role::Stop => stop_gate(client, payload, crate::stop::Autonomy::Unknown),
        Role::Observer => Outcome::Silent(SilenceReason::CannotInject),
    }
}

/// The guard. Reads the command out of the payload and answers locally.
///
/// A payload it cannot read resolves to ALLOW, never to silence: on Cursor
/// this hook runs under `failClosed: true`, so emitting nothing is itself a
/// block on the user's command. "I could not tell" must therefore be an
/// explicit allow, which is the opposite of the default everywhere else in
/// this binary — and is why it is written down here.
fn guard(client: Client, payload: &str) -> Outcome {
    // THE EDIT HALF, FIRST. Sprint 28's binary read only a shell command and
    // never looked at which tool fired, so a front-door `Edit` of a `.java`
    // file went through unblocked — caught by the 3.7.3 dogfood, and the reason
    // this role was reverted to its script for months. A guard that enforces
    // half its contract is the failure this whole sprint exists to end.
    if let Some(tool) = tool_name_in(payload) {
        // A Bash command declaring `jawata-author:` OPENS the window rather
        // than being judged by it — authoring new code is not a refactor, and
        // the declaration is the audit trail.
        if tool.eq_ignore_ascii_case("Bash") && payload.contains(crate::guard::AUTHOR_DECLARATION) {
            if let (Some(home), Some(session)) = (home_dir(), session_id_in(payload)) {
                let reason = after_marker(payload, crate::guard::AUTHOR_DECLARATION);
                crate::editgate::open_window(&home, &session, &reason);
            }
            return emit_permission(client, true, String::new());
        }
        if let Some(path) = edit_path_in(payload) {
            let window_open = match (home_dir(), session_id_in(payload)) {
                (Some(home), Some(session)) => crate::editgate::window_is_open(&home, &session),
                _ => false,
            };
            let exists = std::path::Path::new(&path).exists();
            match crate::editgate::judge_edit(&tool, &path, payload, window_open, exists) {
                crate::editgate::EditVerdict::Denied(reason) => {
                    return emit_permission(client, false, reason)
                }
                crate::editgate::EditVerdict::Allowed(_) => {
                    return emit_permission(client, true, String::new())
                }
                crate::editgate::EditVerdict::NotApplicable => {}
            }
        }
    }
    let command = command_in(payload).unwrap_or_default();
    let emission = match crate::guard::judge(&command) {
        crate::guard::Verdict::Allow => Emission::Permission {
            allowed: true,
            reason: String::new(),
        },
        crate::guard::Verdict::Deny { reason } => Emission::Permission { allowed: false, reason },
    };
    match emit::render(client, &emission) {
        Some(rendered) => Outcome::Emitted(rendered),
        None => Outcome::Silent(SilenceReason::CannotInject),
    }
}

/// Render one permission decision. Shared by the edit half and the shell half
/// so both speak the client's dialect through exactly one code path.
fn emit_permission(client: Client, allowed: bool, reason: String) -> Outcome {
    match emit::render(client, &Emission::Permission { allowed, reason }) {
        Some(rendered) => Outcome::Emitted(rendered),
        None => Outcome::Silent(SilenceReason::CannotInject),
    }
}

/// Which tool fired, from either client's payload shape.
fn tool_name_in(payload: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    for key in ["tool_name", "toolName", "tool"] {
        if let Some(s) = value.get(key).and_then(|v| v.as_str()) {
            return Some(s.to_string());
        }
    }
    None
}

/// The file an editing tool is about to write.
fn edit_path_in(payload: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    for path in [
        &["tool_input", "file_path"][..], // Claude Code, Edit/Write/MultiEdit
        &["tool_input", "path"][..],
        &["file_path"][..], // Cursor
        &["path"][..],
    ] {
        let mut cursor = &value;
        let mut found = true;
        for key in path {
            match cursor.get(key) {
                Some(next) => cursor = next,
                None => {
                    found = false;
                    break;
                }
            }
        }
        if found {
            if let Some(s) = cursor.as_str() {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// The session this call belongs to — the authoring window's scope.
fn session_id_in(payload: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    for key in ["session_id", "sessionId", "conversation_id"] {
        if let Some(s) = value.get(key).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// The user's home, for the authoring window's directory.
///
/// `USERPROFILE` is checked too: on Windows `HOME` is often unset, and this
/// role now runs there natively rather than through a shell that would have
/// supplied it.
fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
}

/// The text following a declaration marker, trimmed and capped.
///
/// Capped because it lands in a file and in the audit trail: an unbounded
/// reason from a payload is an unbounded write.
fn after_marker(payload: &str, marker: &str) -> String {
    payload
        .find(marker)
        .map(|i| &payload[i + marker.len()..])
        .map(|rest| {
            rest.split(['"', '\\', '\n'])
                .next()
                .unwrap_or("")
                .trim()
                .chars()
                .take(200)
                .collect::<String>()
        })
        .unwrap_or_default()
}

/// The shell command, from either client's payload shape.
fn command_in(payload: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    for path in [
        &["tool_input", "command"][..],   // Claude Code, PreToolUse/Bash
        &["command"][..],                 // Cursor, beforeShellExecution
    ] {
        let mut cursor = &value;
        let mut found = true;
        for key in path {
            match cursor.get(key) {
                Some(next) => cursor = next,
                None => {
                    found = false;
                    break;
                }
            }
        }
        if found {
            if let Some(s) = cursor.as_str() {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn primer(client: Client, store: &dyn Store) -> Outcome {
    let answer = store.ask(serde_json::json!({
        "kind": "primer", "format": "text", "limit": 12
    }));
    finish(client, Role::Primer, answer, "JAWATA domain primer (what this codebase is about):")
}

fn recall(role: Role, client: Client, payload: &str, store: &dyn Store) -> Outcome {
    let cues = match cues_for(role, payload) {
        Ok(c) => c,
        Err(reason) => return Outcome::Silent(reason),
    };

    // Symbol cues first — they are precise, and they fire independently of the
    // two-token gate. Then symptoms. The FIRST answer wins; an absence falls
    // through to the next cue, which is why an observed absence must be
    // distinguishable from a failure here.
    let mut last_failure: Option<QueryError> = None;
    for (key, cue) in cues
        .symbols
        .iter()
        .map(|c| ("symbol", c))
        .chain(cues.symptoms.iter().map(|c| ("symptom", c)))
    {
        match store.ask(serde_json::json!({ "kind": "recall", "format": "text", key: cue })) {
            Ok(Answer::Text(text)) => {
                return finish(
                    client,
                    role,
                    Ok(Answer::Text(text)),
                    "JAWATA recalled candidate prior knowledge for this topic — these are \
                     NOMINEES, not vouched answers; judge whether each fits before relying on it:",
                )
            }
            Ok(Answer::Nothing) => continue,
            Err(e) => {
                // Remember it, keep trying: one unreachable attempt should not
                // discard the cues we have not tried. But do NOT let the run
                // end reporting "the store had nothing" when it never answered.
                last_failure = Some(e);
            }
        }
    }
    match last_failure {
        Some(e) => Outcome::Silent(SilenceReason::QueryFailed(format!("{e:?}"))),
        None => Outcome::Silent(SilenceReason::StoreHadNothing),
    }
}

fn finish(
    client: Client,
    role: Role,
    answer: Result<Answer, QueryError>,
    heading: &str,
) -> Outcome {
    match answer {
        Ok(Answer::Text(text)) => {
            let body = format!("{heading}\n{text}");
            match emit::context_for(role, client, body) {
                Emission::Silent => Outcome::Silent(SilenceReason::CannotInject),
                other => match emit::render(client, &other) {
                    Some(rendered) => Outcome::Emitted(rendered),
                    None => Outcome::Silent(SilenceReason::CannotInject),
                },
            }
        }
        Ok(Answer::Nothing) => Outcome::Silent(SilenceReason::StoreHadNothing),
        Err(e) => Outcome::Silent(SilenceReason::QueryFailed(format!("{e:?}"))),
    }
}

/// Pull the prompt out of the client's event payload.
///
/// Both clients put it under `prompt`; Claude's `PreToolUse` payload instead
/// carries a tool input. `serde_json`, never a regex — a payload whose shape
/// moved must be a named failure, not an empty prompt.
/// Cues for a recall, derived PER ROLE — because the two roles receive
/// different kinds of text and the difference is load-bearing.
///
/// A UserPrompt payload is TYPED text: the slash-command rule applies. A
/// ToolRecall payload is a tool target — a symbol, a file path, a command —
/// and applying the typed rules to it is the 3.7.2 dogfood bug F2: every
/// absolute path begins with `/`, so every Read/Edit recall was skipped as a
/// "slash command" and the role went silent on Linux entirely.
fn cues_for(role: Role, payload: &str) -> Result<crate::cue::Cues, SilenceReason> {
    let value = parse_payload(payload)?;
    match role {
        Role::UserPrompt => {
            let prompt = string_at(&value, &["prompt"]).ok_or_else(|| {
                SilenceReason::PayloadUnreadable(
                    "the payload carried no `prompt` — the event shape moved".into(),
                )
            })?;
            crate::cue::extract(&prompt)
                .map_err(|skip| SilenceReason::NoCues(format!("{skip:?}")))
        }
        _ => tool_cues(&value),
    }
}

/// Cues for a TOOL event, in the script generation's own priority order
/// (Sprint 21a dogfood: the subject identifiers win — a rename carrying
/// `symbol` + `newName` must query the OLD name, so `newName` is last):
/// the refactor-subject keys, then the edited file's type name
/// (`Foo.java` → `Foo`), then the raw strings through the untyped extractor.
fn tool_cues(value: &serde_json::Value) -> Result<crate::cue::Cues, SilenceReason> {
    for key in ["typeName", "symbol", "query", "newName"] {
        if let Some(sym) = string_at(value, &["tool_input", key]) {
            return Ok(crate::cue::Cues {
                symbols: vec![sym],
                symptoms: Vec::new(),
                content_tokens: 0,
            });
        }
    }
    if let Some(path) = string_at(value, &["tool_input", "file_path"]) {
        if let Some(sym) = crate::cue::symbol_from_path(&path) {
            return Ok(crate::cue::Cues {
                symbols: vec![sym],
                symptoms: Vec::new(),
                content_tokens: 0,
            });
        }
        return crate::cue::extract_tool_target(&path)
            .map_err(|skip| SilenceReason::NoCues(format!("{skip:?}")));
    }
    if let Some(cmd) = string_at(value, &["tool_input", "command"]) {
        return crate::cue::extract_tool_target(&cmd)
            .map_err(|skip| SilenceReason::NoCues(format!("{skip:?}")));
    }
    Err(SilenceReason::PayloadUnreadable(
        "the payload carried no recognised tool input — the event shape moved".into(),
    ))
}

fn parse_payload(payload: &str) -> Result<serde_json::Value, SilenceReason> {
    if payload.trim().is_empty() {
        return Err(SilenceReason::PayloadUnreadable("the event payload was empty".into()));
    }
    serde_json::from_str(payload)
        .map_err(|e| SilenceReason::PayloadUnreadable(format!("payload is not JSON: {e}")))
}

/// The non-empty string at a key path, or `None` — an empty string is an
/// absence, not a cue.
fn string_at(value: &serde_json::Value, path: &[&str]) -> Option<String> {
    let mut cursor = value;
    for key in path {
        cursor = cursor.get(key)?;
    }
    cursor
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
}


/// The stop gate. Reads the transcript the HARNESS wrote — never a marker the
/// agent writes, because skipping such a write would be passing the gate.
///
/// Fails OPEN on every unreadable condition (no payload, no path, no file),
/// but RECORDS which one: the previous generation of this hook failed open
/// silently, and a silent fail-open is indistinguishable from a pass.
fn stop_gate(client: Client, payload: &str, autonomy: crate::stop::Autonomy) -> Outcome {
    use crate::stop::{self, StopFacts, StopVerdict};

    let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) else {
        return Outcome::Silent(SilenceReason::PayloadUnreadable(
            "stop payload is not JSON".to_string(),
        ));
    };
    let already_bounced = v
        .get("stop_hook_active")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let Some(path) = v.get("transcript_path").and_then(|p| p.as_str()) else {
        return Outcome::Silent(SilenceReason::NoTranscript);
    };
    // BOUNDED, and from the END. Reading the whole file was measured at 4,983
    // ms of parsing on a 330 MB session transcript — past the 4,000 ms
    // watchdog, which exits the process from its own thread BEFORE the silence
    // log is written. The gate then stayed silent and recorded nothing: the
    // exact two-week-outage signature this stage exists to end, reproduced by
    // the stage's own code.
    //
    // Only the window since the last human message is needed, so the tail is
    // sufficient — and the read is now O(1) in session length rather than
    // O(history).
    let Ok(text) = read_tail(path, TRANSCRIPT_TAIL_BYTES) else {
        return Outcome::Silent(SilenceReason::NoTranscript);
    };
    let turn = match stop::read_turn(&text) {
        Ok(t) => t,
        Err(reason) => return Outcome::Silent(reason),
    };

    // RULE B's honest position: production passes `Unknown` from `run`,
    // because nothing readable here says whether the human granted autonomy.
    // It is a PARAMETER rather than a constant so the blocking paths — and the
    // anti-loop wire that guards them — are reachable from a test. Hard-coding
    // it made `stop_hook_active` hollow: seeding that read to either constant
    // left the whole suite green, so the anti-wedge valve could be deleted and
    // nothing would notice.

    match stop::judge(&StopFacts { already_bounced, turn, autonomy }) {
        StopVerdict::Block { reason } => {
            match crate::emit::render(client, &crate::emit::Emission::StopDecision { reason }) {
                Some(rendered) => Outcome::Emitted(rendered),
                None => Outcome::Silent(SilenceReason::CannotInject),
            }
        }
        // Report WHICH allow this was. Logging `autonomy-unknown` for every
        // pass was fine only while production could not observe autonomy; the
        // moment Studio supplies it, every judged autonomous stop would file a
        // false reason.
        StopVerdict::Allow => Outcome::Silent(match autonomy {
            crate::stop::Autonomy::Unknown => SilenceReason::AutonomyUnknown,
            _ => SilenceReason::StopAllowed,
        }),
    }
}

/// How much of a transcript's tail the stop gate reads. Generous enough to
/// hold many turns, small enough that parsing cannot approach the watchdog.
pub const TRANSCRIPT_TAIL_BYTES: u64 = 1_048_576;

/// Read at most `max` bytes from the END of a file, starting at a line
/// boundary so the first record is never half a line.
fn read_tail(path: &str, max: u64) -> std::io::Result<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    let truncated = len > max;
    if truncated {
        // Seek ONE BYTE EARLIER than the window so we can tell whether the
        // boundary already fell on a record edge. Dropping the first line
        // unconditionally destroyed a COMPLETE record whenever it did —
        // measured: a 100-byte file of ten records, window 50, lost the whole
        // record at the boundary and returned four lines where five were
        // readable.
        f.seek(SeekFrom::Start(len - max - 1))?;
    }
    let mut buf = Vec::with_capacity((max + 1).min(len) as usize);
    f.take(max + 1).read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    if !truncated {
        return Ok(text);
    }
    // The probe byte decides. If it is a newline the window already began at a
    // record edge and everything after it is whole.
    if let Some(rest) = text.strip_prefix('\n') {
        // Even here the window may hold no COMPLETE record: the probe byte was
        // the only newline. Returning it would let read_turn parse zero records
        // and report "this turn launched nothing" — the manufactured absence
        // again, on the branch the first fix did not cover.
        // `contains` subsumes `ends_with`; the latter was dead.
        return if rest.contains('\n') {
            Ok(rest.to_string())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "transcript tail holds no complete record",
            ))
        };
    }
    {
        // Otherwise the first line is a fragment and must go.
        return match text.find('\n') {
            Some(i) => Ok(text[i + 1..].to_string()),
            // No newline anywhere in the window means we could not read a
            // single whole record. Returning the fragment would let the caller
            // parse zero records and report "this turn launched nothing" — a
            // manufactured absence, which is the failure this crate exists to
            // end. Say we could not look.
            None => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "transcript tail holds no complete record",
            )),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Stub(Result<Answer, QueryError>);
    impl Store for Stub {
        fn ask(&self, _: serde_json::Value) -> Result<Answer, QueryError> {
            self.0.clone()
        }
    }

    fn config(client: &str) -> HookConfig {
        HookConfig {
            url: "http://127.0.0.1:1/mcp".into(),
            token: "t".into(),
            client: client.into(),
            timeout_ms: Some(50),
        }
    }

    #[test]
    fn a_recall_with_an_answer_injects_it() {
        let store = Stub(Ok(Answer::Text("[lesson] a line".into())));
        let out = run(
            Role::UserPrompt,
            &config("claude-code"),
            r#"{"prompt":"the importer classifier regression"}"#,
            &store,
        );
        match out {
            Outcome::Emitted(s) => {
                let v: serde_json::Value = serde_json::from_str(&s).unwrap();
                let ctx = v["hookSpecificOutput"]["additionalContext"].as_str().unwrap();
                assert!(ctx.contains("NOMINEES"), "the label must say what these are");
                assert!(ctx.contains("[lesson] a line"));
            }
            other => panic!("expected an emission: {other:?}"),
        }
    }

    #[test]
    fn an_unreachable_store_is_never_reported_as_an_absence() {
        // THE hazard: the resident is down. This must not end the run saying
        // "the store had nothing", which is a claim about the store.
        let store = Stub(Err(QueryError::Unreachable("connection refused".into())));
        let out = run(
            Role::UserPrompt,
            &config("claude-code"),
            r#"{"prompt":"importer classifier regression"}"#,
            &store,
        );
        match out {
            Outcome::Silent(SilenceReason::QueryFailed(why)) => {
                assert!(why.contains("Unreachable"), "{why}")
            }
            other => panic!("an unreachable store must be QueryFailed, got {other:?}"),
        }
    }

    #[test]
    fn an_observed_absence_says_so_and_is_not_a_failure() {
        let store = Stub(Ok(Answer::Nothing));
        let out = run(
            Role::UserPrompt,
            &config("claude-code"),
            r#"{"prompt":"importer classifier regression"}"#,
            &store,
        );
        assert_eq!(Outcome::Silent(SilenceReason::StoreHadNothing), out);
    }

    #[test]
    fn cursor_queries_the_prompt_hook_but_emits_nothing() {
        let store = Stub(Ok(Answer::Text("[lesson] a line".into())));
        let out = run(
            Role::UserPrompt,
            &config("cursor"),
            r#"{"prompt":"importer classifier regression"}"#,
            &store,
        );
        assert_eq!(Outcome::Silent(SilenceReason::CannotInject), out);
    }

    #[test]
    fn a_role_absent_on_the_client_says_so() {
        let store = Stub(Ok(Answer::Text("x".into())));
        assert_eq!(
            Outcome::Silent(SilenceReason::RoleAbsentOnClient),
            run(Role::Stop, &config("cursor"), "{}", &store)
        );
    }

    #[test]
    fn a_moved_payload_is_named_not_treated_as_an_empty_prompt() {
        let store = Stub(Ok(Answer::Text("x".into())));
        let out = run(
            Role::UserPrompt,
            &config("claude-code"),
            r#"{"userMessage":"the field was renamed"}"#,
            &store,
        );
        match out {
            Outcome::Silent(SilenceReason::PayloadUnreadable(why)) => {
                assert!(why.contains("shape moved"), "{why}")
            }
            other => panic!("a moved payload must name itself: {other:?}"),
        }
    }

    #[test]
    fn a_slash_command_is_skipped_with_the_cue_modules_reason() {
        let store = Stub(Ok(Answer::Text("x".into())));
        let out = run(
            Role::UserPrompt,
            &config("claude-code"),
            r#"{"prompt":"/memorize this"}"#,
            &store,
        );
        match out {
            Outcome::Silent(SilenceReason::NoCues(why)) => assert!(why.contains("SlashCommand")),
            other => panic!("expected NoCues: {other:?}"),
        }
    }

    #[test]
    fn the_whole_pipeline_survives_every_shape_without_panicking() {
        // The boundary catches panics; not needing it is better.
        let stubs = [
            Ok(Answer::Text("x".into())),
            Ok(Answer::Nothing),
            Err(QueryError::Status(503)),
            Err(QueryError::ShapeChanged("data moved".into())),
            Err(QueryError::ToolRefused { code: "X".into(), message: "y".into() }),
        ];
        let payloads = ["", "{}", "not json", r#"{"prompt":""}"#, r#"{"prompt":"a b c"}"#];
        for stub in stubs {
            for payload in payloads {
                for client in ["claude-code", "cursor", "windsurf"] {
                    for role in [Role::Primer, Role::UserPrompt, Role::Guard, Role::Stop] {
                        let out = run(role, &config(client), payload, &Stub(stub.clone()));
                        // C5 audit F7: `len() > 3` made this a panic smoke
                        // test wearing an assertion, and the Emitted arm was
                        // never checked at all. Both arms now carry a real
                        // obligation — anything we emit must be JSON the
                        // client can read, and anything silent must name a
                        // cause a log could print.
                        match out {
                            Outcome::Emitted(text) => {
                                serde_json::from_str::<serde_json::Value>(&text).unwrap_or_else(
                                    |e| panic!("emitted non-JSON for {role:?}/{client}: {e}\n{text}"),
                                );
                                assert!(!text.contains('\n'), "an emission must be one line");
                            }
                            Outcome::Silent(reason) => {
                                // C5 audit round 2, R5: "non-empty and contains
                                // an uppercase char" is true of every derived
                                // Debug on a PascalCase enum — it could not
                                // fail. The real obligation is that the reason
                                // FITS: a run that never reached the store must
                                // not claim the store had nothing, which is the
                                // specific lie this crate exists to stop.
                                if matches!(reason, SilenceReason::StoreHadNothing) {
                                    assert!(
                                        matches!(stub, Ok(Answer::Nothing)),
                                        "reported StoreHadNothing for {role:?}/{client} with \
                                         payload {payload:?} while the store answered \
                                         {stub:?} — that is a claim about the store this run \
                                         never earned"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

        use crate::stop::{Autonomy, StopFacts, StopVerdict, Turn, ToolUse};

    /// THE WIRE TEST. Seeding `Role::Stop => stop_gate(...)` back to
    /// `CannotInject` left all eleven suites green — the gate was HOLLOW, the
    /// very shape this sprint exists to catch, one hour after building the
    /// detector for it. Every assertion below goes through `run`, so the arm
    /// in the match is load-bearing.
    fn transcript(body: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("jawata-stopwire-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        let p = d.join(format!("t-{}.jsonl", body.len()));
        std::fs::write(&p, body).unwrap();
        p
    }


    #[test]
    fn stop_reaches_the_gate_through_run() {
        // A turn with no communicator and nothing armed. Autonomy is Unknown in
        // production today, so the honest outcome is the RECORDED reason — not
        // a block, and not the inherited CannotInject the hollow arm produced.
        let p = transcript(
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"done\"}]}}\n",
        );
        // serde_json, NOT format!. A Windows path is C:\Users\... and a raw
        // backslash in a JSON string is an ESCAPE — the payload parsed on Linux
        // and was invalid JSON on Windows, where these tests had never run until
        // the crate's tests became a CI gate.
        let payload = serde_json::json!({
            "transcript_path": p, "stop_hook_active": false
        }).to_string();
        let out = run(Role::Stop, &config("claude-code"), &payload, &Stub(Ok(Answer::Nothing)));
        assert_eq!(
            Outcome::Silent(SilenceReason::AutonomyUnknown),
            out,
            "Stop must reach the gate and record why it did not enforce"
        );
    }

    #[test]
    fn a_stop_payload_without_a_transcript_names_itself_through_run() {
        let out = run(Role::Stop, &config("claude-code"), "{}", &Stub(Ok(Answer::Nothing)));
        assert_eq!(Outcome::Silent(SilenceReason::NoTranscript), out);
    }

    /// The gate's blocking path renders the third dialect. Driven at the
    /// judge+emit seam because production cannot yet produce `Granted`.
    #[test]
    fn a_block_renders_claudes_stop_dialect() {
        let facts = StopFacts {
            already_bounced: false,
            turn: Turn { final_text: "summary".into(), launches: vec![], refusals_emitted: 0, asks_the_human: false, seats_invoked: vec![], gate_ran: true },
            autonomy: Autonomy::Granted,
        };
        let StopVerdict::Block { reason } = crate::stop::judge(&facts) else {
            panic!("must block");
        };
        let rendered = crate::emit::render(
            crate::roles::Client::ClaudeCode,
            &crate::emit::Emission::StopDecision { reason },
        )
        .expect("claude renders a stop decision");
        let v: serde_json::Value = serde_json::from_str(&rendered).expect("valid JSON");
        assert_eq!("block", v["decision"], "got {rendered}");
        assert!(v["reason"].as_str().unwrap().contains("communicator"));
    }

    /// Cursor has no Stop event, so the dialect must render to nothing at all
    /// rather than to an empty object a client could read as a decision.
    #[test]
    fn cursor_renders_no_stop_decision() {
        assert_eq!(
            None,
            crate::emit::render(
                crate::roles::Client::Cursor,
                &crate::emit::Emission::StopDecision { reason: "x".into() }
            )
        );
    }

    #[test]
    fn the_communicator_never_counts_as_armed_work() {
        let c = ToolUse { name: "Agent".into(), subagent: Some("communicator".into()), backgrounded: false };
        assert!(!c.arms_work(), "else the two rules cancel each other out");
    }

    /// HOLLOW-WIRE FIX. A sweep that seeded every arm of `run` found the guard
    /// arm load-bearing for NOTHING: `guard::judge` is well unit-tested, but no
    /// test drove it through the pipeline, so the arm could be deleted and all
    /// 132 tests stayed green. Production reaches it; a regression would not
    /// have been caught.
    #[test]
    fn the_guard_reaches_its_verdict_through_run() {
        let out = run(
            Role::Guard,
            &config("claude-code"),
            r#"{"tool_input":{"command":"grep -rn 'foo' src/main/java/Thing.java"}}"#,
            &Stub(Ok(Answer::Nothing)),
        );
        match out {
            Outcome::Emitted(s) => {
                let v: serde_json::Value = serde_json::from_str(&s).unwrap();
                assert_eq!("deny", v["hookSpecificOutput"]["permissionDecision"], "got {s}");
            }
            other => panic!("the guard must decide through run: {other:?}"),
        }
    }

    /// Same sweep, same hole: every existing test passed `Role::UserPrompt`,
    /// which shares an arm with `ToolRecall`, so the recall role was never
    /// itself exercised. Asserting the EVENT NAME pins the role rather than
    /// merely the shared code path.
    #[test]
    fn tool_recall_reaches_the_store_through_run() {
        let store = Stub(Ok(Answer::Text("[lesson] a line".into())));
        let out = run(
            Role::ToolRecall,
            &config("claude-code"),
            r#"{"tool_input":{"file_path":"src/main/java/com/example/Importer.java"}}"#,
            &store,
        );
        match out {
            Outcome::Emitted(s) => {
                let v: serde_json::Value = serde_json::from_str(&s).unwrap();
                assert_eq!("PreToolUse", v["hookSpecificOutput"]["hookEventName"], "got {s}");
            }
            other => panic!("expected a PreToolUse recall: {other:?}"),
        }
    }

    /// 3.7.2 dogfood F2, pinned at the run() level: an ABSOLUTE path must
    /// recall, not be skipped as a slash command. The existing test above used
    /// a relative path, which is why the bug lived through it.
    #[test]
    fn tool_recall_on_an_absolute_path_queries_the_type_symbol() {
        struct SymbolAsserting;
        impl Store for SymbolAsserting {
            fn ask(&self, args: serde_json::Value) -> Result<Answer, QueryError> {
                assert_eq!("ProjectImporter", args["symbol"], "the .java stem is the cue");
                Ok(Answer::Text("[lesson] a line".into()))
            }
        }
        let out = run(
            Role::ToolRecall,
            &config("claude-code"),
            r#"{"tool_input":{"file_path":"/home/u/org/jawata/core/ProjectImporter.java"}}"#,
            &SymbolAsserting,
        );
        assert!(matches!(out, Outcome::Emitted(_)), "an absolute path must recall: {out:?}");
    }

    /// The subject-key priority carried over from the script generation: a
    /// rename carrying `symbol` AND `newName` queries the OLD name.
    #[test]
    fn tool_recall_prefers_the_subject_key_over_new_name_and_path() {
        struct SymbolAsserting;
        impl Store for SymbolAsserting {
            fn ask(&self, args: serde_json::Value) -> Result<Answer, QueryError> {
                assert_eq!("com.example.Old#field", args["symbol"]);
                Ok(Answer::Text("[hazard] a line".into()))
            }
        }
        let out = run(
            Role::ToolRecall,
            &config("claude-code"),
            r#"{"tool_input":{"newName":"renamed","symbol":"com.example.Old#field","file_path":"/x/Y.java"}}"#,
            &SymbolAsserting,
        );
        assert!(matches!(out, Outcome::Emitted(_)), "{out:?}");
    }

    /// And the TYPED slash-command skip still holds for the prompt role —
    /// the fix must not have widened UserPrompt.
    #[test]
    fn user_prompt_still_skips_slash_commands_after_the_path_fix() {
        let out = run(
            Role::UserPrompt,
            &config("claude-code"),
            r#"{"prompt":"/sprint resume"}"#,
            &Stub(Ok(Answer::Text("x".into()))),
        );
        match out {
            Outcome::Silent(SilenceReason::NoCues(why)) => assert!(why.contains("SlashCommand")),
            other => panic!("expected NoCues(SlashCommand): {other:?}"),
        }
    }

    /// The observer emits nothing BY DESIGN — but until now nothing said so,
    /// and an arm whose body is the same as the default is indistinguishable
    /// from an arm nobody wrote.
    #[test]
    fn the_observer_stays_silent_through_run_and_says_why() {
        assert_eq!(
            Outcome::Silent(SilenceReason::CannotInject),
            run(Role::Observer, &config("claude-code"), "{}", &Stub(Ok(Answer::Nothing)))
        );
    }

    /// N1: the window boundary landing EXACTLY on a record edge used to destroy
    /// the whole record that began there — the unconditional first-line drop.
    #[test]
    fn a_window_boundary_on_a_record_edge_keeps_every_record() {
        let d = std::env::temp_dir().join(format!("jawata-tail-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        let p = d.join("edge.jsonl");
        // Ten 10-byte records; a 50-byte window falls exactly on an edge.
        let body: String = (0..10).map(|i| format!("L{i:08}\n")).collect();
        std::fs::write(&p, &body).unwrap();
        let got = read_tail(p.to_str().unwrap(), 50).expect("reads");
        assert_eq!(5, got.lines().count(), "five whole records fit the window: {got:?}");
        assert!(got.starts_with("L00000005"), "the edge record must survive: {got:?}");
    }

    /// N2: a window holding no complete record used to come back as an empty
    /// turn — a MANUFACTURED absence, read downstream as "this turn launched
    /// nothing". It must say it could not look.
    #[test]
    fn a_window_with_no_complete_record_is_an_error_not_an_empty_turn() {
        let d = std::env::temp_dir().join(format!("jawata-tail-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        let p = d.join("no-newline.jsonl");
        std::fs::write(&p, "x".repeat(500)).unwrap();
        assert!(
            read_tail(p.to_str().unwrap(), 50).is_err(),
            "an unreadable window must not become a positive 'nothing happened'"
        );
    }

    #[test]
    fn a_file_smaller_than_the_window_is_returned_whole() {
        let d = std::env::temp_dir().join(format!("jawata-tail-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        let p = d.join("small.jsonl");
        std::fs::write(&p, "a\nb\nc\n").unwrap();
        assert_eq!("a\nb\nc\n", read_tail(p.to_str().unwrap(), 4096).expect("reads"));
    }

    /// F5: the anti-loop wire. `stop_hook_active` is read from the payload and
    /// feeds `judge`'s short-circuit, but seeding that read to either constant
    /// used to leave all 140 tests green — the valve that stops the gate
    /// wedging a session could be deleted unnoticed. These two drive the same
    /// transcript through `stop_gate` under Granted, differing ONLY in the JSON
    /// key, and must disagree.
    #[test]
    fn the_anti_loop_flag_is_read_from_the_payload() {
        let d = std::env::temp_dir().join(format!("jawata-antiloop-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        let p = d.join("t.jsonl");
        std::fs::write(
            &p,
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"done\"}]}}\n",
        )
        .unwrap();

        let first = serde_json::json!({"transcript_path": p, "stop_hook_active": false}).to_string();
        let again = serde_json::json!({"transcript_path": p, "stop_hook_active": true}).to_string();

        let blocked = stop_gate(Client::ClaudeCode, &first, crate::stop::Autonomy::Granted);
        assert!(
            matches!(blocked, Outcome::Emitted(_)),
            "first pass under autonomy must block: {blocked:?}"
        );
        let allowed = stop_gate(Client::ClaudeCode, &again, crate::stop::Autonomy::Granted);
        assert!(
            !matches!(allowed, Outcome::Emitted(_)),
            "a second pass must NOT block again — that wedges the session: {allowed:?}"
        );
    }


    /// F2: the probe-is-newline branch was correct but UNFORCED — mutating its
    /// guard to `if true` left all 141 tests green. The existing manufactured-
    /// absence test uses a window with no newline at all, so it takes the other
    /// branch.
    #[test]
    fn a_window_whose_only_newline_is_the_probe_byte_is_an_error() {
        let d = std::env::temp_dir().join(format!("jawata-probe-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        let p = d.join("probe.jsonl");
        // 60 bytes: a newline exactly at the probe position, then no other.
        let mut body = "A".repeat(9);
        body.push('\n');
        body.push_str(&"B".repeat(50));
        std::fs::write(&p, &body).unwrap();
        assert!(
            read_tail(p.to_str().unwrap(), 50).is_err(),
            "a window holding no COMPLETE record must not become an empty turn"
        );
    }

    #[test]
    fn a_probe_newline_followed_by_a_complete_record_is_read() {
        let d = std::env::temp_dir().join(format!("jawata-probe-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        let p = d.join("probe-ok.jsonl");
        let body = format!("{}\n{}\n", "A".repeat(9), "B".repeat(40));
        std::fs::write(&p, &body).unwrap();
        let got = read_tail(p.to_str().unwrap(), 50).expect("reads");
        assert!(got.starts_with('B'), "the whole record must survive: {got:?}");
    }

    /// F3: the autonomy -> reason branch was unforced — reverting it to always
    /// log `autonomy-unknown` left all 141 tests green, so the moment Studio
    /// supplies real autonomy every judged stop would file a false reason.
    #[test]
    fn a_judged_allow_is_not_logged_as_autonomy_unknown() {
        let d = std::env::temp_dir().join(format!("jawata-judged-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        let p = d.join("t.jsonl");
        std::fs::write(
            &p,
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"x\"}]}}\n",
        )
        .unwrap();
        let payload = serde_json::json!({"transcript_path": p, "stop_hook_active": false}).to_string();

        assert_eq!(
            Outcome::Silent(SilenceReason::StopAllowed),
            stop_gate(Client::ClaudeCode, &payload, crate::stop::Autonomy::NotGranted),
            "a KNOWN autonomy that allows must not claim it was unknown"
        );
        assert_eq!(
            Outcome::Silent(SilenceReason::AutonomyUnknown),
            stop_gate(Client::ClaudeCode, &payload, crate::stop::Autonomy::Unknown),
            "and unknown must still say unknown"
        );
    }

    /// F4: the Observer table and the code must not drift apart again. Both
    /// rows previously declared a store query the pipeline never made, and the
    /// "test that pinned the contradiction" I claimed did not exist.
    #[test]
    fn the_observer_table_matches_what_the_pipeline_does() {
        for client in [Client::ClaudeCode, Client::Cursor] {
            if let Some(spec) = crate::roles::spec(Role::Observer, client) {
                assert!(
                    !spec.concerns.query,
                    "the Observer row claims a store query the pipeline does not make"
                );
            }
        }
        assert_eq!(
            Outcome::Silent(SilenceReason::CannotInject),
            run(Role::Observer, &config("claude-code"), "{}", &Stub(Ok(Answer::Nothing)))
        );
    }

}
