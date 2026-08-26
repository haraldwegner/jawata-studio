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
//! and answering a question is precisely where the agent has been stopping. It
//! ends when he says so, or when the session does.
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

/// The words that end it.
///
/// Spelled out rather than derived, because a revoke that the human types and
/// the gate does not honour is the worse failure of the two: he would believe
/// he had taken the leash off and the session would keep pushing itself.
const REVOKES: [&str; 4] = [
    "stop autocontinue",
    "no autocontinue",
    "end autocontinue",
    "cancel autocontinue",
];

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
    // Revoke is checked FIRST and wins. "stop autocontinue" contains the grant
    // word, so any other order grants on the message that was meant to end it.
    if REVOKES.iter().any(|r| text.contains(r)) {
        return std::fs::remove_file(file(base, session)).is_ok();
    }
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

/// Record what this turn did: work resets the count, an empty turn advances it.
///
/// Called on every stop under a grant, whatever the verdict, because the number
/// is about the TURN and not about the ruling. Deciding it inside one verdict
/// arm is how a measurement silently becomes conditional on the thing it is
/// supposed to be measuring.
pub fn note_turn(base: &Path, session: &str, did_work: bool) {
    if session.is_empty() {
        return;
    }
    let f = file(base, session);
    if !f.exists() {
        return;
    }
    let next = if did_work { 0 } else { empty_turns(base, session) + 1 };
    let _ = std::fs::write(&f, next.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "jawata-autonomy-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
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

    #[test]
    fn revoke_wins_over_the_grant_word_it_contains() {
        let d = tmp();
        note_prompt(&d, "s", "autocontinue");
        assert_eq!(state(&d, "s"), Autonomy::Granted);
        // "stop autocontinue" CONTAINS "autocontinue" — checked in the wrong
        // order it re-grants on the message meant to end it.
        note_prompt(&d, "s", "stop autocontinue please");
        assert_eq!(state(&d, "s"), Autonomy::NotGranted);
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

    #[test]
    fn no_session_id_is_unknown_not_a_denial() {
        let d = tmp();
        assert_eq!(state(&d, ""), Autonomy::Unknown);
        assert!(!note_prompt(&d, "", "autocontinue"));
    }
}
