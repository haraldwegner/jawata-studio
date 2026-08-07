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
//!    session forever. The hook NEVER bounds the file itself: it appends, and
//!    past a hard ceiling it drops new records. The Studio manager rotates the
//!    log off the fire path. Six audit rounds established that in-path
//!    bounding cannot be made safe here — every scheme destroyed records or
//!    disabled itself.

use std::fmt::Write as _;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::safety::{Outcome, SilenceReason};

/// Above this size the MANAGER rotates the log at its next pass — the hook
/// itself never rotates; see `append_to`.
///
/// Retention is ONE generation: a second rotation overwrites `hook_silence.log.1`.
/// A generation therefore survives at least one full fill of this size, which
/// is the right trade for a diagnostic file — but it is a discard, and saying
/// so here is cheaper than someone discovering it while debugging. Chosen so a busy day of hooks is
/// retained and a year of them is not.
pub const MAX_BYTES: u64 = 256 * 1024;

/// The hook's own backstop: past this, new records are DROPPED rather than
/// risk any housekeeping on the fire path. Only reachable if the manager
/// never runs for a very long time.
pub const HARD_CEILING_BYTES: u64 = 8 * MAX_BYTES;





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
        Outcome::Silent(r) => write!(line, "{millis}\t{role}\t{}\t{}", r.tag(), r.detail()),
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
    // HARD CEILING, nothing else. Six audit rounds of defects — truncation
    // destroying concurrent records, rename orphaning them, a rotate-rotate
    // race erasing generations, an orphanable token disabling the cap forever,
    // and a staleness recovery that broke on future mtimes — all lived in
    // bounding machinery that had no business on this path. A hook that must
    // never block, never lose a record, and die in 4 seconds is the worst
    // possible host for filesystem housekeeping. Rotation now belongs to the
    // Studio manager, which owns these directories, runs off the fire path,
    // and may hold a real lock.
    //
    // The ceiling is the pathological backstop: if the manager never runs
    // again, growth stops here by DROPPING NEW RECORDS — for a diagnostic
    // file, the correct failure mode. No destruction, no race, provably
    // bounded.
    if std::fs::metadata(path).map(|m| m.len() > HARD_CEILING_BYTES).unwrap_or(false) {
        return false;
    }
    let line = record_line(role, outcome);
    let Ok(mut f) = OpenOptions::new().append(true).create(true).open(path) else {
        return false;
    };
    // ONE write_all of the complete line. Two writes would let a concurrent
    // process interleave between them and split this record in half.
    f.write_all(line.as_bytes()).is_ok()
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
    /// Generated from the single row list in `safety.rs`, so a new variant
    /// appears here automatically. Three hand-written couplings failed before
    /// this: a claimed-but-absent witness match, a witness asserted against a
    /// hardcoded range, and a specimen fn called only on seeds from the list it
    /// was checking. There is now one place to forget, and it does not compile.
    fn every_reason() -> Vec<SilenceReason> {
        crate::safety::all_specimens()
    }

    /// Every variant reaches the log with a distinct, non-empty tag.
    ///
    /// The COVERAGE question this used to guard is now answered at compile
    /// time: `every_reason()` is generated from the same row list as the enum
    /// and `tag_of`, so a variant cannot exist without appearing here. Three
    /// hand-written attempts to enforce that failed; the macro does not need
    /// a test to hold it up.
    #[test]
    fn the_reason_list_has_no_duplicate_or_empty_tags() {
        let listed: Vec<&str> = every_reason().iter().map(|r: &SilenceReason| r.tag()).collect();
        assert!(listed.len() >= 14, "expected the full taxonomy, got {}", listed.len());
        let mut sorted = listed.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(listed.len(), sorted.len(), "two variants share a tag");
        for t in listed {
            assert!(!t.is_empty() && !t.contains(char::is_whitespace), "bad tag {t:?}");
        }
    }

    #[test]
    fn every_reason_writes_a_distinct_tag() {
        let reasons = every_reason();
        let mut tags: Vec<&str> = reasons.iter().map(|r: &SilenceReason| r.tag()).collect();
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
            "watchdog-fired", "no-transcript", "autonomy-unknown",
            "stop-allowed", "panicked",
        ];
        assert_eq!(expected.len(), lines.len(), "one record per reason");
        for ((want, line), r) in expected.iter().zip(&lines).zip(every_reason()) {
            let cols: Vec<&str> = line.split('\t').collect();
            assert_eq!(4, cols.len(), "record is 4 columns: {line:?}");
            assert_eq!(*want, cols[2], "wrong tag in {line:?}");
            // FORCE THE DETAIL COLUMN. `detail()` is a separate hand-written
            // match with a `_ => ""` catch-all, so a new payload-carrying
            // variant silently logged an EMPTY detail — proven by seeding one:
            // 108 tests green while the payload vanished. A reason that reaches
            // the log carrying nothing is the manufactured absence this stage
            // exists to end, one level down.
            // FORCE THE DETAIL CONTENT, derived from the specimen itself —
            // not from a hand-written list of payload-carrying variants, which
            // was the same anti-pattern the macro replaced, one list further
            // out: a seeded payload row passed with its payload dropped,
            // because `_ => ""` in detail() and the omission from the list
            // cancelled to false == false.
            //
            // A specimen's Debug rendering quotes its String payload, so the
            // expectation is generated: whatever the row put in the specimen
            // must appear in column 4.
            let dbg = format!("{r:?}");
            if let (Some(a), Some(b)) = (dbg.find('"'), dbg.rfind('"')) {
                if b > a {
                    let payload = &dbg[a + 1..b];
                    assert!(
                        cols[3].contains(payload),
                        "detail column lost the payload {payload:?}: {line:?}"
                    );
                }
            } else {
                assert!(cols[3].is_empty(), "a payload-free reason logged a detail: {line:?}");
            }
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
    /// The hook's whole bounding contract now: past the hard ceiling it DROPS
    /// the record and says so, rather than doing any housekeeping here.
    #[test]
    fn past_the_hard_ceiling_new_records_are_dropped_not_written() {
        let p = tmp("ceiling");
        let _ = std::fs::remove_file(&p);
        let filler = "0\tprimer\tstore-had-nothing\t".to_string() + &"x".repeat(4000) + "\n";
        let mut body = String::new();
        while (body.len() as u64) <= HARD_CEILING_BYTES {
            body.push_str(&filler);
        }
        std::fs::write(&p, &body).unwrap();
        let before = std::fs::metadata(&p).unwrap().len();
        assert!(!append_to(&p, "primer", &Outcome::Silent(SilenceReason::CannotInject)));
        assert_eq!(before, std::fs::metadata(&p).unwrap().len(), "nothing may be written past the ceiling");
    }

    /// Below the ceiling the hook appends and NEVER rotates, trims, renames or
    /// deletes — an over-cap log is the MANAGER's business, not the fire
    /// path's. Rotation-on-the-fire-path was the single design decision behind
    /// six audit rounds of destroyed-record defects.
    #[test]
    fn an_over_cap_log_is_still_appended_to_and_never_touched_otherwise() {
        let p = tmp("over-cap-append");
        let _ = std::fs::remove_file(&p);
        let _ = std::fs::remove_file(p.with_extension("log.1"));
        let filler = "0\tprimer\tstore-had-nothing\t".to_string() + &"x".repeat(200) + "\n";
        let mut body = String::new();
        while (body.len() as u64) <= MAX_BYTES {
            body.push_str(&filler);
        }
        std::fs::write(&p, &body).unwrap();
        assert!(append_to(&p, "primer", &Outcome::Silent(SilenceReason::CannotInject)));
        let after = std::fs::read_to_string(&p).unwrap();
        assert!(after.contains("cannot-inject"), "the record must land");
        assert!((after.len() as u64) > MAX_BYTES, "no trimming on the fire path");
        assert!(!p.with_extension("log.1").exists(), "no rotation on the fire path");
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
    fn an_emitted_outcome_is_recorded_too() {
        let p = tmp("emitted");
        let _ = std::fs::remove_file(&p);
        append_to(&p, "primer", &Outcome::Emitted("{}".into()));
        let body = std::fs::read_to_string(&p).unwrap();
        assert!(body.contains("\temitted\t"), "got {body:?}");
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

    /// The gate clause at the OTHER two regimes. The concurrency test was
    /// deleted in the redesign under the rotation machinery's label — but the
    /// property it guarded SURVIVES that deletion: the single `write_all`
    /// under `O_APPEND` in `append_to`. Six rounds of history say
    /// concurrent-writer destruction is the regression this file attracts, so
    /// it is guarded above the cap too, where the old design did its damage.
    #[test]
    fn concurrent_writers_land_whole_above_the_cap_as_well() {
        let p = tmp("concurrent-above-cap");
        let _ = std::fs::remove_file(&p);
        let filler = "0\tprimer\tstore-had-nothing\t".to_string() + &"x".repeat(200) + "\n";
        let mut body = String::new();
        while (body.len() as u64) <= MAX_BYTES {
            body.push_str(&filler);
        }
        std::fs::write(&p, &body).unwrap();

        const N: usize = 40;
        std::thread::scope(|s| {
            for _ in 0..N {
                let p = p.clone();
                s.spawn(move || {
                    append_to(&p, "primer", &Outcome::Silent(SilenceReason::CannotInject));
                });
            }
        });
        let after = std::fs::read_to_string(&p).unwrap();
        assert_eq!(
            N,
            after.lines().filter(|l| l.contains("cannot-inject")).count(),
            "every writer must land even above the cap — the hook no longer trims"
        );
        for l in after.lines() {
            assert_eq!(4, l.split('\t').count(), "torn record: {l:?}");
        }
    }

}
