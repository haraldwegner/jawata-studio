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

/// Characters past which a message to the human must have been judged.
/// Length is a trigger BECAUSE it is wording-independent.
pub const LENGTH_BUDGET: usize = 2200;

/// Abbreviations a reader of this project already holds.
const KNOWN_TERMS: &[&str] = &[
    "API","MCP","JDT","CPU","JVM","CI","PR","TDD","AST","JSON","HTTP","URL","ID",
    "OK","DONE","STOP","NOT","AND","THE","ALL","NEW","YOUR","BOTH","RED","YES","NO",
    "MSI","NSIS","DMG","DEB","XML","SHIM","E2E","OS","UI","IDE","GUI","SDK","LTS",
];

/// Capitalised terms the message never defines. A term counts as defined when
/// the text explains it in parentheses on either side.
fn undefined_terms(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.split(|c: char| !c.is_ascii_alphanumeric()) {
        // 2..=10, not 2..=5. The first version skipped TOCTOU, SIGPIPE and
        // ETXTBSY — every term in its own test — because real jargon is
        // usually longer than five characters.
        if raw.len() < 2 || raw.len() > 10 { continue; }
        if !raw.chars().all(|c| c.is_ascii_uppercase()) { continue; }
        if KNOWN_TERMS.contains(&raw) { continue; }
        let defined = text.contains(&format!("{raw} (")) || text.contains(&format!("({raw}"));
        if !defined && !out.iter().any(|o| o == raw) {
            out.push(raw.to_string());
        }
    }
    out
}

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
    /// Refusals the AGENT ITSELF emitted in this window — not strings it read.
    /// The bash gate counted the raw transcript and fired on any session that
    /// merely READ a file containing the word; this counts assistant text only.
    pub refusals_emitted: usize,
    /// Whether the final message asks the human for a word, ruling or decision.
    pub asks_the_human: bool,
    /// Seat commands invoked in this window (/refactor, /cover, ...).
    pub seats_invoked: Vec<String>,
    /// Whether a verification gate ran after them.
    pub gate_ran: bool,
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

    // PORTED FROM THE SCRIPT GENERATION, and deliberately NOT gated on
    // autonomy — which is why these two can fire today while Rules A and B
    // cannot. The parity contract in hook-events.json lists what the script
    // still holds alone; pointing a client at this binary before these were
    // ported would have STRIPPED five working protections to gain two inert
    // ones. Found by the first dogfood probe after the release.

    // The audit-fix loop. Every other trigger in the script generation waits
    // for a CHECKPOINT marker, and a churn loop never produces one — so the
    // failure state and the trigger condition were mutually exclusive. This
    // counts the loop instead.
    if facts.turn.refusals_emitted >= 3 {
        return StopVerdict::Block {
            reason: format!(
                "AUDIT-FIX LOOP: {} refusals you emitted in this window with no \
                 checkpoint reached. Repeated defect-in-the-fixing-commit is a \
                 DESIGN alarm, not a bug streak. Stop fixing findings and run the \
                 architect seat as a watch-diff against the ARCHITECTURE artifact: \
                 design fix or bandage?",
                facts.turn.refusals_emitted
            ),
        };
    }

    // The unjudged ask. Independent of autonomy: an ask is an ask.
    if facts.turn.asks_the_human && !facts.turn.communicator_ran() {
        return StopVerdict::Block {
            reason: "UNJUDGED ASK: this message asks for a word, a ruling or a \
                     decision, and no communicator subagent ran since the human's \
                     last turn. Run the communicator on it, apply its verdict, \
                     then stop."
                .to_string(),
        };
    }

    // PORTED: the length budget. Harald's own suggestion, and stronger than a
    // phrase list because it does not depend on wording — which is exactly how
    // the ten-phrase ask detector failed on its first live outing.
    if facts.turn.final_text.len() > LENGTH_BUDGET && !facts.turn.communicator_ran() {
        return StopVerdict::Block {
            reason: format!(
                "TOO LONG: {} characters, and the communicator has not judged it. \
                 Length is noise. Cut to what the reader needs, run the \
                 communicator, then send.",
                facts.turn.final_text.len()
            ),
        };
    }

    // PORTED: seat discipline. A seat that proposes without running its gates
    // is proposing work it has not verified.
    if !facts.turn.seats_invoked.is_empty() && !facts.turn.gate_ran {
        return StopVerdict::Block {
            reason: format!(
                "SEAT DISCIPLINE: {} invoked with no verification gate after it. \
                 A gate you did not run has NOT passed. Run it before proposing.",
                facts.turn.seats_invoked.join(", ")
            ),
        };
    }

    // PORTED: undefined jargon. The reader cannot decide on terms he cannot
    // resolve, and this is the rule he most wanted kept.
    let undefined = undefined_terms(&facts.turn.final_text);
    if undefined.len() > 2 {
        return StopVerdict::Block {
            reason: format!(
                "UNDEFINED TERMS: {} — define every abbreviation at first use, or \
                 cut it. He cannot decide on words he cannot resolve.",
                undefined.join(", ")
            ),
        };
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
                                // Count refusals the AGENT EMITTED, from
                                // assistant text only — never the raw window,
                                // which made the script fire on any session
                                // that merely READ the word.
                                turn.refusals_emitted += t.matches("REFUSE").count();
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
    turn.asks_the_human = asks_the_human(&turn.final_text);
    for seat in ["/refactor", "/cover", "/javadocs", "/debug", "/profile"] {
        if transcript_text.contains(seat) && !turn.seats_invoked.iter().any(|s| s == seat) {
            turn.seats_invoked.push(seat.to_string());
        }
    }
    turn.gate_ran = ["compile_workspace", "run_tests", "get_diagnostics", "cargo test"]
        .iter()
        .any(|g| transcript_text.contains(g));
    Ok(turn)
}

/// Does this message ask the human for something?
///
/// FIRST LIVE FAILURE, on the day it shipped: this was a list of ten phrases,
/// and the very next ask the agent wrote — "you decide whether it becomes
/// v3.7.2 or waits" — matched none of them. The deployed gate returned allow.
/// That is the SAME defect this function's own comment claimed to fix about the
/// script generation: keying on tokens the agent authors freely.
///
/// STATE THE LIMIT PLAINLY: a closed list of phrasings, written by the party it
/// constrains, can always be walked around — usually by accident, as here. The
/// list below is materially wider (a direct question, the decision verbs, the
/// option-offering shapes) and that is a REDUCTION IN LEAKAGE, not a fix.
///
/// The only non-evadable form is the inversion Rule A describes: in an
/// autonomous session EVERY stop must show a communicator pass, whatever the
/// message says. That requires the autonomy signal the hook cannot yet read —
/// which makes the Studio autonomy file the difference between a rule that
/// leaks and a rule that holds.
fn asks_the_human(text: &str) -> bool {
    let u = text.to_uppercase();
    // Explicit requests for a ruling.
    const PHRASES: &[&str] = &[
        "YOUR WORD", "NEEDS YOUR", "YOUR CALL", "YOUR RULING", "YOUR SIGN-OFF",
        "YOUR DECISION", "SHALL I", "WANT ME TO", "DO YOU WANT", "MAY I",
        "DECISION:", "LET ME KNOW", "UP TO YOU", "YOU DECIDE", "YOU CHOOSE",
        "IF YOU'D RATHER", "IF YOU PREFER", "SAY THE WORD", "ON YOUR WORD",
        "AWAITING", "AWAIT YOUR", "SHOULD I", "WOULD YOU LIKE", "PREFER THAT I",
    ];
    if PHRASES.iter().any(|p| u.contains(p)) {
        return true;
    }
    // A DIRECT QUESTION to the reader. The phrase list above is what the agent
    // remembers; a question mark is what the agent cannot avoid while asking.
    // Scoped to the last few lines so a question quoted mid-report — an audit
    // brief, a rhetorical framing — does not trip it.
    text.lines()
        .rev()
        .take(6)
        .any(|l| l.trim_end().ends_with('?'))
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
            turn: Turn { final_text: "done".into(), launches, refusals_emitted: 0, asks_the_human: false, seats_invoked: vec![], gate_ran: true },
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
    /// PORTED RULE 1, and it fires WITHOUT autonomy — which is the whole point.
    /// The parity contract listed this as script-only; cutting a client over to
    /// the binary before it existed would have stripped a working protection.
    #[test]
    fn the_audit_fix_loop_blocks_without_needing_autonomy() {
        let mut f = facts(Autonomy::Unknown, vec![]);
        f.turn.refusals_emitted = 3;
        match judge(&f) {
            StopVerdict::Block { reason } => {
                assert!(reason.contains("AUDIT-FIX LOOP"), "{reason}");
                assert!(reason.contains("architect"), "must name the actuator: {reason}");
            }
            StopVerdict::Allow => panic!("three emitted refusals must block"),
        }
        // Two is not a loop.
        f.turn.refusals_emitted = 2;
        assert_eq!(StopVerdict::Allow, judge(&f));
    }

    /// PORTED RULE 2, likewise ungated by autonomy: an ask is an ask.
    #[test]
    fn an_unjudged_ask_blocks_and_a_judged_one_does_not() {
        let mut f = facts(Autonomy::Unknown, vec![]);
        f.turn.asks_the_human = true;
        match judge(&f) {
            StopVerdict::Block { reason } => assert!(reason.contains("UNJUDGED ASK"), "{reason}"),
            StopVerdict::Allow => panic!("an unjudged ask must block"),
        }
        f.turn.launches = vec![communicator()];
        assert_eq!(StopVerdict::Allow, judge(&f), "a judged ask must pass");
    }

    /// Refusals are counted from ASSISTANT TEXT, never the raw window. The
    /// script generation counted the whole transcript and fired on any session
    /// that merely READ the word — reading is not refusing.
    #[test]
    fn refusals_are_counted_only_from_what_the_agent_emitted() {
        let quoted = "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"tool_result\",\"content\":\"REFUSE REFUSE REFUSE\"}]}}\n{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"all green\"}]}}\n";
        let t = read_turn(quoted).expect("parses");
        assert_eq!(0, t.refusals_emitted, "quoted refusals are not emitted ones");

        let emitted = "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"round 1 REFUSE\"}]}}\n{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"round 2 REFUSE\"}]}}\n";
        assert_eq!(2, read_turn(emitted).expect("parses").refusals_emitted);
    }

    /// The ask detector must not key on a shape the agent authors freely. A
    /// message that asks in plain words is caught even without `DECISION:`.
    #[test]
    fn an_ask_phrased_without_the_token_is_still_an_ask() {
        let s = "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"One thing needs your word before I push.\"}]}}\n";
        assert!(read_turn(s).expect("parses").asks_the_human);
        let plain = "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Committed and green, continuing.\"}]}}\n";
        assert!(!read_turn(plain).expect("parses").asks_the_human);
    }

    /// THE LIVE MISS, as a test. On the day the check shipped, the next ask the
    /// agent wrote matched none of its ten phrases and the deployed gate
    /// allowed it. This is that exact sentence.
    #[test]
    fn the_ask_that_slipped_past_the_first_phrase_list_is_caught() {
        for ask in [
            "This is dogfood output; you decide whether it becomes v3.7.2 or waits.",
            "Not pushed. Up to you whether we ship it.",
            "Do we cut a patch, or leave it?",
            "Let me know which you'd prefer.",
        ] {
            let s = format!(
                "{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{ask}\"}}]}}}}\n"
            );
            assert!(
                read_turn(&s).expect("parses").asks_the_human,
                "must be read as an ask: {ask:?}"
            );
        }
    }

    /// And ordinary reporting must still pass, or the check is turned off by
    /// the first person it annoys.
    #[test]
    fn a_report_that_asks_nothing_is_not_an_ask() {
        for plain in [
            "Committed and green, continuing to Stage 9.",
            "The suite is 156 tests, five clean runs. Nothing outstanding.",
            "I fixed the escaping and re-tagged; all five targets published.",
        ] {
            let s = format!(
                "{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{plain}\"}}]}}}}\n"
            );
            assert!(
                !read_turn(&s).expect("parses").asks_the_human,
                "must not be read as an ask: {plain:?}"
            );
        }
    }

    /// The three ported checks, each fired without autonomy.
    #[test]
    fn the_length_budget_blocks_an_unjudged_wall_of_text() {
        let mut f = facts(Autonomy::Unknown, vec![]);
        f.turn.final_text = "Committed and green. ".repeat(140);
        match judge(&f) {
            StopVerdict::Block { reason } => assert!(reason.contains("TOO LONG"), "{reason}"),
            StopVerdict::Allow => panic!("a wall of text must be judged first"),
        }
        // A judged one passes — the check must be satisfiable by judging, not
        // only by staying quiet.
        f.turn.launches = vec![communicator()];
        assert_eq!(StopVerdict::Allow, judge(&f));
    }

    #[test]
    fn a_seat_without_its_gate_blocks() {
        let mut f = facts(Autonomy::Unknown, vec![]);
        f.turn.seats_invoked = vec!["/refactor".into()];
        f.turn.gate_ran = false;
        match judge(&f) {
            StopVerdict::Block { reason } => assert!(reason.contains("SEAT DISCIPLINE"), "{reason}"),
            StopVerdict::Allow => panic!("a seat that skipped its gate must block"),
        }
        f.turn.gate_ran = true;
        assert_eq!(StopVerdict::Allow, judge(&f));
    }

    #[test]
    fn undefined_jargon_blocks_and_defined_jargon_does_not() {
        let mut f = facts(Autonomy::Unknown, vec![]);
        f.turn.final_text = "The TOCTOU in the SIGPIPE path broke the ETXTBSY retry.".into();
        match judge(&f) {
            StopVerdict::Block { reason } => assert!(reason.contains("UNDEFINED TERMS"), "{reason}"),
            StopVerdict::Allow => panic!("three undefined terms must block"),
        }
        f.turn.final_text =
            "TOCTOU (a check-then-act race) hit the SIGPIPE (broken-pipe signal) path.".into();
        assert_eq!(StopVerdict::Allow, judge(&f), "defined terms must pass");
    }

}
