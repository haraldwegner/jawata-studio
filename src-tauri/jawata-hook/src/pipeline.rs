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
        Role::Observer | Role::Stop => Outcome::Silent(SilenceReason::CannotInject),
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
    let prompt = match extract_prompt(payload) {
        Ok(p) => p,
        Err(reason) => return Outcome::Silent(reason),
    };
    let cues = match crate::cue::extract(&prompt) {
        Ok(c) => c,
        Err(skip) => return Outcome::Silent(SilenceReason::NoCues(format!("{skip:?}"))),
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
fn extract_prompt(payload: &str) -> Result<String, SilenceReason> {
    if payload.trim().is_empty() {
        return Err(SilenceReason::PayloadUnreadable("the event payload was empty".into()));
    }
    let value: serde_json::Value = serde_json::from_str(payload)
        .map_err(|e| SilenceReason::PayloadUnreadable(format!("payload is not JSON: {e}")))?;
    for path in [
        &["prompt"][..],
        &["tool_input", "file_path"][..],
        &["tool_input", "command"][..],
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
                if !s.trim().is_empty() {
                    return Ok(s.to_string());
                }
            }
        }
    }
    Err(SilenceReason::PayloadUnreadable(
        "the payload carried no `prompt` and no recognised tool input — the event shape moved"
            .into(),
    ))
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
}
