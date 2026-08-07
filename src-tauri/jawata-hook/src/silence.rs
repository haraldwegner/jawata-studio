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

/// Above this size the log is trimmed to its most recent half. Chosen so a
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
        SilenceReason::NoTranscript => "no-transcript",
        SilenceReason::AutonomyUnknown => "autonomy-unknown",
        SilenceReason::Panicked(_) => "panicked",
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

/// Truncate to the most recent half when the cap is exceeded, on a record
/// boundary. Failure to trim is not failure to log: the append still runs.
fn trim_if_oversized(path: &Path) {
    let Ok(meta) = std::fs::metadata(path) else { return };
    if meta.len() <= MAX_BYTES {
        return;
    }
    let Ok(body) = std::fs::read_to_string(path) else { return };
    let keep_from = body.len() / 2;
    // Advance to the next record boundary so the first surviving line is whole.
    let cut = body[keep_from..]
        .find('\n')
        .map(|i| keep_from + i + 1)
        .unwrap_or(body.len());
    let _ = std::fs::write(path, &body[cut..]);
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
    /// The compile-time guarantee is asserted below instead, by matching a
    /// witness exhaustively.
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
            SilenceReason::NoTranscript,
            SilenceReason::AutonomyUnknown,
            SilenceReason::Panicked("attempt to divide by zero".into()),
        ]
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
            "no-transcript", "autonomy-unknown", "panicked",
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
    fn the_log_is_capped_and_keeps_the_recent_half() {
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
            "must be trimmed, got {} bytes",
            after.len()
        );
        assert!(after.contains("cannot-inject"), "the new record must survive");
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
}
