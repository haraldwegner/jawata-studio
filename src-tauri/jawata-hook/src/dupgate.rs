//! Sprint 28f Stage 8 D3 — THE DUPLICATE GATE.
//!
//! The moment this whole sprint is aimed at. An agent is about to write a method
//! that already exists somewhere in the codebase, under a different name, doing
//! the same job. Nobody is at fault: the agent cannot know, because nothing told
//! it. This gate is the thing that tells it, at the only moment the telling is
//! free — before the method is written.
//!
//! It is [`crate::recallgate`]'s shape, deliberately, so the two read alike: a
//! [`Mode`] with a documented kill switch, a [`Verdict`] every variant of which
//! can explain itself in one log line, a disposition token that takes a REASON,
//! and fail-open on an unavailable knowledge layer.
//!
//! # The one place it differs from its sibling, and the reason
//!
//! `recallgate` ships in [`Mode::Observe`] — it records what it WOULD have
//! blocked, so promotion is argued from a measured count. This one ships in
//! [`Mode::Block`], on the ruling recorded in the plan. The difference is not
//! boldness, it is what the two gates are about: an undispositioned recall costs
//! the agent a fact it did not read, and a re-derived job costs the codebase a
//! second implementation that will drift from the first and be fixed only on one
//! side. The second is the defect this product has shipped repeatedly and had to
//! close as a class each time.
//!
//! # Fail OPEN, and say which
//!
//! When the engine cannot answer — `KNOWLEDGE_UNAVAILABLE`, a dead resident, a
//! timeout — the write PROCEEDS and the reason is recorded. A gate that blocks
//! when the knowledge layer is down converts an outage into a work stoppage,
//! which is a worse failure than the one it prevents. Crucially it is recorded
//! as "could not answer" and never as "nothing like this exists": those are
//! different facts and this sprint exists because they used to print the same.

use serde_json::Value;

/// How much authority the gate has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Off entirely — the documented kill switch.
    Off,
    /// Record what would have been denied; deny nothing.
    Observe,
    /// Deny an undispositioned write that duplicates a known job. THE SHIPPING
    /// DEFAULT, per the plan's ruling.
    Block,
}

impl Mode {
    /// Read the mode from the config value.
    ///
    /// TWO DIFFERENT ABSENCES, and they must not answer the same — which is this
    /// sprint's own subject applied to its own configuration:
    ///
    /// * **Nothing configured** is not a preference, it is the shipping default,
    ///   and the ruling is that this gate ships in [`Mode::Block`].
    /// * **A word we do not understand** IS a preference, badly spelled. Reading
    ///   it as `Block` would hand someone who typed `observ` more authority than
    ///   they asked for, which is the trap [`crate::recallgate::Mode::parse`]
    ///   documents. It falls to [`Mode::Observe`]: still recorded, never denied.
    pub fn parse(configured: Option<&str>) -> Mode {
        match configured.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            None => Mode::Block,
            Some("") => Mode::Block,
            Some("off") | Some("false") | Some("disabled") => Mode::Off,
            Some("block") => Mode::Block,
            Some("observe") => Mode::Observe,
            _ => Mode::Observe,
        }
    }
}

/// What the gate concluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Not an editing tool, or not a `.java` path — not our business.
    NotAJavaEdit,
    /// The kill switch is off.
    Disabled,
    /// The agent already said why the duplicate is deliberate; the write
    /// proceeds and the reason is logged for the architect's watch.
    Dispositioned { reason: String },
    /// The draft declares no method the current file does not already have, so
    /// there is nothing to ask about.
    NoDraftMethods,
    /// The lane was asked and knows no job like this one.
    NoMatch,
    /// The engine could not answer. The write proceeds and the reason is
    /// recorded — NOT as "nothing like this exists".
    Unavailable { why: String },
    /// The draft re-derives a job the codebase already does. In [`Mode::Block`]
    /// this is the denial; in [`Mode::Observe`] it is recorded and proceeds.
    Duplicate { method: String, job: String, location: String },
}

/// The declaration that says the second implementation is DELIBERATE.
///
/// It takes a reason, and a bare token is not a disposition. A one-word bypass
/// is how a gate decays into a ritual, and here the reason is load-bearing
/// beyond this gate: the architect seat's report must carry it, so an order for
/// a second implementation that nobody justified is refused downstream too.
pub const DUPLICATE: &str = "jawata-duplicate:";

/// Did this write already say why the duplicate is deliberate?
pub fn disposition_in(payload: &str) -> Option<String> {
    let lower = payload.to_lowercase();
    let at = lower.find(DUPLICATE)?;
    let reason: String = payload[at + DUPLICATE.len()..]
        .chars()
        .take_while(|c| *c != '\n' && *c != '"')
        .collect();
    let reason = reason.trim();
    if reason.is_empty() {
        return None;
    }
    Some(reason.to_string())
}

/// The draft text a write would put on disk.
///
/// `Write` carries the whole file in `content`; `Edit` carries a FRAGMENT in
/// `new_string`; `MultiEdit` carries several. All three are forwarded as-is and
/// the engine decides what to do with them — it is the side that holds JDT and
/// can parse a working copy, and a gate that tried to assemble the resulting
/// file here would be a second, worse implementation of the engine's job.
pub fn draft_text(payload: &str) -> Option<String> {
    let value: Value = serde_json::from_str(payload).ok()?;
    if let Some(content) = value.get("content").and_then(Value::as_str) {
        return Some(content.to_string());
    }
    if let Some(new_string) = value.get("new_string").and_then(Value::as_str) {
        return Some(new_string.to_string());
    }
    if let Some(edits) = value.get("edits").and_then(Value::as_array) {
        let joined: Vec<&str> = edits
            .iter()
            .filter_map(|e| e.get("new_string").and_then(Value::as_str))
            .collect();
        if !joined.is_empty() {
            return Some(joined.join("\n"));
        }
    }
    None
}

/// THE PURE CORE: does the engine's structured answer name a job this draft
/// re-derives?
///
/// Structured, because a rendered line carries prose and no addresses. Reading a
/// match off prose would be the regex mistake this crate exists to end.
pub fn first_match(data: &Value) -> Option<(String, String, String)> {
    let matches = data.get("matches")?.as_array()?;
    for m in matches {
        let method = m.get("method").and_then(Value::as_str).unwrap_or_default();
        let job = m.get("job").and_then(Value::as_str).unwrap_or_default();
        let location = m.get("location").and_then(Value::as_str).unwrap_or_default();
        // A match with no LOCATION is not actionable: the whole point is to send
        // the reader at the code that already does this, and "something like
        // this exists somewhere" is the unhelpful half of the answer.
        if !location.is_empty() && !job.is_empty() {
            return Some((method.to_string(), job.to_string(), location.to_string()));
        }
    }
    None
}

/// Judge one write.
///
/// `ask` is a closure rather than the `Store` trait so this module stays
/// independent of the pipeline's transport, and so a test can hand it a fixture
/// without standing up an engine.
///
/// The order is the design: a DISPOSITION short-circuits before the engine is
/// asked at all. An agent that has already said why the duplicate is deliberate
/// should not pay a round trip to be told it may proceed.
pub fn judge<F>(mode: Mode, tool_name: &str, path: &str, payload: &str, ask: F) -> Verdict
where
    F: Fn(&str, &str) -> Result<Value, crate::query::QueryError>,
{
    if mode == Mode::Off {
        return Verdict::Disabled;
    }
    // The editing-tool and `.java` tests belong to the edit gate and are reused
    // rather than rewritten. A second copy of "is this a Java edit" is precisely
    // the smell the sibling detector reports.
    if !crate::editgate::is_editing_tool(tool_name) || !crate::editgate::is_java_source(path) {
        return Verdict::NotAJavaEdit;
    }
    if let Some(reason) = disposition_in(payload) {
        return Verdict::Dispositioned { reason };
    }
    let Some(draft) = draft_text(payload) else {
        return Verdict::NoDraftMethods;
    };
    match ask(path, &draft) {
        Ok(answer) => match first_match(&answer) {
            Some((method, job, location)) => Verdict::Duplicate { method, job, location },
            None => Verdict::NoMatch,
        },
        Err(e) => Verdict::Unavailable { why: format!("{e:?}") },
    }
}

/// The line the agent is shown when the gate holds a write.
pub fn steering(method: &str, job: &str, location: &str) -> String {
    format!(
        "JAWATA — this job is already done. `{method}` looks like a second implementation \
         of work the codebase already has:\n  {location} — {job}\n\nRead that first. If it \
         does what you need, call it instead of writing this. If it does NOT, say so and \
         proceed: put `{DUPLICATE} <why a second implementation is right here>` in the \
         call. A reason is required, and it is not a formality — the architect's report \
         carries it, so a second implementation nobody justified is refused there too."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn engine_answer(matches: &[(&str, &str, &str)]) -> Value {
        json!({
            "matches": matches.iter()
                .map(|(m, j, l)| json!({"method": m, "job": j, "location": l}))
                .collect::<Vec<_>>()
        })
    }

    fn write_of(text: &str) -> String {
        json!({"content": text}).to_string()
    }

    #[test]
    fn a_draft_re_deriving_a_known_job_is_denied_and_the_job_is_named() {
        let v = judge(
            Mode::Block,
            "Write",
            "/p/src/Reader.java",
            &write_of("private static Tree parse(Source s) { return null; }"),
            |_, _| {
                Ok(engine_answer(&[(
                    "parse",
                    "Parse a compilation unit with binding resolution",
                    "org.jawata.mcp.tools.shared.SourceScan#parse",
                )]))
            },
        );
        match v {
            Verdict::Duplicate { method, job, location } => {
                assert_eq!("parse", method);
                assert!(job.contains("binding resolution"), "{job}");
                assert_eq!("org.jawata.mcp.tools.shared.SourceScan#parse", location);
                // The steering must NAME the thing that already does it — a gate
                // that says "this is a duplicate" without saying of what leaves
                // the reader exactly where they started.
                let s = steering(&method, &job, &location);
                assert!(s.contains("SourceScan#parse"), "{s}");
            }
            other => panic!("expected a duplicate, got {other:?}"),
        }
    }

    #[test]
    fn a_declared_duplicate_proceeds_and_carries_its_reason() {
        let payload = json!({
            "content": "private static Tree parse(Source s) { return null; }",
            "why": "jawata-duplicate: the shared one resolves bindings and this path must not"
        })
        .to_string();
        match judge(Mode::Block, "Write", "/p/A.java", &payload, |_, _| {
            panic!("a dispositioned write must not pay a round trip to the engine")
        }) {
            Verdict::Dispositioned { reason } => {
                assert!(reason.contains("must not"), "{reason}");
            }
            other => panic!("expected a disposition, got {other:?}"),
        }
    }

    #[test]
    fn a_bare_token_with_no_reason_is_not_a_disposition() {
        // The one-word bypass this gate is shaped to refuse.
        assert_eq!(None, disposition_in("jawata-duplicate:"));
        assert_eq!(None, disposition_in("jawata-duplicate:    "));
        assert_eq!(
            Some("the two differ on the error path".to_string()),
            disposition_in("jawata-duplicate: the two differ on the error path")
        );
    }

    #[test]
    fn an_unavailable_engine_lets_the_write_through_and_says_which() {
        match judge(Mode::Block, "Write", "/p/A.java", &write_of("void x() {}"), |_, _| {
            Err(crate::query::QueryError::ToolRefused {
                code: "KNOWLEDGE_UNAVAILABLE".into(),
                message: "the store is rebuilding".into(),
            })
        }) {
            Verdict::Unavailable { why } => {
                assert!(why.contains("KNOWLEDGE_UNAVAILABLE"), "{why}");
            }
            other => panic!("an outage must not become a work stoppage, got {other:?}"),
        }
    }

    #[test]
    fn silence_from_the_lane_is_not_a_duplicate() {
        assert_eq!(
            Verdict::NoMatch,
            judge(Mode::Block, "Write", "/p/A.java", &write_of("void x() {}"), |_, _| {
                Ok(json!({"matches": []}))
            })
        );
    }

    #[test]
    fn a_match_with_no_location_is_not_actionable_and_does_not_deny() {
        // "Something like this exists somewhere" is the unhelpful half of the
        // answer, and denying on it would spend the agent's time for nothing.
        assert_eq!(
            Verdict::NoMatch,
            judge(Mode::Block, "Write", "/p/A.java", &write_of("void x() {}"), |_, _| {
                Ok(engine_answer(&[("parse", "parses something", "")]))
            })
        );
    }

    #[test]
    fn only_java_edits_are_our_business() {
        let p = write_of("void x() {}");
        assert_eq!(
            Verdict::NotAJavaEdit,
            judge(Mode::Block, "Read", "/p/A.java", &p, |_, _| Ok(json!({})))
        );
        assert_eq!(
            Verdict::NotAJavaEdit,
            judge(Mode::Block, "Write", "/p/notes.txt", &p, |_, _| Ok(json!({})))
        );
    }

    #[test]
    fn the_kill_switch_is_off_and_nothing_is_asked() {
        assert_eq!(
            Verdict::Disabled,
            judge(Mode::Off, "Write", "/p/A.java", &write_of("void x() {}"), |_, _| {
                panic!("Off must not reach the engine")
            })
        );
    }

    #[test]
    fn it_ships_in_block_but_a_misspelling_never_escalates() {
        // Nothing configured is the RULING's default.
        assert_eq!(Mode::Block, Mode::parse(None));
        assert_eq!(Mode::Block, Mode::parse(Some("")));
        assert_eq!(Mode::Block, Mode::parse(Some("block")));
        assert_eq!(Mode::Off, Mode::parse(Some("off")));
        assert_eq!(Mode::Observe, Mode::parse(Some("observe")));
        // A word nobody recognises is a preference badly spelled, and reading it
        // as Block would grant more authority than was asked for.
        assert_eq!(Mode::Observe, Mode::parse(Some("observ")));
        assert_eq!(Mode::Observe, Mode::parse(Some("blok")));
    }

    #[test]
    fn an_edit_fragment_is_forwarded_as_the_draft() {
        // An Edit carries a fragment rather than a file, and it must still reach
        // the engine: adding one method inside a class body is exactly how a
        // re-derived job arrives.
        let payload = json!({"new_string": "  private Tree parse(Source s) { return null; }"})
            .to_string();
        let seen = std::cell::RefCell::new(String::new());
        let v = judge(Mode::Block, "Edit", "/p/A.java", &payload, |_, draft| {
            *seen.borrow_mut() = draft.to_string();
            Ok(json!({"matches": []}))
        });
        assert_eq!(Verdict::NoMatch, v);
        assert!(seen.borrow().contains("parse"), "the fragment must reach the engine");
    }

    #[test]
    fn observe_records_the_same_verdict_it_would_have_denied_on() {
        // The mode decides what the PIPELINE does with a verdict; the verdict
        // itself is the same fact either way, which is what makes a would-block
        // count comparable with a block count.
        let v = judge(Mode::Observe, "Write", "/p/A.java", &write_of("void x() {}"), |_, _| {
            Ok(engine_answer(&[("x", "does x", "com.example.X#x")]))
        });
        assert!(matches!(v, Verdict::Duplicate { .. }), "{v:?}");
    }
}
