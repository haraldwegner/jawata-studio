//! Why the hook stayed quiet — recorded, so silence stops being ambiguous.
//!
//! Sprint 28 Stage 8. The failure this crate exists to end was two weeks of a
//! hook injecting nothing while every suite stayed green: the scripts produced
//! an empty cue, and an empty cue is indistinguishable from "the store had
//! nothing to say". Both are silence. Only one is a bug.
//!
//! The reason already existed — [`SilenceReason`] is computed on every path and
//! carried to the exit. It was simply thrown away one line before it could be
//! useful. This module writes it down.
//!
//! Three properties, each of which the log would be worthless without:
//!
//! 1. **Writing the log can never change the hook's behaviour.** Every function
//!    here swallows its own errors. A full disk, a read-only home, a path that
//!    is a directory — the hook still exits 0, still on time. A diagnostic that
//!    can break the thing it diagnoses is worse than no diagnostic.
//! 2. **Concurrent invocations both survive.** Hooks fire on overlapping
//!    events; two processes will write at the same moment. Each record is a
//!    single `write_all` of one complete line under `O_APPEND`, which the OS
//!    orders atomically for writes below the pipe buffer. No lock file, because
//!    a lock introduces a way to block — and blocking is the one thing the fire
//!    path may never do.
//! 3. **It cannot grow without bound.** A hook fires on every prompt of every
//!    session forever. The log is capped and truncated from the front.

use std::fmt::Write as _;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::safety::{Outcome, SilenceReason};

/// Above this size the log is ROTATED — renamed aside, a fresh one begun. Chosen so a
/// busy day of hooks is retained and a year of them is not.
pub const MAX_BYTES: u64 = 256 * 1024;

/// The stable tag for a reason — the thing a human greps for, and the thing a
/// test asserts on. Deliberately NOT `Debug`: a Debug rendering is a
/// refactoring hazard (renaming a variant silently changes the log format and
/// every grep that reads it), and it interleaves the payload with the tag.
///
/// The payload travels in its own column, so `PayloadUnreadable` and
/// `QueryFailed` keep their detail without the tag absorbing it.
pub fn tag(reason: &SilenceReason) -> &'static str {
    match reason {
        SilenceReason::UnknownRole(_) => "unknown-role",
        SilenceReason::RoleAbsentOnClient => "role-absent-on-client",
        SilenceReason::NotConfigured => "not-configured",
        SilenceReason::StdinTimedOut => "stdin-timed-out",
        SilenceReason::PayloadUnreadable(_) => "payload-unreadable",
        SilenceReason::NoCues(_) => "no-cues",
        SilenceReason::StoreHadNothing => "store-had-nothing",
        SilenceReason::QueryFailed(_) => "query-failed",
        SilenceReason::CannotInject => "cannot-inject",
        SilenceReason::WatchdogFired => "watchdog-fired",
        SilenceReason::NoTranscript => "no-transcript",
        SilenceReason::AutonomyUnknown => "autonomy-unknown",
        SilenceReason::Panicked(_) => "panicked",
    }
}

/// A specimen of EVERY variant, built by an exhaustive match rather than a
/// hand-written list.
///
/// This is the coupling the doc used to claim and not have. The previous
/// version paired a hand-written `Vec` with an assertion against a HARDCODED
/// `0..=12` range — so a 14th variant could be added to the enum, to `tag()`,
/// and to the witness, while the list stayed at thirteen and every test still
/// passed. The audit proved exactly that.
///
/// Here the match on `seed` is exhaustive, so a new variant fails to compile
/// until it is named; and because the list is BUILT from that match, it cannot
/// then be forgotten.
#[cfg(test)]
pub(crate) fn specimen(seed: &SilenceReason) -> SilenceReason {
    match seed {
        SilenceReason::UnknownRole(_) => SilenceReason::UnknownRole("jawata-hook-typo".into()),
        SilenceReason::RoleAbsentOnClient => SilenceReason::RoleAbsentOnClient,
        SilenceReason::NotConfigured => SilenceReason::NotConfigured,
        SilenceReason::StdinTimedOut => SilenceReason::StdinTimedOut,
        SilenceReason::PayloadUnreadable(_) => SilenceReason::PayloadUnreadable("bad json".into()),
        SilenceReason::NoCues(_) => SilenceReason::NoCues("TooFewContentTokens".into()),
        SilenceReason::StoreHadNothing => SilenceReason::StoreHadNothing,
        SilenceReason::QueryFailed(_) => SilenceReason::QueryFailed("refused".into()),
        SilenceReason::CannotInject => SilenceReason::CannotInject,
        SilenceReason::WatchdogFired => SilenceReason::WatchdogFired,
        SilenceReason::NoTranscript => SilenceReason::NoTranscript,
        SilenceReason::AutonomyUnknown => SilenceReason::AutonomyUnknown,
        SilenceReason::Panicked(_) => SilenceReason::Panicked("divide by zero".into()),
    }
}

/// The detail a reason carries, if any. Newlines and tabs are replaced so one
/// record is always exactly one line — a panic message is multi-line, and an
/// unescaped one would turn a single record into several malformed ones.
fn detail(reason: &SilenceReason) -> String {
    let raw = match reason {
        SilenceReason::UnknownRole(s) => s.as_str(),
        SilenceReason::PayloadUnreadable(s) => s.as_str(),
        SilenceReason::NoCues(s) => s.as_str(),
        SilenceReason::QueryFailed(s) => s.as_str(),
        SilenceReason::Panicked(s) => s.as_str(),
        _ => "",
    };
    raw.chars()
        .map(|c| if c == '\n' || c == '\r' || c == '\t' { ' ' } else { c })
        .collect()
}

/// One record: `<unix-millis>\t<role>\t<tag>\t<detail>`.
///
/// Tab-separated rather than JSON because the writer must not be able to fail
/// on the fire path, and because a half-written JSON object corrupts a parser
/// while a half-written line only loses itself.
pub fn record_line(role: &str, outcome: &Outcome) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut line = String::with_capacity(96);
    // write! to a String is infallible; the result is discarded deliberately
    // rather than unwrapped, because an unwrap here would be a panic on the
    // fire path.
    let _ = match outcome {
        Outcome::Emitted(_) => write!(line, "{millis}\t{role}\temitted\t"),
        Outcome::Silent(r) => write!(line, "{millis}\t{role}\t{}\t{}", tag(r), detail(r)),
    };
    line.push('\n');
    line
}

/// Where the log lives: beside the hook binary's config, so an install carries
/// its own diagnostics and two installs never share one.
pub fn log_path_for(exe: &Path) -> Option<PathBuf> {
    crate::config::config_path_for(exe).map(|c| {
        c.parent()
            .map(|d| d.join("hook_silence.log"))
            .unwrap_or_else(|| PathBuf::from("hook_silence.log"))
    })
}

/// Append one record. Errors are swallowed by contract — see the module docs.
///
/// Returns whether the write landed, for tests only. The fire path ignores it.
pub fn append_to(path: &Path, role: &str, outcome: &Outcome) -> bool {
    trim_if_oversized(path);
    let line = record_line(role, outcome);
    let Ok(mut f) = OpenOptions::new().append(true).create(true).open(path) else {
        return false;
    };
    // ONE write_all of the complete line. Two writes would let a concurrent
    // process interleave between them and split this record in half.
    f.write_all(line.as_bytes()).is_ok()
}

/// ROTATE when the cap is exceeded — never truncate, never rewrite in place.
///
/// Two earlier shapes both DESTROYED records, and each reported success while
/// doing it:
///
/// * `read_to_string` + `fs::write` (O_TRUNC): every concurrent `O_APPEND`
///   writer landing between the truncate and the write was overwritten.
///   Measured at the cap: 60 writers, 60 reported written, as few as ONE
///   present.
/// * write-a-tail-and-`rename`: atomic for the reader, but a concurrent
///   appender still holds the OLD inode open, so its record lands in an
///   orphaned file nobody will ever read. Fewer losses, same class.
///
/// Rotation has neither failure. `rename` moves the whole file, so an appender
/// holding the old inode writes into `hook_silence.log.1` — a file that still
/// exists and is still readable. No record is destroyed; at worst it is one
/// file older than expected. And no lock is taken, so the fire path still
/// cannot block, which was the original reason to avoid one.
fn trim_if_oversized(path: &Path) {
    let Ok(meta) = std::fs::metadata(path) else { return };
    if meta.len() <= MAX_BYTES {
        return;
    }
    // ONLY ONE ROTATOR. `metadata` then `rename` is TOCTOU: two observers both
    // see an oversized file, the first rotates, a third appends creating a
    // fresh one-record log, and the second's STALE observation renames that
    // over `.log.1` — erasing the entire capped generation. Measured naturally
    // reachable: 4 of 250 rounds at 200-way concurrency lost all 1150 prior
    // records. Every hook process races here, and the watchdog thread calls
    // `record` concurrently with main, so one invocation can self-race.
    //
    // `create_new` is an atomic test-and-set: exactly one caller creates the
    // token and rotates; the losers skip and append to a slightly-over-cap
    // file, which is harmless. It never blocks, so the fire path keeps its
    // no-blocking contract — the reason a lock was refused in the first place.
    let token = path.with_extension("rotating");
    let Ok(_) = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&token)
    else {
        return;
    };
    // Re-check under the token: the winner may be acting on a stale read too.
    if std::fs::metadata(path).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
        let _ = std::fs::rename(path, path.with_extension("log.1"));
    }
    let _ = std::fs::remove_file(&token);
}

/// The fire-path entry: resolve the log beside this executable and append.
/// Every failure mode — no exe path, no config dir, unwritable file — ends as
/// a no-op rather than an error, because nothing here may change the exit.
pub fn record(role: &str, outcome: &Outcome) {
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(path) = log_path_for(&exe) else { return };
    append_to(&path, role, outcome);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jawata-silence-{}-{name}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        d.join("hook_silence.log")
    }

    /// The gate says "each reason forced by a test".
    ///
    /// This list is HAND-WRITTEN, and the earlier claim here — that adding a
    /// variant without adding its tag "stops compiling" — was false and was
    /// falsified by this very file: `NoTranscript` and `AutonomyUnknown` were
    /// added and neither list noticed. Only `tag()`'s own match is exhaustive.
    /// The compile-time guarantee is `exhaustive_witness` below: it matches a
    /// reason exhaustively, so a new variant fails to compile until it is
    /// named there AND added here. The earlier version of this comment claimed
    /// that guarantee existed when no such match had been written.
    fn every_reason() -> Vec<SilenceReason> {
        vec![
            SilenceReason::UnknownRole("jawata-hook-typo".into()),
            SilenceReason::RoleAbsentOnClient,
            SilenceReason::NotConfigured,
            SilenceReason::StdinTimedOut,
            SilenceReason::PayloadUnreadable("expected value at line 1".into()),
            SilenceReason::NoCues("TooFewContentTokens { found: 1 }".into()),
            SilenceReason::StoreHadNothing,
            SilenceReason::QueryFailed("ConnectionRefused".into()),
            SilenceReason::CannotInject,
            SilenceReason::WatchdogFired,
            SilenceReason::NoTranscript,
            SilenceReason::AutonomyUnknown,
            SilenceReason::Panicked("attempt to divide by zero".into()),
        ]
    }

    /// EVERY variant must appear in `every_reason()` — enforced without any
    /// hardcoded count.
    ///
    /// `specimen` matches exhaustively, so a new variant breaks compilation
    /// there first. This test then requires that each specimen's TAG appears
    /// among the tags `every_reason()` produces. A variant added to the enum
    /// and to `tag()` but forgotten in the list now fails here, which is
    /// precisely what the previous `0..=12` assertion could not do.
    #[test]
    fn the_reason_list_covers_every_variant() {
        let listed: Vec<&str> = every_reason().iter().map(tag).collect();
        for seed in every_reason() {
            let t = tag(&specimen(&seed));
            assert!(listed.contains(&t), "every_reason() is missing {t}");
        }
        // And the list has no duplicates hiding a gap.
        let mut sorted = listed.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(listed.len(), sorted.len(), "every_reason() repeats a variant");
    }

    #[test]
    fn every_reason_writes_a_distinct_tag() {
        let reasons = every_reason();
        let mut tags: Vec<&str> = reasons.iter().map(tag).collect();
        let before = tags.len();
        tags.sort_unstable();
        tags.dedup();
        assert_eq!(before, tags.len(), "two reasons share a tag: {tags:?}");
        // And every tag is non-empty and greppable — no whitespace, since the
        // format is tab-separated.
        for t in tags {
            assert!(!t.is_empty() && !t.contains(char::is_whitespace), "bad tag {t:?}");
        }
    }

    #[test]
    fn every_reason_lands_in_the_log_as_one_line() {
        let p = tmp("all-reasons");
        let _ = std::fs::remove_file(&p);
        for r in every_reason() {
            assert!(append_to(&p, "primer", &Outcome::Silent(r)));
        }
        let body = std::fs::read_to_string(&p).unwrap();
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(
            every_reason().len(),
            lines.len(),
            "one record per invocation; got {body}"
        );
        // A HARD-CODED table, never `tag(r)`. The previous version asserted
        // `tag(r) == cols[2]` while cols[2] was produced BY `tag(r)` — it
        // compared the function with itself and could never fail. Ten of the
        // twelve tags were unforced because of it.
        let expected = [
            "unknown-role", "role-absent-on-client", "not-configured",
            "stdin-timed-out", "payload-unreadable", "no-cues",
            "store-had-nothing", "query-failed", "cannot-inject",
            "watchdog-fired", "no-transcript", "autonomy-unknown", "panicked",
        ];
        assert_eq!(expected.len(), lines.len(), "one record per reason");
        for (want, line) in expected.iter().zip(&lines) {
            let cols: Vec<&str> = line.split('\t').collect();
            assert_eq!(4, cols.len(), "record is 4 columns: {line:?}");
            assert_eq!(*want, cols[2], "wrong tag in {line:?}");
        }
    }

    /// A panic message is multi-line. Unescaped it would turn one record into
    /// three, and the reader would parse two fragments as records.
    #[test]
    fn a_multiline_detail_stays_one_record() {
        let p = tmp("multiline");
        let _ = std::fs::remove_file(&p);
        let r = SilenceReason::Panicked("panicked at\n  src/x.rs:1\nnote: backtrace".into());
        append_to(&p, "primer", &Outcome::Silent(r));
        let body = std::fs::read_to_string(&p).unwrap();
        assert_eq!(1, body.lines().count(), "must be one line: {body:?}");
        assert!(body.contains("backtrace"), "detail must survive: {body:?}");
    }

    #[test]
    fn an_emitted_outcome_is_recorded_too() {
        let p = tmp("emitted");
        let _ = std::fs::remove_file(&p);
        append_to(&p, "primer", &Outcome::Emitted("{}".into()));
        let body = std::fs::read_to_string(&p).unwrap();
        assert!(body.contains("\temitted\t"), "got {body:?}");
    }

    /// The gate's own clause: two concurrent invocations BOTH land. Threads
    /// rather than processes because the property under test is the single
    /// `write_all` under `O_APPEND` — a torn record would show up as a line
    /// with the wrong column count.
    #[test]
    fn concurrent_writers_both_land_whole() {
        let p = tmp("concurrent");
        let _ = std::fs::remove_file(&p);
        const N: usize = 40;
        std::thread::scope(|s| {
            for i in 0..N {
                let p = p.clone();
                s.spawn(move || {
                    let r = if i % 2 == 0 {
                        SilenceReason::StoreHadNothing
                    } else {
                        SilenceReason::CannotInject
                    };
                    append_to(&p, "primer", &Outcome::Silent(r));
                });
            }
        });
        let body = std::fs::read_to_string(&p).unwrap();
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(N, lines.len(), "every writer must land");
        for l in &lines {
            assert_eq!(4, l.split('\t').count(), "torn record: {l:?}");
        }
    }

    #[test]
    fn the_log_is_capped_by_rotating_not_by_discarding() {
        let p = tmp("capped");
        let _ = std::fs::remove_file(&p);
        // Fill past the cap with identifiable records.
        let filler = "0\tprimer\tstore-had-nothing\t".to_string() + &"x".repeat(200) + "\n";
        let mut body = String::new();
        while body.len() as u64 <= MAX_BYTES {
            body.push_str(&filler);
        }
        std::fs::write(&p, &body).unwrap();
        append_to(&p, "primer", &Outcome::Silent(SilenceReason::CannotInject));
        let after = std::fs::read_to_string(&p).unwrap();
        assert!(
            (after.len() as u64) < MAX_BYTES,
            "the live log must be small again, got {} bytes",
            after.len()
        );
        assert!(after.contains("cannot-inject"), "the new record must survive");
        // ROTATED, not destroyed: the old records are still readable.
        let previous = p.with_extension("log.1");
        assert!(previous.exists(), "the capped log must be kept, not deleted");
        // Every surviving line is whole — the trim cut on a record boundary.
        for l in after.lines() {
            assert_eq!(4, l.split('\t').count(), "trim split a record: {l:?}");
        }
    }

    /// The contract that matters most: a log that cannot be written changes
    /// nothing. A directory where the file should be is the cheapest way to
    /// make every open fail on every platform.
    #[test]
    fn an_unwritable_log_is_a_no_op_not_an_error() {
        let d = std::env::temp_dir().join(format!("jawata-silence-blocked-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&d);
        // `d` is a directory; opening it for append must fail.
        assert!(!append_to(&d, "primer", &Outcome::Silent(SilenceReason::CannotInject)));
    }

    #[test]
    fn the_log_sits_beside_the_config() {
        let exe = Path::new("/opt/jawata/jawata-hook-primer");
        let log = log_path_for(exe).expect("a log path");
        let cfg = crate::config::config_path_for(exe).expect("a config path");
        assert_eq!(cfg.parent(), log.parent(), "log must sit beside the config");
        assert_eq!(Some("hook_silence.log".as_ref()), log.file_name());
    }
    /// THE GATE CLAUSE, at the cap. `concurrent_writers_both_land_whole` starts
    /// from a FRESH log and so never enters the trim path — which is where the
    /// loss was. The audit measured 60 concurrent writers at the cap: 60
    /// reported written, as few as ONE present. `append_to` returned true for
    /// every destroyed record.
    #[test]
    fn concurrent_writers_survive_a_trim() {
        let p = tmp("concurrent-at-cap");
        let _ = std::fs::remove_file(&p);
        // Start AT the cap so the very first append triggers a trim.
        let filler = "0\tprimer\tstore-had-nothing\t".to_string() + &"x".repeat(200) + "\n";
        let mut body = String::new();
        while (body.len() as u64) <= MAX_BYTES {
            body.push_str(&filler);
        }
        std::fs::write(&p, &body).unwrap();

        const N: usize = 60;
        std::thread::scope(|s| {
            for _ in 0..N {
                let p = p.clone();
                s.spawn(move || {
                    append_to(&p, "primer", &Outcome::Silent(SilenceReason::CannotInject));
                });
            }
        });
        // Count across BOTH generations. Rotation's contract is that nothing is
        // DESTROYED — a writer holding the pre-rotation inode legitimately
        // lands in `.log.1`, which still exists and is still readable. An
        // earlier version of this test counted only the live log and so failed
        // about one run in three on correct behaviour: the test was wrong, not
        // the rotation.
        let live = std::fs::read_to_string(&p).unwrap_or_default();
        let rotated = std::fs::read_to_string(p.with_extension("log.1")).unwrap_or_default();
        let landed = live.lines().chain(rotated.lines())
            .filter(|l| l.contains("cannot-inject"))
            .count();
        assert_eq!(
            N, landed,
            "every writer reported success; only {landed} of {N} records survive across both generations"
        );
        for l in live.lines().chain(rotated.lines()) {
            assert_eq!(4, l.split('\t').count(), "torn record: {l:?}");
        }
    }

}
