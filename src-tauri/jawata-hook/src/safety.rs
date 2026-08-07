//! The fail-safe boundary: one exit, and its rule outranks correctness.
//!
//! > Any error, panic, timeout, missing config or malformed response →
//! > **emit nothing, exit 0.**
//!
//! A hook fires on every prompt and every shell command. If it hangs, the
//! editor hangs; if it exits non-zero under Cursor's `failClosed` guard, the
//! user's command is BLOCKED. So this module's job is not to be right — it is
//! to be harmless when everything else is wrong.
//!
//! `catch_unwind` alone is not enough, and the design says why. It does not
//! cover a stack overflow, an OOM, or a panic inside a `Drop`; it is disarmed
//! entirely by `panic = "abort"`; and it does nothing at all about a `stdin`
//! that never closes. All four are covered here.
//!
//! Everything is split so the hazards are TESTABLE: [`run_guarded`] returns an
//! [`Outcome`] a test can inspect, and only [`exit_with`] actually leaves the
//! process — a boundary whose sole implementation called `exit(0)` could not
//! be tested without killing the test runner, which is how a fail-safe becomes
//! a fail-safe nobody has ever run.

use std::io::Read;
use std::sync::mpsc;
use std::time::Duration;

/// Total wall-clock budget. Deliberately below every client timeout we write
/// (5s on Cursor's non-primer entries), because the deadline that protects the
/// user must be OURS: a client timeout fires after the user has already
/// waited, and on `failClosed` it fires as a block.
pub const TOTAL_DEADLINE: Duration = Duration::from_millis(4_000);

/// Budget for reading the event payload from stdin. Measured on Claude Code:
/// EOF at 4.3 ms. Cursor is unmeasured, and Cursor's own guidance is to bound
/// your own read — so we do, rather than trusting a client to close the pipe.
pub const STDIN_DEADLINE: Duration = Duration::from_millis(1_500);

/// Why this run emitted nothing. Stage 8 writes these to the silence log; the
/// point of the enum is that "the hook ran and said nothing" is never the whole
/// story available.
/// The silence taxonomy, declared ONCE.
///
/// This macro exists because the coupling it provides failed THREE TIMES when
/// written by hand: a doc comment claiming a witness match that did not exist;
/// a witness asserted against a hardcoded `0..=12`, defeated by adding a 14th
/// variant; and a `specimen()` that was exhaustive but only ever called on
/// seeds drawn from the very list it was meant to check — circular.
///
/// Here the variant, its stable log tag, and its test specimen are ONE row.
/// There is a single place to forget, and forgetting does not compile.
macro_rules! silence_reasons {
    ($( $(#[$doc:meta])* $variant:ident $(($ty:ty))? => $tag:literal, $specimen:expr );* $(;)?) => {
        /// Why the hook stayed quiet.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub enum SilenceReason {
            $( $(#[$doc])* $variant $(($ty))? ),*
        }

        /// The stable log tag — never a `Debug` rendering, which would tie the
        /// on-disk format to a Rust identifier and break every grep on rename.
        pub fn tag_of(reason: &SilenceReason) -> &'static str {
            match reason { $( SilenceReason::$variant { .. } => $tag ),* }
        }

        /// One specimen of EVERY variant, generated from the same row list.
        /// A new variant appears here automatically; it cannot be forgotten.
        #[cfg(test)]
        pub fn all_specimens() -> Vec<SilenceReason> {
            vec![ $( $specimen ),* ]
        }
    };
}

silence_reasons! {
    /// `argv[0]` was not a name we own.
    UnknownRole(String) => "unknown-role", SilenceReason::UnknownRole("jawata-hook-typo".into());
    /// The role exists but this client has no such event.
    RoleAbsentOnClient => "role-absent-on-client", SilenceReason::RoleAbsentOnClient;
    /// No endpoint configured — the studio has not deployed here.
    NotConfigured => "not-configured", SilenceReason::NotConfigured;
    /// The payload could not be read within our own deadline.
    StdinTimedOut => "stdin-timed-out", SilenceReason::StdinTimedOut;
    /// The payload was read but did not parse.
    PayloadUnreadable(String) => "payload-unreadable", SilenceReason::PayloadUnreadable("expected value".into());
    /// The prompt yielded no cues, and the cue module said why.
    NoCues(String) => "no-cues", SilenceReason::NoCues("TooFewContentTokens".into());
    /// The store was asked and genuinely had nothing.
    StoreHadNothing => "store-had-nothing", SilenceReason::StoreHadNothing;
    /// The store could not be asked, or answered in a shape we do not know.
    QueryFailed(String) => "query-failed", SilenceReason::QueryFailed("ConnectionRefused".into());
    /// This role cannot inject on this client.
    CannotInject => "cannot-inject", SilenceReason::CannotInject;
    /// The body ran past the deadline; the watchdog ended it and said so.
    WatchdogFired => "watchdog-fired", SilenceReason::WatchdogFired;
    /// The stop gate had no transcript it could read.
    NoTranscript => "no-transcript", SilenceReason::NoTranscript;
    /// The stop gate could not observe autonomy, so it did not judge.
    AutonomyUnknown => "autonomy-unknown", SilenceReason::AutonomyUnknown;
    /// The stop gate judged and allowed the turn to end.
    StopAllowed => "stop-allowed", SilenceReason::StopAllowed;
    /// The body panicked. Recorded, never propagated.
    Panicked(String) => "panicked", SilenceReason::Panicked("divide by zero".into());
}

/// What one guarded run produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Emitted(String),
    Silent(SilenceReason),
}

/// Run the body inside the boundary. Never panics.
///
/// The body returns `Ok(Some(text))` to emit, `Ok(None)`… — no: it must always
/// say WHY it is silent, which is why the silent arm carries a reason rather
/// than being an `Option`.
pub fn run_guarded<F>(body: F) -> Outcome
where
    F: FnOnce() -> Outcome + std::panic::UnwindSafe,
{
    // Silence the default panic printer: a hook that writes a backtrace to
    // stderr can still confuse a client that reads it, and the panic is being
    // handled here anyway.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(body);
    std::panic::set_hook(previous);

    match result {
        Ok(outcome) => outcome,
        // &*payload, NOT &payload: the latter is a &Box<dyn Any>, whose
        // concrete type is the Box itself, so every downcast misses and every
        // panic would be logged as "carrying no message" — a silence log full
        // of entries saying nothing, which is the failure this crate exists to
        // stop. The test that names the message is what caught it.
        Err(payload) => Outcome::Silent(SilenceReason::Panicked(describe_panic(&*payload))),
    }
}

fn describe_panic(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "a panic carrying no message".to_string()
    }
}

/// Arm the watchdog: whatever the main thread is doing when the deadline
/// passes, the process exits 0.
///
/// This is the layer `catch_unwind` cannot be. A stack overflow, an OOM, a
/// panic inside a `Drop`, a transport wedged below its own timeout — none of
/// them unwind into our handler, and all of them would otherwise leave the
/// client waiting on a process that never returns.
pub fn arm_watchdog(deadline: Duration) {
    arm_watchdog_recording(deadline, String::new());
}

/// Arm the watchdog so that firing RECORDS ITSELF before exiting.
///
/// The C8 audit found the one silence the log could not explain: on a 331 MB
/// transcript the body took 4,983 ms, the watchdog fired at 4,000 ms and called
/// `exit(0)` from this thread — while `silence::record` runs in `main` AFTER
/// the body returns, which it never did. The hook exited 0, emitted nothing,
/// and wrote no reason. That is exactly the two-week outage this crate exists
/// to end, and it was reachable for EVERY role, not just the slow one.
///
/// So the deadline path now writes its own record first. It is the last thing
/// that happens before the process ends. NOT free: `record` may first trim an
/// oversized log, which reads and rewrites up to `silence::MAX_BYTES`. Measured
/// overshoot past the deadline is ~3 ms, which is why this is acceptable rather
/// than because it does no work — an earlier version of this comment claimed
/// "one write_all of one line", which was simply false.
pub fn arm_watchdog_recording(deadline: Duration, role: String) {
    std::thread::spawn(move || {
        std::thread::sleep(deadline);
        // Record BEFORE exiting. A timeout that reports nothing is
        // indistinguishable from a hook that had nothing to say — the exact
        // ambiguity this crate was built to remove.
        if !role.is_empty() {
            crate::silence::record(&role, &Outcome::Silent(SilenceReason::WatchdogFired));
        }
        // Nothing else to flush: an emission is written and flushed before this
        // can matter, and a partial write is worse than none.
        std::process::exit(0);
    });
}

/// Read the event payload from stdin under OUR deadline.
///
/// The read happens on a helper thread. A blocked `read_to_end` cannot be
/// cancelled, so the deadline is enforced by ignoring the thread rather than
/// by stopping it — the watchdog guarantees the process still ends.
pub fn read_stdin(deadline: Duration) -> Result<String, SilenceReason> {
    read_with_deadline(deadline, || {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map(|_| buf)
            .unwrap_or_default()
    })
}

/// The deadline mechanism, with the source injectable so the never-closing
/// case is testable without a real terminal.
pub fn read_with_deadline<F>(deadline: Duration, source: F) -> Result<String, SilenceReason>
where
    F: FnOnce() -> String + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(source());
    });
    rx.recv_timeout(deadline).map_err(|_| SilenceReason::StdinTimedOut)
}

/// The only place this process leaves. Writes the emission, if any, and exits
/// 0 — always 0, whatever happened.
pub fn exit_with(outcome: &Outcome) -> ! {
    if let Outcome::Emitted(text) = outcome {
        use std::io::Write;
        let stdout = std::io::stdout();
        let mut lock = stdout.lock();
        // Errors deliberately ignored: a client that closed the pipe is not a
        // reason to fail, and there is nothing useful left to do about it.
        let _ = lock.write_all(text.as_bytes());
        let _ = lock.write_all(b"\n");
        let _ = lock.flush();
    }
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panicking_body_is_caught_and_named() {
        let outcome = run_guarded(|| panic!("the role exploded"));
        match outcome {
            Outcome::Silent(SilenceReason::Panicked(why)) => {
                assert!(why.contains("exploded"), "the panic's message is kept: {why}")
            }
            other => panic!("a panic must not escape the boundary: {other:?}"),
        }
    }

    #[test]
    fn a_panic_with_no_message_still_produces_a_reason() {
        let outcome = run_guarded(|| std::panic::panic_any(42u8));
        assert!(matches!(outcome, Outcome::Silent(SilenceReason::Panicked(_))));
    }

    #[test]
    fn a_body_that_indexes_out_of_bounds_is_caught_too() {
        // Not a deliberate panic! — the shape a real bug takes.
        let outcome = run_guarded(|| {
            let v: Vec<u8> = Vec::new();
            Outcome::Emitted(format!("{}", v[3]))
        });
        assert!(matches!(outcome, Outcome::Silent(SilenceReason::Panicked(_))));
    }

    #[test]
    fn a_normal_body_passes_through_untouched() {
        let outcome = run_guarded(|| Outcome::Emitted("hello".into()));
        assert_eq!(Outcome::Emitted("hello".into()), outcome);
    }

    #[test]
    fn a_stdin_that_never_closes_hits_OUR_deadline_not_the_clients() {
        // THE hazard: a client that opens the pipe and never closes it. The
        // script generation had no answer to this at all — it blocked in `cat`
        // until the client gave up, which on Cursor's failClosed guard is a
        // blocked user command.
        let started = std::time::Instant::now();
        let result = read_with_deadline(Duration::from_millis(120), || {
            std::thread::sleep(Duration::from_secs(30));
            "never arrives".to_string()
        });
        let elapsed = started.elapsed();
        assert_eq!(Err(SilenceReason::StdinTimedOut), result);
        assert!(
            elapsed < Duration::from_secs(2),
            "the deadline must fire on OUR schedule, took {elapsed:?}"
        );
    }

    #[test]
    fn a_payload_that_arrives_in_time_is_returned() {
        let result = read_with_deadline(Duration::from_millis(500), || "{\"prompt\":\"x\"}".into());
        assert_eq!(Ok("{\"prompt\":\"x\"}".to_string()), result);
    }

    #[test]
    fn our_deadline_is_under_every_client_timeout_we_write() {
        // The deploy writes timeout: 5 on Cursor's guard/recall/observer
        // entries and 15 on the primer. Ours must fire first, or the client's
        // timeout owns the outcome — and on failClosed that outcome is a block.
        assert!(
            TOTAL_DEADLINE < Duration::from_secs(5),
            "the total budget must beat the tightest client timeout (5s)"
        );
        assert!(
            STDIN_DEADLINE < TOTAL_DEADLINE,
            "the stdin read must not be able to consume the whole budget"
        );
    }

    #[test]
    fn a_panicking_body_never_escapes_the_boundary() {
        // panic = "abort" would disarm every catch_unwind above WITHOUT any
        // test failing — the process would simply die non-zero, which is the
        // one thing the boundary exists to prevent. Asserted at runtime rather
        // than by grepping a Cargo.toml, because the EFFECTIVE setting is what
        // matters and a member crate's [profile] is silently ignored anyway.
        // THE ASSERTION THAT USED TO LIVE HERE WAS VACUOUS, and the C5 audit
        // proved it with a control: cfg!(panic = "unwind") reads the TEST
        // profile, and Cargo IGNORES `panic` for test/bench profiles — an
        // isolated crate declaring panic = "abort" passed the identical
        // assertion under both `cargo test` and `cargo test --release`. It
        // could not fail, which made it worse than no test: it read as the
        // guarantee catch_unwind depends on.
        //
        // The real check asks rustc for the effective cfg of the RELEASE
        // profile — the one that ships — and lives in
        // tests/fail_safe_boundary.rs, where it can spawn a subprocess.
        // What stays here is the behaviour, which IS falsifiable in-process:
        // a panicking body must produce a Panicked outcome, not an escape.
        assert!(
            matches!(run_guarded(|| panic!("probe")), Outcome::Silent(SilenceReason::Panicked(_))),
            "a panic escaped the boundary"
        );
    }

    #[test]
    fn the_watchdog_ends_a_wedged_process() {
        // Cannot be asserted in-process without killing the runner, so it is
        // asserted on the mechanism: arming must not block the caller, and the
        // thread must outlive the function that spawned it.
        let started = std::time::Instant::now();
        arm_watchdog(Duration::from_secs(3_600));
        assert!(
            started.elapsed() < Duration::from_millis(50),
            "arming the watchdog must be immediate — it protects a hot path"
        );
    }

    #[test]
    fn every_silence_reason_can_say_something_useful() {
        // A reason that renders to nothing is the empty string again.
        let reasons = [
            SilenceReason::UnknownRole("jawata-hook-nope".into()),
            SilenceReason::RoleAbsentOnClient,
            SilenceReason::NotConfigured,
            SilenceReason::StdinTimedOut,
            SilenceReason::PayloadUnreadable("bad".into()),
            SilenceReason::NoCues("SlashCommand".into()),
            SilenceReason::StoreHadNothing,
            SilenceReason::QueryFailed("ShapeChanged".into()),
            SilenceReason::CannotInject,
            SilenceReason::Panicked("boom".into()),
        ];
        // A length check on a derived Debug is true by construction and says
        // nothing (C5 audit F7). What matters is that the reasons are
        // DISTINCT — a log in which two causes render alike is a log that
        // cannot tell them apart — and that the ones carrying a payload
        // actually show it.
        let rendered: Vec<String> = reasons.iter().map(|r| format!("{r:?}")).collect();
        let unique: std::collections::HashSet<&String> = rendered.iter().collect();
        assert_eq!(
            rendered.len(),
            unique.len(),
            "two silence reasons render identically, so the log cannot tell them apart: {rendered:?}"
        );
        assert!(rendered.iter().any(|r| r.contains("jawata-hook-nope")),
            "UnknownRole must show WHICH name was not ours");
        assert!(rendered.iter().any(|r| r.contains("ShapeChanged")),
            "QueryFailed must carry the underlying reason, not just its own name");
        assert!(rendered.iter().any(|r| r.contains("boom")),
            "Panicked must carry the panic's message");
    }
}
