//! The recall gate: when the store holds a record anchored at exactly the
//! symbol a tool call is about, the agent says what it did with it.
//!
//! **Why a gate at all.** Retrieval already fires on every prompt and every
//! tool call, and the channel works — measured. What does not work is the
//! consumer: on 2026-08-18 the store handed over the record describing the very
//! approach under discussion, and the agent reasoned past it, twice, with
//! nothing anywhere observing the miss. The one mechanism in this binary that
//! has never been reasoned past is the `jawata-fallback:` lattice, and the
//! reason is structural rather than persuasive: **the call does not proceed
//! until the agent says something.** This module applies that shape to recall.
//!
//! **Deliberately narrow, and the narrowness is the design.** It fires only
//! when a record's own anchor IS the cue — a `Type#member` cue answered by a
//! record anchored at that member. Not the type's other lessons, not the
//! package's, not analogies. Those are nominees; gating on them would fire on
//! nearly every call in this codebase (`org.jawata.core` plus `org.jawata.mcp`
//! is most of it), and a gate that fires constantly turns its own declaration
//! into a reflex token — the fluency failure wearing a compliance badge.
//!
//! **The admission that comes with it:** the incident that motivated the gate
//! was a PACKAGE-anchored record, so this gate would NOT have caught it. That
//! case belongs to the observer, which watches dispositions across a whole
//! session at every anchor level. This gate covers the narrow, sound case; the
//! observer covers the case that actually bit. Neither is the other's backup.
//!
//! **Failure direction: OPEN.** A store that cannot answer does not gate — and
//! says so. A gate that blocks when the knowledge layer is down would convert
//! an outage into a work stoppage, which is a worse failure than the one it
//! prevents.

use serde_json::Value;

/// How much authority the gate has. It ships in [`Mode::Observe`]: it records
/// what it WOULD have blocked and blocks nothing, so promotion is decided on a
/// measured would-block count rather than on intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Off entirely — the documented kill switch.
    Off,
    /// Record would-block events; never block. THE SHIPPING DEFAULT.
    Observe,
    /// Block an undispositioned call.
    Block,
}

impl Mode {
    /// Read the mode from the config value. An unknown word is `Observe`, not
    /// `Block`: a typo must never silently grant a mechanism more authority
    /// than the person writing it asked for.
    pub fn parse(configured: Option<&str>) -> Mode {
        match configured.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            Some("off") | Some("false") | Some("disabled") => Mode::Off,
            Some("block") => Mode::Block,
            _ => Mode::Observe,
        }
    }
}

/// What the gate concluded. Every variant carries enough to write one honest
/// line in the outcomes log — a verdict that cannot explain itself is the
/// silence this whole binary exists to end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The call named no member-level symbol, so there is nothing to gate on.
    NoMemberCue,
    /// The kill switch is off.
    Disabled,
    /// The agent already said what it did — `recall-applied` or
    /// `recall-rejected: <reason>` — and the call proceeds.
    Dispositioned { token: String },
    /// The store holds no record anchored AT this member. Nominees may exist;
    /// they do not gate.
    NoAnchoredRecord,
    /// The knowledge layer could not answer. The call proceeds, and the reason
    /// is recorded rather than being reported as "nothing known".
    Unavailable { why: String },
    /// A record anchored at exactly this member exists and the call carried no
    /// disposition. In [`Mode::Observe`] this is recorded and the call
    /// proceeds; in [`Mode::Block`] it is the denial.
    Undispositioned { cue: String, summary: String },
}

/// The declaration that says the agent USED what it was given.
pub const APPLIED: &str = "recall-applied";
/// The declaration that says the agent judged it and it did not fit. It takes
/// a reason, because "not relevant" with no reason is the reflex token this
/// gate is shaped to avoid producing.
pub const REJECTED: &str = "recall-rejected:";

/// Member cues in a tool payload: the `Type#member` forms only.
///
/// A bare type name is not a member cue. The gate's whole claim to being sound
/// is that the record's anchor and the cue are the SAME symbol, and a type cue
/// cannot establish that about a member-anchored record.
pub fn member_cues(target: &str) -> Vec<String> {
    match crate::cue::extract_tool_target(target) {
        Ok(cues) => cues.symbols.into_iter().filter(|s| s.contains('#')).collect(),
        Err(_) => Vec::new(),
    }
}

/// Did this call already say what it did with the recalled knowledge?
pub fn disposition_in(payload: &str) -> Option<String> {
    let lower = payload.to_lowercase();
    if let Some(at) = lower.find(REJECTED) {
        let reason: String = payload[at + REJECTED.len()..]
            .chars()
            .take_while(|c| *c != '\n' && *c != '"')
            .collect();
        let reason = reason.trim();
        // A bare `recall-rejected:` with nothing after it is not a judgement.
        // Treating it as one would hand the agent a one-word bypass, which is
        // exactly how a gate decays into a token.
        if !reason.is_empty() {
            return Some(format!("{REJECTED} {reason}"));
        }
        return None;
    }
    if lower.contains(APPLIED) {
        return Some(APPLIED.to_string());
    }
    None
}

/// THE PURE CORE: given a cue and the store's STRUCTURED answer, does a record
/// anchored at exactly that symbol exist?
///
/// Structured, because the rendered text line carries type, summary, status and
/// details — and no anchor. Classifying anchor level off prose would be the
/// regex mistake this crate was written to end, one layer up.
pub fn anchored_at(cue: &str, data: &Value) -> Option<String> {
    let entries = data.get("entries")?.as_array()?;
    for entry in entries {
        let symbol = entry.get("symbol").and_then(Value::as_str).unwrap_or_default();
        if !symbol.is_empty() && symbol.eq_ignore_ascii_case(cue) {
            return Some(
                entry
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or("(no summary)")
                    .to_string(),
            );
        }
    }
    None
}

/// Judge one tool call.
///
/// `ask` is a closure rather than the `Store` trait so this module stays
/// independent of the pipeline's transport — and so a test can hand it a
/// fixture without standing up a store.
///
/// The order matters and is the design: a DISPOSITION short-circuits before the
/// store is asked at all. An agent that has already said what it did should not
/// pay a round trip to be told it may proceed.
pub fn judge<F>(mode: Mode, payload: &str, ask: F) -> Verdict
where
    F: Fn(&str) -> Result<serde_json::Value, crate::query::QueryError>,
{
    if mode == Mode::Off {
        return Verdict::Disabled;
    }
    if let Some(token) = disposition_in(payload) {
        return Verdict::Dispositioned { token };
    }
    let cues = member_cues(payload);
    let Some(cue) = cues.first() else {
        return Verdict::NoMemberCue;
    };
    match ask(cue) {
        Ok(answer) => match anchored_at(cue, &answer) {
            Some(summary) => Verdict::Undispositioned { cue: cue.clone(), summary },
            None => Verdict::NoAnchoredRecord,
        },
        // FAIL OPEN, AND SAY SO. A gate that blocks when the knowledge layer is
        // down converts an outage into a work stoppage — a worse failure than
        // the one it prevents. jawata-mcp#37's typed unavailable is what makes
        // this branch distinguishable from "the store looked and found nothing".
        Err(e) => Verdict::Unavailable { why: format!("{e:?}") },
    }
}

/// The line the agent is shown when the gate holds a call.
pub fn steering(cue: &str, summary: &str) -> String {
    format!(
        "JAWATA GATE — the store holds a record anchored at exactly `{cue}`, the symbol this \
         call is about:\n  {summary}\n\nSay what you did with it before proceeding: put \
         `{APPLIED}` in the call when you used it, or `{REJECTED} <why it does not fit>` when \
         you judged it and it does not. A reason is required — \"not relevant\" with nothing \
         behind it is the reflex this gate exists to prevent. This is not a nominee: it is a \
         record about this exact symbol."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn store_answer(anchors: &[(&str, &str)]) -> Value {
        json!({
            "result": "match",
            "entries": anchors.iter()
                .map(|(sym, sum)| json!({"symbol": sym, "summary": sum}))
                .collect::<Vec<_>>()
        })
    }

    #[test]
    fn a_record_anchored_at_the_member_gates() {
        let answer = store_answer(&[(
            "org.jawata.mcp.importer.ProjectImporter#addDependencyEntries",
            "the bundle pool already resolves these",
        )]);
        assert_eq!(
            Some("the bundle pool already resolves these".to_string()),
            anchored_at("org.jawata.mcp.importer.ProjectImporter#addDependencyEntries", &answer)
        );
    }

    #[test]
    fn a_record_anchored_at_the_type_does_not_gate_a_member_cue() {
        // The nominee case. It is offered to the agent by the ordinary recall
        // injection; it does not stop the call.
        let answer = store_answer(&[(
            "org.jawata.mcp.importer.ProjectImporter",
            "this importer writes its preconditions in javadoc",
        )]);
        assert_eq!(
            None,
            anchored_at("org.jawata.mcp.importer.ProjectImporter#addDependencyEntries", &answer)
        );
    }

    #[test]
    fn a_package_scoped_record_does_not_gate_and_that_is_the_admission() {
        // THE MOTIVATING INCIDENT'S OWN SHAPE. The record that was ignored on
        // 2026-08-18 is package-anchored: `scope.packages` set, `symbol` empty.
        // This gate does not catch it, deliberately — gating on a package
        // anchor would fire on most calls in this repository. The observer
        // covers this case; this test exists so the hole is a written decision
        // rather than a thing someone discovers later.
        let answer = json!({
            "result": "match",
            "entries": [{
                "scope": {"packages": ["org.jawata.core", "org.jawata.mcp"], "symbols": []},
                "summary": "Sprint 23 already resolves PDE bundles from a local pool"
            }]
        });
        assert_eq!(
            None,
            anchored_at("org.jawata.mcp.importer.ProjectImporter#addDependencyEntries", &answer)
        );
    }

    #[test]
    fn an_absence_gates_nothing() {
        let answer = json!({"result": "absence", "entries": []});
        assert_eq!(None, anchored_at("com.example.Foo#bar", &answer));
    }

    #[test]
    fn only_member_shaped_cues_are_gate_cues() {
        assert_eq!(
            vec!["com.example.Foo#bar".to_string()],
            member_cues("com.example.Foo#bar")
        );
        assert!(
            member_cues("com.example.Foo").is_empty(),
            "a bare type is a nominee cue, never a gate cue"
        );
        assert!(member_cues("/home/harald/src/Foo.java").is_empty());
    }

    #[test]
    fn a_disposition_is_recognised_in_either_form() {
        assert_eq!(Some(APPLIED.to_string()), disposition_in("... recall-applied ..."));
        assert_eq!(
            Some("recall-rejected: it is about the other overload".to_string()),
            disposition_in("recall-rejected: it is about the other overload")
        );
    }

    #[test]
    fn a_bare_rejection_with_no_reason_is_not_a_disposition() {
        // The one-word bypass. If this passed, the gate would teach agents a
        // token instead of a judgement.
        assert_eq!(None, disposition_in("recall-rejected:"));
        assert_eq!(None, disposition_in("recall-rejected:   "));
    }

    #[test]
    fn the_kill_switch_is_off_and_an_unknown_word_never_grants_authority() {
        assert_eq!(Mode::Off, Mode::parse(Some("off")));
        assert_eq!(Mode::Block, Mode::parse(Some("block")));
        assert_eq!(Mode::Observe, Mode::parse(None));
        assert_eq!(Mode::Observe, Mode::parse(Some("blockk")),
            "a typo must not silently promote the gate to blocking");
    }

    // ---- judge(): the whole decision, end to end -------------------------

    const MEMBER_CALL: &str =
        r#"{"tool_input":{"symbol":"com.example.Importer#addDependencyEntries"}}"#;

    fn answers(anchors: &[(&str, &str)]) -> impl Fn(&str) -> Result<Value, crate::query::QueryError> {
        let answer = store_answer(anchors);
        move |_cue: &str| Ok(answer.clone())
    }

    #[test]
    fn a_member_anchored_record_holds_an_undispositioned_call() {
        let verdict = judge(
            Mode::Observe,
            MEMBER_CALL,
            answers(&[(
                "com.example.Importer#addDependencyEntries",
                "the pool already resolves these",
            )]),
        );
        match verdict {
            Verdict::Undispositioned { cue, summary } => {
                assert_eq!("com.example.Importer#addDependencyEntries", cue);
                assert!(summary.contains("pool"));
            }
            other => panic!("expected the gate to hold this call, got {other:?}"),
        }
    }

    #[test]
    fn a_dispositioned_call_proceeds_without_asking_the_store_at_all() {
        let verdict = judge(
            Mode::Observe,
            r#"{"tool_input":{"symbol":"com.example.Importer#addDependencyEntries",
                "reason":"recall-applied — following the bundle-pool record"}}"#,
            |_| panic!("the store must not be asked once the agent has already answered"),
        );
        assert!(matches!(verdict, Verdict::Dispositioned { .. }));
    }

    #[test]
    fn a_rejection_with_a_reason_also_proceeds() {
        let verdict = judge(
            Mode::Observe,
            r#"{"tool_input":{"symbol":"com.example.Importer#addDependencyEntries",
                "reason":"recall-rejected: that record is about the other overload"}}"#,
            |_| panic!("already answered"),
        );
        match verdict {
            Verdict::Dispositioned { token } => assert!(token.contains("other overload")),
            other => panic!("expected a disposition, got {other:?}"),
        }
    }

    #[test]
    fn an_unreadable_store_never_blocks_and_names_the_reason() {
        // THE FAIL-OPEN DIRECTION. A gate that blocked here would turn a store
        // outage into a work stoppage.
        let verdict = judge(Mode::Block, MEMBER_CALL, |_| {
            Err(crate::query::QueryError::ToolRefused {
                code: "KNOWLEDGE_UNAVAILABLE".into(),
                message: "the store did not answer within 1200ms".into(),
            })
        });
        match verdict {
            Verdict::Unavailable { why } => assert!(why.contains("KNOWLEDGE_UNAVAILABLE"), "{why}"),
            other => panic!("an outage must not block, got {other:?}"),
        }
    }

    #[test]
    fn the_kill_switch_stops_the_gate_before_anything_else() {
        let verdict = judge(Mode::Off, MEMBER_CALL, |_| panic!("Off must not reach the store"));
        assert_eq!(Verdict::Disabled, verdict);
    }

    #[test]
    fn a_call_with_no_member_cue_is_not_gated() {
        let verdict = judge(
            Mode::Observe,
            r#"{"tool_input":{"typeName":"com.example.Importer"}}"#,
            |_| panic!("a type cue must not reach the gate's store question"),
        );
        assert_eq!(Verdict::NoMemberCue, verdict);
    }

    #[test]
    fn the_steering_demands_a_reason_and_names_the_symbol() {
        let line = steering("com.example.Foo#bar", "the pool already resolves these");
        assert!(line.contains("com.example.Foo#bar"));
        assert!(line.contains(APPLIED) && line.contains(REJECTED));
        assert!(line.contains("reason is required"));
    }
}
