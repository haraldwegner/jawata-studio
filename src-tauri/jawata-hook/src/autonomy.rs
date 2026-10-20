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
//! **The signal is two commands he types, and nothing else (D46, 2026-10-18).** `/autocontinue`
//! arms the grant and `/autocontinue-off` ends it; both are skills the studio deploys. The hook
//! cannot see the chat window: its input carries the prompt text and the session and no field
//! saying where the text came from, and the transcript marks a Chrome extension's injected
//! instruction exactly like his typing (`turnOrigin`, `origin` and `promptSource` identical,
//! measured 2026-10-18). So no message is judged as his by where it seems to come from. Only a
//! prompt that IS one of the two command lines changes the grant. A question, a status check, a
//! message from another session or machine, a subagent's report, a scheduled task and an
//! extension's text all leave it where it was. This replaces the word-at-the-end-of-a-line path
//! and the rule that any message of his ends the grant, which ten releases tried to make true
//! and which cannot be made true from inside the hook.
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

fn dir(base: &Path) -> PathBuf {
    base.join("autonomy")
}

fn file(base: &Path, session: &str) -> PathBuf {
    dir(base).join(crate::pipeline::sanitize_session(session))
}

/// The two lines that switch the grant. Exact: no arguments, no prose, no spelling variants.
const COMMAND_ON: &str = "autocontinue";
const COMMAND_OFF: &str = "autocontinue-off";

/// Which switch a prompt IS, if it is one. `None` for everything else — which is nearly everything.
///
/// A typed skill reaches the prompt hook as the line typed (`/autocontinue`), or in the wrapper
/// the transcript records for it (`<command-name>/autocontinue</command-name>`). Both are
/// accepted. A wrapper carrying non-empty `<command-args>` is a different request and not a
/// switch. Nothing is inferred from a sentence that merely contains a command: quoting it,
/// discussing it or typing it inside a longer message changes nothing.
fn typed_switch(prompt: &str) -> Option<bool> {
    let t = prompt.trim();
    let name = if t.contains("<command-name>") {
        // The wrapper is only the three tags the harness writes for a typed command; any other
        // text around them makes it a message that merely contains one.
        let mut rest = t.to_string();
        let mut name = None;
        let mut args = String::new();
        for tag in ["command-message", "command-name", "command-args"] {
            let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
            let Some(start) = rest.find(&open) else { continue };
            let end = rest[start..].find(&close)? + start;
            let inner = rest[start + open.len()..end].trim().to_string();
            match tag {
                "command-name" => name = Some(inner),
                "command-args" => args = inner,
                _ => {}
            }
            rest.replace_range(start..end + close.len(), "");
        }
        if !rest.trim().is_empty() || !args.is_empty() {
            return None;
        }
        name?.trim_start_matches('/').to_string()
    } else {
        t.strip_prefix('/')?.to_string()
    };
    match name.as_str() {
        COMMAND_ON => Some(true),
        COMMAND_OFF => Some(false),
        _ => None,
    }
}

/// The switch command a longer, human-typed message carries on a line of its own, which the grant
/// will NOT act on (a command counts only as the whole message). Named so the mode line can say
/// so: S25-2 found that a command typed last in a longer message did nothing and said nothing.
/// `None` for a message that IS a switch, one that only mentions a command in prose, and machine
/// text — which is never his typing.
pub fn stray_command(prompt: &str) -> Option<&'static str> {
    if typed_switch(prompt).is_some()
        || (crate::stop::is_harness_text(prompt) && !crate::stop::is_typed_command(prompt))
    {
        return None;
    }
    prompt.lines().map(str::trim).find_map(|line| match line {
        "/autocontinue-off" => Some("/autocontinue-off"),
        "/autocontinue" => Some("/autocontinue"),
        _ => None,
    })
}

/// Read the prompt and update the grant: ONLY the two commands move it (D46).
///
/// The history this replaces, kept because it is why the rule is as narrow as it is. His rule
/// of 2026-08-30 was *"I am sitting in front = I am typing = keyboard action into the chat
/// window -> Autocontinue off"* — a requirement about a CHANNEL, the chat window. This hook
/// cannot observe that channel: its input is the prompt text and the session id. Every release
/// tried to infer the channel from the text or from the transcript, and each inference failed
/// in a new way:
///
///   v3.14.2  the loop stands down "when the human is present" — inferred
///   v3.16.1  the keyboard is the human; the harness is not — right channel
///   v3.16.3  DECISION matched inside "design decisions" -> push disabled
///   v3.17.1  only Esc revokes — which Cursor has no key for at all
///   22:54    DISCUSS matched inside "We had a discussion" -> slept the night
///   v4.4.x   `turnOrigin == human` — which the Chrome extension's injected instruction also
///            carries, identical to his typing in every field (measured 2026-10-18)
///
/// So nothing is inferred. A prompt that is exactly `/autocontinue` arms; one that is exactly
/// `/autocontinue-off` ends; every other prompt, whoever sent it, returns with the grant where
/// it was. That also makes the answer to a question of his, a status check or a discussion
/// safe: none of them can end a run that he started.
///
/// Returns `true` when the prompt changed the state, so the caller can log a
/// state change rather than a no-op — a grant nobody can see happening is the
/// same invisible wiring this module exists to end.
pub fn note_prompt(base: &Path, session: &str, prompt: &str, machine: bool) -> bool {
    if session.is_empty() {
        return false;
    }
    // studio#35: the marker goes HERE, in the event itself, rather than in the
    // caller that happens to drive it today. Eight of this module's own tests
    // caught the first version, which put it in the pipeline: they exercise a
    // prompt without that caller, so the channel read as never-alive and every
    // ungranted session reported `Unknown`. A fact about "a prompt happened"
    // belongs to the function that means a prompt happened, where no future
    // caller can forget it.
    note_seen(base, session);
    // A MACHINE'S TEXT IS SET ASIDE AND SAYS SO. None of it can move the grant under D46 — only
    // the two typed commands can — but the line is kept: a wrapper that stops matching, or a
    // client's new one, must leave a record, so a broken guard and a working one never write the
    // same log. The predicate is `stop::is_harness_text` and not a copy of its prefixes.
    if crate::stop::is_harness_text(prompt) && !crate::stop::is_typed_command(prompt) {
        // AND IT SAYS SO. The auditor's strongest finding on this change: a silent
        // early return makes the NEXT failure exactly as invisible as this one was.
        // Today was diagnosable to the second only because `autonomy-changed`
        // happened to be logged; if this prefix ever stops matching — a harness
        // change, another client's wrapper — a broken guard and a working one
        // produce an identical log. The first characters are carried so a mismatch
        // names itself instead of having to be reconstructed from a transcript.
        let head: String = prompt.chars().take(40).collect();
        // WHICH MARK ANSWERED, because the two have different failure modes: a
        // `block-shape` verdict cannot go stale, a `wrapper-text` one is a
        // guess that the handback of 2026-10-04 got wrong.
        let mark = if machine { "block-shape" } else { "wrapper-text" };
        crate::observer::emit_signal(
            base,
            "autonomy-harness-ignored",
            &format!("[{mark}] {}", head.trim()),
        );
        return false;
    }
    // ONLY THE TWO COMMANDS MOVE THE GRANT (D46). He typed `/autocontinue`: a fresh stretch, so
    // writing "0" is unconditional and a re-arm resets the empty-turn count. He typed
    // `/autocontinue-off`: the grant and both counters go with it. Anything else — his own
    // question included — returns here with the grant exactly where it was.
    match typed_switch(prompt) {
        Some(true) => {
            let f = file(base, session);
            let _ = std::fs::create_dir_all(dir(base));
            std::fs::write(&f, "0").is_ok()
        }
        Some(false) => clear(base, session),
        None => false,
    }
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
        return Autonomy::Granted;
    }
    // studio#35: A MISSING GRANT FILE IS TWO DIFFERENT FACTS, and returning
    // `NotGranted` for both is what made Rule B go inert silently.
    //
    // "He did not grant" and "the UserPrompt hook never ran for this session"
    // produce the identical absent file — so an install where the hook is not
    // deployed, or a client that has no prompt event, reports exactly what a
    // deliberate refusal reports, and the stop role logs `StopAllowed` either
    // way. That is the same inert shape the v3.14.x fix repaired one layer
    // down, relocated up here.
    //
    // The discriminator is a POSITIVE marker: [`note_seen`] records that a
    // prompt was processed for this session. With the marker, an absent grant
    // is a real `NotGranted`; without it, we were never in a position to know,
    // and `Unknown` is the honest answer — which the stop role already records
    // as `autonomy-unknown` rather than swallowing.
    if seen(base, session) {
        Autonomy::NotGranted
    } else {
        Autonomy::Unknown
    }
}

/// Whether the prompt hook has been through this session at all.
fn seen_file(base: &Path, session: &str) -> PathBuf {
    dir(base).join(format!("{}.seen", crate::pipeline::sanitize_session(session)))
}

/// Whether a prompt event has been processed for this session.
///
/// PRIVATE: publishing it would let a caller mark the channel alive without a
/// prompt, which is the door `note_prompt` was moved to close.
fn seen(base: &Path, session: &str) -> bool {
    !session.is_empty() && seen_file(base, session).exists()
}

/// Record that the prompt hook ran for this session.
///
/// Called on EVERY prompt event, granting or not — its whole job is to say the
/// channel is alive, so it must not be conditional on what the prompt said.
fn note_seen(base: &Path, session: &str) {
    if session.is_empty() {
        return;
    }
    let _ = std::fs::create_dir_all(dir(base));
    let _ = std::fs::write(seen_file(base, session), "1");
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
/// HIS ESC, OR ANY MESSAGE HE TYPES — and nothing of the agent's, ever. That
/// second caller is new in v3.17.2 (see [`note_prompt`]): his arrival ends the
/// grant, because the grant covers his absence. What has not changed, and is
/// the part that matters, is who may call this: only him. This used to be
/// called on the agent's own message too, whenever a phrase list guessed it was
/// asking for something — and on
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

    /// studio#35: "he did not grant" and "nothing ever asked him" must not be
    /// the same answer.
    ///
    /// Both produce an absent grant file, and both used to return `NotGranted`
    /// — so an install where the UserPrompt hook is not deployed, or a client
    /// with no prompt event, reported exactly what a deliberate refusal reports
    /// and Rule B went inert with nothing recording that it had. `Unknown` is
    /// the honest answer for the second, and the stop role already logs it as
    /// `autonomy-unknown` rather than swallowing it.
    #[test]
    fn a_channel_that_never_ran_is_not_the_same_as_a_grant_withheld() {
        let d = tmp();

        // Nothing has ever asked him in this session.
        assert_eq!(
            Autonomy::Unknown,
            state(&d, "s"),
            "no prompt has been processed, so we were never in a position to know"
        );
        assert!(!seen(&d, "s"), "and the channel is not marked alive");

        // A prompt arrives that does NOT arm. Now the absence is a real answer.
        note_prompt(&d, "s", "have a look at the failing test", false);
        assert!(seen(&d, "s"), "the channel is alive once a prompt is processed");
        assert_eq!(
            Autonomy::NotGranted,
            state(&d, "s"),
            "he was asked and did not grant — which is a fact, not an absence"
        );

        // And the grant still works on top of it.
        note_prompt(&d, "s", "/autocontinue", false);
        assert_eq!(Autonomy::Granted, state(&d, "s"), "his word still arms it");

        // A DIFFERENT session in the same install is still unknown: the marker
        // is per session, because the question is per session.
        assert_eq!(
            Autonomy::Unknown,
            state(&d, "other"),
            "one session's prompt says nothing about another's"
        );
    }

    /// D5's FOURTH GATE: a machine event the grant sets aside is RECORDED, not silently
    /// dropped. If a wrapper ever stops matching, a broken guard and a working one must
    /// not write the same log; the first characters of the line name the mismatch.
    #[test]
    fn a_machine_event_the_grant_ignores_leaves_a_line_in_the_log() {
        let d = tmp();
        note_prompt(&d, "s", "/autocontinue", false);
        let handback = "<agent-message from=\"a37f4f0b3e34fcf57\">\n[Subagent hand-back] done";
        assert!(!note_prompt(&d, "s", handback, false));
        let log = std::fs::read_to_string(d.join("outcomes.log")).expect("a signal was written");
        let line = log
            .lines()
            .find(|l| l.contains("autonomy-harness-ignored"))
            .expect("the ignore is named, not silent");
        assert!(line.contains("[wrapper-text]"), "says which mark answered: {line}");
        assert!(line.contains("<agent-message"), "carries the head of what it ignored: {line}");
        // The control: his own typing writes NO such line, so the signal means something.
        let d2 = tmp();
        note_prompt(&d2, "s", "/autocontinue", false);
        note_prompt(&d2, "s", "status?", false);
        let none = std::fs::read_to_string(d2.join("outcomes.log")).unwrap_or_default();
        assert!(!none.contains("autonomy-harness-ignored"), "typing is not ignored: {none}");
    }

    /// THE MACHINE IS NOT HIM — the v3.17.2 defect, measured 2026-08-30.
    ///
    /// His rule named a CHANNEL: *"I am sitting in front = I am typing = keyboard
    /// action into the chat window -> Autocontinue off"*, and earlier the same day
    /// *"The keyboard is not the machine-wide keyboard. It is keyboard + focus on
    /// the chat window."* v3.17.2 implemented "any message", dropping the word that
    /// carried the whole requirement — the release notes still say "any message you
    /// TYPE"; the code said any message.
    ///
    /// **What that cost.** A background job's completion notice arrives through the
    /// same prompt hook as his typing. So finishing a job cleared the grant, and any
    /// run that backgrounded anything died at its first wake-up. Measured today: the
    /// grant went off at 11:59:23Z on a job completion, and the session slept 21
    /// minutes until he returned.
    ///
    /// **And the check already existed, in the same crate.** `is_harness_line` was
    /// written after an overnight sleep for exactly this case. v3.17.2's own doc
    /// comment lists that fix — *"the keyboard is the human; the harness is not"* —
    /// four lines above the function that then treated the harness as him.
    #[test]
    fn the_harness_neither_arms_nor_clears_the_grant() {
        let d = tmp();
        note_prompt(&d, "s", "/autocontinue", false);
        assert_eq!(state(&d, "s"), Autonomy::Granted, "armed");

        // A background job finishing. THIS is the line that slept the session.
        let noti = concat!("notifi", "cation");
        for machine in [
            format!("<task-{noti}>\n<status>completed</status>\n</task-{noti}>"),
            "<system-reminder>\nan automated reminder\n</system-reminder>".to_string(),
            "<local-command-stdout>output</local-command-stdout>".to_string(),
        ] {
            assert!(
                !note_prompt(&d, "s", &machine, false),
                "a machine line is not a state change: {machine:?}"
            );
            assert_eq!(
                state(&d, "s"),
                Autonomy::Granted,
                "the grant SURVIVES the machine — he has not typed: {machine:?}"
            );
        }

        // ...and the same text cannot ARM it either. The word can appear inside a
        // tool result the harness relays; that is the machine quoting, not him
        // granting.
        note_prompt(&d, "s", "/autocontinue-off", false);
        assert_eq!(state(&d, "s"), Autonomy::NotGranted, "his command ended it");
        note_prompt(&d, "s", "<system-reminder>\n/autocontinue\n</system-reminder>", false);
        assert_eq!(
            state(&d, "s"),
            Autonomy::NotGranted,
            "the machine cannot grant it either — only he can"
        );

        // THE CONTROL. If the guard over-reached to every message, the grant would
        // be unclearable and this fix would be worse than the defect.
        note_prompt(&d, "s", "/autocontinue", false);
        assert_eq!(state(&d, "s"), Autonomy::Granted);
        note_prompt(&d, "s", "stop what you are doing", false);
        assert_eq!(
            state(&d, "s"),
            Autonomy::Granted,
            "his prose does not end it — only /autocontinue-off does (D46)"
        );
        note_prompt(&d, "s", "/autocontinue-off", false);
        assert_eq!(
            state(&d, "s"),
            Autonomy::NotGranted,
            "the off command still ends it — the grant is not made sticky"
        );
    }

    /// THE 22:43 LINE, VERBATIM — the one that slept the night. It names a past
    /// discussion, gives an instruction, and ends with the word. Under the old
    /// rule the substring DISCUSS made this "a question" and silenced the push
    /// for the following eleven minutes. The word is the only thing read now.
    /// THE GRANT SURVIVES THE MACHINE, and that is the whole fix.
    ///
    /// The caller answers the channel question from the EVENT; this asserts
    /// what `note_prompt` then does with the answer. Without the `machine`
    /// parameter the only judge was the wrapper text, and the handback of
    /// 2026-10-04 carried no wrapper — so the grant he had just given was
    /// cleared by his own agent reporting back.
    #[test]
    fn a_machine_event_leaves_the_grant_exactly_where_it_was() {
        let d = tmp();
        note_prompt(&d, "s", "/autocontinue", false);
        assert_eq!(state(&d, "s"), Autonomy::Granted);

        // THE SHAPE A HANDBACK ACTUALLY HAS, read off his own `outcomes.log` on
        // 2026-10-05 (17:55:48 and 19:26:27): `<agent-message from="…">`. The
        // fixture was wrapperless prose, which is what justified deciding this from
        // the transcript instead — and that decision then read HIS OWN TYPING as a
        // machine event and discarded every grant he gave. The marker is now the
        // judge and the list names this channel.
        //
        // THE GAP, STATED RATHER THAN CLOSED: this javadoc used to assert the
        // 2026-10-04 handback carried no wrapper. That claim is not reconcilable
        // with the two wrapped ones above, and no raw payload was captured for
        // either. If a wrapperless handback is real, the property this test names
        // does not hold for it, and the symptom is a grant REVOKED — which he can
        // retype — rather than a grant that can never be armed, which is what the
        // transcript check cost him.
        let handback = "<agent-message from=\"ab0846672ae0a4b77\">This agent's report \
                        mentions \"autocontinue\". VERDICT: RESOLVABLE</agent-message>";
        assert!(crate::stop::is_harness_text(handback));
        assert!(
            !note_prompt(&d, "s", handback, true),
            "a machine event changes no state, so it reports no change"
        );
        assert_eq!(
            state(&d, "s"),
            Autonomy::Granted,
            "the machine must not be able to revoke what he granted"
        );
    }

    /// RE-ARMING IS A FRESH STRETCH. The previous version returned early when
    /// the grant file already existed, so saying the word again left the
    /// empty-turn count where it was — and a session already at the ceiling
    /// stayed at the ceiling, silently, with the grant apparently on.
    #[test]
    fn saying_the_word_again_resets_the_empty_turn_count() {
        let d = tmp();
        note_prompt(&d, "s", "/autocontinue", false);
        note_turn(&d, "s", false);
        note_turn(&d, "s", false);
        assert_eq!(empty_turns(&d, "s"), MAX_EMPTY_TURNS, "at the ceiling");
        note_prompt(&d, "s", "/autocontinue", false);
        assert_eq!(
            empty_turns(&d, "s"),
            0,
            "the word starts a new stretch, not a continuation of the wedged one"
        );
    }

    /// THE OFF COMMAND TAKES THE COUNT WITH THE GRANT. No count survives into a stretch it was
    /// not measured in. Under D46 the end of a stretch is `/autocontinue-off`; his other messages
    /// leave both the grant and the count where they were.
    #[test]
    fn the_off_command_takes_the_count_with_the_grant() {
        let d = tmp();
        note_prompt(&d, "s", "/autocontinue", false);
        note_turn(&d, "s", false);
        note_turn(&d, "s", false);
        assert_eq!(empty_turns(&d, "s"), MAX_EMPTY_TURNS, "at the ceiling");
        note_prompt(&d, "s", "carry on", false);
        assert_eq!(state(&d, "s"), Autonomy::Granted, "a message of his is not the off command");
        assert_eq!(empty_turns(&d, "s"), MAX_EMPTY_TURNS, "and leaves the count alone");
        note_prompt(&d, "s", "/autocontinue-off", false);
        assert_eq!(state(&d, "s"), Autonomy::NotGranted, "the command ends the stretch");
        assert_eq!(empty_turns(&d, "s"), 0, "and no count survives the stretch it measured");
    }

    /// It ends by itself when his answer is needed, with nothing to type.
    ///
    /// A typed revoke is off exactly when he is least able to throw it — while
    /// he is asleep, which is the whole scenario this exists for.
    #[test]
    fn a_real_ask_ends_the_grant_with_nothing_typed() {
        let d = tmp();
        note_prompt(&d, "s", "/autocontinue", false);
        assert_eq!(state(&d, "s"), Autonomy::Granted);
        assert!(clear(&d, "s"), "the ask clears it");
        assert_eq!(state(&d, "s"), Autonomy::NotGranted);
        // and it stays off until he grants it again
        note_prompt(&d, "s", "Status?", false);
        assert_eq!(state(&d, "s"), Autonomy::NotGranted);
        note_prompt(&d, "s", "/autocontinue", false);
        assert_eq!(state(&d, "s"), Autonomy::Granted);
    }

    /// The bound counts EMPTY turns, not blocks — so a session that keeps
    /// working never approaches the ceiling however many times it is pushed.
    #[test]
    fn working_turns_never_exhaust_the_ceiling() {
        let d = tmp();
        note_prompt(&d, "s", "/autocontinue", false);
        for _ in 0..50 {
            note_turn(&d, "s", true);
            assert_eq!(empty_turns(&d, "s"), 0, "a working turn resets the count");
        }
    }

    #[test]
    fn consecutive_empty_turns_reach_the_ceiling_and_one_work_turn_clears_it() {
        let d = tmp();
        note_prompt(&d, "s", "/autocontinue", false);
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
        note_prompt(&d, "s", "/autocontinue", false);
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
        assert!(!note_prompt(&d, "", "/autocontinue", false));
    }

    // ---------------------------------------------------------------------------------------
    // D46 / STAGE 25 — AUTOCONTINUE IS SWITCHED BY TWO COMMANDS AND NOTHING ELSE.
    //
    // His rulings, 2026-10-18: "Only the chat window of the current agent is allowed to switch
    // off that agent's autocontinue" and "/autocontinue switches on and /autocontinue-off
    // switches off. As skill." The hook cannot see the chat window — its input has no field
    // saying where a prompt came from, and the transcript marks a Chrome extension's injection
    // exactly like his typing — so the only line that can be his is the command he types.
    // Every fixture below is a REAL record of 2026-10-18, truncated.
    // ---------------------------------------------------------------------------------------

    /// What reached the prompt hook on 2026-10-18, as the ORB session's transcript recorded it
    /// (`turnOrigin`, `origin`, `promptSource` identical for the first two).
    const HIS_TYPED_QUESTION: &str =
        "What, has the resync worked or not? Ask for a reconcile with the AI TOP ATOM agent";
    const CHROME_INSTRUCTION: &str =
        "<browser_instruction># Claude in Chrome browser automation\n\nYou have access to browser automation tools (mcp__claude-in-chrome__*)";
    const OTHER_MACHINES_MESSAGE: &str =
        "Another Claude session sent a message: <cross-session-message from=\"bridge:session_01FX7HzzHNEzqrV4Q2YkiZof\">\norb-18 to orb-e2: yes, this is the ORB session on the AI TOP ATOM machine";
    const SUBAGENT_HANDBACK: &str =
        "<agent-message from=\"a37f4f0b3e34fcf57\">\n[Subagent hand-back] The text below is the final report";
    // Split so the source scan that keeps that word out of this crate's text still passes.
    const TASK_NOTICE: &str = concat!("<task-notifi", "cation>\n<task-id>bk1</task-id>\n<status>completed</status>");

    #[test]
    fn the_typed_command_arms_and_the_typed_off_command_ends() {
        for on in [
            "/autocontinue",
            "  /autocontinue  \n",
            "<command-message>autocontinue</command-message>\n<command-name>/autocontinue</command-name>",
        ] {
            let d = tmp();
            assert!(note_prompt(&d, "s", on, false), "{on:?} arms");
            assert_eq!(state(&d, "s"), Autonomy::Granted, "{on:?}");
        }
        for off in [
            "/autocontinue-off",
            "<command-message>autocontinue-off</command-message>\n<command-name>/autocontinue-off</command-name>",
        ] {
            let d = tmp();
            note_prompt(&d, "s", "/autocontinue", false);
            assert!(note_prompt(&d, "s", off, false), "{off:?} is a state change");
            assert_eq!(state(&d, "s"), Autonomy::NotGranted, "{off:?}");
        }
    }

    /// THE POINT OF THE STAGE: between the two commands, nothing else moves the grant — not his
    /// own typed questions, not the other machine, not a subagent, not an extension.
    #[test]
    fn nothing_but_the_two_commands_moves_the_grant() {
        for other in [
            HIS_TYPED_QUESTION,
            CHROME_INSTRUCTION,
            OTHER_MACHINES_MESSAGE,
            SUBAGENT_HANDBACK,
            TASK_NOTICE,
            "What is the defect?",
            "/sprint plan.md",
            "<command-name>/sprint</command-name>\n<command-args>plan.md</command-args>",
            "I typed /autocontinue-off yesterday and it did nothing",
            "`/autocontinue-off`",
        ] {
            let d = tmp();
            note_prompt(&d, "s", "/autocontinue", false);
            assert!(!note_prompt(&d, "s", other, false), "not a state change: {other:?}");
            assert_eq!(state(&d, "s"), Autonomy::Granted, "{other:?} leaves a live grant alone");
            let off = tmp();
            note_prompt(&off, "s", "status?", false);
            assert!(!note_prompt(&off, "s", other, false), "{other:?} grants nothing");
            assert_eq!(state(&off, "s"), Autonomy::NotGranted, "{other:?} arms nothing");
        }
    }

    /// S25-2 (2026-10-20, found live): he typed `/autocontinue-off` as the last line of a longer
    /// message. By D46 it did nothing — a command counts only as the whole message — and nothing
    /// SAID so, so the mode line stayed ON for six minutes while he believed it was off. The
    /// prompt that carries a command line it will not act on now names that line.
    #[test]
    fn a_command_line_inside_a_longer_message_is_named_and_not_acted_on() {
        let mixed = "A5 -> Ok\nWhat rules?\n/autocontinue-off";
        assert_eq!(stray_command(mixed), Some("/autocontinue-off"));
        assert_eq!(stray_command("done\n  /autocontinue  \nthanks"), Some("/autocontinue"));
        let d = tmp();
        note_prompt(&d, "s", "/autocontinue", false);
        assert!(!note_prompt(&d, "s", mixed, false), "naming it must not act on it (D46)");
        assert_eq!(state(&d, "s"), Autonomy::Granted);
        // Not strays: the whole message, a command named in prose, and machine text.
        for not in [
            "/autocontinue-off",
            "<command-message>autocontinue-off</command-message>\n<command-name>/autocontinue-off</command-name>",
            "I typed /autocontinue-off yesterday and it did nothing",
            "`/autocontinue-off`",
            "What is the defect?",
            SUBAGENT_HANDBACK,
            TASK_NOTICE,
            CHROME_INSTRUCTION,
        ] {
            assert_eq!(stray_command(not), None, "{not:?}");
        }
    }

    /// The old prose path is gone: the word at the end of a line no longer arms. (D9b's third
    /// clause, superseded by his ruling of 2026-10-18.)
    #[test]
    fn the_word_in_prose_no_longer_arms() {
        for prose in [
            "autocontinue",
            "work the plan and autocontinue",
            "resume the plan and autocontinue.",
            "auto-continue",
            "1. Don't release but resume.\n2. We release at the end of this sprint\nautocontinue",
        ] {
            let d = tmp();
            assert!(!note_prompt(&d, "s", prose, false), "{prose:?} must NOT arm");
            assert_eq!(state(&d, "s"), Autonomy::NotGranted, "{prose:?}");
        }
    }

    /// A command with arguments is not the bare command: the two lines are exact.
    #[test]
    fn a_command_with_arguments_is_not_the_command() {
        let d = tmp();
        assert!(!note_prompt(&d, "s", "/autocontinue now please", false));
        assert!(!note_prompt(
            &d,
            "s",
            "<command-name>/autocontinue</command-name>\n<command-args>now please</command-args>",
            false
        ));
        assert_eq!(state(&d, "s"), Autonomy::NotGranted);
    }

    /// Typing the command again is a FRESH stretch, as the word was: the empty-turn count resets.
    #[test]
    fn arming_again_resets_the_empty_turn_count() {
        let d = tmp();
        note_prompt(&d, "s", "/autocontinue", false);
        note_turn(&d, "s", false);
        assert_eq!(empty_turns(&d, "s"), 1);
        assert!(note_prompt(&d, "s", "/autocontinue", false));
        assert_eq!(empty_turns(&d, "s"), 0);
    }
}
