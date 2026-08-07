//! The stop gate — may this turn end?
//!
//! Sprint 28 Stage 8b. Two rules were asked for. Only one of them can be
//! honestly enforced, and saying which is which IS the deliverable:
//!
//! * **Rule A — do not send an upward message the communicator has not
//!   judged.** Enforceable IN PRINCIPLE; **not enforced today**. Two limits,
//!   both established by audit rather than assumed:
//!   - The transcript is NOT agent-proof. It is mode 600 owned by the uid the
//!     agent runs as, and [`read_turn`] accepts any JSONL line anywhere in the
//!     file — so one appended line satisfies [`Turn::communicator_ran`]. The
//!     earlier claim here, that the agent "cannot forge or skip its way past
//!     it", was false.
//!   - Even unforged, it proves only that the communicator was CALLED. It
//!     cannot read the verdict, so running it and ignoring the answer passes.
//! * **Rule B — do not stop when autonomy is granted and nothing is armed.**
//!   OBSERVABLE ONLY. Its trigger — whether the human granted autonomous
//!   continuation — is underivable from anything this hook can read. It is
//!   RECORDED (see [`SilenceReason::AutonomyUnknown`]) rather than enforced,
//!   because a gate that reads as enforcement and enforces nothing is the exact
//!   defect this sprint exists to end.
//!
//! NEITHER RULE CAN FIRE IN PRODUCTION TODAY: [`crate::pipeline`] always
//! supplies [`Autonomy::Unknown`], and both rules are gated on `Granted`,
//! which is constructed only in tests. Both are RECORDED, not enforced.
//! Marking one of them "enforceable" in the present tense — as this comment
//! previously did — is the specific claim this sprint exists to stop.
//!
//! The transcript is still preferred over a marker file the agent writes,
//! because a marker can be skipped silently while a missing transcript entry
//! requires a deliberate forgery. That is a difference in cost, not the
//! absolute asymmetry originally claimed here.
//!
//! The second asymmetry, in [`judge`]: "the turn launched nothing" PROVES
//! nothing is armed, while "the turn launched something" proves only that
//! something once started — it may already have finished. So the gate blocks
//! only on the direction it can actually prove.

use crate::safety::SilenceReason;

/// Whether the human granted autonomous continuation for this session.
///
/// `Unknown` is the honest default and today the only value produced: nothing
/// the hook can read carries the answer. It becomes real when Studio writes a
/// session-scoped file on the human's own toggle — an act of the human's,
/// which the agent cannot fake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Autonomy {
    Granted,
    NotGranted,
    Unknown,
}

/// One tool invocation recorded in the transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolUse {
    pub name: String,
    /// For an `Agent` call, the `subagent_type` it was given.
    pub subagent: Option<String>,
    /// Whether it was started in the background — i.e. whether it can wake us.
    pub backgrounded: bool,
}

impl ToolUse {
    /// Did this call arm work that will wake the agent later?
    ///
    /// An `Agent` spawn qualifies regardless of flags — the harness notifies on
    /// completion. A `Bash` call qualifies only when backgrounded.
    ///
    /// EXCEPT the communicator, and the exception is the point: it JUDGES the
    /// message being sent, it is not work that continues afterwards. Counting
    /// it would let the two rules cancel each other out — run the communicator,
    /// satisfy Rule A, and have Rule B read the same call as "something is
    /// armed", so a turn that ends with a judged summary and nothing running
    /// would pass both. That is precisely the stop this gate exists to catch.
    pub fn arms_work(&self) -> bool {
        if self.is_communicator() {
            return false;
        }
        self.name == "Agent" || self.backgrounded
    }

    /// Was this the communicator judging an upward message?
    pub fn is_communicator(&self) -> bool {
        self.name == "Agent" && self.subagent.as_deref() == Some("communicator")
    }
}

/// The last assistant turn, as read from the transcript.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Turn {
    /// The final assistant text — what the human would actually read.
    pub final_text: String,
    /// Tool calls made since the last human message.
    pub launches: Vec<ToolUse>,
}

impl Turn {
    pub fn communicator_ran(&self) -> bool {
        self.launches.iter().any(ToolUse::is_communicator)
    }
    pub fn armed_anything(&self) -> bool {
        self.launches.iter().any(ToolUse::arms_work)
    }
}

/// Everything the gate decides from.
#[derive(Debug, Clone)]
pub struct StopFacts {
    /// The client's own anti-loop flag: true on a second pass, meaning we
    /// already blocked once. Blocking again would trap the session.
    pub already_bounced: bool,
    pub turn: Turn,
    pub autonomy: Autonomy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopVerdict {
    Allow,
    /// The reason is fed back to the model, so it must say what to DO.
    Block { reason: String },
}

/// The decision. Pure — no I/O, so every branch is reachable from a test.
pub fn judge(facts: &StopFacts) -> StopVerdict {
    // The anti-loop flag wins over everything. A gate that can block twice can
    // wedge a session, and a wedged session is worse than an unjudged message.
    if facts.already_bounced {
        return StopVerdict::Allow;
    }

    // RULE A. In an autonomous session the default inverts: EVERY stop must
    // show a communicator pass. Keying on the message's SHAPE instead — the
    // previous generation looked for "DECISION:" and similar tokens — is
    // defeatable by wording, and the agent authors the wording.
    if facts.autonomy == Autonomy::Granted && !facts.turn.communicator_ran() {
        return StopVerdict::Block {
            reason: "Autonomy is granted and this turn ends with a message that \
                     the communicator has not judged. Run the communicator \
                     subagent on it, apply its verdict, then stop."
                .to_string(),
        };
    }

    // RULE B, decisive direction only. "Launched nothing" proves nothing is
    // armed. The converse does not hold, so it is not asserted.
    if facts.autonomy == Autonomy::Granted && !facts.turn.armed_anything() {
        return StopVerdict::Block {
            reason: "Autonomy is granted and this turn armed no background work, \
                     so ending here sleeps until the human returns. Start the \
                     next piece of work, or state plainly that you are blocked \
                     on the human."
                .to_string(),
        };
    }

    StopVerdict::Allow
}

/// Parse the last assistant turn out of transcript TEXT.
///
/// Takes the text, not a path, for the reason `safety::read_with_deadline`
/// takes a source rather than opening stdin itself: it makes every branch
/// reachable without a client, a file, or a session.
///
/// Tolerant by construction — a transcript is appended to live, so the final
/// line can be a partial write. An unparseable line is SKIPPED, never fatal:
/// failing the gate because the harness was mid-flush would block a turn for a
/// reason that has nothing to do with the rules.
pub fn read_turn(transcript_text: &str) -> Result<Turn, SilenceReason> {
    if transcript_text.trim().is_empty() {
        return Err(SilenceReason::NoTranscript);
    }
    let mut turn = Turn::default();
    for line in transcript_text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue; // partial flush or a shape we do not know
        };
        match v.get("type").and_then(|t| t.as_str()) {
            // A human message resets the window: we only care about what has
            // happened since the human last spoke.
            Some("user") if !is_tool_result(&v) => {
                turn = Turn::default();
            }
            Some("assistant") => {
                let content = v
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array());
                let Some(blocks) = content else { continue };
                for b in blocks {
                    match b.get("type").and_then(|t| t.as_str()) {
                        Some("text") => {
                            if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                                turn.final_text = t.to_string();
                            }
                        }
                        Some("tool_use") => {
                            let name = b
                                .get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or_default()
                                .to_string();
                            let input = b.get("input");
                            turn.launches.push(ToolUse {
                                name,
                                subagent: input
                                    .and_then(|i| i.get("subagent_type"))
                                    .and_then(|s| s.as_str())
                                    .map(str::to_string),
                                backgrounded: input
                                    .and_then(|i| i.get("run_in_background"))
                                    .and_then(serde_json::Value::as_bool)
                                    .unwrap_or(false),
                            });
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    Ok(turn)
}

/// A `user` entry carrying a tool RESULT is the harness echoing our own tool
/// call back, not the human speaking. Treating it as a human message would
/// reset the window on every single tool call and make the gate blind.
fn is_tool_result(v: &serde_json::Value) -> bool {
    v.get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
        .map(|blocks| {
            blocks
                .iter()
                .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result"))
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str) -> ToolUse {
        ToolUse { name: name.into(), subagent: None, backgrounded: false }
    }
    fn communicator() -> ToolUse {
        ToolUse { name: "Agent".into(), subagent: Some("communicator".into()), backgrounded: false }
    }
    fn facts(autonomy: Autonomy, launches: Vec<ToolUse>) -> StopFacts {
        StopFacts {
            already_bounced: false,
            turn: Turn { final_text: "done".into(), launches },
            autonomy,
        }
    }

    #[test]
    fn without_autonomy_the_gate_allows() {
        // An ordinary conversational session must be untouched — the rules are
        // about autonomous runs, and a gate that fires in normal use would be
        // turned off by the first person it annoyed.
        for a in [Autonomy::NotGranted, Autonomy::Unknown] {
            assert_eq!(StopVerdict::Allow, judge(&facts(a, vec![])));
        }
    }

    #[test]
    fn autonomy_without_a_communicator_pass_blocks() {
        let v = judge(&facts(Autonomy::Granted, vec![tool("Bash")]));
        match v {
            StopVerdict::Block { reason } => assert!(reason.contains("communicator")),
            StopVerdict::Allow => panic!("must block"),
        }
    }

    /// THE DISCRIMINATOR: one fixture, two runs, differing only by the
    /// communicator call. Without this the suite proves nothing — a gate that
    /// always allowed would pass every other test here.
    #[test]
    fn removing_only_the_communicator_call_flips_the_verdict() {
        let armed = ToolUse { name: "Agent".into(), subagent: Some("general-purpose".into()), backgrounded: false };
        let with = facts(Autonomy::Granted, vec![communicator(), armed.clone()]);
        let without = facts(Autonomy::Granted, vec![armed]);
        assert_eq!(StopVerdict::Allow, judge(&with));
        assert!(matches!(judge(&without), StopVerdict::Block { .. }), "must flip");
    }

    #[test]
    fn autonomy_with_nothing_armed_blocks_even_after_the_communicator() {
        let v = judge(&facts(Autonomy::Granted, vec![communicator()]));
        match v {
            StopVerdict::Block { reason } => assert!(reason.contains("armed no background work")),
            StopVerdict::Allow => panic!("a turn that armed nothing must not end"),
        }
    }

    #[test]
    fn an_agent_spawn_counts_as_armed_but_a_foreground_bash_does_not() {
        assert!(ToolUse { name: "Agent".into(), subagent: None, backgrounded: false }.arms_work());
        assert!(!tool("Bash").arms_work());
        assert!(ToolUse { name: "Bash".into(), subagent: None, backgrounded: true }.arms_work());
    }

    /// The anti-loop flag must win, or the gate can wedge a session — worse
    /// than the problem it solves.
    #[test]
    fn a_second_pass_always_allows() {
        let mut f = facts(Autonomy::Granted, vec![]);
        f.already_bounced = true;
        assert_eq!(StopVerdict::Allow, judge(&f));
    }

    // ---- read_turn ----

    const TRANSCRIPT: &str = r#"
{"type":"user","message":{"content":[{"type":"text","text":"continue and autocontinue"}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","input":{"subagent_type":"communicator"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","content":"ok"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"the summary"}]}}
"#;

    #[test]
    fn the_turn_is_read_from_the_transcript() {
        let t = read_turn(TRANSCRIPT).expect("parses");
        assert_eq!("the summary", t.final_text);
        assert!(t.communicator_ran(), "the communicator call must be seen");
    }

    /// A tool RESULT arrives as a `user` entry. If that reset the window, the
    /// gate would forget every tool call the moment its result came back — it
    /// would be blind in exactly the sessions it exists for.
    #[test]
    fn a_tool_result_does_not_reset_the_window() {
        let t = read_turn(TRANSCRIPT).expect("parses");
        assert_eq!(1, t.launches.len(), "the tool_use must survive its own result");
    }

    #[test]
    fn a_real_human_message_does_reset_the_window() {
        let s = format!(
            "{TRANSCRIPT}\n{}",
            r#"{"type":"user","message":{"content":[{"type":"text","text":"new ask"}]}}"#
        );
        let t = read_turn(&s).expect("parses");
        assert!(!t.communicator_ran(), "the window must reset on a human turn");
        assert_eq!("", t.final_text);
    }

    /// A transcript is appended to live, so the final line can be half-written.
    #[test]
    fn a_partial_final_line_is_skipped_not_fatal() {
        let s = format!("{TRANSCRIPT}\n{{\"type\":\"assist");
        let t = read_turn(&s).expect("a partial line must not fail the parse");
        assert_eq!("the summary", t.final_text);
    }

    #[test]
    fn an_empty_transcript_names_itself() {
        assert_eq!(Err(SilenceReason::NoTranscript), read_turn("   "));
    }

    #[test]
    fn a_backgrounded_bash_is_read_as_armed() {
        let s = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash","input":{"run_in_background":true}}]}}"#;
        let t = read_turn(s).expect("parses");
        assert!(t.armed_anything());
    }
}
