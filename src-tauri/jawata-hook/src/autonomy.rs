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

/// QUOTES MEAN MENTION (v3.17.5) — the prompt with its quoted and backticked
/// spans removed, so only what survives can ARM the grant.
///
/// This is not intent-reading, which five releases of this file failed at and
/// which his ruling bans. Quoting is a SYNTAX he produces, not a meaning we
/// guess — and his own corpus already separates cleanly along it, unprompted:
/// every arming order is bare (*"resume the plan and autocontinue"*), every
/// discussing message quotes (*'Update the "autocontinue" flag'*). That last
/// one is the measured damage: on 2026-08-31 its quoted mention armed the
/// grant and pushed the agent through a whole morning nobody had granted.
///
/// Pairs are removed pair-wise for `"…"`, `` `…` `` and the typographic
/// `„…“` / `“…”`. An UNBALANCED opener removes nothing past itself — a lone
/// quotation mark must not swallow the rest of the message, because whatever
/// stands after it was not written inside quotes. The failure direction is the
/// established one: a mention that goes unquoted still arms, which costs a
/// push he can end by typing; the reverse — an order swallowed by a stray
/// quote — would cost a night, and cannot happen pair-wise.
///
/// CLEARING IS NOT TOUCHED. Any message of his still ends the grant, quoted or
/// not: the clear is about his presence, and a mention proves presence exactly
/// as an order does.
fn without_mentions(prompt: &str) -> String {
    let mut out = String::with_capacity(prompt.len());
    let mut chars = prompt.chars().peekable();
    while let Some(c) = chars.next() {
        let closer = match c {
            '"' => Some('"'),
            '`' => Some('`'),
            '\u{201E}' /* „ */ => Some('\u{201C}'),
            '\u{201C}' /* “ */ => Some('\u{201D}'),
            _ => None,
        };
        let Some(closer) = closer else {
            out.push(c);
            continue;
        };
        // Look ahead for the matching closer. Found: drop the span. Not found:
        // the opener was a stray character, so everything scanned is REAL text
        // and goes back into the output.
        let mut span = String::new();
        let mut closed = false;
        for d in chars.by_ref() {
            if d == closer {
                closed = true;
                break;
            }
            span.push(d);
        }
        if !closed {
            out.push(c);
            out.push_str(&span);
        }
    }
    out
}

/// Read the human's prompt and update the grant.
///
/// HIS ARRIVAL IS THE EVENT, AND NOTHING HERE READS WHAT HE MEANT. Harald,
/// 2026-08-30, after four releases of narrowing one guess after another:
/// *"I am sitting in front = I am typing = keyboard action into the chat
/// window -> Autocontinue off"*, and *"You cannot just filter for keywords
/// like Discuss"*.
///
/// So this function classifies nothing. It looks for ONE exact token and
/// treats every other message he sends as him being at the machine. Whether
/// he asked a question, gave an order, or thought out loud is not a fact this
/// gate needs, has ever needed, or can determine — and every attempt to
/// determine it has failed in a new way:
///
///   v3.14.2  the loop stands down "when the human is present" — inferred
///   v3.16.1  the keyboard is the human; the harness is not — right channel
///   v3.16.3  DECISION matched inside "design decisions" -> push disabled
///   v3.17.1  only Esc revokes — which Cursor has no key for at all
///   22:54    DISCUSS matched inside "We had a discussion" -> slept the night
///
/// Each release improved the guess. None removed it. This removes it.
///
/// Returns `true` when the prompt changed the state, so the caller can log a
/// state change rather than a no-op — a grant nobody can see happening is the
/// same invisible wiring this module exists to end.
pub fn note_prompt(base: &Path, session: &str, prompt: &str) -> bool {
    if session.is_empty() {
        return false;
    }
    // THE MACHINE IS NOT HIM, and this line is the whole of v3.17.2's defect.
    //
    // His rule named a CHANNEL — "I am sitting in front = I am typing = keyboard
    // action into the chat window -> Autocontinue off" — and v3.17.2 implemented
    // "any message", dropping the word that carried the requirement. But a
    // background job's completion notice arrives HERE, through the same hook as
    // his typing. So finishing a job cleared the grant, and any run that
    // backgrounded anything died at its first wake-up: measured 2026-08-30, the
    // grant went off at 11:59:23Z on a job completion and the session slept 21
    // minutes until he came back.
    //
    // The predicate is `stop::is_harness_text` and NOT a copy of its prefixes.
    // The stop side has had this check since an earlier overnight sleep; two
    // lists nothing forces to agree is how the same fix lands in one of them,
    // which happened twice in this crate this week.
    if crate::stop::is_harness_text(prompt) {
        // AND IT SAYS SO. The auditor's strongest finding on this change: a silent
        // early return makes the NEXT failure exactly as invisible as this one was.
        // Today was diagnosable to the second only because `autonomy-changed`
        // happened to be logged; if this prefix ever stops matching — a harness
        // change, another client's wrapper — a broken guard and a working one
        // produce an identical log. The first characters are carried so a mismatch
        // names itself instead of having to be reconstructed from a transcript.
        let head: String = prompt.chars().take(40).collect();
        crate::observer::emit_signal(base, "autonomy-harness-ignored", head.trim());
        return false;
    }
    // Mentions are stripped BEFORE the spelling collapse: quotes are what he
    // typed and survive lowercasing, so the order costs nothing — and only the
    // ARM below reads this. The clear at the bottom still sees every message.
    let text = normalised(&without_mentions(prompt));
    // HIS WORD ARMS IT, in the same breath as anything else. 2026-08-29 22:43
    // was exactly that shape: a past discussion named, an instruction given,
    // and the word at the end. He is leaving and saying so, and the word is
    // the saying — nothing else in the message is read.
    //
    // Re-arming is IDEMPOTENT and must stay so: the previous version returned
    // early if the file already existed, which meant a re-arm never reset the
    // empty-turn count. Writing "0" unconditionally is the whole point of the
    // word — it is a fresh stretch, not a continuation of the last one.
    if text.contains(GRANT) {
        let f = file(base, session);
        let _ = std::fs::create_dir_all(dir(base));
        return std::fs::write(&f, "0").is_ok();
    }
    // EVERY OTHER MESSAGE OF HIS ENDS THE GRANT. He typed, so he is here, so
    // the grant — which covers his ABSENCE and nothing else — is over. There
    // is no question detector, no imperative list and no `?` check, because
    // there is no longer any question being asked about his text.
    //
    // This also gives Cursor an off switch it never had. `clear` used to be
    // reachable only from an Esc marker in the transcript, and Cursor has no
    // Esc — it has a stop button that writes no marker — so on that client the
    // grant could be turned on and never off. His typing reaches every client.
    //
    // The two counters go with the grant: `clear` removes the rounds file, and
    // the empty-turn count lives in the grant file itself. Both measure a
    // stretch of autonomy, and this is the end of one.
    clear(base, session)
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

    /// ANY MESSAGE OF HIS ENDS THE GRANT, and nothing about it is classified.
    ///
    /// The four shapes below are the ones four releases argued over — a
    /// question, a work-order, a bare acknowledgement, and a sentence naming a
    /// past discussion. Under this rule they are the same event: he typed.
    /// A test that had to know which was which is the bug.
    #[test]
    fn every_message_of_his_ends_the_grant_and_only_his_word_restores_it() {
        for his_line in [
            "What is the defect?",
            "I want a list of what is left",
            "ok",
            "We had a discussion before and I did not tell you to move on",
        ] {
            let d = tmp();
            note_prompt(&d, "s", "autocontinue");
            assert_eq!(state(&d, "s"), Autonomy::Granted, "armed before {his_line:?}");
            assert!(
                note_prompt(&d, "s", his_line),
                "his arrival is a state change: {his_line:?}"
            );
            assert_eq!(
                state(&d, "s"),
                Autonomy::NotGranted,
                "he typed, so he is here: {his_line:?}"
            );
            assert!(note_prompt(&d, "s", "carry on and autocontinue"));
            assert_eq!(state(&d, "s"), Autonomy::Granted, "his word re-arms it");
        }
    }

    /// QUOTES MEAN MENTION — every line below is one of HIS real messages,
    /// which is the only corpus this rule may be judged against.
    ///
    /// The damage case is verbatim from 2026-08-31: 'Update the "autocontinue"
    /// flag as discussed above' armed the grant off its quoted mention and the
    /// agent ran a whole morning under it. The control half is as important:
    /// his arming orders are BARE, and stripping quotes must not touch them.
    #[test]
    fn a_quoted_mention_is_not_a_command() {
        let d = tmp();
        // His discussing messages — each contains the word ONLY inside quotes
        // or backticks, and none may arm.
        for mention in [
            r#"Update the "autocontinue" flag as discussed above"#,
            r#"If I say "autocontinue" then the hook puts the parameter to yes"#,
            r#"How can we add the "autocontinue" flag that you don't read the word mention as a command?"#,
            "the `autocontinue` grant file",
            "er sagte \u{201E}autocontinue\u{201C} und ging", // the German quotes he types
        ] {
            assert!(
                !note_prompt(&d, "s", mention),
                "a QUOTED word is a mention, and this one armed a whole morning: {mention:?}"
            );
            assert_eq!(Autonomy::NotGranted, state(&d, "s"), "{mention:?}");
        }

        // His arming orders — bare, and they must still arm after the strip.
        for order in [
            "resume the plan and autocontinue",
            "continue implementing the plan and autocontinue",
            "carry on and auto-continue",
        ] {
            assert!(note_prompt(&d, "s", order), "a bare order must arm: {order:?}");
            assert_eq!(Autonomy::Granted, state(&d, "s"), "{order:?}");
            clear(&d, "s");
        }
    }

    /// A STRAY QUOTE MUST NOT SWALLOW AN ORDER. Pair-wise stripping is the
    /// load-bearing property: an unbalanced opener costs nothing, because the
    /// reverse failure — an arming word eaten by a lone quotation mark earlier
    /// in the message — would silently cost a night.
    #[test]
    fn an_unbalanced_quote_does_not_eat_the_order() {
        let d = tmp();
        assert!(
            note_prompt(&d, "s", r#"fix the "flag and then autocontinue"#),
            "one lone quote, the word outside any completed pair — it must arm"
        );
        assert_eq!(Autonomy::Granted, state(&d, "s"));

        // And both in one message: a quoted mention does not shield a bare use.
        clear(&d, "s");
        assert!(
            note_prompt(&d, "s", r#"the "autocontinue" fix is in — autocontinue"#),
            "a bare use beside a quoted mention is still an order"
        );
        assert_eq!(Autonomy::Granted, state(&d, "s"));
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
        note_prompt(&d, "s", "work the plan and autocontinue");
        assert_eq!(state(&d, "s"), Autonomy::Granted, "armed");

        // A background job finishing. THIS is the line that slept the session.
        let noti = concat!("notifi", "cation");
        for machine in [
            format!("<task-{noti}>\n<status>completed</status>\n</task-{noti}>"),
            "<system-reminder>\nan automated reminder\n</system-reminder>".to_string(),
            "<local-command-stdout>output</local-command-stdout>".to_string(),
        ] {
            assert!(
                !note_prompt(&d, "s", &machine),
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
        note_prompt(&d, "s", "stop");
        assert_eq!(state(&d, "s"), Autonomy::NotGranted, "his word ended it");
        note_prompt(&d, "s", "<system-reminder>\nplease autocontinue\n</system-reminder>");
        assert_eq!(
            state(&d, "s"),
            Autonomy::NotGranted,
            "the machine cannot grant it either — only he can"
        );

        // THE CONTROL. If the guard over-reached to every message, the grant would
        // be unclearable and this fix would be worse than the defect.
        note_prompt(&d, "s", "carry on and autocontinue");
        assert_eq!(state(&d, "s"), Autonomy::Granted);
        note_prompt(&d, "s", "stop what you are doing");
        assert_eq!(
            state(&d, "s"),
            Autonomy::NotGranted,
            "HIS typing still ends it — the guard is about the channel, not about \
             making the grant sticky"
        );
    }

    /// THE 22:43 LINE, VERBATIM — the one that slept the night. It names a past
    /// discussion, gives an instruction, and ends with the word. Under the old
    /// rule the substring DISCUSS made this "a question" and silenced the push
    /// for the following eleven minutes. The word is the only thing read now.
    #[test]
    fn the_word_arms_it_however_the_rest_of_the_message_reads() {
        let d = tmp();
        note_prompt(
            &d,
            "s",
            "We had a discussion before and I did not tell you to move on. \
             Move on with the plan and autocontinue",
        );
        assert_eq!(
            state(&d, "s"),
            Autonomy::Granted,
            "the word is the grant; nothing else in the message is read"
        );
    }

    /// RE-ARMING IS A FRESH STRETCH. The previous version returned early when
    /// the grant file already existed, so saying the word again left the
    /// empty-turn count where it was — and a session already at the ceiling
    /// stayed at the ceiling, silently, with the grant apparently on.
    #[test]
    fn saying_the_word_again_resets_the_empty_turn_count() {
        let d = tmp();
        note_prompt(&d, "s", "autocontinue");
        note_turn(&d, "s", false);
        note_turn(&d, "s", false);
        assert_eq!(empty_turns(&d, "s"), MAX_EMPTY_TURNS, "at the ceiling");
        note_prompt(&d, "s", "autocontinue");
        assert_eq!(
            empty_turns(&d, "s"),
            0,
            "the word starts a new stretch, not a continuation of the wedged one"
        );
    }

    /// HIS SPEAKING TAKES THE COUNT WITH THE GRANT, and this test used to say
    /// the opposite on its last line: *"and the grant stands"*.
    ///
    /// It was right for the rule it was written under, where a message of his
    /// left the grant alone and only cleared the wedge counter — so the counter
    /// had to be cleared separately or the ceiling would revoke the grant on
    /// his third question. Under v3.17.2 his message ends the grant outright,
    /// so the counter cannot outlive it: both live in the same file, and
    /// `clear` removes it.
    ///
    /// KEPT RATHER THAN DELETED, because the property it guards still matters
    /// — no count survives into a stretch it was not measured in. What changed
    /// is which fact makes that true.
    #[test]
    fn a_turn_of_his_takes_the_count_with_the_grant() {
        let d = tmp();
        note_prompt(&d, "s", "autocontinue");
        note_turn(&d, "s", false);
        note_turn(&d, "s", false);
        assert_eq!(empty_turns(&d, "s"), MAX_EMPTY_TURNS, "at the ceiling");
        note_prompt(&d, "s", "carry on");
        assert_eq!(
            state(&d, "s"),
            Autonomy::NotGranted,
            "he typed, so the stretch is over — 'carry on' without the word is him arriving"
        );
        assert_eq!(
            empty_turns(&d, "s"),
            0,
            "and no count survives the stretch it measured"
        );
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
