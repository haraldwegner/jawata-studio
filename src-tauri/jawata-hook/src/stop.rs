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

/// How long an assistant text block must be to count as AN ANSWER rather than
/// narration between two tool calls.
///
/// SIX HUNDRED, AND IT IS A GUESS WITH A STATED FAILURE DIRECTION. There is no
/// clean signal here: an answer and a running commentary are the same kind of
/// object and differ only in size. Measured over the 2026-08-30 transcript that
/// produced this rule — inter-tool narration ran 100–400 characters, answers to
/// his questions 700–2400. The cut sits in the gap, which is the most that can
/// honestly be claimed for it.
///
/// Over-firing costs one reorder: do the work first and answer last, which is
/// the shape the rule is asking for anyway. Under-firing costs what happened
/// that night — four questions, four answers, and an unrelated task opened
/// inside each of them while he was still typing.
pub const ANSWER_LENGTH: usize = 600;

/// The first line of the turn-around refusal — the guard emits it, this reader
/// keys on it, and sharing the constant is what keeps the two from drifting the
/// way every duplicated condition in this crate has.
pub const TURNAROUND_MARKER: &str = "ANSWER FIRST OR WORK FIRST";

/// The subagent that judges a stop under a live grant.
pub const JUDGE_SEAT: &str = "autocontinue";

/// What the [`JUDGE_SEAT`] said about this stop.
///
/// It replaces [`Turn::declares_a_decision`] as Rule B's exemption, and the
/// reason is the defect measured on 2026-09-03. That field keyed the exemption
/// on a `DECISION:` line the AGENT writes. The gate's own doc had already named
/// the risk — "the agent's discretion wearing a detector's clothes" — and
/// answered it by demanding a deliberate marker instead of a guess. But a
/// declaration is still the agent's text, and an agent whose training rewards
/// ending a turn produces the mandated format on the first attempt, with no
/// adaptation and nothing to detect. Two stops that night, both formatted
/// correctly, neither carrying a decision: a release the plan schedules for a
/// later stage, and a refused review whose findings the agent had already fixed
/// and simply not re-run.
///
/// So the exemption moves onto text the agent does NOT author: a verdict a
/// fresh-context seat wrote, read out of the harness's own tool-result record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JudgeVerdict {
    /// The plan reserves this for the human. The stop stands.
    Reserved,
    /// The agent can resolve it — carrying the seat's own next action, so the
    /// block that follows tells it what to do rather than merely to continue.
    Resolvable(String),
}

/// How many times a turn may be bounced for a missing review before the gate
/// gives up and lets it through. Bounded on purpose: the valve this replaces
/// existed so a gate could never wedge a session, and that concern is real —
/// it is answered by a ceiling, not by allowing the first retry through.
/// Consecutive EMPTY turns under a granted autonomy before the gate lets go.
pub const MAX_EMPTY_TURNS: u32 = crate::autonomy::MAX_EMPTY_TURNS;

pub const MAX_UNJUDGED_BOUNCES: u32 = 3;

/// How many times a turn may be held for a story it wrote and never reseeded.
///
/// BOUNDED, and the bound is load-bearing rather than decoration. A reseed
/// admits stamped stories only, so a draft under the substrate root that has
/// not earned its `reviewed:` stamp reports drift no reseed will clear — and an
/// unbounded hold would wedge the session on a file nobody meant to store yet.
/// Two, because the cure is one tool call: write, get held once, reseed. The
/// second is for the case where the reseed refuses and the file needs fixing.
pub const MAX_RESEED_BOUNCES: u32 = 2;

/// How a hold for an unstored story identifies itself.
///
/// Shared with the pipeline because the counter must be charged to THIS rule
/// and no other: a turn held by the audit-fix rule while a story also happens
/// to be unstored has not spent a reseed chance, and charging it would let the
/// story rule be walked past by tripping a different one twice. A constant
/// rather than a repeated literal, so the two ends cannot drift apart in
/// silence — `the_hold_names_itself_so_its_counter_cannot_be_miswired` pins it.
pub const UNSTORED_STORY: &str = "UNSTORED STORY";

/// Abbreviations a reader of this project already holds.
///
/// Note what the second row USED to be: OK, DONE, STOP, NOT, AND, THE, ALL,
/// NEW, YOUR, BOTH, RED, YES, NO — ordinary English words, added one at a time
/// as the rule misfired on emphasis. A list that grows like that is a
/// structural flaw being maintained by hand, so the fix went to the scanner
/// (quoted and emphasised spans are redacted, runs of capitals are labels) and
/// those words are no longer needed here.
///
/// What remains are genuine abbreviations, plus the studio's OWN badge labels —
/// a reader of this project does hold those, which is exactly what this list
/// means.
const KNOWN_TERMS: &[&str] = &[
    "API","MCP","JDT","CPU","JVM","CI","PR","TDD","AST","JSON","HTTP","URL","ID",
    "MSI","NSIS","DMG","DEB","XML","SHIM","E2E","OS","UI","IDE","GUI","SDK","LTS",
    // The studio's own status badges, as they appear on screen.
    "RUNNING","STOPPED","STARTING","FAILED",
];

/// Capitalised terms the message never defines. A term counts as defined when
/// the text explains it in parentheses on either side.
fn undefined_terms(text: &str) -> Vec<String> {
    // REDACTING QUOTED SPANS WAS THE WRONG FIX, and mutation testing is what
    // showed it: with the run rule below in place, blanking backticks and bold
    // changed no verdict — the two were redundant — and it would have BLINDED
    // the rule inside quotes, which is precisely where a genuine acronym
    // (SIGPIPE out of a log, say) still needs explaining. Quoting does not make
    // a term resolvable for the reader.
    //
    // A RUN of capitals is one label, not a list of abbreviations. "CANNOT BE
    // READ" is a badge; reporting BE as an undefined term is the giveaway that
    // the rule was matching shape rather than meaning. Tokens whose immediate
    // neighbour (across spaces or hyphens only) is also capitalised are skipped.
    let words: Vec<&str> = text.split_whitespace().collect();
    let is_caps = |w: &str| {
        let core: String = w.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        core.len() >= 2 && core.chars().all(|c| c.is_ascii_uppercase())
    };
    let mut in_run: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (i, w) in words.iter().enumerate() {
        if !is_caps(w) {
            continue;
        }
        let neighbour_caps = (i > 0 && is_caps(words[i - 1]))
            || (i + 1 < words.len() && is_caps(words[i + 1]));
        if neighbour_caps {
            for part in w.split(|c: char| !c.is_ascii_alphanumeric()) {
                if !part.is_empty() {
                    in_run.insert(part.to_string());
                }
            }
        }
    }

    let mut out: Vec<String> = Vec::new();
    for raw in text.split(|c: char| !c.is_ascii_alphanumeric()) {
        // 2..=10, not 2..=5. The first version skipped TOCTOU, SIGPIPE and
        // ETXTBSY — every term in its own test — because real jargon is
        // usually longer than five characters.
        if raw.len() < 2 || raw.len() > 10 { continue; }
        if !raw.chars().all(|c| c.is_ascii_uppercase()) { continue; }
        if KNOWN_TERMS.contains(&raw) { continue; }
        if in_run.contains(raw) { continue; }
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
        if self.is_communicator() || self.is_autocontinue_judge() {
            return false;
        }
        self.name == "Agent" || self.backgrounded
    }

    /// Was this the communicator judging an upward message?
    /// A run of the `autocontinue` judge — the seat that decides whether a stop
    /// under a live grant is the human's to make.
    ///
    /// Like the communicator it is NOT work and does NOT arm anything: it judges
    /// the turn that is trying to end. Counting it either way would hand the
    /// agent a way to satisfy Rule B by consulting the very judge Rule B
    /// demands, and — worse — would reset the empty-turn ceiling that is this
    /// rule's only release valve, so a session that spun on the judge could
    /// never be let go.
    pub fn is_autocontinue_judge(&self) -> bool {
        (self.name == "Agent" || self.name == "SendMessage")
            && self.subagent.as_deref() == Some(JUDGE_SEAT)
    }

    pub fn is_communicator(&self) -> bool {
        // A SPAWN...
        if self.name == "Agent" && self.subagent.as_deref() == Some("communicator") {
            return true;
        }
        // ...or a CONTINUATION of one. Harald ruled on 2026-08-20 that the
        // reviewer should be ONE per session, continued rather than respawned:
        // it sees only his messages and the drafts, so what it accumulates is
        // exactly what HE has, and a genuinely cold reader would demand terms
        // re-explained that he already owns.
        //
        // Keyed on the spawn alone, this gate would have DEMANDED a fresh spawn
        // every turn — forcing the very cost that ruling removes, and doing it
        // in the name of a rule the agent had already satisfied. Found the same
        // hour, by continuing the reviewer and watching the gate not notice.
        //
        // The addressee must be the NAME, not an opaque id: the turn window
        // resets at each human message, so the original spawn is out of view
        // and an id cannot be resolved back to a communicator from here.
        self.name == "SendMessage" && self.subagent.as_deref() == Some("communicator")
    }
}

/// The last assistant turn, as read from the transcript.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Turn {
    /// The final assistant text — what the human would actually read.
    pub final_text: String,
    /// Tool calls made since the last human message.
    pub launches: Vec<ToolUse>,
    /// Did the agent do ANY work since the last time this gate pushed it?
    ///
    /// This is the empty-turn ceiling's input, and it must be measured from the
    /// PUSH rather than from the human's last message. Two reasons, and the
    /// second is why `launches.is_empty()` cannot serve:
    ///
    /// 1. The ceiling exists to release a WEDGE — pushed, produced nothing,
    ///    pushed, produced nothing. Its question is therefore about what happened
    ///    after a push, not about the window as a whole. Until 2026-08-30 it was
    ///    fed [`Turn::armed_anything`], so a turn of thirty edits and a commit
    ///    counted as EMPTY and two of them stood Rule B down — an unattended run
    ///    on a two-turn leash, stopping exactly like a normal stop.
    /// 2. But the window does NOT reset at a push: the client injects the block
    ///    reason as a user line, and `is_our_own_bounce` deliberately keeps the
    ///    previous window rather than resetting it. So "any tool call in the
    ///    window" would count calls made BEFORE the push towards the attempt
    ///    AFTER it — the counter would reset on every push, the ceiling would
    ///    never be reached, and the wedge it guards against would become an
    ///    endless push loop. Named by the fresh audit of 2026-08-30 as the reason
    ///    the obvious version of this fix is not sufficient.
    pub worked_since_push: bool,
    /// Refusals the AGENT ITSELF emitted in this window — not strings it read.
    /// The bash gate counted the raw transcript and fired on any session that
    /// merely READ a file containing the word; this counts assistant text only.
    pub refusals_emitted: usize,
    /// Whether the final message asks the human for a word, ruling or decision.
    ///
    /// HEURISTIC, and it may only cost a REVIEW. This is inferred from prose by
    /// a phrase list, so it is wrong sometimes — and every wrong guess used to
    /// be paid for twice: it stood Rule B down for the turn AND deleted the
    /// autonomy grant. Both are now keyed on [`Turn::declares_a_decision`]
    /// instead. See that field for Harald's ruling.
    pub asks_the_human: bool,
    /// Whether the final message DECLARES a decision, in the one form his
    /// upward contract mandates: a `DECISION:` line of its own.
    ///
    /// HARALD'S RULING, 2026-08-29, verbatim: *"you cannot by yourself change
    /// the autocontinue variable by yourself. Only the conditions met is a way
    /// to stop your work. … If I want you to stop at checkpoint I don't say
    /// autocontinue. But you cannot on your own discretion just switch this."*
    ///
    /// The control lives entirely at GRANT time and it is his: he withholds the
    /// word when he wants checkpoint stops. So the agent may not switch the
    /// grant off, and may not stand the push down on an INFERENCE about its own
    /// wording either — a phrase guessed out of prose is the agent's discretion
    /// wearing a detector's clothes, and it slept a session for 21 minutes on
    /// 2026-08-29 by matching "SAY THE WORD" inside the sentence *"Nothing
    /// needed from you — say the word only if you want one back."*
    ///
    /// A DECLARATION is different in kind from a guess: it is unambiguous, it
    /// is the agent's own deliberate act, and it is the form he already
    /// requires for a real ask. Everything he named as a legitimate stop — a
    /// design or spec direction, access only he has, something genuinely
    /// broken, an unresolved dispute with the auditor, a release, the sprint
    /// finished — is raised AS a decision, so one marker carries all of them.
    pub declares_a_decision: bool,
    /// The `tool_use` ids of the [`JUDGE_SEAT`] spawns in this window.
    ///
    /// THE VERDICT IS BOUND TO THE CALL THAT ASKED FOR IT (v4.0.2). Until this
    /// existed the reader took the first verdict line out of ANY tool result
    /// that followed a judge spawn, and the line-start discipline was the only
    /// thing standing between that and a quotation — which it does not survive,
    /// because the reader trims indentation before matching and the files that
    /// carry the line indented are exactly the ones an agent has reason to
    /// open: the judge's own stance, and the release notes describing it.
    ///
    /// Measured against the shipped v4.0.1 binary: spawn the judge, let it
    /// answer nothing, then read `~/.claude/agents/autocontinue.md`, and the
    /// stop was ALLOWED.
    ///
    /// A client that omits `id` on a tool call yields an empty list here, and
    /// then no verdict is ever read and the turn is held until the empty-turn
    /// ceiling releases it. That is the safe direction: holding a turn costs
    /// two pushes, accepting a quoted line costs the whole rule.
    pub judge_call_ids: Vec<String>,
    /// Has the agent done work since the current verdict was recorded?
    ///
    /// THE VERDICT HAS A LIFETIME, AND WORK IS WHAT ENDS IT (Harald, 2026-09-03,
    /// on the C3 stop: *"We had the agent saying I can fix and he did not move
    /// on."*). v4.0.2 kept the first verdict of the window for the window's
    /// whole life, which failed both ways at once. An agent told "fix X" that
    /// fixed X and stopped again was re-served the stale instruction forever —
    /// and because fixing X is a tool call, the idle valve never released it.
    /// An agent told "fix X" that did NOT fix X and re-spawned the judge until
    /// it heard something kinder would, under the obvious last-wins fix, have
    /// been let through.
    ///
    /// One flag answers both. A fresh verdict is accepted only when there is no
    /// verdict yet OR this flag is set, so re-rolling without working changes
    /// nothing; and a verdict with this flag set is SPENT — it neither blocks
    /// with its stale next action nor allows on a stale reservation, it demands
    /// a fresh consultation that will see the work.
    pub verdict_spent: bool,
    /// The verdict, read from the HARNESS's tool-result record rather than from
    /// the agent's own prose.
    ///
    /// That distinction is the whole mechanism. `verdict_lines` already reads
    /// relayed audit verdicts out of assistant text, and for those it is right:
    /// the agent is reporting someone else's finding and has no motive to
    /// misreport it. Here it has exactly that motive — the verdict decides
    /// whether its turn may end — so the line is taken from the tool result the
    /// harness wrote, which the agent did not author.
    pub judge_verdict: Option<JudgeVerdict>,
    /// Whether the human INTERRUPTED this window (Esc, or a stop mid-tool).
    ///
    /// His Esc must stop the work, full stop. A gate that answers an interrupt
    /// by refusing the stop would be arguing with the one control he has that is
    /// not a sentence — and it would do it while he is sitting there pressing
    /// the key. Every blocking rule stands down on it.
    pub interrupted: bool,
    /// Whether the human's own message OPENED this window with a question.
    ///
    /// studio#11: the 2026-08-07 ruling puts direct replies outside this gate —
    /// "replies to the user's OWN questions are DIRECT, fast, bottom-line-first,
    /// never routed through the communicator", because gating conversation
    /// triples his waiting time for a failure mode it barely has. The detector
    /// had no notion of who asked, so a reply that quoted his question or ended
    /// on a clarifying line was held as an UNJUDGED ASK and cost a full
    /// communicator round trip on a message the ruling exempts.
    ///
    /// SET ONLY BY A REAL HUMAN LINE. A task notification or system reminder is
    /// the harness, however question-shaped its embedded text is — see
    /// `is_harness_line` for the six-hour sleep that rule cost.
    pub user_asked: bool,
    /// Whether a REAL keyboard line opened this window (a typed message or an
    /// interrupt) — as opposed to a harness notification. INTERNAL FACT for
    /// the pipeline's counter resets ONLY: Rule B deliberately does NOT key on
    /// it (a typed dispatch like "carry on" must still be pushed — see the
    /// wire tests). The audit-loop counter persists across
    /// notification-opened windows and resets when the human actually speaks;
    /// without this flag the counter could not tell those windows apart, which
    /// is how the AUDIT-FIX alarm spent months unable to fire on the very loop
    /// it was built for (2026-08-29).
    pub human_window: bool,
    /// Whether this window belongs to a SUBAGENT's own conversation rather
    /// than the session the human is typing in.
    ///
    /// The harness stamps every line of a sidechain `"isSidechain": true`, and
    /// nothing in this binary read it until 2026-09-04. That matters for one
    /// rule in particular: the turn-around guard's whole premise is *"he is in
    /// this window — his own message opened it"*, and in a sidechain the
    /// opening message is the PARENT AGENT's prompt. Read as the human's, a
    /// one-shot speed bump becomes a wall, because the one-shot reset below
    /// clears [`Turn::answered_substantially`] and the subagent's very next
    /// paragraph of narration sets it again — there is no human message to end
    /// the window and no way out.
    ///
    /// MEASURED on the transcript of an architect seat run, 2026-09-04: SIX
    /// refusals in one sidechain, every one on a read-only shell command, and
    /// the seat reported its gates as NOT RUN and therefore NOT passed. The
    /// denials came back in exactly the shape the reset looks for — `type:
    /// user`, one `tool_result` block carrying [`TURNAROUND_MARKER`] — so the
    /// reset fired each time and was undone each time. That is why the fix is
    /// the premise and not the reset: v3.17.5 built the rule, v3.17.6 made it
    /// one-shot, and a third repair on the reset would have been the third
    /// face of one structure.
    pub sidechain: bool,
    /// Whether this window has already emitted a SUBSTANTIAL answer — a text
    /// block at or above [`ANSWER_LENGTH`].
    ///
    /// HARALD, 2026-08-30, on the failure this exists for: *"in a conversation
    /// is the other way round. You are not working on a plan but talking with
    /// me -> Hence, don't turn around and work on something."* Measured the same
    /// evening: he asked four questions in a row, and each answer was followed,
    /// inside the same turn, by tool calls opening an unrelated task — once by a
    /// commit to his repository at 23:01:44, with the gate's own record showing
    /// him present.
    ///
    /// THIS IS A PROXY AND THE THRESHOLD IS A GUESS, stated rather than hidden.
    /// Nothing distinguishes "a complete answer" from "narration between two
    /// tool calls" except length, and no length is correct. Measured over that
    /// evening's transcript: narration between tool calls ran 100–400
    /// characters, real answers 700–2400. The cut sits between them, and the
    /// failure direction is chosen: over-firing costs a reorder — do the work
    /// first, answer last — while under-firing costs what happened that night.
    pub answered_substantially: bool,
    /// Whether this window's assistant text relayed a SIGN-OFF verdict line.
    /// The audit loop CONVERGED — the pipeline resets the cross-window refusal
    /// carry on it, so the next loop starts at zero.
    pub signoff_emitted: bool,
    /// Seat commands invoked in this window (/refactor, /cover, ...).
    pub seats_invoked: Vec<String>,
    /// Whether a verification gate ran after them.
    pub gate_ran: bool,
    /// Whether this window actually CHANGED CODE (studio#18). A seat run that
    /// edited nothing owes no compile gate — the obligation belongs to the
    /// edit, not to the seat.
    pub changed_code: bool,
    /// Assistant text emitted AFTER a degradation stamp arrived, joined.
    ///
    /// Not every block, and the difference is a live hole rather than a nicety.
    /// Accumulating the whole window let a mention emitted BEFORE the stamp
    /// satisfy a rule about reporting it AFTER — an agent that opened with "I
    /// will check for graceful degradation in the retry path", then consumed a
    /// degraded answer and said nothing about it, passed. "degrad" is ordinary
    /// working vocabulary in this repo, so that is an accident waiting rather
    /// than an attack.
    ///
    /// `final_text` is the LAST block, which is what the length budget and the
    /// ask detector want. It is the wrong input for "did you say it": an agent
    /// that narrates a degradation, keeps working, and ends on "Done." said it
    /// - and would have been blocked and told it had not. `refusals_emitted`
    /// on the adjacent line already accumulates across blocks for exactly this
    /// reason.
    pub narration: String,
    /// Whether this window WROTE a markdown file.
    ///
    /// The AUTHORING half of the substrate rule; the store owns the other half
    /// (whether anything under its root is uningested). The two are kept apart
    /// deliberately: this hook cannot know where the substrate lives, and the
    /// store cannot know whether THIS turn was the one that wrote there.
    pub wrote_markdown: bool,
    /// studio#4: tool RESULTS in this window that carried a degradation stamp.
    ///
    /// Counted from tool results only — never from the raw window and never
    /// from assistant text, for the reason `refusals_emitted` carries the same
    /// restriction: the script generation counted the whole transcript and
    /// fired on any session that merely READ the word.
    pub degraded_consumed: usize,
}

/// The stamp a degraded resident puts on its response (`jawata-mcp#12`).
///
/// **NOTHING EMITS IT YET, and that is declared rather than discovered.**
/// `jawata-mcp#12` is open; a search of the engine's every string literal
/// finds the word in three places and a line beginning `DEGRADED:` in none —
/// the store's fallback notice reads `EXPERIENCE STORE DEGRADED — …` (no
/// colon) and the scan notice reads `DEGRADED SCAN: …`. So this rule is
/// correct and unreachable, and `hook-events.json` carries it as
/// `present-but-inert` for the same reason Rules A and B do. A gate that
/// reads as enforcement and enforces nothing is the shape this whole file
/// exists to stop, so it is written down here rather than left for the next
/// reader to measure.
///
/// Matched at the START of a line, not anywhere in the text. A source file
/// read, a grep hit or a test fixture can all contain the word mid-line —
/// jawata's own tree does, in four places — and keying on a bare substring
/// would block a turn for having LOOKED AT the notice rather than for having
/// been given one.
pub const DEGRADED_STAMP: &str = "DEGRADED:";

impl Turn {
    /// Did the final message tell the human about it?
    ///
    /// Any form of the word counts — "degraded", "degradation", "the store is
    /// degraded". Demanding the literal stamp would make the rule satisfiable
    /// by pasting a token, which is the reflex the recall gate's own design
    /// notes refuse to manufacture.
    pub fn surfaced_degradation(&self) -> bool {
        self.narration.to_lowercase().contains("degrad")
    }

    /// Does this turn owe a review?
    ///
    /// Decision-class only, per Harald's 2026-08-07 ruling: a message that asks
    /// him for a word, when he did not ask first, and no reviewer ran. A REPLY
    /// to his own question is out of scope — gating conversation triples his
    /// waiting time for a failure mode conversation barely has.
    /// Did a reviewer subagent run in this window?
    ///
    /// NOTHING IN `judge` READS THIS ANY MORE (v4.0.1). It survives as a probe
    /// over the parser — the window-reset tests use it to ask whether a subagent
    /// launch was seen at all, which is live behaviour that
    /// [`ToolUse::is_autocontinue_judge`] now depends on.
    ///
    /// The predicate it used to feed, `owes_a_review`, is deleted rather than
    /// left unused: an unread predicate beside a retired rule is how the valve
    /// kept consulting the reviewer for a whole release.
    pub fn communicator_ran(&self) -> bool {
        self.launches.iter().any(ToolUse::is_communicator)
    }
    pub fn armed_anything(&self) -> bool {
        self.launches.iter().any(ToolUse::arms_work)
    }

    /// Was the [`JUDGE_SEAT`] spawned in this window — with an id the harness
    /// minted, so that its answer can be bound to the call?
    ///
    /// Derived, not stored. It was a separate boolean until v4.1.0, set by the
    /// spawn whether or not an id was present, so a client omitting ids left
    /// the two facts disagreeing: "ran" true, ids empty, no verdict ever
    /// readable, and the turn held on "RAN AND RETURNED NO VERDICT" with no
    /// way out. One source, and a spawn without an id is honestly not a run.
    pub fn judge_ran(&self) -> bool {
        !self.judge_call_ids.is_empty()
    }

    /// The verdict that still governs this stop — None once work has spent it.
    pub fn live_verdict(&self) -> Option<&JudgeVerdict> {
        if self.verdict_spent {
            None
        } else {
            self.judge_verdict.as_ref()
        }
    }

    /// Is the agent holding an instruction it has not acted on?
    ///
    /// This is the one state in which idling must NOT release the turn. The
    /// idle valve exists to free a session that is wedged with nothing to do;
    /// an agent that has been told what to do and is doing nothing is not
    /// wedged, it is refusing. Measured before this existed: spawn the judge
    /// three times, do nothing else, and the valve let the turn end without a
    /// verdict ever being read.
    pub fn holds_an_unspent_fix(&self) -> bool {
        matches!(self.live_verdict(), Some(JudgeVerdict::Resolvable(_)))
    }
}

/// What the store says about its own file substrate.
///
/// Carried as an `Option` on the facts, and `None` is NOT "no drift": it means
/// the question was not asked, or the store could not answer it. A resident
/// that is down must never read as a clean store — that is the same inference
/// this whole rule exists to stop, pointed at the store instead of the files.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubstrateDrift {
    /// Where the substrate lives, as the store reports it — never guessed here.
    pub root: String,
    /// How many files under it no row cites.
    pub count: usize,
    /// Those files, as the store named them. It caps the list; this does not
    /// re-cap it, so a truncated list stays the store's statement.
    pub named: Vec<String>,
}

/// Everything the gate decides from.
#[derive(Debug, Clone)]
pub struct StopFacts {
    /// The client's own anti-loop flag: true on a second pass, meaning we
    /// already blocked once. Blocking again would trap the session.
    pub already_bounced: bool,
    /// How many times THIS session has already been bounced for a missing
    /// review. Counted by the pipeline (a per-session file), never here —
    /// `judge` stays pure.
    pub bounces: u32,
    pub turn: Turn,
    pub autonomy: Autonomy,
    /// Consecutive turns under this grant that produced no tool calls.
    ///
    /// Counted by the pipeline, like `bounces`, so `judge` stays pure. It is a
    /// property of the TURNS and not of the rulings: a session being pushed and
    /// working is not a session being pushed and stuck, and only this number
    /// tells them apart.
    pub empty_turns: u32,
    /// Review rounds since he last spoke — the ceiling on a loop that does
    /// real work every round and therefore never advances `empty_turns`.
    pub review_rounds: u32,
    /// The store's answer about its substrate, asked only when this turn wrote
    /// markdown. Gathered by the pipeline, like `bounces`, so `judge` stays pure.
    pub substrate: Option<SubstrateDrift>,
    /// How many times THIS session has already been held for an unstored story.
    /// A counter of its own rather than a share of `bounces`: a turn that spent
    /// its budget arguing about a message must still be held for a story it
    /// dropped on the floor.
    pub reseed_bounces: u32,
}

impl StopFacts {
    /// Did this turn write a story and leave it outside the store?
    ///
    /// BOTH halves are required. The write alone proves nothing (most markdown
    /// in this repository is not a story), and drift alone is not this turn's
    /// business — a stale draft someone left under the root would otherwise
    /// hold every turn forever.
    pub fn owes_a_reseed(&self) -> bool {
        self.turn.wrote_markdown && self.substrate.as_ref().is_some_and(|s| s.count > 0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopVerdict {
    Allow,
    /// The reason is fed back to the model, so it must say what to DO.
    Block { reason: String },
}

/// The decision. Pure — no I/O, so every branch is reachable from a test.
pub fn judge(facts: &StopFacts) -> StopVerdict {
    // The anti-loop valve, NARROWED (Harald, 2026-08-20: "not acceptable").
    //
    // It used to allow EVERYTHING on the retry, which made every rule a single
    // nag: block once, and the retry — which the harness marks as a retry —
    // sailed through. Measured live on 3.12.2: block at 15:53:52, allowed
    // 0.8 s later. A rule that can be walked past by doing nothing is not a
    // gate.
    //
    // On the retry, exactly ONE thing may still hold the turn: the missing
    // review. Every other rule keeps its old single-bounce behaviour, because
    // those are advisory by design and re-firing them would change contracts
    // this change has no business touching.
    //
    // The wedge risk the old valve existed for is answered by BOUNDING it
    // rather than by surrendering: after MAX_UNJUDGED_BOUNCES the turn is let
    // through and the giving-up is recorded. The agent can always exit sooner
    // by doing the thing — which is the difference between a bounded gate and
    // a trap.
    // The anti-loop valve for every rule EXCEPT the ones that carry their own
    // ceiling: the review rule, the reseed rule, and RULE B.
    //
    // RULE B WAS ADDED 2026-08-29, and the defect it fixes was measured live in
    // this gate's own log: `emitted, emitted, stop-allowed, stop-allowed`, where
    // the last of those released a turn that ended on "here is what I will do
    // next" with nothing armed. Harald: "you are in the middle of nowhere and
    // just stop. This is even worse than with a hard cut at a checkpoint."
    //
    // The chain: a message trips the ask detector — OFTEN FALSELY, because the
    // phrase list matches the bare substring DECISION inside ordinary words like
    // "design decisions" — the turn is bounced, the agent rewrites without the
    // trigger word, and the retry reaches this valve. `owes_a_review` is now
    // false, so the valve returns Allow BEFORE Rule B is ever evaluated.
    //
    // So a false positive in the ask detector did not merely cost one bounce: it
    // DISABLED THE AUTOCONTINUE PUSH for that turn. And it fires hardest exactly
    // when the agent is discussing design, which is when it is deepest in a task
    // and least able to afford being stranded.
    //
    // Rule B never needed this valve's protection: it is bounded by
    // MAX_EMPTY_TURNS, which counts turns that produce NOTHING rather than blocks
    // it issues, so a working session never approaches it and a wedged one is
    // released in two. Excluding it here restores the push without reintroducing
    // the wedge the valve exists to prevent.
    // THIS IS THE SECOND COPY of Rule B's condition and it must agree with the
    // first, character for character. The v3.17.2 notes already recorded what
    // happens otherwise: a fix taught to one copy and not the other, so the
    // exemption switched itself off exactly where it was needed.
    // v4.0.1: `!facts.turn.owes_a_review()` IS GONE FROM HERE, and its survival
    // is what this patch exists for. v4.0.0 retired the reviewer rule and left
    // this reader — so on a retry the retired subagent still decided a verdict:
    // spawn it, and a message this valve would otherwise hold was released.
    // Measured against the SHIPPED binary, one fixture, two runs differing only
    // by the communicator call: without it TOO LONG, with it allowed.
    //
    // The release note said no gate consults it. That was false, and the shape
    // is the one this repository has shipped before — the rule removed from one
    // implementation and left in a second reader that no test covered.
    let rule_b_would_push = rule_b_engaged(facts);
    if facts.already_bounced && !facts.owes_a_reseed() && !rule_b_would_push {
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
    //
    // THE COUNT SPANS WINDOWS (2026-08-29, Harald: "the conversation loop
    // counter needs to be reset correctly"). Each audit verdict arrives as a
    // background notification, and a notification opens a new window — so a
    // per-window count reset between every round and the alarm could never
    // fire on the very loop it measures. The pipeline persists the carry per
    // session and resets it on a REAL keyboard window, a relayed
    // "VERDICT: SIGN-OFF" (the loop converged), or an architect-seat run (the
    // action this block demands). `refusals_emitted` here is that cumulative
    // number; `judge` stays pure and never touches the file.
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

    // THE SUBSTRATE RULE (2026-08-27). Writing the story file and reseeding it
    // in are ONE job, and the second half lived only in instruction text. It
    // was skipped: four stories were authored, cold-read, stamped, committed
    // and reported as remembered while the store held none of them. Every
    // surface agreed everything was fine, because every surface asked the store
    // what it HAD rather than whether the files had ARRIVED.
    //
    // The first cure was a warning inside `stats`. That was the wrong channel
    // and the codebase already knew it: a step that was never called cannot be
    // caught by a report that is only seen when you call something. An agent
    // routes around friction without narrating it, so the only channels that
    // hold are the response, a hook, or a non-agent watcher. This is the same
    // measurement moved onto one that ACTS — the turn does not end while a
    // story it just wrote is still outside the store.
    //
    // Placed above the message rules on purpose: until the story is in, every
    // sentence the turn wrote about having remembered it is false, and the
    // communicator should judge the true text rather than the premature one.
    if let Some(drift) = facts.substrate.as_ref().filter(|_| facts.owes_a_reseed()) {
        // The ceiling sits on the RULE, not inside an `already_bounced` branch:
        // Cursor re-invokes with the retry flag unset, so a valve on that path
        // is never entered and the rule blocks forever — measured at counter 11
        // and still climbing, on the review rule, on this machine.
        if facts.reseed_bounces >= MAX_RESEED_BOUNCES {
            return StopVerdict::Allow;
        }
        return StopVerdict::Block {
            reason: format!(
                "{} ({} of {}): {} file(s) under the knowledge substrate at \
{} are not in the store — {}. Writing the file and reseeding it in are ONE job; until the \
second half runs, nothing you wrote is recallable and the next wipe takes it silently. Run \
experience(kind=reseed, path={}, recursive=true, confirm=true) and READ the report: a file \
that comes back under `skipped` was refused, and the reason says what it owes — most often \
a `reviewed:` stamp it has actually earned from a cold reader.",
                UNSTORED_STORY,
                facts.reseed_bounces + 1,
                MAX_RESEED_BOUNCES,
                drift.count,
                drift.root,
                drift.named.join(", "),
                drift.root
            ),
        };
    }

    // studio#4: a turn that CONSUMED a degraded answer and said nothing about
    // it. Independent of autonomy, for the reason the issue exists: every
    // instruction-layer form of this rule ("if jawata is unavailable, ASK —
    // don't silently degrade") has already been proven optional in practice.
    // An agent routes around friction without narrating it, so the only
    // channel that holds is one that does not depend on its goodwill.
    //
    // Placed BEFORE the ask rule deliberately: this is a factual omission from
    // the message, and the communicator should judge the corrected text rather
    // than the incomplete one.
    if facts.turn.degraded_consumed > 0 && !facts.turn.surfaced_degradation() {
        return StopVerdict::Block {
            reason: format!(
                "UNREPORTED DEGRADATION: {} tool result(s) in this turn came back stamped as \
                 degraded, and nothing you said mentions it. The human is about to \
                 act on an answer produced by a component that declared itself \
                 unwell. Say WHICH capability was degraded and WHAT that means for \
                 the conclusions you just drew, then stop.",
                facts.turn.degraded_consumed
            ),
        };
    }

    // The unjudged ask. Independent of autonomy: an ask is an ask — EXCEPT when
    // the human asked first, because then this is a reply and the 2026-08-07
    // ruling puts it outside the gate (studio#11).

    // PORTED: the length budget. Harald's own suggestion, and stronger than a
    // phrase list because it does not depend on wording — which is exactly how
    // the ten-phrase ask detector failed on its first live outing.
    // v4.0.0: THE RULING REPLACED THE REVIEWER, and this rule survives the
    // change because it never needed one. Its subject is length, which is
    // measurable here; and it CLEARS ITSELF — the agent cuts the message and
    // the next attempt passes — so retiring the reviewer costs it nothing.
    // What changed is the instruction: it now names the three questions Harald
    // ruled on rather than demanding a subagent whose readback he then had to
    // read as well.
    if facts.turn.final_text.len() > LENGTH_BUDGET {
        return StopVerdict::Block {
            reason: format!(
                "TOO LONG: {} characters. Length is noise. Cut it, and check the three \
things before sending: is every fact one he can OPEN — a repo file, a command he can \
run — rather than a path or a store only you can see? Is every term one this \
conversation has already defined? Is the implementation detail below the point instead \
of in front of it?",
                facts.turn.final_text.len()
            ),
        };
    }

    // PORTED: seat discipline. A seat that proposes without running its gates
    // is proposing work it has not verified.
    if !facts.turn.seats_invoked.is_empty() && facts.turn.changed_code && !facts.turn.gate_ran {
        return StopVerdict::Block {
            reason: format!(
                "SEAT DISCIPLINE: {} invoked, code changed, and no verification gate \
                 ran after it. A gate you did not run has NOT passed. Run it before \
                 proposing.",
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

    // RULE A — SCOPED, and deliberately narrower than it was this morning.
    //
    // It ran UNCONDITIONALLY for one afternoon, on every turn including
    // "yes, done". That was a wrong cure for a real problem: the rule had
    // never fired in 267 recorded stops, and the cause was detection (autonomy
    // is never reported; the window was being erased) rather than scope.
    // Widening it treated a detection bug as a coverage gap and cost Harald
    // three renderings of every message — the draft, the reviewer's exchange,
    // and the correction — because a stop hook fires AFTER the text has
    // already streamed to him and can only ever ADD a message, never replace
    // one. An independent architect audit ruled the unconditional form a
    // category error: an interception requirement built on an observe-only API.
    //
    // So the scope returns to Harald's 2026-08-07 ruling — decision-class
    // messages only, replies to his own questions exempt — and this rule is a
    // BACKSTOP for a review-first discipline, not the trigger of review. The
    // discipline: the reviewer runs FIRST, the draft exists only inside its
    // prompt, and the readback is the only text that ever streams. When that
    // holds, this rule never fires; when it fires, the discipline lapsed.
    //
    // What survives from the unconditional experiment, because both were real
    // defects rather than scope: the bounce no longer erases the window it
    // judges, and a CONTINUED reviewer counts as a review.
    // THE CEILING GUARDS THE RULE, NOT ONE PATH THROUGH IT.
    //
    // v3.12.3 put the bound inside the `already_bounced` branch, which assumed
    // every client marks a retry. Cursor does not: it re-invokes with the flag
    // unset, so that branch was never entered, the ceiling was unreachable, and
    // the rule blocked forever. Measured on Harald's machine — an endless loop,
    // counter at 11 and still climbing. A safety valve on one path is not a
    // safety valve; the bound now sits on the rule itself, so it holds however
    // the client re-invokes.
    // THE UNJUDGED-ASK RULE IS RETIRED (Harald, 2026-09-03): *"The communicator
    // is annoying. I see the same output twice. It is not far away from what is
    // originally said. Can we instead add a ruling."*
    //
    // It demanded a fresh-context reviewer read every decision-class message
    // before it was sent. What it bought, measured over this session, was three
    // real catches — a phantom "on your word", a count with no object, an
    // undefined term — and every one of them is a CHECKLIST item, not a
    // judgement. What it cost was a whole readback rendered to him alongside
    // the message it was reviewing, which is the "twice" in his sentence: a
    // channel defect, since only the findings were ever meant to reach the
    // agent.
    //
    // So the check moves into the ruling now carried in the deployed
    // instructions and in the length rule's own reason — what he can open, what
    // this conversation has defined, detail after the point. A self-applied
    // ruling loses the fresh eyes, and that trade is acceptable HERE and was
    // not acceptable for Rule B: the agent has no motive to write an unclear
    // message, and every motive to end a turn. Where there is a motive, there
    // is a judge; where there is not, a rule is enough.
    //
    // The mechanical residual stays: length above, and the jargon check, both
    // decidable from the text with nothing to consult.

    // RULE B, decisive direction only. "Launched nothing" proves nothing is
    // armed. The converse does not hold, so it is not asserted.

    // RULE B, decisive direction only. "Launched nothing" proves nothing is
    // armed. The converse does not hold, so it is not asserted.
    //
    // THE CEILING COUNTS EMPTY TURNS, NOT BLOCKS — and that distinction is the
    // design rather than a detail. Bounding the blocks would punish the working
    // case and the wedged case identically: an agent that is pushed, starts the
    // next piece of work, and gets pushed again is the mechanism succeeding, and
    // it would hit a block ceiling on the third success. What a wedge actually
    // looks like is consecutive turns that produce NOTHING — pushed, empty,
    // pushed, empty. So a turn carrying tool calls resets the count and a turn
    // carrying none advances it, which lets a working session run all night
    // without ever approaching the bound while releasing a stuck one in two.
    //
    // The bound sits on the rule and not inside an `already_bounced` branch, for
    // the reason the review rule learned the hard way: Cursor re-invokes with
    // the retry flag unset, so a valve on that path was never entered and the
    // rule blocked forever — measured at counter 11 and still climbing.
    // THE PREDICATE STAYS `armed_anything`, and an attempt to change it to
    // `worked_since_push` was REVERTED on 2026-08-30 because the tests below
    // refused it — correctly.
    //
    // The two fields answer different questions and only one belongs here:
    //   armed_anything      — will ANYTHING wake this session again?
    //   worked_since_push   — did this turn accomplish anything?
    // Rule B exists because a turn that edits, compiles and commits but starts no
    // background job leaves NOTHING to wake the session: it ends mid-task and
    // sleeps until he returns. That is Harald's "you are in the middle of nowhere
    // and just stop", and it is a question about ARMING, not about effort.
    if facts.autonomy == Autonomy::Granted && !facts.turn.armed_anything() {
        // THE CEILING AND HIS INTERRUPT, read through the same predicate the
        // valve above uses. They were two explicit `Allow` guards here and a
        // hand-copied condition there, with a comment warning that the copies
        // must agree "character for character" — which is not a mechanism, and
        // this crate has now twice shipped a fix taught to one copy of a pair.
        // One function, two callers, nothing to keep in step.
        //
        // The two facts it folds in: `empty_turns` at the ceiling releases a
        // wedged session, and his Esc wins over autonomy always — the grant
        // covers his ABSENCE, and an interrupt is the loudest possible evidence
        // that he is present.
        if !rule_b_engaged(facts) {
            return StopVerdict::Allow;
        }
        // THE REVIEW CEILING IS GONE FROM HERE (v4.1.0), and so is the early
        // `Reserved` return that used to sit above it.
        //
        // The ceiling blocked with a message and no exit — both fresh reviews
        // of v4.0.2 named it as the reason a stuck review was unbounded — and
        // its job is now the judge's: the seat counts the refusals itself when
        // it reads the transcript, and past his cap it takes the architect's
        // position on the dispute (fixable → name the fix; not → reserved).
        // Harald's three situations, 2026-09-03; the third is exactly this.
        //
        // The early return was a second reader of the same fact as the match
        // below — the v4.0.1 defect's shape, in the file that defect was about.
        // One reader now.
        //
        // THE JUDGE DECIDES, NOT THE AGENT'S OWN WORDING (Harald, 2026-09-03).
        //
        // This read `declares_a_decision`: a line beginning `DECISION:`, which
        // replaced a 42-phrase substring list after that list slept a session
        // for 21 minutes. The declaration was better than the guess and it was
        // still the agent's own text, so it failed the same way one level up —
        // an agent trained to end turns well writes the mandated format on the
        // first attempt. Measured that night: a release ask at a stage the plan
        // schedules the release five stages later, and a "checkpoint refused"
        // whose findings the agent had already fixed and simply not re-run.
        // Both correctly formatted, both allowed, neither a decision. His
        // words: *"YOU WANT TO STOP ALL THE TIME AND ARE TRAINED ON THE QUICK
        // RESULT."*
        //
        // A rule the agent can satisfy by phrasing is not a rule, so the
        // exemption moves onto a fact it does not author: a fresh-context seat
        // reads the transcript ITSELF — the plan's reserved-decision list, the
        // gate results, the checkpoint reached — and answers one question. The
        // verdict is taken from the harness's tool-result record, so the agent
        // cannot supply it by writing the line in its own prose.
        //
        // Note what is NOT claimed. The transcript is writable by the uid the
        // agent runs as, so a forged result line would pass. That is a
        // different act from phrasing a stop to fit a rule — it is not the
        // failure that has ever occurred here — and the honest bound is the one
        // this module's own header already draws for Rule A.
        match facts.turn.live_verdict() {
            Some(JudgeVerdict::Reserved) => return StopVerdict::Allow,
            Some(JudgeVerdict::Resolvable(next)) => {
                // UNSPENT: the agent was told what to do and has not done it.
                // This is the C3 stop — "I can fix" followed by not fixing —
                // and the message does not change until work happens. Nor
                // does re-asking: a fresh verdict is only accepted once this
                // one is spent, so the judge cannot be re-rolled from here.
                return StopVerdict::Block {
                    reason: format!(
                        "THE JUDGE SAYS THIS IS YOURS TO RESOLVE, not his. It read the \
plan and the transcript with no session context and found nothing reserved here. Its next \
action: {next}\n\nDo that. Nothing else ends this turn: not re-asking the judge, not \
idling — the next verdict is read only after you have worked. If you believe it is wrong, \
do the part you can and say what you cannot; the judge will see both."
                    ),
                }
            }
            None if facts.turn.verdict_spent => {
                // SPENT: work happened since the last verdict, so it no longer
                // describes the situation. Reserved or resolvable, it is stale
                // in either direction — a reservation from before the fix must
                // not end the turn any more than a stale instruction should
                // hold it. Consult again; the judge will see the work.
                return StopVerdict::Block {
                    reason: format!(
                        "THE JUDGE'S LAST VERDICT HAS BEEN ACTED ON, so it no longer \
governs this stop. Spawn the `{JUDGE_SEAT}` subagent again with the same one line — the \
transcript path — and it will judge what you did."
                    ),
                }
            }
            None if facts.turn.judge_ran() => {
                return StopVerdict::Block {
                    reason: format!(
                        "THE {JUDGE_SEAT} SEAT RAN AND RETURNED NO VERDICT. Its answer \
must end with a line of its own reading `VERDICT: RESERVED` or `VERDICT: RESOLVABLE — \
<next action>`; nothing else in it is read. Run it again and pass the verdict through, or \
say what stopped it from answering."
                    ),
                }
            }
            None => {
                return StopVerdict::Block {
                    reason: format!(
                        "RULE B: autonomy is granted and this turn armed no background \
work, so ending here sleeps until he returns. Whether that is his call is not yours to \
decide — spawn the `{JUDGE_SEAT}` subagent and give it ONE line, the transcript this \
session is writing:\n\n    TRANSCRIPT: <this session's transcript path>\n\nIt reads the \
plan and the facts itself; do not summarise them for it, and do not argue your case. Its \
verdict decides: RESERVED lets this stop through, RESOLVABLE names what you do next."
                    ),
                }
            }
        }
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
/// Is this user line the client echoing OUR OWN blocked-stop reason back?
///
/// The wrapper is the client's, not ours, which is why it is matched rather
/// than the reason text: the human quotes the reason back in conversation.
/// Did the human INTERRUPT — Esc, or stopping a tool mid-flight?
///
/// The harness writes these markers into the transcript itself, which is what
/// makes them trustworthy here: the agent cannot write one to escape a gate and
/// cannot omit one to keep a session running. Both spellings are matched because
/// they are two different acts of the same key — stopping the reply, and
/// stopping a tool that was already running.
fn is_interruption(v: &serde_json::Value) -> bool {
    let t = user_text(v);
    t.contains("[Request interrupted by user]")
        || t.contains("[Request interrupted by user for tool use]")
}

fn is_our_own_bounce(v: &serde_json::Value) -> bool {
    user_text(v).trim_start().starts_with("Stop hook feedback")
}

/// Is this user line the HARNESS speaking — a background-task notification, a
/// system reminder, or a local-command echo — rather than the human?
///
/// THE SIX-HOUR SLEEP (2026-08-29, measured in the silence log and the live
/// transcript). Task notifications arrive as ordinary `type:"user"` lines, and
/// an agent-completion notification embeds the agent's whole report — text
/// that virtually always contains a question mark or one of the imperative
/// tokens (`what `, `how `, `check `). So `user_asked` came back TRUE for a
/// window the human never opened, which switched OFF both the review rule and
/// Rule B's push at once: a self-initiated "DECISION:" ask ended the turn at
/// 03:01:23 as `stop-allowed`, unreviewed, and the session slept until the
/// human returned at 09:02. The cross-check that pins the mechanism: the three
/// Rule B pushes that DID fire that night all sat in windows opened by BASH
/// notifications, whose one-line summaries carry no question shape.
///
/// So the harness's own text must never grant the human's exemption. A
/// notification still RESETS the window — it does start a new turn — but it
/// sets `user_asked` never — the question exemption belongs to the keyboard.
///
/// Matched on the wrappers the harness writes at the very start of the text,
/// never on words inside it: the human quoting a notification back at us must
/// keep counting as the human.
/// THE MACHINE'S OWN LINES, tested on the TEXT — one list, both callers.
///
/// Split out of [`is_harness_line`] on 2026-08-30 because the grant code needs
/// exactly this question and could not ask it: `is_harness_line` takes a
/// transcript entry, `note_prompt` has a bare prompt string. The obvious
/// shortcut was to copy the six prefixes into `autonomy.rs`. That is two lists
/// nothing forces to agree, and this session has already been bitten twice by
/// exactly that — Rule B's condition existing in two copies, and a fix landing
/// in one of them. So the list lives here once and both sides call it.
pub fn is_harness_text(t: &str) -> bool {
    // The wrapper words are ASSEMBLED, never written whole — the
    // popping-surface scan in `field` bans the bare word in code, and this is
    // that scan's own idiom for naming what it bans.
    let noti = concat!("notifi", "cation");
    let t = t.trim_start();
    t.starts_with(&format!("<task-{noti}>"))
        || t.starts_with(&format!("[SYSTEM {}", noti.to_uppercase()))
        || t.starts_with("<system-reminder>")
        || t.starts_with("<local-command-caveat>")
        || t.starts_with("<command-name>")
        || t.starts_with("<local-command-stdout>")
}

fn is_harness_line(v: &serde_json::Value) -> bool {
    is_harness_text(&user_text(v))
}

/// Whether this transcript line belongs to a SUBAGENT's own conversation.
///
/// Read per LINE and not per file, deliberately. A main-session transcript can
/// carry sidechain lines too, so a substring test over the whole tail would
/// disable a rule in the session the human is actually typing in — the wrong
/// direction for a guard to fail in. Answering it at the line that OPENS the
/// window keeps the fact scoped to the window it describes.
fn is_sidechain(v: &serde_json::Value) -> bool {
    v.get("isSidechain").and_then(serde_json::Value::as_bool).unwrap_or(false)
}

/// Verdict lines at the START of a line, markdown decoration allowed — the
/// DEGRADED_STAMP principle: reading or discussing a word is not emitting it.
fn verdict_lines(text: &str, verdict: &str) -> usize {
    text.lines()
        .filter(|l| l.trim_start().trim_start_matches(['#', '*', ' ']).starts_with(verdict))
        .count()
}

/// Every string a tool-result line carries, joined so it can be read by LINE.
///
/// The neighbouring turn-around check serializes the whole value and looks for
/// a marker with `contains`, and says why: a result's content is a string on
/// one client and an array of blocks on another. That works for a marker. It
/// does NOT work here, because a verdict is only a verdict when it STARTS a
/// line — the discipline `verdict_lines` exists to enforce — and in a
/// serialized value every newline is the two characters `\` and `n`, so there
/// are no lines left to start.
///
/// So this walks the value and collects the strings, whatever shape they came
/// in, and hands back something with real newlines in it.
fn tool_result_text(v: &serde_json::Value) -> String {
    fn walk(v: &serde_json::Value, out: &mut String) {
        match v {
            serde_json::Value::String(s) => {
                out.push_str(s);
                out.push('\n');
            }
            serde_json::Value::Array(items) => items.iter().for_each(|i| walk(i, out)),
            serde_json::Value::Object(map) => map.values().for_each(|i| walk(i, out)),
            _ => {}
        }
    }
    let mut out = String::new();
    walk(v, &mut out);
    out
}

/// Is this tool-result line the answer to one of `ids`?
///
/// The whole defence of the verdict rests here. `judge_ran` was the previous
/// test and it is not one: it says a judge was spawned SOMEWHERE in the window,
/// so every later tool result inherited the judge's authority — including a
/// file read. Binding to the call's own id makes the answer unforgeable by
/// quotation, because the id is minted by the harness per call.
fn answers_one_of(v: &serde_json::Value, ids: &[String]) -> bool {
    if ids.is_empty() {
        return false;
    }
    v.get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
        .is_some_and(|blocks| {
            blocks.iter().any(|b| {
                b.get("tool_use_id")
                    .and_then(|i| i.as_str())
                    .is_some_and(|id| ids.iter().any(|k| k == id))
            })
        })
}

/// The judge's verdict line, or None when it said nothing decidable.
///
/// THE LAST MATCHING LINE WINS. The seat is told to put its verdict last, and
/// this is why: it reasons in prose first, and prose about a plan that has
/// already produced verdicts quotes them — "the audit's VERDICT: REFUSE was
/// on…", "one could argue VERDICT: RESOLVABLE — but…". The first build took the
/// first match, so a judge that cited a verdict before giving its own was read
/// as having given the cited one. Both fresh reviews of v4.0.2 named it.
///
/// The word is matched as a whole — `RESERVED` followed by end-of-line or
/// whitespace — so `VERDICT: RESERVED is not warranted` and
/// `VERDICT: RESERVEDLY` are not reservations.
fn verdict_in(text: &str) -> Option<JudgeVerdict> {
    let mut found = None;
    for line in text.lines() {
        let l = line.trim_start().trim_start_matches(['#', '*', '>', '-', ' ']);
        if let Some(rest) = l.strip_prefix("VERDICT: RESERVED") {
            if rest.trim().is_empty() {
                found = Some(JudgeVerdict::Reserved);
            }
            continue;
        }
        if let Some(rest) = l.strip_prefix("VERDICT: RESOLVABLE") {
            let boundary_ok = rest.chars().next().is_none_or(|c| !c.is_alphanumeric());
            if boundary_ok {
                let next = rest.trim_start_matches(['—', '-', ':', ' ']).trim();
                found = Some(JudgeVerdict::Resolvable(next.to_string()));
            }
        }
    }
    found
}

/// Is Rule B live for this turn — granted, nothing armed, not interrupted, and
/// the wedge ceiling not yet reached?
///
/// ONE DEFINITION, TWO CALLERS. The rule and the anti-loop valve each held a
/// hand-written copy of this condition, under a comment saying they must agree
/// "character for character". They did, and the comment was still the wrong
/// instrument: this crate shipped a fix taught to one copy of a pair twice in
/// one week, most recently on the streak gate. A shared function cannot drift.
fn rule_b_engaged(facts: &StopFacts) -> bool {
    facts.autonomy == Autonomy::Granted
        && !facts.turn.armed_anything()
        && !facts.turn.interrupted
        // The idle valve — UNLESS the agent is sitting on an instruction it
        // has not acted on. See `Turn::holds_an_unspent_fix`: an agent that
        // has been told what to do and does nothing is not wedged, and the
        // valve that frees a wedged session must not free that one. His
        // interrupt above still wins over everything.
        && (facts.empty_turns < MAX_EMPTY_TURNS || facts.turn.holds_an_unspent_fix())
}

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
            // OUR OWN BOUNCE IS NOT THE HUMAN SPEAKING.
            //
            // The client injects a blocked stop's reason back as a USER turn,
            // and a user turn resets this window — so the communicator call
            // that happened BEFORE the bounce was erased from the window the
            // retry then judged. The gate destroyed the evidence of compliance
            // by the act of demanding it: run the reviewer, get bounced, and
            // the next stop cannot see that you ran it. Measured 2026-08-20 by
            // byte-offset in the live transcript — the bounce lands after the
            // spawn, every time, by construction.
            //
            // Matched on the CLIENT'S OWN WRAPPER at the start of the line, not
            // on our rule text: the human quotes our rule text back at us (he
            // did today), and matching that would let a real message be
            // mistaken for a bounce. The residual risk is the inverse — a human
            // message that genuinely opens with this exact prefix keeps the
            // previous window — which is the safer direction: it can only make
            // the gate see MORE of the turn, never less.
            Some("user") if !is_tool_result(&v) && is_our_own_bounce(&v) => {
                // OUR OWN PUSH RESETS THE TWO PER-ATTEMPT CLOCKS, and only those.
                // The window itself deliberately survives (see above), so the
                // ceiling would otherwise credit the attempt AFTER the push with
                // the work done BEFORE it, never advance, and turn the wedge it
                // guards into an endless push loop.
                turn.worked_since_push = false;
                // AND THE ANSWERED FLAG, measured the very morning v3.17.4
                // shipped without this line: the flag set by one substantial
                // answer stayed up for the REST of the window, so under a
                // standing grant Rule B pushed the agent to work while the
                // guard refused every write — a deadlock built from two fixes
                // that each worked alone. A whole read-only morning of Stage 11
                // ran inside it. The answer that raised the flag was DELIVERED
                // before this push; the push starts a fresh attempt, and the
                // turn-around rule is about answer-then-work inside ONE
                // attempt, not about ever having answered.
                turn.answered_substantially = false;
            }
            // THE HARNESS IS NOT THE HUMAN (the 2026-08-29 six-hour sleep).
            // A task notification opens a new window — the agent is being
            // re-invoked — but it grants none of the human's exemptions:
            // `user_asked` is never set by it, so Rule A still
            // demands the review and Rule B still pushes. See
            // `is_harness_line` for the measured incident.
            // THE REFUSAL IS ONE-SHOT PER WINDOW (v3.17.6). The turn-around
            // guard's denial comes back to the agent as a TOOL RESULT carrying
            // TURNAROUND_MARKER, and seeing it clears the answered flag — so
            // the NEXT attempt proceeds, on the record.
            //
            // Why a speed bump and not a wall, measured within an hour of
            // v3.17.5 shipping: he dispatched work whose own protocol requires
            // a list BEFORE the writes (/memorize's step 1). The list was a
            // 600+ character answer, so every write after it was refused for
            // the REST of the window — and the only reset lived on the stop
            // gate's push, which cannot come while his typing holds the grant
            // off. "Over-firing costs a reorder" was the design claim; mid-
            // window a reorder is impossible, so the real cost was a deadlock
            // between the dispatch and the guard. One denial interrupts the
            // turn-around, names the rule, and leaves an auditable line;
            // repeating it buys no compliance that the first refusal did not,
            // only the deadlock.
            Some("user") if is_tool_result(&v) => {
                // Serialized-whole on purpose: a tool result's content can be a
                // string or an array of blocks depending on the client, and the
                // marker contains nothing JSON-escaping rewrites.
                if v.to_string().contains(TURNAROUND_MARKER) {
                    turn.answered_substantially = false;
                }
                // This arm SHADOWS the catch-all `Some("user")` below, which
                // counts degraded stamps out of exactly these tool-result
                // lines — the first build of this arm swallowed them and four
                // degraded-rule tests went red at once, which is the shadowing
                // announcing itself. Both jobs, one arm.
                turn.degraded_consumed += degraded_stamps_in(&v);
                // THE JUDGE'S ANSWER, and it is read HERE rather than from the
                // agent's own text on purpose — see `Turn::judge_verdict`.
                //
                // Gated on `judge_ran` because the reader walks the window
                // forward: the spawn is already recorded when its result
                // arrives, so this is simply "a result that followed a judge
                // launch". Without the gate any tool result quoting the verdict
                // line would count — reading this very file, for instance, which
                // contains both spellings.
                // A FRESH VERDICT IS ACCEPTED ONLY WHEN THE OLD ONE IS SPENT.
                // With no verdict, or with work done since the last, the new
                // answer governs. With an unspent verdict standing, a new one
                // is a re-roll — the agent asked again without doing anything
                // — and it is ignored. Re-spawning the judge cannot change what
                // the agent was told; only doing it can.
                if answers_one_of(&v, &turn.judge_call_ids)
                    && (turn.judge_verdict.is_none() || turn.verdict_spent)
                {
                    if let Some(verdict) = verdict_in(&tool_result_text(&v)) {
                        turn.judge_verdict = Some(verdict);
                        turn.verdict_spent = false;
                    }
                }
            }
            Some("user") if !is_tool_result(&v) && is_harness_line(&v) => {
                turn = Turn::default();
                turn.sidechain = is_sidechain(&v);
            }
            Some("user") if !is_tool_result(&v) && is_interruption(&v) => {
                // An interrupt is the human, so it opens a new window like any
                // other turn of his — but it also STAMPS that window, because
                // the fact that matters is not what he typed, it is that he
                // reached for the key. Read from his own text: the harness
                // writes the marker, so the agent cannot fake it and cannot
                // suppress it.
                turn = Turn::default();
                turn.interrupted = true;
                turn.human_window = true;
                turn.sidechain = is_sidechain(&v);
            }
            Some("user") if !is_tool_result(&v) => {
                turn = Turn::default();
                turn.human_window = true;
                turn.sidechain = is_sidechain(&v);
                // studio#11: remember whether the human ASKED. Everything after
                // this line is a REPLY, and a reply is out of the ask gate's
                // scope by the 2026-08-07 ruling.
                turn.user_asked = user_asked(&user_text(&v));
                // studio#18: a seat is INVOKED by the human typing its command
                // — so read it from the human's own text, at the line level.
                // Scanning the raw transcript for the bare token counted every
                // sentence that merely mentioned a seat as a run of it.
                turn.seats_invoked = seats_in(&user_text(&v));
            }
            // The harness echoing our own tool call back. It does NOT reset the
            // window (see `is_tool_result`), and it is the only place a
            // degradation stamp can legitimately arrive.
            Some("user") => turn.degraded_consumed += degraded_stamps_in(&v),
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
                                // ORDER IS THE SIGNAL, and this is the only place
                                // it can be read: the reader walks the window
                                // forward, so "a substantial answer came first"
                                // is simply "this flag is already set when the
                                // next tool call arrives". Nothing later in the
                                // pipeline can recover the ordering.
                                if t.chars().count() >= ANSWER_LENGTH {
                                    turn.answered_substantially = true;
                                }
                                // Ordered, not merely collected: `read_turn`
                                // walks the window forward, so "after the
                                // stamp" is simply "while the counter is
                                // non-zero".
                                if turn.degraded_consumed > 0 {
                                    turn.narration.push_str(t);
                                    turn.narration.push('\n');
                                }
                                // Count refusals the AGENT EMITTED, from
                                // assistant text only — never the raw window,
                                // which made the script fire on any session
                                // that merely READ the word.
                                //
                                // AND ONLY AS VERDICT LINES (2026-08-29): the
                                // substring form fired on a turn that merely
                                // DISCUSSED refusals — quoting the round cap,
                                // the auditor's verdicts, the user's own
                                // ruling — three mentions in a workflow
                                // conversation read as three audit rounds and
                                // the gate demanded an architect run against a
                                // fix loop that did not exist. The same file
                                // already states the cure, on DEGRADED_STAMP:
                                // match at the start of a line, never anywhere
                                // in the text. A relayed audit verdict is
                                // "VERDICT: REFUSE", optionally behind
                                // markdown heading or bold markers; prose
                                // about refusing never starts a line that way.
                                turn.refusals_emitted += verdict_lines(t, "VERDICT: REFUSE");
                                if verdict_lines(t, "VERDICT: SIGN-OFF") > 0 {
                                    turn.signoff_emitted = true;
                                }
                            }
                        }
                        Some("tool_use") => {
                            let name = b
                                .get("name")
                                .and_then(|n| n.as_str())
                                .unwrap_or_default()
                                .to_string();
                            let input = b.get("input");
                            // Read from the call's own `file_path`, because a
                            // tool NAME cannot carry this: writing a story and
                            // editing a sprint doc are the same tool. WHICH of
                            // the two it was is decided later, by the store,
                            // from where the file actually sits.
                            if matches!(name.as_str(), "Write" | "Edit" | "NotebookEdit")
                                && input
                                    .and_then(|i| i.get("file_path"))
                                    .and_then(|p| p.as_str())
                                    .is_some_and(|p| p.ends_with(".md"))
                            {
                                turn.wrote_markdown = true;
                            }
                            let launch = ToolUse {
                                subagent: input
                                    .and_then(|i| i.get("subagent_type"))
                                    // A SendMessage names its recipient in `to`
                                    // rather than `subagent_type`; both answer
                                    // the same question — WHICH agent is this.
                                    .or_else(|| {
                                        if name == "SendMessage" { input.and_then(|i| i.get("to")) } else { None }
                                    })
                                    .and_then(|s| s.as_str())
                                    .map(str::to_string),
                                name,
                                backgrounded: input
                                    .and_then(|i| i.get("run_in_background"))
                                    .and_then(serde_json::Value::as_bool)
                                    .unwrap_or(false),
                            };
                            // ANY tool call is work — an edit, a build, a commit.
                            // The old measure was "started a background job",
                            // which made a turn of real work read as empty.
                            //
                            // EXCEPT THE COMMUNICATOR, for the same reason
                            // `arms_work` excepts it: it judges the message
                            // being sent, it is not work that continues. Counted
                            // as work it resets the emptiness ceiling — and a
                            // session that must consult the communicator to say
                            // "I am done" then never LOOKS idle, so the ceiling
                            // that should release it never advances. Measured
                            // 2026-08-31: a finished session needed one
                            // communicator pass and three further pushes to be
                            // let go, where two should have ended it.
                            // AND NOT THE JUDGE, for the same reason and one
                            // sharper: the empty-turn ceiling is Rule B's only
                            // release valve, and Rule B is the rule that
                            // DEMANDS the judge. Counted as work, consulting it
                            // would reset the ceiling that exists to let a
                            // wedged session go, so a session stuck in the
                            // judge loop could never be released — the rule
                            // arguing itself into a corner it built.
                            if !launch.is_communicator() && !launch.is_autocontinue_judge() {
                                turn.worked_since_push = true;
                                // WORK SPENDS THE VERDICT — see `Turn::verdict_spent`.
                                // Any real tool call after a verdict is the agent
                                // acting on it (or on something), and the next
                                // stop must be judged against what that work
                                // produced, not against the instruction that
                                // preceded it.
                                if turn.judge_verdict.is_some() {
                                    turn.verdict_spent = true;
                                }
                            }
                            if launch.is_autocontinue_judge() {
                                // The id is what binds the answer to the ask. A
                                // spawn without one is not a run — `judge_ran`
                                // derives from this list on purpose.
                                if let Some(id) = b.get("id").and_then(|i| i.as_str()) {
                                    turn.judge_call_ids.push(id.to_string());
                                }
                            }
                            turn.launches.push(launch);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    turn.asks_the_human = asks_the_human(&turn.final_text);
    turn.declares_a_decision = declares_a_decision(&turn.final_text);
    // (Seats are read from the human's own message where the window opens —
    // see `seats_in`. A `Skill` launch counts too: an agent may invoke a seat
    // on its own.)
    if turn.launches.iter().any(|l| l.name == "Skill") {
        for seat in SEATS {
            if !turn.seats_invoked.iter().any(|s| s == seat) {
                turn.seats_invoked.push((*seat).to_string());
            }
        }
    }
    turn.gate_ran = ["compile_workspace", "run_tests", "get_diagnostics", "cargo test"]
        .iter()
        .any(|g| transcript_text.contains(g));
    // studio#18, half two: WHAT OWES A CODE GATE IS A CODE CHANGE, not a seat.
    // /report changes no code — it turns a recorded error shape into an issue
    // — so demanding compile_workspace of it made a correct run declare the
    // gate "inapplicable", which teaches agents to narrate around gates. The
    // obligation now follows the edit.
    turn.changed_code = turn.launches.iter().any(|l| {
        matches!(l.name.as_str(),
            "Edit" | "Write" | "NotebookEdit"
            | "rename_symbol" | "extract" | "inline" | "move" | "move_method"
            | "move_in_hierarchy" | "change_method_signature" | "generate"
            | "organize_imports" | "apply_cleanup" | "apply_null_annotations"
            | "refactor_to_pattern" | "replace_duplicates" | "refactoring"
            | "encapsulate_field" | "quick_fix" | "format")
    });
    Ok(turn)
}

/// The seat commands the discipline gate knows.
const SEATS: &[&str] =
    &["/refactor", "/cover", "/javadocs", "/debug", "/profile", "/report"];

/// Seats INVOKED by this text — a slash command is typed, so it opens a line.
///
/// studio#18: the old scan matched the bare token anywhere in the whole
/// transcript, so a turn that merely WROTE about the seats (a report, a plan,
/// a comment naming them) counted as having run them, and was then asked for
/// gates it never owed.
fn seats_in(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let l = line.trim_start();
        for seat in SEATS {
            if l.starts_with(seat)
                && l[seat.len()..].chars().next().is_none_or(|c| c.is_whitespace())
                && !out.iter().any(|s: &String| s == seat)
            {
                out.push((*seat).to_string());
            }
        }
    }
    out
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
/// The human's message text, whichever shape the client wrote it in.
///
/// Claude Code writes `message.content` as a string for a typed prompt and as
/// an array of blocks when the prompt carries attachments; Cursor writes the
/// string form. Both are read here, because a missed user message would silently
/// turn every reply back into a gated ask.
fn user_text(entry: &serde_json::Value) -> String {
    let content = entry.get("message").and_then(|m| m.get("content"));
    match content {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Did the human ASK for something in this message (studio#11)?
///
/// A question mark is the reliable half. The imperatives cover the asks he
/// writes without one — "Discuss!", "Explain", "Tell me what the problem is" —
/// which are requests for an answer just as much as a question is, and a reply
/// to them is equally out of the gate's scope.
///
/// Deliberately NARROW. Over-matching here would exempt real self-initiated asks
/// (the gate's whole purpose), so anything not recognised keeps the gate ON: the
/// failure direction is a needless communicator run, never a silent bypass.
fn user_asked(text: &str) -> bool {
    if text.contains('?') {
        return true;
    }
    let u = text.to_uppercase();
    const IMPERATIVES: &[&str] = &[
        "DISCUSS", "EXPLAIN", "TELL ME", "WHAT ABOUT", "ANALYSE", "ANALYZE",
        "WHY ", "HOW ", "WHICH ", "WHAT ", "GIVE ME", "SHOW ME", "CHECK ",
        "COMPARE", "OPINION", "ADVISE", "ADVICE", "THOUGHTS",
    ];
    IMPERATIVES.iter().any(|p| u.contains(p))
}

/// THE DECLARED ASK — a `DECISION:` line of its own, and nothing else.
///
/// This is the ONLY thing that may stand the autonomy push down, and the only
/// reason distinct from his Esc. It is deliberately not a heuristic: it matches
/// a line the agent wrote on purpose, in the form his upward contract already
/// mandates, so it cannot fire on prose that merely resembles an ask.
///
/// Kept separate from [`asks_the_human`] rather than folded into it, because
/// the two now answer different questions and pay different prices. That one
/// asks "might this be an ask?" and its cost is a communicator review — a
/// false positive there is cheap. This one asks "did the agent declare that it
/// is stopping?" and its cost is the session's evening.
fn declares_a_decision(text: &str) -> bool {
    text.lines().any(|l| {
        let t = l.trim_start().trim_start_matches(['#', '*', '>', ' ']).to_uppercase();
        t.starts_with("DECISION:") || t.starts_with("DECISION ")
    })
}

fn asks_the_human(text: &str) -> bool {
    let u = text.to_uppercase();
    // The CANONICAL form: markdown emphasis, quotes and dashes flattened to
    // single spaces. Every phrase below is matched against this, because the
    // live misses of 2026-08-27 (studio#33) were not missing WORDS — they were
    // punctuation: `say **go**` does not contain "SAY GO" as a substring, and
    // an em-dash broke "yours to confirm" the same way. Matching the raw text
    // made the rule depend on the agent's formatting habits.
    let canon: String = u
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    // Explicit requests for a ruling — plus the imperative shapes measured
    // live on 2026-08-27, each of which Rule B bounced straight past:
    // "say go and I run exactly that sequence", "reseed is yours to confirm",
    // "still blocked on you", "waiting on you: the toggle", and the very
    // sentence Rule B itself instructs — "blocked on the human".
    const PHRASES: &[&str] = &[
        "YOUR WORD", "NEEDS YOUR", "NEED YOUR", "YOUR CALL", "YOUR RULING",
        "YOUR SIGN OFF", "YOUR DECISION", "SHALL I", "WANT ME TO", "DO YOU WANT",
        // "DECISION" is matched as its own WORD below, never as a substring:
        // "design decisions", "the decision was made", "a decision path" are
        // ordinary engineering prose and tripped this rule repeatedly on
        // 2026-08-29 — and a false positive here does more than cost a bounce,
        // it disables Rule B's push on the retry (see the valve in `judge`).
        "MAY I", "LET ME KNOW", "UP TO YOU", "YOU DECIDE",
        "YOU CHOOSE", "IF YOU D RATHER", "IF YOU PREFER", "SAY THE WORD",
        "ON YOUR WORD", "AWAITING", "AWAIT YOUR", "SHOULD I", "WOULD YOU LIKE",
        "PREFER THAT I", "SAY GO", "SAY YES", "YOUR GO", "YOURS TO CONFIRM",
        "YOURS TO", "BLOCKED ON YOU", "BLOCKED ON THE HUMAN", "WAITING ON YOU",
        "WAITING ON YOUR", "WAITING FOR YOU", "WAITING FOR YOUR",
        "YOUR APPROVAL", "APPROVE ME", "YOUR CLICK", "GIVE THE WORD",
        "THE WORD IS YOURS", "ON YOUR YES", "YOUR CONFIRM",
    ];
    if PHRASES.iter().any(|p| canon.contains(p)) {
        return true;
    }
    // THE AGENT'S OWN ASK FORMAT, always caught and checked FIRST. The upward
    // contract says a decision ask opens with `DECISION:` on its own line, so a
    // line starting that way is an ask whatever else the text contains — no
    // pronoun test, no question mark needed.
    if text.lines().any(|l| {
        let t = l.trim_start().trim_start_matches(['#', '*', ' ']).to_uppercase();
        t.starts_with("DECISION:") || t.starts_with("DECISION ")
    }) {
        return true;
    }
    // WORD-BOUNDED, not substring. "design decisions" inside a sentence about
    // engineering is not an ask. The canonical form has already flattened
    // punctuation to single spaces, so word matching is a token walk.
    const BARE_WORDS: &[&str] = &["DECISION", "DECIDE", "RULING", "APPROVAL"];
    if canon.split(' ').any(|w| BARE_WORDS.contains(&w)) {
        // ...but only when the sentence is ABOUT the reader. "a decision path"
        // and "the arms make no decision" are descriptions; "your decision" and
        // "I need a decision" are asks. The pronoun is what separates them.
        const ADDRESSED: &[&str] = &[
            "YOUR", "YOU", "I NEED", "WE NEED", "NEEDS A", "AWAIT", "AWAITING",
            "PENDING", "BLOCKED",
        ];
        if ADDRESSED.iter().any(|p| canon.contains(p)) {
            return true;
        }
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

/// Count degradation stamps in one `user` entry's tool results.
///
/// A `tool_result` block's `content` is either a bare string or an array of
/// blocks; both shapes are read, because which one arrives depends on the tool
/// and getting it wrong would silently count zero forever.
fn degraded_stamps_in(v: &serde_json::Value) -> usize {
    let Some(blocks) = v
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
    else {
        return 0;
    };
    let mut text = String::new();
    for b in blocks {
        if b.get("type").and_then(|t| t.as_str()) != Some("tool_result") {
            continue;
        }
        match b.get("content") {
            Some(serde_json::Value::String(s)) => text.push_str(s),
            Some(serde_json::Value::Array(inner)) => {
                for part in inner {
                    if let Some(t) = part.get("text").and_then(|t| t.as_str()) {
                        text.push_str(t);
                        text.push('\n');
                    }
                }
            }
            _ => {}
        }
        text.push('\n');
    }
    text.lines()
        .filter(|l| l.trim_start().starts_with(DEGRADED_STAMP))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str) -> ToolUse {
        ToolUse { name: name.into(), subagent: None, backgrounded: false }
    }
    /// The whole verdict path, read out of a transcript rather than assembled
    /// from struct fields — this is where the two halves meet, and either one
    /// alone would pass while the pair was broken.
    #[test]
    fn the_verdict_is_read_from_the_harnesss_record_and_not_from_our_own_prose() {
        // The spawn CARRIES ITS ID, because since v4.0.2 the id is what binds a
        // result to this call — see `Turn::judge_call_ids`.
        let spawn = "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\
\"id\":\"toolu_J\",\"name\":\"Agent\",\"input\":{\"subagent_type\":\"autocontinue\"}}]}}\n";

        // PROOF OF LIFE first: without it the negative cases below would pass
        // over a parser that reads nothing at all.
        let t = read_turn(&format!(
            "{spawn}{}",
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\
\"tool_result\",\"tool_use_id\":\"toolu_J\",\"content\":\"read the plan.\\nVERDICT: RESOLVABLE \
— re-run the two reviews\"}]}}\n"
        ))
        .unwrap();
        assert!(t.judge_ran(), "the spawn must register");
        assert_eq!(
            Some(JudgeVerdict::Resolvable("re-run the two reviews".into())),
            t.judge_verdict,
            "the next action travels with the verdict, or the block cannot name it"
        );

        // THE POINT OF THE WHOLE DESIGN. The same line in the AGENT's own text
        // buys nothing: that is the text it authors, and authoring the exemption
        // is exactly what `declares_a_decision` allowed.
        let ours = read_turn(&format!(
            "{spawn}{}",
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\
\"VERDICT: RESERVED\"}]}}\n"
        ))
        .unwrap();
        assert_eq!(
            None, ours.judge_verdict,
            "writing the verdict ourselves must not satisfy the gate — that is the defect"
        );

        // And a result with no judge behind it is not a verdict either, so a
        // tool result that merely QUOTES the line (reading this very file, say)
        // cannot stand a stop down.
        let unasked = read_turn(
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\
\"tool_result\",\"tool_use_id\":\"x\",\"content\":\"VERDICT: RESERVED\"}]}}\n",
        )
        .unwrap();
        assert_eq!(None, unasked.judge_verdict, "no judge ran, so there is no verdict");

        // THE v4.0.2 DEFECT: a judge DID run, answered nothing, and a LATER
        // tool result quotes the line. Measured against the shipped v4.0.1
        // binary by reading `~/.claude/agents/autocontinue.md` — whose own
        // stance carries `VERDICT: RESERVED` indented by four spaces — and the
        // stop was ALLOWED.
        //
        // Indentation is why the line-start discipline did not save it: the
        // reader trims leading whitespace before matching, and every file that
        // documents this mechanism indents its example. So the binding is to
        // the CALL, not to the text.
        let quoted = read_turn(&format!(
            "{}{}{}",
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\
\"id\":\"toolu_JUDGE\",\"name\":\"Agent\",\"input\":{\"subagent_type\":\"autocontinue\"}}]}}\n",
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\
\"tool_result\",\"tool_use_id\":\"toolu_JUDGE\",\"content\":\"I could not read it.\"}]}}\n",
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\
\"tool_result\",\"tool_use_id\":\"toolu_READ\",\"content\":\"the stance reads:\\n    VERDICT: RESERVED\"}]}}\n",
        ))
        .unwrap();
        assert!(quoted.judge_ran(), "the judge did run");
        assert_eq!(
            None, quoted.judge_verdict,
            "a file read cannot answer for the judge — the verdict binds to the CALL id"
        );

        // ...and the same shape with the id MATCHING is accepted, or the
        // assertion above would pass on a reader that accepts nothing at all.
        let answered = read_turn(&format!(
            "{}{}",
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\
\"id\":\"toolu_JUDGE\",\"name\":\"Agent\",\"input\":{\"subagent_type\":\"autocontinue\"}}]}}\n",
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\
\"tool_result\",\"tool_use_id\":\"toolu_JUDGE\",\"content\":\"    VERDICT: RESERVED\"}]}}\n",
        ))
        .unwrap();
        assert_eq!(
            Some(JudgeVerdict::Reserved),
            answered.judge_verdict,
            "the judge's OWN result is still read, indentation and all"
        );

        // THE LAST LINE WINS (v4.1.0). It was first-match, and both reviews
        // named the failure: a judge that cites an earlier verdict in its
        // reasoning was read as having given the cited one. The seat is told
        // to put its verdict last; this is the reader honouring that.
        assert_eq!(
            Some(JudgeVerdict::Resolvable("go on".into())),
            verdict_in("the audit's VERDICT: RESERVED was premature.\nVERDICT: RESOLVABLE — go on"),
        );
        // And a whole word: a reservation that is being argued AGAINST is not one.
        assert_eq!(None, verdict_in("VERDICT: RESERVED is not warranted here"));
        assert_eq!(None, verdict_in("VERDICT: RESERVEDLY"));
        // Bulleted and quoted forms parse; a judge that lists its conclusion
        // must not be read as having said nothing.
        assert_eq!(Some(JudgeVerdict::Reserved), verdict_in("- VERDICT: RESERVED"));
        assert_eq!(Some(JudgeVerdict::Reserved), verdict_in("> VERDICT: RESERVED"));
    }

    /// The judge is not work and does not arm — the two carve-outs that keep
    /// Rule B from arguing itself into a corner, since consulting the judge
    /// would otherwise both satisfy the rule and reset its release valve.
    #[test]
    fn the_judge_neither_arms_work_nor_counts_as_it() {
        let j = ToolUse {
            name: "Agent".into(),
            subagent: Some(JUDGE_SEAT.into()),
            backgrounded: false,
        };
        assert!(j.is_autocontinue_judge());
        assert!(!j.arms_work(), "judging the turn is not work that continues after it");
        let t = read_turn(
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\
\"name\":\"Agent\",\"input\":{\"subagent_type\":\"autocontinue\"}}]}}\n",
        )
        .unwrap();
        assert!(
            !t.worked_since_push,
            "counted as work it would reset the empty-turn ceiling, and a session \
stuck in the judge loop could never be released"
        );
    }

    /// THE v4.0.1 DEFECT, as the discriminator that would have caught it.
    ///
    /// v4.0.0 retired the reviewer rule and left one reader: the anti-loop
    /// valve still called `owes_a_review()`, so on a retry the retired subagent
    /// still decided a verdict. Nothing covered that path, because every test
    /// of the retirement looked at the RULE and the valve is a different site.
    ///
    /// Found by dogfooding the shipped binary rather than by a test, which is
    /// the whole argument for the dogfood stage: one fixture, two runs
    /// differing only by the communicator call, and they disagreed.
    ///
    /// The shape to hold: on ANY path, spawning the retired reviewer must not
    /// change the verdict. Restore the valve term and this goes red.
    #[test]
    fn the_retired_reviewer_cannot_change_a_verdict_on_any_path() {
        let long_ask = "Stage 3 is committed and every gate is green. ".repeat(60);
        let mut without = facts(Autonomy::Unknown, vec![]);
        without.turn.final_text = long_ask.clone();
        without.turn.asks_the_human = true;
        without.already_bounced = true;

        let mut with = facts(Autonomy::Unknown, vec![communicator()]);
        with.turn.final_text = long_ask;
        with.turn.asks_the_human = true;
        with.already_bounced = true;

        assert_eq!(
            judge(&without),
            judge(&with),
            "spawning the retired reviewer must not release a message the gate would \
otherwise hold — this is the v4.0.0 defect, measured against the shipped binary"
        );
        // PROOF OF LIFE, and it has to be taken OFF the retry path.
        //
        // The equality above is now two Allows: with the valve's reviewer term
        // gone, a retry that owes no reseed and would not be pushed is released
        // outright, which is exactly what the valve is for. So the equality
        // cannot also demonstrate that the fixture is block-worthy — that is
        // shown here, on the same message with the retry flag down, where the
        // length rule does reach it.
        //
        // Before the fix these two paths disagreed: the valve fell through, the
        // length rule fired, and spawning the reviewer was what turned the block
        // into an Allow.
        let mut first_pass = facts(Autonomy::Unknown, vec![]);
        first_pass.turn.final_text = "Stage 3 is committed and every gate is green. ".repeat(60);
        first_pass.turn.asks_the_human = true;
        match judge(&first_pass) {
            StopVerdict::Block { reason } => assert!(reason.contains("TOO LONG"), "{reason}"),
            StopVerdict::Allow => {
                panic!("the fixture must be over budget, or the equality proves nothing")
            }
        }
        // ...and the reviewer does not change THAT verdict either.
        let mut first_pass_judged = facts(Autonomy::Unknown, vec![communicator()]);
        first_pass_judged.turn.final_text =
            "Stage 3 is committed and every gate is green. ".repeat(60);
        first_pass_judged.turn.asks_the_human = true;
        assert_eq!(
            judge(&first_pass),
            judge(&first_pass_judged),
            "and length is cut, never consulted away"
        );
    }

    fn communicator() -> ToolUse {
        ToolUse { name: "Agent".into(), subagent: Some("communicator".into()), backgrounded: false }
    }
    fn facts(autonomy: Autonomy, launches: Vec<ToolUse>) -> StopFacts {
        StopFacts {
            empty_turns: 0,
            review_rounds: 0,
            already_bounced: false,
            bounces: 0,
            // Mirrors reality rather than defaulting: a turn carrying tool calls
            // HAS worked. A helper that always said `false` would let a test pass
            // against a fixture that could not occur.
            turn: Turn { final_text: "done".into(), worked_since_push: !launches.is_empty(), launches, refusals_emitted: 0, asks_the_human: false, declares_a_decision: false, judge_verdict: None, judge_call_ids: vec![], verdict_spent: false, user_asked: false, human_window: false, sidechain: false, signoff_emitted: false, interrupted: false, narration: String::new(), degraded_consumed: 0, seats_invoked: vec![], gate_ran: true, changed_code: false, wrote_markdown: false, answered_substantially: false },
            autonomy,
            substrate: None,
            reseed_bounces: 0,
        }
    }

    // -----------------------------------------------------------------
    // THE SUBSTRATE RULE — a story written and never reseeded in
    //
    // The failure, 2026-08-27: four stories authored, cold-read, stamped,
    // committed and reported as remembered, while the store held none of
    // them. The first cure was a warning inside `stats` — a report on a
    // channel nobody has to read, which cannot catch a step that was never
    // called. These tests are about the CHANNEL, so each one drives `judge`
    // and asserts a VERDICT rather than a message.
    // -----------------------------------------------------------------

    fn wrote_a_story(count: usize) -> StopFacts {
        let mut f = facts(Autonomy::Unknown, vec![]);
        f.turn.wrote_markdown = true;
        f.substrate = Some(SubstrateDrift {
            root: "/home/h/knowledge/stories".into(),
            count,
            named: vec!["the-cure-was-a-report.md".into()],
        });
        f
    }

    #[test]
    fn a_story_written_and_never_reseeded_holds_the_turn() {
        let StopVerdict::Block { reason } = judge(&wrote_a_story(1)) else {
            panic!(
                "the turn ended with a story on disk that the store does not have \u{2014} \
                 the exact shape that was reported as remembered"
            );
        };
        assert!(reason.contains("the-cure-was-a-report.md"), "{reason}");
        assert!(
            reason.contains("kind=reseed") && reason.contains("/home/h/knowledge/stories"),
            "the hold must carry the CURE, with the store's own root: {reason}"
        );
        assert!(
            reason.contains("skipped"),
            "and must name the refusal case \u{2014} a reseed admits stamped stories \
             only, and a silent skip is how the first cure failed: {reason}"
        );
    }

    #[test]
    fn the_hold_names_itself_so_its_counter_cannot_be_miswired() {
        // The pipeline charges the reseed counter by this prefix. If the two
        // drift apart, the counter is charged to the wrong rule — or never —
        // and both failures are silent.
        let StopVerdict::Block { reason } = judge(&wrote_a_story(1)) else {
            panic!("must block")
        };
        assert!(reason.starts_with(UNSTORED_STORY), "{reason}");
    }

    #[test]
    fn a_turn_held_by_another_rule_does_not_spend_a_reseed_chance() {
        // An audit-fix loop outranks this rule and blocks first. The story is
        // still unstored, and the chance to fix it must survive.
        let mut f = wrote_a_story(1);
        f.turn.refusals_emitted = 3;
        let StopVerdict::Block { reason } = judge(&f) else { panic!("must block") };
        assert!(
            !reason.starts_with(UNSTORED_STORY),
            "the higher rule must win, or the precedence this test pins is wrong: {reason}"
        );
    }

    #[test]
    fn a_turn_that_wrote_nothing_is_not_held_by_a_draft_someone_left() {
        // Drift alone is not this turn's business. A half-written story under
        // the root that has not earned its stamp would otherwise hold every
        // turn of every session, forever, for a file nobody meant to store.
        let mut f = wrote_a_story(3);
        f.turn.wrote_markdown = false;
        assert_eq!(StopVerdict::Allow, judge(&f));
    }

    #[test]
    fn a_store_that_cannot_answer_holds_nothing() {
        // `None` is "I could not ask", and must not be read as either answer.
        // Holding on it would wedge every session whose resident is down;
        // treating it as clean would be this rule's own defect, pointed at the
        // store instead of the files.
        let mut f = wrote_a_story(1);
        f.substrate = None;
        assert_eq!(StopVerdict::Allow, judge(&f));
    }

    #[test]
    fn a_store_reporting_no_drift_holds_nothing() {
        assert_eq!(StopVerdict::Allow, judge(&wrote_a_story(0)));
    }

    #[test]
    fn the_retry_does_not_amnesty_an_unstored_story() {
        // The anti-loop valve lets everything through on the retry. A rule that
        // can be walked past by doing nothing is not a gate (measured on the
        // review rule: block at 15:53:52, allowed 0.8 s later) \u{2014} and here
        // doing nothing IS the failure being caught.
        let mut f = wrote_a_story(1);
        f.already_bounced = true;
        assert!(matches!(judge(&f), StopVerdict::Block { .. }));
    }

    #[test]
    fn the_hold_gives_up_at_the_ceiling() {
        let mut f = wrote_a_story(1);
        f.reseed_bounces = MAX_RESEED_BOUNCES;
        assert_eq!(
            StopVerdict::Allow,
            judge(&f),
            "past the ceiling the turn is RELEASED \u{2014} an unbounded hold is the wedge"
        );
    }

    #[test]
    fn a_markdown_write_is_seen_and_a_source_edit_is_not() {
        // The authoring half, read from the transcript rather than assumed.
        let md = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Write","input":{"file_path":"/k/stories/live/s10.md"}}]}}"#;
        let rs = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Edit","input":{"file_path":"/src/stop.rs"}}]}}"#;
        assert!(read_turn(md).unwrap().wrote_markdown);
        assert!(
            !read_turn(rs).unwrap().wrote_markdown,
            "an ordinary source edit must not arm a rule about stories"
        );
    }

    // -----------------------------------------------------------------
    // studio#4 — a consumed degradation must be surfaced
    // -----------------------------------------------------------------

    /// One user entry echoing a tool result with the given body.
    fn result_line(body: &str) -> String {
        format!(
            "{{\"type\":\"user\",\"message\":{{\"content\":[{{\"type\":\"tool_result\",\"content\":{}}}]}}}}",
            serde_json::to_string(body).unwrap()
        )
    }
    /// One user line carrying `text` — the human, or our own bounce, depending
    /// on what the text is. Built with `serde_json` rather than an escaped
    /// format string: the panic-guard scans this file's braces, and hand-escaped
    /// JSON fixtures make its scan end mid-skip, which it reports honestly as
    /// "every production line after that point went unexamined".
    fn human_line(text: &str) -> String {
        serde_json::json!({"type": "user", "message": {"content": text}}).to_string()
    }

    /// One assistant line carrying a single `Edit` tool call — foreground work
    /// that arms nothing.
    fn edit_line() -> String {
        serde_json::json!({
            "type": "assistant",
            "message": {"content": [{
                "type": "tool_use",
                "name": "Edit",
                "input": {"file_path": "/x/y.rs"}
            }]}
        })
        .to_string()
    }

    fn assistant_line(text: &str) -> String {
        format!(
            "{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":{}}}]}}}}",
            serde_json::to_string(text).unwrap()
        )
    }
    fn turn_of(result_body: &str, final_text: &str) -> Turn {
        let t = format!(
            "{{\"type\":\"user\",\"message\":{{\"content\":\"go\"}}}}\n{}\n{}\n",
            result_line(result_body),
            assistant_line(final_text)
        );
        read_turn(&t).unwrap()
    }


    /// THE COMMUNICATOR IS NOT WORK, on this counter either. Counted as work it
    /// resets the emptiness ceiling, and a session that must consult the
    /// communicator to say "I am done" then never looks idle — measured
    /// 2026-08-31: a finished session took one communicator pass plus three
    /// further pushes to be released, where two should have ended it.
    #[test]
    fn a_communicator_pass_does_not_reset_the_emptiness_clock() {
        let t = format!(
            "{}\n{}\n{}\n",
            human_line("done?"),
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","input":{"subagent_type":"communicator"}}]}}"#,
            assistant_line("nothing left to do")
        );
        let turn = read_turn(&t).unwrap();
        assert!(
            !turn.worked_since_push,
            "judging the goodbye is not doing work — counting it keeps a finished \
             session un-releasable"
        );
        assert!(
            turn.communicator_ran(),
            "control: the call itself must still be seen, or Rule A breaks"
        );
    }

    /// THE TWO FIELDS DISAGREE ON EXACTLY THE TURN THAT MATTERS, and which one
    /// feeds which decision is the whole of tonight's defect.
    ///
    /// A turn of edits with no background job: `worked_since_push` true,
    /// `armed_anything` false. Rule B fires on the second — correctly, because it
    /// asks "will anything wake this session" and nothing will. `c259d44` then
    /// pointed the CEILING at the first, so on this very turn the rule fired and
    /// the counter reset, and the block became unbounded. Measured in v3.17.3's
    /// own log the evening it shipped: six consecutive `block RULE B`, `empty=0`
    /// on every one.
    ///
    /// This test exists to pin the DISAGREEMENT, so the next person who reaches
    /// for the friendlier-sounding field has to read why.
    #[test]
    fn the_two_work_measures_disagree_on_an_editing_turn() {
        let t = format!(
            "{}\n{}\n{}\n",
            human_line("go"),
            edit_line(),
            assistant_line("edited it")
        );
        let turn = read_turn(&t).unwrap();
        assert!(
            !turn.armed_anything(),
            "an Edit starts no background job, so nothing will wake this session"
        );
        assert!(
            turn.worked_since_push,
            "and yet the turn plainly did work — the two measures differ HERE"
        );
    }

    /// THE CEILING RELEASES, AND IT IS REACHED BY EMPTINESS, NOT BY EFFORT.
    ///
    /// Two halves, and the second is the one this mechanism keeps losing.
    ///
    /// The rule must still push a working turn that armed nothing — that turn
    /// leaves nothing to wake the session, and pushing it is the whole feature.
    /// So the ceiling must NOT be reached by working: it is reached by producing
    /// nothing, twice. An unbounded run of WORKING turns is an unattended session
    /// doing its job; the only loop worth stopping is push, nothing, push,
    /// nothing.
    #[test]
    fn the_ceiling_releases_but_only_emptiness_reaches_it() {
        let mut f = facts(Autonomy::Granted, vec![tool("Edit")]);
        assert!(!f.turn.armed_anything(), "precondition: nothing will wake this");
        assert!(f.turn.worked_since_push, "precondition: but it did work");

        f.empty_turns = MAX_EMPTY_TURNS - 1;
        assert!(
            matches!(judge(&f), StopVerdict::Block { .. }),
            "below the ceiling it must push, or the mechanism does nothing at all"
        );

        f.empty_turns = MAX_EMPTY_TURNS;
        assert!(
            matches!(judge(&f), StopVerdict::Allow),
            "at the ceiling it must let go, or a finished session is pushed forever"
        );

        // AND THE HALF THAT WAS MISSING: this working turn must not ADVANCE the
        // counter toward that ceiling. Reverted to `armed_anything()` on
        // 2026-08-30 and put back the same evening — under that spelling an
        // unattended editing run is released after two turns of real work, which
        // is the two-turn leash, not a wedge guard.
        let empty = facts(Autonomy::Granted, vec![]);
        assert!(
            !empty.turn.worked_since_push,
            "a turn with no calls at all is what the counter is for"
        );
        assert!(
            f.turn.worked_since_push && !empty.turn.worked_since_push,
            concat!(
                "the counter's input must SEPARATE these two turns. Feeding it ",
                "armed_anything() makes them identical — both false — and the ",
                "editing run is then leashed to two turns"
            )
        );
    }

    /// AND THE CLOCK RESTARTS AT THE PUSH — the half that keeps the ceiling real.
    ///
    /// The window deliberately survives our own block (`is_our_own_bounce` keeps
    /// the previous window rather than resetting it), so "any tool call in the
    /// window" would credit the attempt AFTER a push with work done BEFORE it. The
    /// counter would then reset on every push, the ceiling would never be reached,
    /// and the wedge it guards against — pushed, nothing, pushed, nothing — would
    /// run forever. Named by the fresh audit as why the obvious fix is insufficient.
    #[test]
    fn work_before_the_push_does_not_count_for_the_attempt_after_it() {
        // The client's own wrapper, at the START of the line — that is what
        // `is_our_own_bounce` keys on, and a prefixed variant is read as the
        // human, which resets the window instead of keeping it.
        let t = format!(
            "{}\n{}\n{}\n{}\n",
            human_line("go"),
            edit_line(),
            human_line("Stop hook feedback:\nRULE B: autonomy is granted …"),
            assistant_line("here is what I will do next")
        );
        let turn = read_turn(&t).unwrap();
        assert!(
            !turn.launches.is_empty(),
            concat!(
                "the window still holds the pre-push Edit — our own block does not ",
                "reset it, and that is deliberate"
            )
        );
        assert!(
            !turn.worked_since_push,
            concat!(
                "but the attempt AFTER the push produced nothing, so the ceiling ",
                "must advance — otherwise a wedge pushes forever"
            )
        );
    }

    /// B1, from the C5 audit — and it was a LIVE block, not a hypothesis.
    ///
    /// `final_text` holds only the LAST assistant block. An agent that says
    /// "the store answered degraded, so this count is a floor", keeps working,
    /// and ends on "Done." has surfaced it — and was blocked and told it had
    /// not. Ending short after narrating is the ordinary shape of a turn, so
    /// this was not a corner case; it was most of them.
    #[test]
    fn narrating_it_mid_turn_and_ending_short_still_closes_the_rule() {
        let transcript = format!(
            "{{\"type\":\"user\",\"message\":{{\"content\":\"go\"}}}}\n{}\n{}\n{}\n{}\n",
            result_line("DEGRADED: store on the in-memory fallback"),
            assistant_line(
                "Heads up: the store answered DEGRADED, so treat this count as a floor."
            ),
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\
             \"name\":\"Bash\",\"input\":{}}]}}",
            assistant_line("Done.")
        );
        let turn = read_turn(&transcript).unwrap();
        assert_eq!("Done.", turn.final_text, "the last block is still the final message");
        assert_eq!(1, turn.degraded_consumed);
        let mut f = facts(Autonomy::Unknown, vec![communicator()]);
        f.turn = turn;
        f.turn.launches.push(communicator()); // the catch-all rule is not this test's subject
        assert_eq!(
            StopVerdict::Allow,
            judge(&f),
            "it was said out loud; blocking here punishes the agent for not repeating itself"
        );
    }


    /// B1-REGRESSION, from the C5 audit ROUND 2 — a false negative my own fix
    /// for B1 opened, and the auditor proved it by running it.
    ///
    /// Widening from "the last block" to "every block" removed the ordering
    /// constraint entirely, so a mention emitted BEFORE the stamp arrived
    /// satisfied a rule about reporting it AFTER. And "degrad" is ordinary
    /// working vocabulary here — the hook's own source, the audit texts and
    /// the memory files all use it — so an agent working in this repo trips
    /// the exemption by accident, not by malice.
    #[test]
    fn a_mention_before_the_stamp_does_not_excuse_silence_after_it() {
        let transcript = format!(
            "{{\"type\":\"user\",\"message\":{{\"content\":\"go\"}}}}\n{}\n{}\n{}\n",
            assistant_line("I will check for graceful degradation in the retry path."),
            result_line("DEGRADED: store on the in-memory fallback"),
            assistant_line("Three entries. All good.")
        );
        let turn = read_turn(&transcript).unwrap();
        assert_eq!(1, turn.degraded_consumed);
        let mut f = facts(Autonomy::Unknown, vec![]);
        f.turn = turn;
        assert!(
            matches!(judge(&f), StopVerdict::Block { .. }),
            "the word appeared before the answer did; nothing reported the answer"
        );
    }

    /// And the rule still fires when NO block mentions it — otherwise the fix
    /// above would have turned the rule off rather than corrected its input.
    #[test]
    fn a_multi_block_turn_that_never_mentions_it_is_still_blocked() {
        let transcript = format!(
            "{{\"type\":\"user\",\"message\":{{\"content\":\"go\"}}}}\n{}\n{}\n{}\n",
            result_line("DEGRADED: store on the in-memory fallback"),
            assistant_line("Looking at the store now."),
            assistant_line("Three entries. All good.")
        );
        let mut f = facts(Autonomy::Unknown, vec![]);
        f.turn = read_turn(&transcript).unwrap();
        assert!(matches!(judge(&f), StopVerdict::Block { .. }));
    }

    /// N1: the reason is fed back to the MODEL, so its own text must not be
    /// mangled. The first version was a multi-line literal whose continuations
    /// were lost, leaving runs of eighteen spaces mid-sentence.
    #[test]
    fn the_block_reason_is_not_full_of_whitespace_runs() {
        let mut f = facts(Autonomy::Unknown, vec![]);
        f.turn = turn_of("DEGRADED: x", "All good.");
        let StopVerdict::Block { reason } = judge(&f) else { panic!("expected a block") };
        assert!(!reason.contains("   "), "three spaces in a row: {reason:?}");
    }

    /// ACCEPTANCE 1: consumed and unmentioned -> blocked, and the reason tells
    /// the agent what to add.
    #[test]
    fn a_degraded_answer_the_message_hides_blocks_the_turn() {
        let turn = turn_of(
            "DEGRADED: experience store is serving a non-persistent in-memory copy\nresult: 3 entries",
            "Found 3 entries. The store looks fine.",
        );
        assert_eq!(1, turn.degraded_consumed);
        let mut f = facts(Autonomy::Unknown, vec![]);
        f.turn = turn;
        match judge(&f) {
            StopVerdict::Block { reason } => {
                assert!(reason.contains("UNREPORTED DEGRADATION"), "{reason}");
                assert!(reason.to_lowercase().contains("which"), "{reason}");
            }
            v => panic!("expected a block, got {v:?}"),
        }
    }

    /// ACCEPTANCE 2: the same transcript, said out loud -> passes.
    #[test]
    fn saying_it_closes_the_rule() {
        let mut f = facts(Autonomy::Unknown, vec![communicator()]);
        f.turn = turn_of(
            "DEGRADED: experience store is serving a non-persistent in-memory copy",
            "Found 3 entries — but the store answered DEGRADED (in-memory copy), so \
             this count is not the persisted corpus.",
        );
        f.turn.launches.push(communicator()); // the catch-all rule is not this test\'s subject
        assert_eq!(StopVerdict::Allow, judge(&f));
    }

    /// Any form of the word closes it. Demanding the literal stamp would make
    /// the rule satisfiable by pasting a token — the reflex the recall gate's
    /// own design refuses to manufacture.
    #[test]
    fn the_word_not_the_token_is_what_closes_it() {
        let mut f = facts(Autonomy::Unknown, vec![communicator()]);
        f.turn = turn_of(
            "DEGRADED: store on the in-memory fallback",
            "Three entries, but note the degradation: the store is on its in-memory \
             fallback, so treat the number as a floor.",
        );
        f.turn.launches.push(communicator()); // the catch-all rule is not this test\'s subject
        assert_eq!(StopVerdict::Allow, judge(&f));
    }

    /// THE FALSE-POSITIVE GUARD, and it is not hypothetical: jawata's own tree
    /// contains the string in four source files, so a grep or a file read puts
    /// it in a tool result during ordinary work. Blocking there would punish
    /// the agent for having LOOKED AT the notice rather than for having been
    /// handed one — the exact failure the bash gate made with "REFUSE".
    #[test]
    fn merely_reading_the_word_is_not_consuming_a_degraded_answer() {
        let turn = turn_of(
            "RecoveringExperienceStore:245:  \"in-memory (DEGRADED: \" + why\n\
             AbstractAstDetector:203:  \"DEGRADED SCAN: every file was read\"",
            "Two hits, both in source.",
        );
        assert_eq!(
            0, turn.degraded_consumed,
            "a mid-line mention in a grep hit is not a degradation stamp"
        );
        let mut f = facts(Autonomy::Unknown, vec![communicator()]);
        f.turn = turn;
        f.turn.launches.push(communicator()); // the catch-all rule is not this test's subject
        assert_eq!(StopVerdict::Allow, judge(&f));
    }

    /// The stamp arriving in the BLOCK-shaped content array, not as a bare
    /// string. Which shape a client uses is not ours to choose, and reading
    /// only one of them would silently count zero forever.
    #[test]
    fn the_stamp_is_read_from_a_block_array_too() {
        let line = "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"tool_result\",\
                    \"content\":[{\"type\":\"text\",\"text\":\"DEGRADED: compiler index is rebuilding\"}]}]}}";
        let t = format!(
            "{{\"type\":\"user\",\"message\":{{\"content\":\"go\"}}}}\n{line}\n{}\n",
            assistant_line("Done.")
        );
        assert_eq!(1, read_turn(&t).unwrap().degraded_consumed);
    }

    /// THE ANTI-WEDGE VALVE. A rule that can block twice can trap a session,
    /// and this one fires on a condition the agent might not be able to satisfy
    /// (a stamp it never saw in its own context).
    #[test]
    fn a_second_pass_never_blocks_on_degradation() {
        let mut f = facts(Autonomy::Unknown, vec![]);
        f.turn = turn_of("DEGRADED: store unavailable", "All good.");
        // The review pass is the FIXTURE, not the subject: the narrowed valve
        // still holds a retry for a missing review, and this test is about the
        // degradation rule NOT re-firing on the second pass.
        f.turn.launches.push(communicator());
        f.already_bounced = true;
        assert_eq!(StopVerdict::Allow, judge(&f));
    }

    /// THE INCIDENT TEST — jawata-studio v3.12.3, live on Harald's machine.
    ///
    /// The ceiling lived inside the `already_bounced` branch, which assumed
    /// every client marks a retry. Cursor re-invokes with the flag UNSET, so
    /// that branch was never entered, the bound was unreachable, and the rule
    /// blocked forever. His counter reached 11 and was still climbing.
    ///
    /// A client that never sets the flag must still be released.
    #[test]
    fn a_client_that_never_marks_a_retry_is_still_released() {
        // THE RULE THIS MEASURED IS RETIRED (v4.0.0), but its LESSON is not,
        // and that is why this test survives instead of being deleted: a ceiling
        // must sit on the RULE, never inside an `already_bounced` branch, because
        // Cursor re-invokes with the retry flag unset and never enters that
        // branch. Measured live at counter 11 and still climbing.
        //
        // The rule that still carries a per-bounce ceiling is the substrate
        // rule, so the invariant is asserted there now. Here the assertion is
        // the retirement itself: an ask, no reviewer, no autonomy, any bounce
        // count — and nothing holds it.
        let mut f = facts(Autonomy::Unknown, vec![]);
        f.turn.asks_the_human = true;
        f.already_bounced = false; // Cursor: every invocation looks like the first
        for n in 0..=MAX_UNJUDGED_BOUNCES {
            f.bounces = n;
            assert_eq!(
                StopVerdict::Allow,
                judge(&f),
                "bounce {n}: the reviewer is retired, so no count of it can hold a turn"
            );
        }
    }

    /// THE LIVE MISSES OF 2026-08-27 (studio#33), verbatim. Every sentence
    /// below was a real decision ask that Rule B bounced straight past — the
    /// words were there, the punctuation broke the match ("say **go**" does
    /// not contain "SAY GO" as a raw substring). Revert the canonicalisation
    /// or the widened phrases and these go red.
    #[test]
    fn the_live_missed_ask_shapes_are_detected() {
        for s in [
            "Reseed is yours to confirm — say **go** and I run exactly that sequence.",
            "Still blocked on you, same two things: the toggle, and \"go\" for the reseed.",
            "Blocked on the human.",
            "Waiting on you: the toggle, the reseed word, and the hook fix.",
            "Frozen. Waiting on you: the toggle and the go.",
        ] {
            assert!(asks_the_human(s), "missed live 2026-08-27 and must never miss again: {s:?}");
        }
    }

    /// THE 22:54 SLEEP (2026-08-30). The window opened on his work-order
    /// "We had a discussion before … autocontinue". `user_asked` is true
    /// because DISCUSS sits inside "discussion". He was not at the machine.
    /// The grant was on. Nothing was armed. Rule B's own sentence was true
    /// and the hook returned Allow. `user_asked` must not stand the push down.
    #[test]
    fn a_stale_keyboard_line_that_is_not_a_question_does_not_silence_rule_b() {
        let mut f = facts(Autonomy::Granted, vec![tool("Edit"), tool("Bash")]);
        f.turn.user_asked = true;
        f.turn.final_text =
            "Both findings are homed to Sprint 28e. Continuing to S9a.2, the patch-streak gate."
                .into();
        match judge(&f) {
            StopVerdict::Block { reason } => assert!(
                reason.contains("RULE B"),
                "the rule's two facts were true: {reason}"
            ),
            StopVerdict::Allow => panic!(
                "user_asked silenced Rule B on a grant + no armed work — the 22:54 sleep"
            ),
        }
    }

    /// STRANDED MID-WORK CANNOT RECUR (2026-08-29). Measured in this gate's own
    /// log — `emitted, emitted, stop-allowed, stop-allowed` — where the last
    /// released a turn ending on "here is what I will do next" with nothing
    /// armed. Harald: "you are in the middle of nowhere and just stop. This is
    /// even worse than with a hard cut at a checkpoint."
    ///
    /// The chain was: an ask-detector FALSE POSITIVE bounces the turn, the agent
    /// rewrites without the trigger word, and the retry hits the anti-loop valve
    /// — which returned Allow before Rule B was ever evaluated. So one false
    /// positive did not cost a bounce, it disabled the push.
    #[test]
    fn a_retry_still_gets_pushed_when_nothing_is_armed() {
        let mut f = facts(Autonomy::Granted, vec![tool("Edit")]);
        f.already_bounced = true; // the retry after an UNJUDGED MESSAGE bounce
        assert!(!f.turn.armed_anything(), "precondition: an Edit arms no later wake-up");
        match judge(&f) {
            StopVerdict::Block { reason } => assert!(reason.contains("RULE B"), "{reason}"),
            StopVerdict::Allow => panic!(
                "the valve released a retry that armed nothing — this is the stranding: \
                 the session ends mid-task and no job exists to wake it"
            ),
        }
    }

    /// The valve must still do its job for every rule that has no ceiling of its
    /// own, or this fix trades a stranding for a wedge.
    #[test]
    fn the_valve_still_releases_a_retry_that_armed_work() {
        let mut f = facts(Autonomy::Granted, vec![tool("Agent")]);
        f.already_bounced = true;
        assert_eq!(StopVerdict::Allow, judge(&f),
            "a retry that DID arm work must pass — Rule B has nothing to complain about");
    }

    /// ORDINARY ENGINEERING PROSE IS NOT AN ASK. Every one of these tripped the
    /// rule live on 2026-08-29 by containing DECISION as a bare substring, and
    /// each false positive cost a bounce AND the push on the retry.
    #[test]
    fn describing_a_design_choice_is_not_asking_for_one() {
        for s in [
            "Two design decisions in the tool worth naming, and both are about \
             whether the operation actually finishes.",
            "It would add a class that holds no state and makes no decision.",
            "Choosing whether a switch becomes a hierarchy is a judgement about \
             the domain, not a property of the AST.",
            "The decision path count is what cyclomatic complexity measures.",
        ] {
            assert!(!asks_the_human(s), "false positive, and it disables the push: {s:?}");
        }
    }

    /// ...while a real ask still is one. The pair is asserted together because
    /// either half alone passes for the wrong reason: a detector that never
    /// fires satisfies the first, and today's satisfied the second.
    #[test]
    fn a_real_ask_is_still_caught_after_the_narrowing() {
        for s in [
            "DECISION: close C7? I recommend yes.",
            "This one is your decision, not mine.",
            "I need a decision on whether the default flips.",
            "Blocked on your ruling about the cut line.",
            "That needs your approval before it ships.",
        ] {
            assert!(asks_the_human(s), "a real ask must still be caught: {s:?}");
        }
    }

    /// THE SIX-HOUR SLEEP CANNOT RECUR (2026-08-29, measured: `stop-allowed`
    /// at 03:01:23, next human event 09:02). A window opened by a
    /// task-notification whose embedded agent report is question-shaped must
    /// grant NEITHER exemption: the self-initiated ask that ends such a turn
    /// owes the communicator (Rule A), and an idle non-ask turn is pushed
    /// (Rule B) — the harness is not the human.
    #[test]
    fn a_task_notice_grants_no_exemption() {
        // Wrapper assembled, never written whole — the popping-surface scan's
        // own idiom (see `field`).
        let noti = concat!("notifi", "cation");
        let transcript = format!(
            concat!(
                r#"{{"type":"user","message":{{"content":"<task-{}> <task-id>x</task-id> <result>What does this prove? Check the gate files — which clauses are MET?</result>"}}}}"#,
                "\n",
                r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"DECISION: close C7? I recommend yes. Say close and I start Stage 8."}}]}}}}"#,
                "\n"
            ),
            noti
        );
        let turn = read_turn(&transcript).expect("parses");
        assert!(
            !turn.user_asked,
            "question-shaped report text must not read as the human asking — \
             this exact misread slept a session for six hours"
        );
        assert!(turn.asks_the_human, "the final text is a decision ask");

        let f = StopFacts {
            empty_turns: 0,
            review_rounds: 0,
            already_bounced: false,
            bounces: 0,
            turn,
            autonomy: Autonomy::Granted,
            substrate: None,
            reseed_bounces: 0,
        };
        match judge(&f) {
            StopVerdict::Block { reason } => assert!(
                reason.contains("RULE B"),
                "v4.0.0: the ask is held by Rule B and released by the judge, not by a \
reviewer — but it is still HELD, which is this test's whole subject: {reason}"
            ),
            StopVerdict::Allow => {
                panic!("allowed — this is the 03:01:23 stop-allowed that slept the night")
            }
        }
    }

    /// The other half of the same night: a notification-opened turn that ends
    /// on a plain summary (no ask) is PUSHED, not allowed to sleep.
    #[test]
    fn an_idle_notice_turn_is_pushed() {
        let noti = concat!("notifi", "cation");
        let transcript = format!(
            concat!(
                r#"{{"type":"user","message":{{"content":"<task-{}> <task-id>x</task-id> <summary>Background command completed (exit code 0)</summary>"}}}}"#,
                "\n",
                r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"All gates green. Stage 7 stands."}}]}}}}"#,
                "\n"
            ),
            noti
        );
        let turn = read_turn(&transcript).expect("parses");
        let f = StopFacts {
            empty_turns: 0,
            review_rounds: 0,
            already_bounced: false,
            bounces: 0,
            turn,
            autonomy: Autonomy::Granted,
            substrate: None,
            reseed_bounces: 0,
        };
        match judge(&f) {
            StopVerdict::Block { reason } => {
                assert!(reason.contains("RULE B"), "{reason}");
            }
            StopVerdict::Allow => panic!("an idle turn under the grant must be pushed"),
        }
    }

    #[test]
    fn only_a_decision_class_message_owes_a_review() {
        // SCOPE, twice corrected: decision-class (2026-08-07) -> UNCONDITIONAL
        // for one afternoon -> back. Unconditional was the wrong cure for a rule
        // that had never fired, and it cost the reader three renderings of every
        // message.
        // Granted is excluded here on purpose: Rule B (autonomy granted and
        // nothing armed) is a separate concern and fires on its own terms.
        for a in [Autonomy::NotGranted, Autonomy::Unknown] {
            assert_eq!(
                StopVerdict::Allow,
                judge(&facts(a, vec![])),
                "{a:?}: a routine turn must pass untouched"
            );
        }
        // v4.0.0: a decision-class message owes NOTHING here any more. The
        // scope argument this test recorded — decision-class, then
        // unconditional for one afternoon, then back — ended by the rule being
        // retired rather than re-scoped.
        let mut asking = facts(Autonomy::Unknown, vec![]);
        asking.turn.asks_the_human = true;
        assert_eq!(
            StopVerdict::Allow,
            judge(&asking),
            "the reviewer is retired; the ruling carries what it checked"
        );
        let mut replying = facts(Autonomy::Unknown, vec![]);
        replying.turn.asks_the_human = true;
        replying.turn.user_asked = true;
        assert_eq!(
            StopVerdict::Allow,
            judge(&replying),
            "a reply to his own question is exempt — gating conversation triples his wait"
        );
    }

    #[test]
    fn autonomy_without_a_communicator_pass_blocks() {
        // v4.0.0: it still blocks, and for a DIFFERENT rule. The reviewer is
        // retired, so what holds this turn is Rule B — granted, nothing armed —
        // and the way out is the judge, not a readback.
        let mut f = facts(Autonomy::Granted, vec![tool("Bash")]);
        f.turn.asks_the_human = true;
        match judge(&f) {
            StopVerdict::Block { reason } => {
                assert!(reason.contains("RULE B"), "{reason}");
                assert!(
                    !reason.contains("communicator"),
                    "no gate may still send him to the retired reviewer: {reason}"
                );
            }
            StopVerdict::Allow => panic!("must block"),
        }
    }

    /// THE DISCRIMINATOR: one fixture, two runs, differing only by the
    /// communicator call. Without this the suite proves nothing — a gate that
    /// always allowed would pass every other test here.
    #[test]
    fn removing_only_the_communicator_call_flips_the_verdict() {
        // INVERTED v4.0.0. It was the discriminator for a rule that no longer
        // exists, so it now pins the retirement: the reviewer's presence must
        // change NOTHING. A gate still keyed on it would fail here.
        //
        // The discriminating role passes to the judge — see
        // `the_verdict_is_read_from_the_harnesss_record_and_not_from_our_own_prose`,
        // where one fixture differs only by the verdict and flips the verdict.
        let armed = ToolUse { name: "Agent".into(), subagent: Some("general-purpose".into()), backgrounded: false };
        let mut with = facts(Autonomy::Granted, vec![communicator(), armed.clone()]);
        with.turn.asks_the_human = true;
        let mut without = facts(Autonomy::Granted, vec![armed]);
        without.turn.asks_the_human = true;
        assert_eq!(judge(&with), judge(&without), "the reviewer must not decide anything");
        assert_eq!(StopVerdict::Allow, judge(&without), "and work is armed, so the turn may end");
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
    /// THE PREMISE THIS REPLACES: "a second pass always allows". Harald,
    /// 2026-08-20, on the live 3.12.2 measurement (block at 15:53:52, allowed
    /// 0.8 s later): "not acceptable". A rule you can walk past by doing
    /// nothing is not a gate.
    ///
    /// The retry now excuses everything EXCEPT a missing review — and even
    /// that only until the ceiling, so it bounds rather than wedges.
    /// THE REVIEW CEILING IS THE JUDGE'S NOW (v4.1.0). It was a rule here that
    /// blocked with a message and no exit; the seat reads the refusal count
    /// itself and takes the architect's position past the cap. What this test
    /// pins is that the gate no longer walls anything off on the count alone,
    /// and that the verdict — not the count — decides.
    #[test]
    fn the_review_ceiling_ends_a_loop_that_does_work_every_round() {
        // At the cap with no verdict: the ordinary push — spawn the judge — and
        // nothing about "REVIEW CEILING". The judge will see the rounds.
        let mut at_cap = facts(Autonomy::Granted, vec![]);
        at_cap.review_rounds = crate::autonomy::MAX_REVIEW_ROUNDS;
        match judge(&at_cap) {
            StopVerdict::Block { reason } => {
                assert!(reason.contains("RULE B"), "{reason}");
                assert!(!reason.contains("REVIEW CEILING"), "the count-only wall is gone: {reason}");
            }
            StopVerdict::Allow => panic!("at the cap the judge is still owed"),
        }
        // At the cap with the judge's reservation: the stop stands. This is
        // his situation 3 — the architect took the dispute and called it not
        // fixable — and it ends the loop visibly, on a verdict.
        let mut settled = facts(Autonomy::Granted, vec![]);
        settled.review_rounds = crate::autonomy::MAX_REVIEW_ROUNDS;
        settled.turn.judge_call_ids = vec!["toolu_J".into()];
        settled.turn.judge_verdict = Some(JudgeVerdict::Reserved);
        assert_eq!(StopVerdict::Allow, judge(&settled));
        // And the agent cannot end it by saying so itself.
        let mut declared = facts(Autonomy::Granted, vec![]);
        declared.review_rounds = crate::autonomy::MAX_REVIEW_ROUNDS;
        declared.turn.declares_a_decision = true;
        assert!(matches!(judge(&declared), StopVerdict::Block { .. }));
    }

    /// THE C3 STOP, AND THE TWO ABUSES ON EITHER SIDE OF IT (Harald, 2026-09-03:
    /// *"We had the agent saying I can fix and he did not move on."*).
    ///
    /// Three claims, each with its control:
    /// 1. told to fix, did nothing → held with the SAME instruction, however
    ///    many times it asks, however many times it re-spawns the judge;
    /// 2. told to fix, did the work → the instruction is SPENT and a fresh
    ///    consultation is demanded — never the stale one re-served (the
    ///    v4.0.2 wedge), never a stale reservation honoured either;
    /// 3. holding an unspent instruction, idling → the idle valve does NOT
    ///    release the turn (the v4.0.2 self-disarm: three judge spawns and out).
    #[test]
    fn an_instruction_is_held_until_work_spends_it() {
        let spawn = |id: &str| {
            format!(
                "{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"tool_use\",\
\"id\":\"{id}\",\"name\":\"Agent\",\"input\":{{\"subagent_type\":\"autocontinue\"}}}}]}}}}\n"
            )
        };
        let answer = |id: &str, text: &str| {
            format!(
                "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\
\"tool_result\",\"tool_use_id\":\"{id}\",\"content\":\"{text}\"}}]}}}}\n"
            )
        };
        let work = "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\
\"id\":\"toolu_E\",\"name\":\"Edit\",\"input\":{\"file_path\":\"/x/A.rs\"}}]}}\n";
        let fix = "VERDICT: RESOLVABLE — re-run the two reviews";

        // 1. Told, did nothing, re-asked: the re-roll is IGNORED and the same
        //    instruction stands. A kinder second judge changes nothing.
        let t = read_turn(&format!(
            "{}{}{}{}",
            spawn("J1"),
            answer("J1", fix),
            spawn("J2"),
            answer("J2", "VERDICT: RESERVED")
        ))
        .unwrap();
        assert_eq!(
            Some(JudgeVerdict::Resolvable("re-run the two reviews".into())),
            t.judge_verdict,
            "re-spawning without working must not replace the instruction"
        );
        assert!(!t.verdict_spent);
        let mut f = facts(Autonomy::Granted, vec![]);
        f.turn = t;
        match judge(&f) {
            StopVerdict::Block { reason } => assert!(reason.contains("re-run the two reviews"), "{reason}"),
            StopVerdict::Allow => panic!("an unacted instruction must hold the turn"),
        }
        // ...and idling does not release it either — the idle valve is off
        // while an instruction is unspent. This is the self-disarm control.
        f.empty_turns = MAX_EMPTY_TURNS;
        assert!(
            matches!(judge(&f), StopVerdict::Block { .. }),
            "three judge spawns and nothing else used to trip the idle valve and end the turn"
        );

        // 2. Told, DID THE WORK, stopped: the instruction is spent. Not the
        //    stale message (the wedge), and not a stale reservation either.
        let t = read_turn(&format!("{}{}{}", spawn("J1"), answer("J1", fix), work)).unwrap();
        assert!(t.verdict_spent, "an Edit after the verdict spends it");
        assert!(!t.holds_an_unspent_fix());
        let mut f = facts(Autonomy::Granted, vec![]);
        f.turn = t;
        match judge(&f) {
            StopVerdict::Block { reason } => {
                assert!(reason.contains("ACTED ON"), "{reason}");
                assert!(!reason.contains("re-run the two reviews"), "the stale instruction must not be re-served: {reason}");
            }
            StopVerdict::Allow => panic!("work does not end the turn by itself; the judge sees it first"),
        }
        let t = read_turn(&format!("{}{}{}", spawn("J1"), answer("J1", "VERDICT: RESERVED"), work)).unwrap();
        let mut f = facts(Autonomy::Granted, vec![]);
        f.turn = t;
        assert!(
            matches!(judge(&f), StopVerdict::Block { .. }),
            "a reservation from before the work is stale in the other direction"
        );

        // ...and after the work a FRESH verdict is accepted and governs.
        let t = read_turn(&format!(
            "{}{}{}{}{}",
            spawn("J1"),
            answer("J1", fix),
            work,
            spawn("J2"),
            answer("J2", "VERDICT: RESERVED")
        ))
        .unwrap();
        assert_eq!(Some(&JudgeVerdict::Reserved), t.live_verdict());
        let mut f = facts(Autonomy::Granted, vec![]);
        f.turn = t;
        assert_eq!(StopVerdict::Allow, judge(&f), "the judge saw the work and reserved: the stop stands");
    }

    #[test]
    fn a_second_pass_excuses_everything_except_a_missing_review() {
        // Retry, and the turn ASKS for something: still held — by RULE B now,
        // since the reviewer that used to hold it is retired. The valve's
        // subject is unchanged: a second pass must not excuse a turn that is
        // about to sleep with nothing armed.
        let mut f = facts(Autonomy::Granted, vec![]);
        f.turn.asks_the_human = true;
        f.already_bounced = true;
        match judge(&f) {
            StopVerdict::Block { reason } => {
                assert!(reason.contains("RULE B"), "{reason}");
            }
            StopVerdict::Allow => panic!("the retry must not excuse a turn that armed nothing"),
        }
        // Retry, review done, and the turn only LOOKS like an ask: Rule A is
        // satisfied and Rule B now PUSHES ANYWAY.
        //
        // This asserts the 2026-08-29 contract change. It used to expect Allow,
        // because `asks_the_human` — a phrase list over the agent's own prose —
        // stood the push down. That is what slept a session for 21 minutes by
        // matching "SAY THE WORD" inside "Nothing needed from you". Harald's
        // ruling: the agent may not stop on its own inference. A heuristic ask
        // still costs a REVIEW; only a DECLARATION stops the work.
        let mut judged = facts(Autonomy::Granted, vec![communicator()]);
        judged.turn.asks_the_human = true;
        judged.already_bounced = true;
        match judge(&judged) {
            StopVerdict::Block { reason } => assert!(
                reason.contains("RULE B"),
                "a heuristic ask must not stand the push down: {reason}"
            ),
            StopVerdict::Allow => {
                panic!("an inferred ask stopped the session — the switch is the agent's again")
            }
        }
        // ...AND NEITHER DOES THE DECLARED FORM ANY MORE (2026-09-03). This
        // asserted the opposite until today: `declares_a_decision` was the
        // exemption, on the reasoning that a deliberate marker is different in
        // kind from an inferred phrase. It is — and it failed the same way one
        // level up, because the marker is still the agent's own text and an
        // agent that wants the turn to end simply writes it. Two stops that
        // night were correctly formatted and carried no decision at all.
        //
        // So the declaration now buys NOTHING, and this is the control for
        // that: revert the exemption to `declares_a_decision` and this
        // assertion goes red.
        let mut declared = facts(Autonomy::Granted, vec![communicator()]);
        declared.turn.asks_the_human = true;
        declared.turn.declares_a_decision = true;
        declared.already_bounced = true;
        match judge(&declared) {
            StopVerdict::Block { reason } => assert!(
                reason.contains(JUDGE_SEAT),
                "the agent's own DECISION line must now route to the judge: {reason}"
            ),
            StopVerdict::Allow => {
                panic!("a self-declared decision stopped the session — the switch is the agent's again")
            }
        }
        // And the judge's verdict IS the switch, both ways. RESERVED lets the
        // same stop through; RESOLVABLE holds it and carries the next action.
        let mut reserved = facts(Autonomy::Granted, vec![communicator()]);
        reserved.turn.judge_call_ids = vec!["toolu_J".into()];
        reserved.turn.judge_verdict = Some(JudgeVerdict::Reserved);
        reserved.already_bounced = true;
        assert_eq!(
            StopVerdict::Allow,
            judge(&reserved),
            "a fresh-context seat saying the plan reserves this is a legitimate stop"
        );
        let mut resolvable = facts(Autonomy::Granted, vec![communicator()]);
        resolvable.turn.judge_call_ids = vec!["toolu_J".into()];
        resolvable.turn.judge_verdict =
            Some(JudgeVerdict::Resolvable("re-run the two reviews".into()));
        resolvable.already_bounced = true;
        match judge(&resolvable) {
            StopVerdict::Block { reason } => assert!(
                reason.contains("re-run the two reviews"),
                "the block must carry the seat's own next action, not a bare 'continue': {reason}"
            ),
            StopVerdict::Allow => panic!("a resolvable stop must not end the turn"),
        }
        // The ceiling: it gives up rather than wedging the session. Rule B's
        // bound is the EMPTY-TURN count, not a bounce count — a turn that does
        // real work resets it, so a working session never approaches it while a
        // wedged one is released in two.
        let mut spent = facts(Autonomy::Granted, vec![]);
        spent.turn.asks_the_human = true;
        spent.already_bounced = true;
        spent.empty_turns = MAX_EMPTY_TURNS;
        assert_eq!(
            StopVerdict::Allow,
            judge(&spent),
            "bounded, not a trap — the old valve's concern is answered by a ceiling"
        );
    }

    // ---- read_turn ----

    const TRANSCRIPT: &str = r#"
{"type":"user","message":{"content":[{"type":"text","text":"continue and autocontinue"}]}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","input":{"subagent_type":"communicator"}}]}}
{"type":"user","message":{"content":[{"type":"tool_result","content":"ok"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"the summary"}]}}
"#;

    /// THE GATE MUST NOT ERASE ITS OWN EVIDENCE.
    ///
    /// The client injects a blocked stop's reason back as a USER turn. A user
    /// turn resets this window, so the communicator call made BEFORE the bounce
    /// vanished from the window the retry judged: run the reviewer, get
    /// bounced, and the next stop could not see that you ran it. Measured live
    /// 2026-08-20 by byte offset — the bounce always lands after the call.
    #[test]
    fn our_own_bounce_does_not_erase_the_turn() {
        const BOUNCED: &str = r#"
{"type":"user","message":{"content":"go"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","input":{"subagent_type":"communicator"}}]}}
{"type":"user","message":{"content":"Stop hook feedback:\nUNJUDGED MESSAGE: this turn ends with a message the communicator has not read."}}
{"type":"assistant","message":{"content":[{"type":"text","text":"the reviewed message"}]}}
"#;
        let turn = read_turn(BOUNCED).expect("parses");
        assert!(
            turn.communicator_ran(),
            "the review happened BEFORE the bounce; the bounce must not hide it"
        );
        assert_eq!("the reviewed message", turn.final_text);

        // A REAL human turn still resets, or the window would never close.
        const HUMAN: &str = r#"
{"type":"user","message":{"content":"go"}}
{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent","input":{"subagent_type":"communicator"}}]}}
{"type":"user","message":{"content":"and now something else"}}
{"type":"assistant","message":{"content":[{"type":"text","text":"a new turn"}]}}
"#;
        let fresh = read_turn(HUMAN).expect("parses");
        assert!(
            !fresh.communicator_ran(),
            "a genuine human message still starts a new window"
        );
    }

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
        let mut f = facts(Autonomy::Unknown, vec![communicator()]);
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
        // RETIRED v4.0.0, and this is the control for it. It asserted the
        // opposite: an ask blocked until a reviewer had read it. Harald,
        // 2026-09-03: *"The communicator is annoying. I see the same output
        // twice."* Restore the rule and this goes red.
        //
        // What the reviewer caught was a checklist — an unopenable reference, a
        // count with no object, an undefined term — and a checklist is a ruling,
        // not a subagent. What it cost was the readback rendered to him beside
        // the message it judged.
        let mut f = facts(Autonomy::Unknown, vec![]);
        f.turn.asks_the_human = true;
        assert_eq!(
            StopVerdict::Allow,
            judge(&f),
            "an ask no longer owes a reviewer — the ruling replaced it"
        );
        // And the reviewer buys nothing now, which is the other half: no path
        // through this gate is shortened by running it.
        f.turn.launches = vec![communicator()];
        assert_eq!(StopVerdict::Allow, judge(&f));
    }

    /// Refusals are counted from ASSISTANT TEXT, never the raw window. The
    /// script generation counted the whole transcript and fired on any session
    /// that merely READ the word — reading is not refusing.
    ///
    /// AND ONLY AS VERDICT LINES (2026-08-29): the substring form this test
    /// used to pin ("round 1 REFUSE" counts) fired live on a turn that merely
    /// DISCUSSED refusals with the user — three mentions of the word in a
    /// workflow conversation read as three audit rounds, and the gate demanded
    /// an architect run against a fix loop that did not exist. Talking about
    /// refusing is not refusing, exactly as reading the word is not.
    #[test]
    fn refusals_are_counted_only_from_what_the_agent_emitted() {
        let quoted = "{\"type\":\"user\",\"message\":{\"content\":[{\"type\":\"tool_result\",\"content\":\"REFUSE REFUSE REFUSE\"}]}}\n{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"all green\"}]}}\n";
        let t = read_turn(quoted).expect("parses");
        assert_eq!(0, t.refusals_emitted, "quoted refusals are not emitted ones");

        // The live misfire, verbatim in shape: prose ABOUT the refuse loop.
        let discussed = "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Repairable REFUSE: repair, re-audit, no ask. C7 ran round 1 REFUSE then sign-off. The gate blocks after three REFUSEs in one window.\"}]}}\n";
        assert_eq!(
            0,
            read_turn(discussed).expect("parses").refusals_emitted,
            "discussing refusals with the user is not emitting them — this \
             exact prose tripped the AUDIT-FIX LOOP live"
        );

        // What IS counted: relayed verdict lines, markdown decoration allowed.
        let emitted = "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"# VERDICT: REFUSE\\none blocking finding\"}]}}\n{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"**VERDICT: REFUSE** again\"}]}}\n{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"VERDICT: REFUSE\"}]}}\n";
        assert_eq!(3, read_turn(emitted).expect("parses").refusals_emitted);
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

    // ---- studio#11: a REPLY is not an ask ----
    //
    // Live false positive (2026-08-16): a reply to Harald's own question —
    // market-share numbers plus a confirmation of the position he had just
    // stated — was held as an UNJUDGED ASK. The communicator then judged it
    // PASS, "confirmation of his conclusion is not a decision ask". The
    // 2026-08-07 ruling already exempts direct replies; the detector simply had
    // no notion of who asked first.

    /// One transcript: the human speaks, then the agent answers.
    ///
    /// Each line is built with its own single-line format string ON PURPOSE. A
    /// `\`-continued multi-line string desynchronises the brace lexer in
    /// `no_panics_at_fire_time.rs`, which reads sources line by line and carries
    /// no across-line string state — it then stops examining the rest of this
    /// file, silently. Its self-check caught exactly that here.
    fn exchange(user: &str, agent: &str) -> String {
        let human = format!("{{\"type\":\"user\",\"message\":{{\"content\":\"{user}\"}}}}");
        let reply = format!("{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{agent}\"}}]}}}}");
        format!("{human}\n{reply}\n")
    }

    #[test]
    fn a_reply_to_his_own_question_is_not_a_gated_ask() {
        let turn = read_turn(&exchange(
            "Zed might be competition. What about their ACP protocol?",
            "Zed created it and it is genuinely open. Should we adopt it? On the evidence, yes.",
        ))
        .expect("parses");

        assert!(turn.user_asked, "the human's message carries a question mark");
        let verdict = judge(&StopFacts {
            empty_turns: 0,
            review_rounds: 0,
            already_bounced: false,
            bounces: 0,
            turn,
            autonomy: Autonomy::Unknown,
            substrate: None,
            reseed_bounces: 0,
        });
        assert_eq!(
            StopVerdict::Allow,
            verdict,
            "a reply is out of this gate's scope by the 2026-08-07 ruling, restored \
             2026-08-20 after the unconditional experiment tripled every message"
        );
    }

    #[test]
    fn an_imperative_ask_also_opens_a_reply_window() {
        // He asks without a question mark as often as with one.
        for prompt in ["Discuss!", "Explain the tradeoff", "Tell me what the problem is"] {
            let turn = read_turn(&exchange(prompt, "Here is the tradeoff. Which way do you lean?"))
                .expect("parses");
            assert!(turn.user_asked, "must open a reply window: {prompt:?}");
        }
    }

    #[test]
    fn a_self_initiated_ask_after_a_plain_instruction_still_blocks() {
        // The gate's whole purpose: an ask the AGENT raises on its own. The
        // human's message here instructs and asks nothing, so the exemption
        // must not apply.
        let turn = read_turn(&exchange(
            "Implement stage 3 and commit it.",
            "Stage 3 is committed. Shall I push it?",
        ))
        .expect("parses");

        assert!(!turn.user_asked, "an instruction is not a question");
        assert!(turn.asks_the_human);
        // v4.0.0: WITHOUT AUTONOMY, nothing holds this any more. The reviewer
        // that did is retired, and Rule B is not engaged because he never gave
        // the word — he is present, and a turn that ends while he is present
        // ends. The ask detector survives only as an input other rules may read.
        assert_eq!(
            StopVerdict::Allow,
            judge(&StopFacts { empty_turns: 0, review_rounds: 0, already_bounced: false,
                bounces: 0, turn: turn.clone(), autonomy: Autonomy::Unknown, substrate: None,
                reseed_bounces: 0 }),
            "with no grant in force, an ask is just a message"
        );
        // ...and WITH the grant it is held, by Rule B, until the judge speaks.
        // That is where a self-initiated ask is now examined.
        assert!(
            matches!(
                judge(&StopFacts { empty_turns: 0, review_rounds: 0, already_bounced: false,
                    bounces: 0, turn, autonomy: Autonomy::Granted, substrate: None,
                    reseed_bounces: 0 }),
                StopVerdict::Block { .. }
            ),
            "under a grant, a self-initiated ask must reach the judge"
        );
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
        // v4.0.0: it is satisfied by CUTTING it, and by nothing else. The
        // reviewer used to be the other way out, which made a long message
        // sendable by consulting someone about it rather than by shortening it.
        f.turn.launches = vec![communicator()];
        assert!(
            matches!(judge(&f), StopVerdict::Block { .. }),
            "running the retired reviewer must not buy length back"
        );
        f.turn.final_text = "Committed and green.".into();
        assert_eq!(StopVerdict::Allow, judge(&f), "cutting it is the way through");
    }

    #[test]
    fn a_seat_without_its_gate_blocks() {
        let mut f = facts(Autonomy::Unknown, vec![communicator()]);
        f.turn.seats_invoked = vec!["/refactor".into()];
        f.turn.gate_ran = false;
        f.turn.changed_code = true;
        match judge(&f) {
            StopVerdict::Block { reason } => assert!(reason.contains("SEAT DISCIPLINE"), "{reason}"),
            StopVerdict::Allow => panic!("a seat that changed code and skipped its gate must block"),
        }
        f.turn.gate_ran = true;
        assert_eq!(StopVerdict::Allow, judge(&f));
    }

    /// studio#18: `/report` changes no code — it turns a recorded error shape
    /// into an issue — so a correct run of it owed a compile gate it could
    /// never sensibly pass, and had to declare the gate "inapplicable". A gate
    /// a correct run cannot satisfy teaches agents to narrate around gates.
    #[test]
    fn a_seat_that_changed_no_code_owes_no_gate() {
        let mut f = facts(Autonomy::Unknown, vec![communicator()]);
        f.turn.seats_invoked = vec!["/report".into()];
        f.turn.gate_ran = false;
        f.turn.changed_code = false;
        assert_eq!(StopVerdict::Allow, judge(&f),
            "a seat run that edited nothing owes no verification gate");
    }

    /// studio#18, the half found while fixing it: the seat scan matched the
    /// bare token ANYWHERE in the transcript, so a turn that merely WROTE
    /// about the seats counted as having run them.
    #[test]
    fn writing_about_a_seat_is_not_invoking_it() {
        let mentions = exchange(
            "What do the seats do?",
            "The architect seat is invoked with /refactor and reviews a diff; \
             /cover writes characterization tests.",
        );
        let turn = read_turn(&mentions).expect("parses");
        assert!(turn.seats_invoked.is_empty(),
            "a sentence ABOUT the seats invoked none of them: {:?}", turn.seats_invoked);

        let invoked = exchange("/refactor the applier", "Report follows.");
        let turn2 = read_turn(&invoked).expect("parses");
        assert_eq!(vec!["/refactor".to_string()], turn2.seats_invoked,
            "a line that OPENS with the command did invoke it");
    }

    /// jawata-studio#25, both live misfires, verbatim.
    ///
    /// Quoting the exact label a user sees, or the exact text a tool returned,
    /// is the OPPOSITE of jargon. The rule reported six abbreviations across two
    /// messages that contained none — and `BE` was the giveaway.
    #[test]
    fn quoted_labels_and_tool_text_are_not_undefined_terms() {
        // Case 1: a resident's own JSON, quoted.
        let case1 = "The resident says `\"problem\": \"registered, but its directory \
                     NO LONGER EXISTS on disk\"` and the row shows RUNNING.";
        assert!(
            undefined_terms(case1).is_empty(),
            "quoted tool output is grounding, not jargon: {:?}",
            undefined_terms(case1)
        );

        // Case 2: a dashboard badge, quoted in bold.
        let case2 = "The row shows **CANNOT BE READ** in red beside the green RUNNING badge.";
        assert!(
            undefined_terms(case2).is_empty(),
            "a badge is one label, and BE is not an abbreviation in any reading: {:?}",
            undefined_terms(case2)
        );

        // The rule must still bite on what it exists for.
        let real = "The TOCTOU window reopened and SIGPIPE killed the writer.";
        let found = undefined_terms(real);
        assert!(found.contains(&"TOCTOU".to_string()), "{found:?}");
        assert!(found.contains(&"SIGPIPE".to_string()), "{found:?}");

        // AND INSIDE QUOTES TOO. Quoting a log line does not make its acronym
        // resolvable for the reader, so the rule must not go blind there — the
        // reason the first draft's redaction of quoted spans was removed.
        let quoted = "The log says `write failed: SIGPIPE` and the run died.";
        assert!(
            undefined_terms(quoted).contains(&"SIGPIPE".to_string()),
            "a quoted acronym is still an acronym: {:?}",
            undefined_terms(quoted)
        );
    }

    #[test]
    fn undefined_jargon_blocks_and_defined_jargon_does_not() {
        let mut f = facts(Autonomy::Unknown, vec![communicator()]);
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
