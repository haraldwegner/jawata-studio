//! D22 (Sprint 28h): the fallback-declaration rate, read back.
//!
//! The eleven tools nobody drove were found by what agents did instead: they reached for a shell text
//! search and declared `jawata-fallback:`. Every declaration the observer lets through writes one `slip`
//! line to the outcomes log; this reads them back so the rate can be recorded against a baseline and
//! seen to fall (or not):
//!
//! ```text
//! jawata-hook-<role> fallback-rate [--since YYYY-MM-DD] [--log <outcomes.log>] [--baseline <file>]
//! ```
//!
//! Lines written since D22 carry the session (`session=<id>`, the last column), so the rate is read PER
//! SESSION. Lines written before it carry none, so they can only be counted PER DAY; the baseline is
//! therefore a per-day rate, recorded over the window before the drivers existed, and the comparison is
//! per day too. A comparison needs a window written AFTER the drivers shipped: until one exists the
//! report says so, and does not say the rate fell.

use std::collections::{BTreeMap, BTreeSet};

/// The subcommand word.
pub const SUBCOMMAND: &str = "fallback-rate";

/// The outcomes-log signal a declared fallback writes.
pub const SIGNAL: &str = "slip";

/// The slips of a window, as read back.
#[derive(Debug, Default, PartialEq)]
pub struct Report {
    /// Slips per calendar day (`YYYY-MM-DD`), every line counted.
    pub per_day: BTreeMap<String, usize>,
    /// Slips per session, for the lines that carry a session.
    pub per_session: BTreeMap<String, usize>,
    /// Lines with no session column (written before D22), counted per day only.
    pub unattributed: usize,
}

impl Report {
    pub fn total(&self) -> usize {
        self.per_day.values().sum()
    }
    /// Slips per day that had at least one slip.
    pub fn per_active_day(&self) -> f64 {
        if self.per_day.is_empty() {
            0.0
        } else {
            self.total() as f64 / self.per_day.len() as f64
        }
    }
    /// Mean slips per session, over the sessions that carry any.
    pub fn per_session_mean(&self) -> Option<f64> {
        (!self.per_session.is_empty()).then(|| self.per_session.values().sum::<usize>() as f64 / self.per_session.len() as f64)
    }
}

/// The recorded baseline: slips per active day over a named window, written before the drivers existed.
#[derive(Debug, PartialEq)]
pub struct Baseline {
    pub per_active_day: f64,
    pub window: String,
}

/// What the comparison of a window against the baseline says.
#[derive(Debug, PartialEq)]
pub enum Verdict {
    /// The window holds no line written after the drivers shipped, so nothing can be compared.
    NoWindowYet,
    Falls { now: f64, baseline: f64 },
    DoesNotFall { now: f64, baseline: f64 },
}

/// Parse a baseline file: `per_active_day=<float>` and `window=<text>`, one per line.
pub fn parse_baseline(text: &str) -> Option<Baseline> {
    let mut rate = None;
    let mut window = String::new();
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("per_active_day=") {
            rate = v.trim().parse::<f64>().ok();
        } else if let Some(v) = line.strip_prefix("window=") {
            window = v.trim().to_string();
        }
    }
    rate.map(|per_active_day| Baseline { per_active_day, window })
}

/// Read the slips out of an outcomes log. Lines that are not this signal, or are malformed, are skipped.
pub fn read(log: &str, since: Option<&str>) -> Report {
    let mut report = Report::default();
    for line in log.lines() {
        let mut cols = line.splitn(4, '\t');
        let (Some(when), Some(_version), Some(signal), Some(detail)) = (cols.next(), cols.next(), cols.next(), cols.next()) else {
            continue;
        };
        if signal != SIGNAL || since.is_some_and(|s| when < s) {
            continue;
        }
        let day = when.get(..10).unwrap_or(when).to_string();
        *report.per_day.entry(day).or_insert(0) += 1;
        match detail.rsplit('\t').next().and_then(|c| c.strip_prefix("session=")).filter(|s| !s.is_empty()) {
            Some(session) => *report.per_session.entry(session.to_string()).or_insert(0) += 1,
            None => report.unattributed += 1,
        }
    }
    report
}

/// Compare a window against the baseline. A window that has no session-carrying line is the window
/// BEFORE the drivers shipped (those lines carry none), so it can be no evidence that the rate fell.
pub fn compare(window: &Report, baseline: &Baseline) -> Verdict {
    if window.per_session.is_empty() {
        return Verdict::NoWindowYet;
    }
    let now = window.per_active_day();
    if now < baseline.per_active_day {
        Verdict::Falls { now, baseline: baseline.per_active_day }
    } else {
        Verdict::DoesNotFall { now, baseline: baseline.per_active_day }
    }
}

pub fn render(report: &Report, baseline: Option<&Baseline>) -> String {
    let mut s = format!(
        "declared fallbacks: {}\n  days with any: {}\n  per active day: {:.2}\n  carrying a session: {}\n  carrying none (before D22, counted per day only): {}\n",
        report.total(),
        report.per_day.len(),
        report.per_active_day(),
        report.per_session.values().sum::<usize>(),
        report.unattributed
    );
    match report.per_session_mean() {
        Some(mean) => {
            let sessions: BTreeSet<&String> = report.per_session.keys().collect();
            s.push_str(&format!("  sessions: {}, mean per session: {:.2}\n", sessions.len(), mean));
        }
        None => s.push_str("  sessions: none yet (no line carries a session)\n"),
    }
    if let Some(b) = baseline {
        s.push_str(&format!("baseline: {:.2} per active day over {}\n", b.per_active_day, b.window));
        s.push_str(&match compare(report, b) {
            Verdict::NoWindowYet => "verdict: no window written after the drivers shipped, so no comparison is possible and nothing is claimed\n".to_string(),
            Verdict::Falls { now, baseline } => format!("verdict: FALLS ({now:.2} per active day against {baseline:.2})\n"),
            Verdict::DoesNotFall { now, baseline } => format!("verdict: does NOT fall ({now:.2} per active day against {baseline:.2})\n"),
        });
    }
    s
}

/// Whether an argv is a fallback-rate call.
pub fn is_fallback_rate_call(args: &[String]) -> bool {
    args.get(1).map(String::as_str) == Some(SUBCOMMAND)
}

/// The whole subcommand. Returns the exit status.
pub fn main(args: &[String]) -> i32 {
    let mut baseline_path: Option<String> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--baseline" {
            baseline_path = it.next().cloned();
        }
    }
    let baseline = baseline_path.as_deref().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| parse_baseline(&t));
    crate::cadence::report_main(SUBCOMMAND, args, &|text, since| render(&read(text, since), baseline.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slip(when: &str, tool: &str, reason: &str, session: Option<&str>) -> String {
        match session {
            Some(s) => format!("{when}\tv4.4.22\tslip\t{tool}\t{reason}\tsession={s}\n"),
            None => format!("{when}\tv4.4.17\tslip\t{tool}\t{reason}\n"),
        }
    }

    fn log() -> String {
        let mut s = String::new();
        s += &slip("2026-10-30T09:00:00Z", "Bash", "old line one", None);
        s += &slip("2026-10-30T09:05:00Z", "Bash", "old line two", None);
        s += &slip("2026-10-31T10:00:00Z", "Bash", "old line three", None);
        s += &slip("2026-11-02T10:00:00Z", "Bash", "new a", Some("sess-1"));
        s += &slip("2026-11-02T10:01:00Z", "Bash", "new b", Some("sess-1"));
        s += &slip("2026-11-02T11:00:00Z", "Grep", "new c", Some("sess-2"));
        s += "2026-11-02T11:05:00Z\tv4.4.22\tstop-verdict\tsomething else\n";
        s += "garbage without tabs\n";
        s
    }

    #[test]
    fn it_counts_per_day_and_per_session_and_leaves_old_lines_unattributed() {
        let r = read(&log(), None);
        assert_eq!(r.total(), 6);
        assert_eq!(r.per_day.get("2026-10-30"), Some(&2));
        assert_eq!(r.unattributed, 3, "lines written before the session column carry none");
        assert_eq!(r.per_session.get("sess-1"), Some(&2));
        assert_eq!(r.per_session.get("sess-2"), Some(&1));
        assert_eq!(r.per_session_mean(), Some(1.5));
    }

    #[test]
    fn since_bounds_the_window() {
        let r = read(&log(), Some("2026-11-01"));
        assert_eq!(r.total(), 3);
        assert_eq!(r.unattributed, 0);
    }

    #[test]
    fn a_window_with_no_session_is_no_evidence_that_the_rate_fell() {
        let b = Baseline { per_active_day: 10.0, window: "w".into() };
        let before = read(&slip("2026-10-30T09:00:00Z", "Bash", "x", None), None);
        assert_eq!(compare(&before, &b), Verdict::NoWindowYet, "a low old window is not a fall");
    }

    #[test]
    fn it_says_falls_only_when_a_session_carrying_window_is_lower_than_the_baseline() {
        let b = Baseline { per_active_day: 2.0, window: "w".into() };
        let lower = read(&log(), Some("2026-11-01")); // 3 slips over 1 day = 3.0 per active day
        assert!(matches!(compare(&lower, &b), Verdict::DoesNotFall { .. }), "3.0 is not below 2.0");
        let b_high = Baseline { per_active_day: 5.0, window: "w".into() };
        assert!(matches!(compare(&lower, &b_high), Verdict::Falls { .. }), "3.0 is below 5.0");
    }

    #[test]
    fn the_baseline_file_parses_and_a_broken_one_does_not() {
        let b = parse_baseline("per_active_day=7.25\nwindow=2026-07-03..2026-11-01\n").expect("parses");
        assert_eq!(b.per_active_day, 7.25);
        assert_eq!(b.window, "2026-07-03..2026-11-01");
        assert!(parse_baseline("window=x\n").is_none());
    }

    /// The recorded baseline is a file in the repository: a rate "against its baseline" needs the
    /// baseline to exist and to read.
    #[test]
    fn the_recorded_baseline_is_in_the_repository_and_reads() {
        let b = parse_baseline(include_str!("../fallback-baseline.txt")).expect("the baseline file parses");
        assert!(b.per_active_day > 1.0, "a recorded rate: {}", b.per_active_day);
        assert!(b.window.starts_with("2026-07-03..2026-11-01"), "{}", b.window);
    }

    #[test]
    fn the_report_states_what_it_cannot_claim() {
        let b = Baseline { per_active_day: 10.0, window: "w".into() };
        let out = render(&read(&slip("2026-10-30T09:00:00Z", "Bash", "x", None), None), Some(&b));
        assert!(out.contains("nothing is claimed"), "{out}");
        assert!(out.contains("sessions: none yet"), "{out}");
    }
}
