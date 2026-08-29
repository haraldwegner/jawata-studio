//! Did the human grant autonomy, and is the agent still using it?
//!
//! [`crate::stop::Autonomy`] has existed since Sprint 26 and production has
//! always supplied [`crate::stop::Autonomy::Unknown`], because nothing readable
//! at the Stop event says whether the human granted it. Both autonomy-gated
//! rules — Rule A's scope and Rule B — are therefore dead in the shipped
//! binary: `stop.rs` records that Rule B "never fired in 267 recorded stops,
//! and the cause was detection". Twenty-odd tests exercise
//! `Autonomy::Granted` and every one of them passes, which is exactly why
//! nothing ever went red over it. Built, tested, and never given its input.
//!
//! **The signal is the human's own word, and nothing else.** Harald grants
//! autonomy by typing `autocontinue` in a prompt. That text arrives at
//! `Role::UserPrompt`, which already parses the prompt and already knows the
//! session — so both ends existed the whole time and were simply never joined.
//! No inference is made from `continue`, `keep going`, `carry on` or any other
//! phrase: a grant the human did not knowingly give is worse than none, because
//! the gate would then hold a session he thinks he can walk away from.
//!
//! **It persists across his turns, which is the entire point.** A grant that
//! expired when he next spoke would be revoked by the first question he asked,
//! and answering a question is precisely where the agent has been stopping.
//!
//! **It ends by itself the moment his answer is genuinely needed** — a decision,
//! a release, anything the agent cannot settle alone. Harald's rule, and it is
//! better than the typed revoke words this module first shipped with: a switch
//! you have to remember to throw is off exactly when you are least able to throw
//! it, which is while you are asleep. The condition is already computed by the
//! gate as `Turn::asks_the_human`, so the grant ends on the same fact that makes
//! the agent stop being able to proceed.
//!
//! # The wedge, and the shape of the bound
//!
//! Rule B blocks a stop whenever autonomy is granted and the turn armed no
//! background work. Wire that with no bound and a session with genuinely
//! nothing left to do is trapped: block, nothing to do, block, forever. That is
//! not hypothetical — the review rule's own bound was moved out of the
//! `already_bounced` branch after exactly that happened on Cursor, "an endless
//! loop, counter at 11 and still climbing", because Cursor re-invokes with the
//! retry flag unset.
//!
//! So the ceiling here counts **consecutive turns that did no work**, not
//! blocks. That distinction is the whole design:
//!
//! * push → the agent starts the next piece of work → the turn carries tool
//!   calls → the counter RESETS. This is the mechanism working, and it can run
//!   all night without ever approaching the ceiling.
//! * push → the agent has nothing and produces an empty turn → push → empty
//!   again → that is a wedge, and it is released.
//!
//! A ceiling on blocks would have punished the working case and the wedged case
//! identically, which is how a safety valve becomes a limit on the feature it
//! is guarding.

use std::path::{Path, PathBuf};

use crate::stop::Autonomy;

/// Consecutive EMPTY turns under a granted autonomy before the gate lets go.
///
/// Two, not one: a single empty turn is common and innocent — the agent
/// answering a question in prose is an empty turn, and that is the exact case
/// this whole mechanism exists to push past. Two in a row means the push
/// produced nothing twice, which is a wedge rather than a pause.
pub const MAX_EMPTY_TURNS: u32 = 2;

/// The word that grants it. His, not a synonym set.
const GRANT: &str = "autocontinue";

fn dir(base: &Path) -> PathBuf {
    base.join("autonomy")
}

fn file(base: &Path, session: &str) -> PathBuf {
    dir(base).join(crate::pipeline::sanitize_session(session))
}

/// Normalise for matching: lowercase, and the three spellings of the one word
/// collapse to one. `auto-continue` and `auto continue` are the same word typed
/// differently, not different words — refusing them would be pedantry that
/// costs him a night.
fn normalised(prompt: &str) -> String {
    prompt
        .to_ascii_lowercase()
        .replace("auto-continue", GRANT)
        .replace("auto continue", GRANT)
}

/// Read the human's prompt and update the grant if it says anything about one.
///
/// Returns `true` when the prompt changed the state, so the caller can log a
/// state change rather than a no-op — a grant nobody can see happening is the
/// same invisible wiring this module exists to end.
pub fn note_prompt(base: &Path, session: &str, prompt: &str) -> bool {
    if session.is_empty() {
        return false;
    }
    let text = normalised(prompt);
    if text.contains(GRANT) {
        let f = file(base, session);
        if f.exists() {
            return false;
        }
        let _ = std::fs::create_dir_all(dir(base));
        return std::fs::write(&f, "0").is_ok();
    }
    // ANY turn of his clears the empty-turn count, and this is a correction to
    // the first version of the bound rather than a convenience.
    //
    // The ceiling exists to release a session that is stuck: pushed, produced
    // nothing, pushed, produced nothing. Three questions answered in prose look
    // identical to that by the count alone — and the FIRST version released the
    // grant on his third question, which is precisely the behaviour the whole
    // mechanism was built to stop. A wedge is consecutive empty turns with NO
    // human input between them; when he has spoken, there is new information in
    // the session and the next empty turn is not evidence of anything.
    if file(base, session).exists() {
        let _ = std::fs::write(file(base, session), "0");
    }
    // ...and the REVIEW-ROUND count with it, for the same reason. The ceiling
    // bounds a loop the agent is running on its own; once he has spoken, the
    // rounds after that are a new stretch and the old ones are not evidence
    // about it. A round count that survived his input would spend a night's
    // budget on the previous conversation.
    let _ = std::fs::remove_file(rounds_file(base, session));
    false
}

/// What the Stop gate should be told about this session.
///
/// `NotGranted` and `Unknown` are deliberately different answers even though
/// Rule B treats them alike: with no session id there is no episode to key on
/// and the honest report is that we could not observe it, while an absent file
/// for a real session means we looked and he had not granted it.
pub fn state(base: &Path, session: &str) -> Autonomy {
    if session.is_empty() {
        return Autonomy::Unknown;
    }
    if file(base, session).exists() {
        Autonomy::Granted
    } else {
        Autonomy::NotGranted
    }
}

/// How many consecutive empty turns this session has had under the grant.
pub fn empty_turns(base: &Path, session: &str) -> u32 {
    if session.is_empty() {
        return 0;
    }
    std::fs::read_to_string(file(base, session))
        .ok()
        .and_then(|t| t.trim().parse().ok())
        .unwrap_or(0)
}

/// End the grant.
///
/// HIS ESC, AND NOTHING ELSE. This used to be called on the agent's own message
/// too, whenever a phrase list guessed it was asking for something — and on
/// 2026-08-29 that guess fired on "SAY THE WORD" inside the sentence *"Nothing
/// needed from you"*, destroyed the grant, and slept the session until he
/// retyped the word. Harald's ruling, verbatim: *"you cannot by yourself change
/// the autocontinue variable by yourself … I can switch off by ESC."*
///
/// The agent can still STOP — Rule B stands down on a declared `DECISION:`
/// line. Stopping and revoking are different powers, and only the second one
/// was ever his to give.
pub fn clear(base: &Path, session: &str) -> bool {
    let _ = std::fs::remove_file(rounds_file(base, session));
    !session.is_empty() && std::fs::remove_file(file(base, session)).is_ok()
}

/// The HARD CEILING on review rounds inside one autonomous stretch.
///
/// FOUR, not three, and Harald chose the shape (2026-08-29): *"3, or
/// 3-with-the-blocking-exception → With the blocking exception."* His `/sprint`
/// rule is "three, four at the outside if round three found something genuinely
/// blocking". So three is the RULE and four is the CEILING: the agent judges
/// whether round three's findings justify a fourth, and the gate guarantees
/// there is never a fifth.
///
/// This exists because the other bound cannot see this loop. [`MAX_EMPTY_TURNS`]
/// counts turns that did NOTHING, and every round of repair-then-re-review does
/// real work — so a review that will not converge resets that counter forever.
/// Measured on the hook as it stood: one bound, and it was blind to exactly the
/// loop he asked about.
pub const MAX_REVIEW_ROUNDS: u32 = 4;

fn rounds_file(base: &Path, session: &str) -> PathBuf {
    dir(base).join(format!(
        "{}.rounds",
        crate::pipeline::sanitize_session(session)
    ))
}

/// Review rounds since he last spoke.
pub fn review_rounds(base: &Path, session: &str) -> u32 {
    if session.is_empty() {
        return 0;
    }
    std::fs::read_to_string(rounds_file(base, session))
        .ok()
        .and_then(|t| t.trim().parse().ok())
        .unwrap_or(0)
}

/// Count this turn if it spawned a reviewer.
///
/// THE SIGNAL IS AN `Agent` LAUNCH THAT IS NOT THE COMMUNICATOR, and both
/// halves were earned.
///
/// The precise signal — "an audit round" — is NOT available: the gate reads
/// seats from HIS message (see `seats_in`), so a fresh-context auditor the
/// AGENT spawns for itself is invisible to it. So the coarse signal it is, and
/// overcounting is the safe direction for a ceiling: ordinary implementation
/// work spawns no subagents at all, while a repair-then-re-review loop spawns
/// one per round.
///
/// The carve-out is the half that nearly shipped wrong. Counting every `Agent`
/// launch also counts the COMMUNICATOR, which runs once per judged message —
/// four reviewed messages in one ordinary conversation would have hit the
/// ceiling with no review loop anywhere near it. `ToolUse::arms_work` already
/// excludes the reviewer for the same underlying reason: it judges the message
/// being sent, it is not work that continues afterwards.
///
/// Harald found it by asking what the deviation actually cost (2026-08-29: *"I
/// do not start any helper by myself. What is the problem here?"*) — which is
/// worth recording, because the answer was a defect and not an explanation.
pub fn note_review_round(base: &Path, session: &str, spawned_reviewer: bool) -> bool {
    if session.is_empty() || !spawned_reviewer {
        return true;
    }
    let f = rounds_file(base, session);
    let next = review_rounds(base, session) + 1;
    let _ = std::fs::create_dir_all(dir(base));
    // Same direction as `note_turn`: an unpersistable bound is SPENT, never
    // zero. A ceiling that silently fails to count is not a ceiling.
    std::fs::write(&f, next.to_string()).is_ok()
}

/// Record what this turn did: work resets the count, an empty turn advances it.
///
/// Called on every stop under a grant, whatever the verdict, because the number
/// is about the TURN and not about the ruling. Deciding it inside one verdict
/// arm is how a measurement silently becomes conditional on the thing it is
/// supposed to be measuring.
pub fn note_turn(base: &Path, session: &str, did_work: bool) -> bool {
    if session.is_empty() {
        return true;
    }
    let f = file(base, session);
    if !f.exists() {
        return true;
    }
    let next = if did_work { 0 } else { empty_turns(base, session) + 1 };
    // THE RETURN VALUE IS THE POINT, and the first version discarded it.
    //
    // The bound lives in this file. If the write fails — a full disk, a
    // read-only home, a permission the installer did not expect — the count
    // stays where it was and Rule B holds the session forever: the exact
    // endless loop the Cursor incident was, reached by a different road and
    // with nothing to see, because a discarded `Result` fails as quietly as a
    // success. So the caller is told, and an unpersistable bound is treated as
    // SPENT rather than as zero. The same direction the bounce counter takes
    // for a missing session id: better a missed push than a stuck session.
    std::fs::write(&f, next.to_string()).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch dir unique per CALL, not per clock tick.
    ///
    /// The counter is the fix for a Windows-only flake that reached the release
    /// workflow (2026-08-27, studio v3.15.0): uniqueness came from
    /// `process::id()` + `SystemTime` nanoseconds, and Windows' system clock is
    /// coarse enough that two tests running on cargo's parallel threads — same
    /// process, so same pid — can land in the SAME directory. Every test here
    /// uses session `"s"`, so a collision lets one test's `clear` delete the
    /// file another is counting, and `empty_turns` reads 0 where 2 was written.
    /// It failed exactly that way: `left: 0, right: 2, at the ceiling`, while
    /// the identical code had passed on Windows two hours earlier.
    ///
    /// Every sibling helper in this crate takes a per-test NAME and never had
    /// the problem; a monotonic counter buys the same guarantee without
    /// touching the call sites, and it cannot be defeated by clock resolution.
    fn tmp() -> PathBuf {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let d = std::env::temp_dir().join(format!(
            "jawata-autonomy-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn his_word_grants_it_and_nothing_else_does() {
        let d = tmp();
        for phrase in ["continue", "keep going", "carry on", "go on until done"] {
            assert!(!note_prompt(&d, "s", phrase), "{phrase:?} must NOT grant");
            assert_eq!(
                state(&d, "s"),
                Autonomy::NotGranted,
                "a grant he did not knowingly give is worse than none: {phrase:?}"
            );
        }
        assert!(note_prompt(&d, "s", "work the plan and autocontinue"));
        assert_eq!(state(&d, "s"), Autonomy::Granted);
    }

    #[test]
    fn the_three_spellings_are_one_word() {
        for spelling in ["autocontinue", "auto-continue", "AUTO CONTINUE"] {
            let d = tmp();
            assert!(note_prompt(&d, "s", spelling), "{spelling:?}");
            assert_eq!(state(&d, "s"), Autonomy::Granted, "{spelling:?}");
        }
    }

    /// The failure this whole mechanism exists to fix: he asks a question, the
    /// agent answers, and the session dies. A grant that expired on his next
    /// turn would be revoked by the very first question he asked.
    #[test]
    fn a_grant_survives_his_questions() {
        let d = tmp();
        note_prompt(&d, "s", "autocontinue");
        for turn in ["Status?", "I want a list of what is left", "why did you stop"] {
            note_prompt(&d, "s", turn);
            assert_eq!(
                state(&d, "s"),
                Autonomy::Granted,
                "answering {turn:?} must not end the grant"
            );
        }
    }

    /// His speaking clears the count. Without this the ceiling releases the
    /// grant on his third question — the exact behaviour the mechanism exists
    /// to prevent, produced by its own safety valve.
    #[test]
    fn a_turn_of_his_clears_the_empty_count() {
        let d = tmp();
        note_prompt(&d, "s", "autocontinue");
        note_turn(&d, "s", false);
        note_turn(&d, "s", false);
        assert_eq!(empty_turns(&d, "s"), MAX_EMPTY_TURNS, "at the ceiling");
        note_prompt(&d, "s", "Status?");
        assert_eq!(
            empty_turns(&d, "s"), 0,
            "he spoke, so the next empty turn is not evidence of a wedge"
        );
        assert_eq!(state(&d, "s"), Autonomy::Granted, "and the grant stands");
    }

    /// It ends by itself when his answer is needed, with nothing to type.
    ///
    /// A typed revoke is off exactly when he is least able to throw it — while
    /// he is asleep, which is the whole scenario this exists for.
    #[test]
    fn a_real_ask_ends_the_grant_with_nothing_typed() {
        let d = tmp();
        note_prompt(&d, "s", "autocontinue");
        assert_eq!(state(&d, "s"), Autonomy::Granted);
        assert!(clear(&d, "s"), "the ask clears it");
        assert_eq!(state(&d, "s"), Autonomy::NotGranted);
        // and it stays off until he grants it again
        note_prompt(&d, "s", "Status?");
        assert_eq!(state(&d, "s"), Autonomy::NotGranted);
        note_prompt(&d, "s", "autocontinue");
        assert_eq!(state(&d, "s"), Autonomy::Granted);
    }

    /// The bound counts EMPTY turns, not blocks — so a session that keeps
    /// working never approaches the ceiling however many times it is pushed.
    #[test]
    fn working_turns_never_exhaust_the_ceiling() {
        let d = tmp();
        note_prompt(&d, "s", "autocontinue");
        for _ in 0..50 {
            note_turn(&d, "s", true);
            assert_eq!(empty_turns(&d, "s"), 0, "a working turn resets the count");
        }
    }

    #[test]
    fn consecutive_empty_turns_reach_the_ceiling_and_one_work_turn_clears_it() {
        let d = tmp();
        note_prompt(&d, "s", "autocontinue");
        note_turn(&d, "s", false);
        assert_eq!(empty_turns(&d, "s"), 1);
        note_turn(&d, "s", false);
        assert_eq!(empty_turns(&d, "s"), MAX_EMPTY_TURNS, "wedged");
        note_turn(&d, "s", true);
        assert_eq!(empty_turns(&d, "s"), 0, "one real turn clears the wedge");
    }

    /// A bound that cannot be written is treated as SPENT, never as zero.
    ///
    /// The ceiling lives in this file. If the write fails, the count stops
    /// advancing and Rule B holds forever — the Cursor endless loop reached by
    /// a different road, and invisible, because a discarded Result fails
    /// exactly as quietly as a success.
    #[test]
    fn an_unwritable_bound_reports_failure_rather_than_freezing_the_count() {
        let d = tmp();
        note_prompt(&d, "s", "autocontinue");
        assert!(note_turn(&d, "s", false), "an ordinary write succeeds");

        // Make the file unwritable and confirm the caller is TOLD.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let f = dir(&d).join("s");
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o444)).unwrap();
            assert!(
                !note_turn(&d, "s", false),
                "a failed write must be reported, or the bound freezes in silence"
            );
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
    }

    #[test]
    fn no_session_id_is_unknown_not_a_denial() {
        let d = tmp();
        assert_eq!(state(&d, ""), Autonomy::Unknown);
        assert!(!note_prompt(&d, "", "autocontinue"));
    }
}
