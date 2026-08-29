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
        if self.is_communicator() {
            return false;
        }
        self.name == "Agent" || self.backgrounded
    }

    /// Was this the communicator judging an upward message?
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
    pub fn owes_a_review(&self) -> bool {
        self.asks_the_human && !self.user_asked && !self.communicator_ran()
    }

    pub fn communicator_ran(&self) -> bool {
        self.launches.iter().any(ToolUse::is_communicator)
    }
    pub fn armed_anything(&self) -> bool {
        self.launches.iter().any(ToolUse::arms_work)
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
    let rule_b_would_push = facts.autonomy == Autonomy::Granted
        && !facts.turn.armed_anything()
        && !facts.turn.interrupted
        && !facts.turn.user_asked
        && facts.empty_turns < MAX_EMPTY_TURNS;
    if facts.already_bounced
        && !facts.turn.owes_a_review()
        && !facts.owes_a_reseed()
        && !rule_b_would_push
    {
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
    if facts.turn.owes_a_review() {
        // Past the ceiling the turn is RELEASED OUTRIGHT rather than falling
        // through to the rules below. Falling through would let a different
        // rule bounce the same turn the review rule just gave up on — a second
        // loop wearing another rule's name, which is the incident again with a
        // different label.
        if facts.bounces >= MAX_UNJUDGED_BOUNCES {
            return StopVerdict::Allow;
        }
        return StopVerdict::Block {
            reason: format!(
                "UNJUDGED MESSAGE ({} of {}): this message asks for a word, a ruling or a \
decision, and the communicator has not read it. Hand it the draft FIRST — before writing \
the answer — and send back what it understood, so the reader sees the text once.",
                facts.bounces + 1,
                MAX_UNJUDGED_BOUNCES
            ),
        };
    }

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
    if facts.autonomy == Autonomy::Granted && !facts.turn.armed_anything() {
        if facts.empty_turns >= MAX_EMPTY_TURNS {
            return StopVerdict::Allow;
        }
        // His Esc wins over autonomy, always. The grant covers his ABSENCE; an
        // interrupt is the loudest possible evidence that he is present, and
        // pushing an agent back into a turn he just stopped would be the gate
        // arguing with the one control he has that is not a sentence.
        if facts.turn.interrupted {
            return StopVerdict::Allow;
        }
        // And nothing to autocontinue past when his answer is what is missing:
        // the next move is genuinely his, and holding here would push an agent
        // that is blocked rather than idle.
        //
        // KEYED ON THE DECLARATION, NOT ON THE GUESS (Harald, 2026-08-29). This
        // read `asks_the_human` — a 42-phrase substring list over the agent's
        // own prose — until it stood a session down for 21 minutes by matching
        // "SAY THE WORD" inside the sentence *"Nothing needed from you — say
        // the word only if you want one back."* His ruling: *"you cannot on
        // your own discretion just switch this."* A phrase inferred from prose
        // IS the agent's discretion wearing a detector's clothes; a `DECISION:`
        // line is a deliberate act in the form his contract already mandates.
        // Every stop reason he named — a design or spec direction, access only
        // he has, something genuinely broken, an unresolved dispute with the
        // auditor, a release, the sprint finished — is raised AS a decision, so
        // one marker carries all of them and nothing else stops the push.
        if facts.turn.declares_a_decision {
            return StopVerdict::Allow;
        }
        // THE REVIEW CEILING — the second bound, for the loop the first cannot
        // see. `empty_turns` counts turns that did nothing; a review that will
        // not converge does real work every round and resets it forever.
        //
        // Reaching it is not a new kind of stop: an unconverged review IS "a
        // dispute with the auditor", one of the five reasons Harald named as
        // legitimately his. So the gate does not fall silent here — it pushes
        // ONE more time, telling the agent to state the dispute as a decision.
        // That turn then declares, the rule above allows, and the loop ends
        // visibly with a reason he can read, rather than by a counter running
        // out in the dark.
        if facts.review_rounds >= crate::autonomy::MAX_REVIEW_ROUNDS {
            return StopVerdict::Block {
                reason: format!(
                    "REVIEW CEILING: {} review rounds since he last spoke, and the cap \
is {}. His rule is three rounds, four at the outside when round three found something \
genuinely blocking — past that, remaining findings are ACCEPTED AS-IS or written down as \
named open items, never another round. If the review has genuinely not converged, that is \
a dispute with the auditor and it is his to settle: say so on a line beginning DECISION, \
and stop. Do not open another round.",
                    facts.review_rounds,
                    crate::autonomy::MAX_REVIEW_ROUNDS
                ),
            };
        }
        // HIS OWN QUESTION OPENED THIS WINDOW -> the turn is conversation, not
        // idle autonomy. The grant covers his ABSENCE, and a fresh question is
        // the same evidence of presence an interrupt is. Measured live
        // 2026-08-27 (studio#33): he asked "what are the dropped story
        // situations?", the agent answered and stopped, and this rule pushed it
        // into new work — which he then had to interrupt. Answering the human
        // and waiting for his reaction IS the next piece of work.
        //
        // ONLY THE KEYBOARD CAN GRANT THIS (Harald, verbatim, 2026-08-29:
        // "Presence is: The question comes from the keyboard, i.e. from the
        // chat window!" — and, same morning: "The keyboard is not the
        // machine-wide keyboard. It is keyboard + focus on the chat window.
        // Otherwise every esc will be regarded as such."). That constraint
        // holds BY CONSTRUCTION and must stay so: this gate reads only the
        // chat transcript, so the sole input it can ever see is what the chat
        // window itself recorded — a keystroke or Esc anywhere else never
        // reaches it. Any future presence signal that is not the transcript
        // breaks his ruling. A task notice carrying an agent report full of
        // question shapes read as him for one night and slept a session six
        // hours — see `is_harness_line`. A dispatch he types without a
        // question ("carry on") deliberately does NOT stand this rule down:
        // that window is the grant at work, and the idle turn after it is
        // exactly what the push exists for.
        if facts.turn.user_asked {
            return StopVerdict::Allow;
        }
        return StopVerdict::Block {
            reason: "RULE B: autonomy is granted and this turn armed no background \
work, so ending here sleeps until the human returns. Start the next piece of \
work, or state plainly that you are blocked on the human."
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
fn is_harness_line(v: &serde_json::Value) -> bool {
    // The wrapper words are ASSEMBLED, never written whole — the
    // popping-surface scan in `field` bans the bare word in code, and this is
    // that scan's own idiom for naming what it bans.
    let noti = concat!("notifi", "cation");
    let t = user_text(v);
    let t = t.trim_start();
    t.starts_with(&format!("<task-{noti}>"))
        || t.starts_with(&format!("[SYSTEM {}", noti.to_uppercase()))
        || t.starts_with("<system-reminder>")
        || t.starts_with("<local-command-caveat>")
        || t.starts_with("<command-name>")
        || t.starts_with("<local-command-stdout>")
}

/// Verdict lines at the START of a line, markdown decoration allowed — the
/// DEGRADED_STAMP principle: reading or discussing a word is not emitting it.
fn verdict_lines(text: &str, verdict: &str) -> usize {
    text.lines()
        .filter(|l| l.trim_start().trim_start_matches(['#', '*', ' ']).starts_with(verdict))
        .count()
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
            Some("user") if !is_tool_result(&v) && is_our_own_bounce(&v) => {}
            // THE HARNESS IS NOT THE HUMAN (the 2026-08-29 six-hour sleep).
            // A task notification opens a new window — the agent is being
            // re-invoked — but it grants none of the human's exemptions:
            // `user_asked` is never set by it, so Rule A still
            // demands the review and Rule B still pushes. See
            // `is_harness_line` for the measured incident.
            Some("user") if !is_tool_result(&v) && is_harness_line(&v) => {
                turn = Turn::default();
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
            }
            Some("user") if !is_tool_result(&v) => {
                turn = Turn::default();
                turn.human_window = true;
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
                            turn.launches.push(ToolUse {
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
    fn communicator() -> ToolUse {
        ToolUse { name: "Agent".into(), subagent: Some("communicator".into()), backgrounded: false }
    }
    fn facts(autonomy: Autonomy, launches: Vec<ToolUse>) -> StopFacts {
        StopFacts {
            empty_turns: 0,
            review_rounds: 0,
            already_bounced: false,
            bounces: 0,
            turn: Turn { final_text: "done".into(), launches, refusals_emitted: 0, asks_the_human: false, declares_a_decision: false, user_asked: false, human_window: false, signoff_emitted: false, interrupted: false, narration: String::new(), degraded_consumed: 0, seats_invoked: vec![], gate_ran: true, changed_code: false, wrote_markdown: false },
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
        let mut f = facts(Autonomy::Unknown, vec![]);
        f.turn.asks_the_human = true;
        f.already_bounced = false; // Cursor: every invocation looks like the first

        for n in 0..MAX_UNJUDGED_BOUNCES {
            f.bounces = n;
            match judge(&f) {
                StopVerdict::Block { reason } => {
                    assert!(reason.contains(&format!("{} of {}", n + 1, MAX_UNJUDGED_BOUNCES)),
                        "each bounce must say where it is: {reason}");
                }
                StopVerdict::Allow => panic!("bounce {n} must still hold"),
            }
        }
        f.bounces = MAX_UNJUDGED_BOUNCES;
        assert_eq!(
            StopVerdict::Allow,
            judge(&f),
            "past the ceiling the turn is RELEASED — a safety valve on one path is not a \
             safety valve"
        );
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

    /// HIS QUESTION OPENED THE WINDOW -> answering it and stopping IS the work
    /// (studio#33). Before this branch, Rule B pushed the answer-turn into new
    /// work and he had to interrupt it — "I just asked a question!".
    #[test]
    fn a_window_his_question_opened_is_never_pushed() {
        let mut f = facts(Autonomy::Granted, vec![]);
        f.turn.user_asked = true;
        assert_eq!(
            StopVerdict::Allow,
            judge(&f),
            "the grant covers his absence; his question is proof of presence"
        );
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
                reason.contains("UNJUDGED MESSAGE"),
                "the unreviewed ask must be held for the communicator: {reason}"
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
        let mut asking = facts(Autonomy::Unknown, vec![]);
        asking.turn.asks_the_human = true;
        match judge(&asking) {
            StopVerdict::Block { reason } => {
                assert!(reason.contains("UNJUDGED MESSAGE"), "{reason}");
                assert!(reason.contains("FIRST"), "it must teach review-FIRST: {reason}");
            }
            StopVerdict::Allow => panic!("a decision-class message owes a review"),
        }
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
        // Scope returned to decision-class on 2026-08-20, so the turn must ASK
        // for something; autonomy alone no longer summons the review rule.
        let mut f = facts(Autonomy::Granted, vec![tool("Bash")]);
        f.turn.asks_the_human = true;
        let v = judge(&f);
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
        let mut with = facts(Autonomy::Granted, vec![communicator(), armed.clone()]);
        with.turn.asks_the_human = true;
        let mut without = facts(Autonomy::Granted, vec![armed]);
        without.turn.asks_the_human = true;
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
    /// THE PREMISE THIS REPLACES: "a second pass always allows". Harald,
    /// 2026-08-20, on the live 3.12.2 measurement (block at 15:53:52, allowed
    /// 0.8 s later): "not acceptable". A rule you can walk past by doing
    /// nothing is not a gate.
    ///
    /// The retry now excuses everything EXCEPT a missing review — and even
    /// that only until the ceiling, so it bounds rather than wedges.
    /// THE REVIEW CEILING, and the reason it exists rather than reusing the
    /// other bound: every round of a repair-then-re-review loop DOES WORK, so
    /// `empty_turns` resets to zero forever and never sees it.
    #[test]
    fn the_review_ceiling_ends_a_loop_that_does_work_every_round() {
        // Below the cap: the push is unaffected. A session that spawns a
        // reviewer now and then is ordinary work, not a loop.
        let mut ok = facts(Autonomy::Granted, vec![]);
        ok.review_rounds = crate::autonomy::MAX_REVIEW_ROUNDS - 1;
        match judge(&ok) {
            StopVerdict::Block { reason } => assert!(
                reason.contains("RULE B"),
                "below the cap the ordinary push must run: {reason}"
            ),
            StopVerdict::Allow => panic!("below the cap nothing should stand the push down"),
        }
        // PROOF OF LIFE for the counter itself: the empty-turn bound cannot
        // reach this case, so if it could, this test would prove nothing.
        assert_eq!(ok.empty_turns, 0, "every round did work, so the other bound is at zero");

        // THE COMMUNICATOR IS NOT A REVIEW ROUND, and this nearly shipped
        // wrong. It is an `Agent` launch like any other, and it runs once per
        // judged message — so counting it would have spent the ceiling on four
        // reviewed messages in one ordinary conversation, with no review loop
        // anywhere near it. `arms_work` carves it out for the same reason.
        assert!(
            !communicator().arms_work(),
            "the reviewer judges the message being sent; it is not work that continues"
        );
        assert!(
            communicator().is_communicator(),
            "the counter's carve-out keys on exactly this, so it must hold here"
        );

        // At the cap: pushed ONE more time, and told to state the dispute.
        let mut spent = facts(Autonomy::Granted, vec![]);
        spent.review_rounds = crate::autonomy::MAX_REVIEW_ROUNDS;
        match judge(&spent) {
            StopVerdict::Block { reason } => {
                assert!(reason.contains("REVIEW CEILING"), "{reason}");
                assert!(
                    reason.contains("DECISION"),
                    "it must name the way OUT, or the ceiling is a wall: {reason}"
                );
            }
            StopVerdict::Allow => panic!("the ceiling must be reached, not passed"),
        }

        // ...and the loop then ENDS, visibly, on a declared decision.
        let mut declared = facts(Autonomy::Granted, vec![]);
        declared.review_rounds = crate::autonomy::MAX_REVIEW_ROUNDS;
        declared.turn.declares_a_decision = true;
        assert_eq!(
            StopVerdict::Allow,
            judge(&declared),
            "an unconverged review is a dispute — his to settle, and a legitimate stop"
        );
    }

    #[test]
    fn a_second_pass_excuses_everything_except_a_missing_review() {
        // Retry, no review, and the turn ASKS for something: still held.
        let mut f = facts(Autonomy::Granted, vec![]);
        f.turn.asks_the_human = true;
        f.already_bounced = true;
        match judge(&f) {
            StopVerdict::Block { reason } => {
                assert!(reason.contains("UNJUDGED MESSAGE"), "{reason}");
                assert!(reason.contains("1 of 3"), "it must say where it is: {reason}");
            }
            StopVerdict::Allow => panic!("the retry must not excuse a missing review"),
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
        // ...and the DECLARED form does stop it, which is the other half: the
        // agent keeps a way to halt, it just has to say so on purpose.
        let mut declared = facts(Autonomy::Granted, vec![communicator()]);
        declared.turn.asks_the_human = true;
        declared.turn.declares_a_decision = true;
        declared.already_bounced = true;
        assert_eq!(
            StopVerdict::Allow,
            judge(&declared),
            "a declared DECISION line is a legitimate stop"
        );
        // The ceiling: it gives up rather than wedging the session.
        let mut spent = facts(Autonomy::Granted, vec![]);
        spent.turn.asks_the_human = true;
        spent.already_bounced = true;
        spent.bounces = MAX_UNJUDGED_BOUNCES;
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
        let mut f = facts(Autonomy::Unknown, vec![]);
        f.turn.asks_the_human = true;
        match judge(&f) {
            StopVerdict::Block { reason } => assert!(reason.contains("UNJUDGED MESSAGE"), "{reason}"),
            StopVerdict::Allow => panic!("an unjudged ask must block"),
        }
        f.turn.launches = vec![communicator()];
        assert_eq!(StopVerdict::Allow, judge(&f), "a judged ask must pass");
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
        assert!(
            matches!(
                judge(&StopFacts { empty_turns: 0, review_rounds: 0, already_bounced: false,
            bounces: 0, turn, autonomy: Autonomy::Unknown, substrate: None,
            reseed_bounces: 0 }),
                StopVerdict::Block { .. }
            ),
            "a self-initiated ask must still be judged before it is sent"
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
        // A judged one passes — the check must be satisfiable by judging, not
        // only by staying quiet.
        f.turn.launches = vec![communicator()];
        assert_eq!(StopVerdict::Allow, judge(&f));
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
