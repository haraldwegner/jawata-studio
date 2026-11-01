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
//! The default baseline is the one embedded in this binary, so an installed product can compare without
//! a file beside it.
//!
//! ## What is measured, and the one declared substitution
//!
//! D22 asks for the rate PER SESSION. Lines written before the drivers existed carry no session, so the
//! baseline (2026-07-03 to 2026-11-01) can only be a rate PER CALENDAR DAY, and the comparison is per
//! calendar day too. Lines written since carry the session (`session=<id>`, the last column) and the
//! report shows the per-session mean beside it, but the verdict does not rest on it. This substitution
//! is declared in the plan for Harald.
//!
//! ## When it may say the rate fell
//!
//! Only over a window WRITTEN AFTER the drivers shipped: lines of any signal whose version is at least
//! the baseline's `shipped_in`. That is what makes a window with NO slip at all (the best outcome) count:
//! the window is the calendar days those lines span, and zero slips over them is a rate of zero. A window
//! shorter than [`MIN_WINDOW_DAYS`] calendar days says so and claims nothing. The denominator is calendar
//! days, not days that happened to hold a slip, so a quiet day lowers the rate instead of vanishing.

use std::collections::{BTreeMap, BTreeSet};

/// The subcommand word.
pub const SUBCOMMAND: &str = "fallback-rate";

/// The outcomes-log signal a declared fallback writes.
pub const SIGNAL: &str = "slip";

/// The fewest calendar days a window may span before it is compared.
pub const MIN_WINDOW_DAYS: i64 = 3;

/// The baseline recorded in the repository and embedded in the binary.
pub const EMBEDDED_BASELINE: &str = include_str!("../fallback-baseline.txt");

/// A version as the log writes it (`v4.4.23`).
pub type Version = (u32, u32, u32);

pub fn parse_version(text: &str) -> Option<Version> {
    let mut parts = text.trim().trim_start_matches('v').split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?;
    Some((major, minor, patch))
}

/// Days since 1970-01-01 of a civil date (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn day_number(day: &str) -> Option<i64> {
    let mut p = day.split('-');
    Some(days_from_civil(p.next()?.parse().ok()?, p.next()?.parse().ok()?, p.next()?.parse().ok()?))
}

/// The slips of a window, as read back.
#[derive(Debug, Default, PartialEq)]
pub struct Report {
    /// Slips per calendar day (`YYYY-MM-DD`), every slip line counted.
    pub per_day: BTreeMap<String, usize>,
    /// Slips per session, for the lines that carry a session.
    pub per_session: BTreeMap<String, usize>,
    /// Slip lines with no session column (written before D22).
    pub unattributed: usize,
    /// Days that hold ANY line written at or after the shipping version: the window the comparison rests on.
    pub covered_days: BTreeSet<String>,
    /// Slips among the lines written at or after the shipping version.
    pub covered_slips: usize,
}

impl Report {
    pub fn total(&self) -> usize {
        self.per_day.values().sum()
    }
    /// Mean slips per session, over the sessions that carry any.
    pub fn per_session_mean(&self) -> Option<f64> {
        (!self.per_session.is_empty()).then(|| self.per_session.values().sum::<usize>() as f64 / self.per_session.len() as f64)
    }
    /// The calendar days from the first to the last covered day, inclusive.
    pub fn window_days(&self) -> i64 {
        match (self.covered_days.iter().next(), self.covered_days.iter().next_back()) {
            (Some(a), Some(b)) => match (day_number(a), day_number(b)) {
                (Some(x), Some(y)) => y - x + 1,
                _ => 0,
            },
            _ => 0,
        }
    }
}

/// The recorded baseline: declared fallbacks per CALENDAR day over a named window written before the
/// drivers existed, and the version in which the drivers ship.
#[derive(Debug, PartialEq)]
pub struct Baseline {
    pub per_calendar_day: f64,
    pub shipped_in: Version,
    pub window: String,
}

/// What the comparison of a window against the baseline says.
#[derive(Debug, PartialEq)]
pub enum Verdict {
    /// No line was written at or after the shipping version: nothing can be compared.
    NoWindowYet,
    /// The window is covered but spans fewer than [`MIN_WINDOW_DAYS`] calendar days.
    TooShort { days: i64 },
    Falls { now: f64, baseline: f64, days: i64 },
    DoesNotFall { now: f64, baseline: f64, days: i64 },
}

/// Parse a baseline file: `per_calendar_day=<float>`, `shipped_in=<version>`, `window=<text>`, one per line.
pub fn parse_baseline(text: &str) -> Option<Baseline> {
    let (mut rate, mut shipped, mut window) = (None, None, String::new());
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("per_calendar_day=") {
            rate = v.trim().parse::<f64>().ok();
        } else if let Some(v) = line.strip_prefix("shipped_in=") {
            shipped = parse_version(v);
        } else if let Some(v) = line.strip_prefix("window=") {
            window = v.trim().to_string();
        }
    }
    Some(Baseline { per_calendar_day: rate?, shipped_in: shipped?, window })
}

/// Read the slips out of an outcomes log. Lines that are malformed are skipped; `shipped` marks which
/// lines count as written after the drivers shipped (any signal, by the version column).
pub fn read(log: &str, since: Option<&str>, shipped: Option<Version>) -> Report {
    let mut report = Report::default();
    for line in log.lines() {
        let mut cols = line.splitn(4, '\t');
        let (Some(when), Some(version), Some(signal), Some(detail)) = (cols.next(), cols.next(), cols.next(), cols.next()) else {
            continue;
        };
        if since.is_some_and(|s| when < s) {
            continue;
        }
        let day = when.get(..10).unwrap_or(when).to_string();
        let after_ship = shipped.zip(parse_version(version)).is_some_and(|(s, v)| v >= s);
        if after_ship {
            report.covered_days.insert(day.clone());
        }
        if signal != SIGNAL {
            continue;
        }
        *report.per_day.entry(day).or_insert(0) += 1;
        if after_ship {
            report.covered_slips += 1;
        }
        match detail.rsplit('\t').next().and_then(|c| c.strip_prefix("session=")).filter(|s| !s.is_empty()) {
            Some(session) => *report.per_session.entry(session.to_string()).or_insert(0) += 1,
            None => report.unattributed += 1,
        }
    }
    report
}

/// Compare a window against the baseline.
pub fn compare(window: &Report, baseline: &Baseline) -> Verdict {
    if window.covered_days.is_empty() {
        return Verdict::NoWindowYet;
    }
    let days = window.window_days();
    if days < MIN_WINDOW_DAYS {
        return Verdict::TooShort { days };
    }
    let now = window.covered_slips as f64 / days as f64;
    if now < baseline.per_calendar_day {
        Verdict::Falls { now, baseline: baseline.per_calendar_day, days }
    } else {
        Verdict::DoesNotFall { now, baseline: baseline.per_calendar_day, days }
    }
}

pub fn render(report: &Report, baseline: Option<&Baseline>) -> String {
    let mut s = format!(
        "declared fallbacks in the log: {}\n  days with any: {}\n  carrying a session: {}\n  carrying none (before D22): {}\n",
        report.total(),
        report.per_day.len(),
        report.per_session.values().sum::<usize>(),
        report.unattributed
    );
    match report.per_session_mean() {
        Some(mean) => s.push_str(&format!("  sessions: {}, mean per session: {:.2}\n", report.per_session.len(), mean)),
        None => s.push_str("  sessions: none yet (no line carries a session)\n"),
    }
    if let Some(b) = baseline {
        s.push_str(&format!(
            "baseline: {:.2} per calendar day over {} (drivers ship in {}.{}.{})\n",
            b.per_calendar_day, b.window, b.shipped_in.0, b.shipped_in.1, b.shipped_in.2
        ));
        s.push_str(&match compare(report, b) {
            Verdict::NoWindowYet => "verdict: no line was written at or after the shipping version, so no comparison is possible and nothing is claimed\n".to_string(),
            Verdict::TooShort { days } => format!("verdict: the window spans {days} calendar day(s), fewer than {MIN_WINDOW_DAYS}, so nothing is claimed yet\n"),
            Verdict::Falls { now, baseline, days } => format!("verdict: FALLS ({now:.2} per calendar day over {days} days, against {baseline:.2})\n"),
            Verdict::DoesNotFall { now, baseline, days } => format!("verdict: does NOT fall ({now:.2} per calendar day over {days} days, against {baseline:.2})\n"),
        });
    }
    s
}

/// Whether an argv is a fallback-rate call.
pub fn is_fallback_rate_call(args: &[String]) -> bool {
    args.get(1).map(String::as_str) == Some(SUBCOMMAND)
}

/// The whole subcommand. Returns the exit status. The baseline is `--baseline <file>` or, by default,
/// the one embedded in this binary.
pub fn main(args: &[String]) -> i32 {
    let mut baseline_path: Option<String> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--baseline" {
            baseline_path = it.next().cloned();
        }
    }
    let text = match baseline_path {
        Some(p) => match std::fs::read_to_string(&p) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{SUBCOMMAND}: cannot read the baseline {p}: {e}");
                return 2;
            }
        },
        None => EMBEDDED_BASELINE.to_string(),
    };
    let Some(baseline) = parse_baseline(&text) else {
        eprintln!("{SUBCOMMAND}: the baseline does not parse (it needs per_calendar_day and shipped_in)");
        return 2;
    };
    crate::cadence::report_main(SUBCOMMAND, args, &|log, since| render(&read(log, since, Some(baseline.shipped_in)), Some(&baseline)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(when: &str, version: &str, signal: &str, tail: &str) -> String {
        format!("{when}\t{version}\t{signal}\t{tail}\n")
    }
    fn slip(when: &str, version: &str, session: Option<&str>) -> String {
        match session {
            Some(s) => line(when, version, "slip", &format!("Bash\treason\tsession={s}")),
            None => line(when, version, "slip", "Bash\treason"),
        }
    }
    fn baseline(rate: f64) -> Baseline {
        Baseline { per_calendar_day: rate, shipped_in: (4, 4, 23), window: "w".into() }
    }

    fn log() -> String {
        let mut s = String::new();
        s += &slip("2026-10-30T09:00:00Z", "v4.4.17", None);
        s += &slip("2026-10-30T09:05:00Z", "v4.4.17", None);
        s += &slip("2026-10-31T10:00:00Z", "v4.4.17", None);
        s += &slip("2026-11-02T10:00:00Z", "v4.4.23", Some("sess-1"));
        s += &slip("2026-11-02T10:01:00Z", "v4.4.23", Some("sess-1"));
        s += &slip("2026-11-02T11:00:00Z", "v4.4.23", Some("sess-2"));
        s += &line("2026-11-03T11:05:00Z", "v4.4.23", "stop-verdict", "x");
        s += &line("2026-11-04T11:05:00Z", "v4.4.24", "stop-verdict", "x");
        s += "garbage without tabs\n";
        s
    }

    #[test]
    fn it_counts_per_day_and_per_session_and_leaves_old_lines_unattributed() {
        let r = read(&log(), None, Some((4, 4, 23)));
        assert_eq!(r.total(), 6);
        assert_eq!(r.per_day.get("2026-10-30"), Some(&2));
        assert_eq!(r.unattributed, 3, "lines written before the session column carry none");
        assert_eq!(r.per_session.get("sess-1"), Some(&2));
        assert_eq!(r.per_session_mean(), Some(1.5));
    }

    #[test]
    fn since_bounds_the_window() {
        let r = read(&log(), Some("2026-11-01"), Some((4, 4, 23)));
        assert_eq!(r.total(), 3);
        assert_eq!(r.unattributed, 0);
    }

    #[test]
    fn the_covered_window_is_the_calendar_span_of_lines_written_at_or_after_the_shipping_version() {
        let r = read(&log(), None, Some((4, 4, 23)));
        assert_eq!(r.covered_days.iter().cloned().collect::<Vec<_>>(), vec!["2026-11-02", "2026-11-03", "2026-11-04"]);
        assert_eq!(r.window_days(), 3);
        assert_eq!(r.covered_slips, 3, "only the slips at or after the shipping version");
    }

    #[test]
    fn an_old_window_is_no_evidence_that_the_rate_fell() {
        let old = read(&slip("2026-10-30T09:00:00Z", "v4.4.17", None), None, Some((4, 4, 23)));
        assert_eq!(compare(&old, &baseline(10.0)), Verdict::NoWindowYet, "a low old window is not a fall");
    }

    /// The best outcome, no slip at all, must register as a fall: the window is the days the log covers,
    /// not the days that hold a slip.
    #[test]
    fn a_window_with_no_slip_at_all_over_enough_days_falls_to_zero() {
        let mut s = String::new();
        for d in ["2026-11-02", "2026-11-03", "2026-11-04"] {
            s += &line(&format!("{d}T09:00:00Z"), "v4.4.23", "stop-verdict", "x");
        }
        let r = read(&s, None, Some((4, 4, 23)));
        assert_eq!(compare(&r, &baseline(5.0)), Verdict::Falls { now: 0.0, baseline: 5.0, days: 3 });
    }

    #[test]
    fn a_window_shorter_than_the_minimum_claims_nothing() {
        let r = read(&slip("2026-11-02T09:00:00Z", "v4.4.23", Some("a")), None, Some((4, 4, 23)));
        assert_eq!(compare(&r, &baseline(5.0)), Verdict::TooShort { days: 1 });
    }

    /// A quiet day lowers the rate: three slips on ONE of three covered days are one per calendar day,
    /// not three per active day.
    #[test]
    fn the_denominator_is_calendar_days_not_days_that_held_a_slip() {
        let r = read(&log(), None, Some((4, 4, 23)));
        assert_eq!(compare(&r, &baseline(1.5)), Verdict::Falls { now: 1.0, baseline: 1.5, days: 3 });
        assert!(matches!(compare(&r, &baseline(1.0)), Verdict::DoesNotFall { .. }), "1.0 is not below 1.0");
    }

    /// Two slips on ONE of three covered days: the rate is two over three calendar days, not two over
    /// the one day that held a slip. A quiet day lowers the rate.
    #[test]
    fn a_quiet_day_lowers_the_rate_instead_of_vanishing_from_the_denominator() {
        let mut s = String::new();
        s += &slip("2026-11-02T09:00:00Z", "v4.4.23", Some("a"));
        s += &slip("2026-11-02T09:01:00Z", "v4.4.23", Some("a"));
        s += &line("2026-11-03T09:00:00Z", "v4.4.23", "stop-verdict", "x");
        s += &line("2026-11-04T09:00:00Z", "v4.4.23", "stop-verdict", "x");
        let r = read(&s, None, Some((4, 4, 23)));
        match compare(&r, &baseline(1.0)) {
            Verdict::Falls { now, days, .. } => {
                assert_eq!(days, 3);
                assert!((now - 2.0 / 3.0).abs() < 1e-9, "{now}");
            }
            other => panic!("two slips over three calendar days is below one a day: {other:?}"),
        }
    }

    #[test]
    fn the_baseline_file_parses_and_a_broken_one_does_not() {
        let b = parse_baseline("per_calendar_day=7.25\nshipped_in=4.4.23\nwindow=2026-07-03..2026-11-01\n").expect("parses");
        assert_eq!(b.per_calendar_day, 7.25);
        assert_eq!(b.shipped_in, (4, 4, 23));
        assert_eq!(b.window, "2026-07-03..2026-11-01");
        assert!(parse_baseline("window=x\n").is_none());
        assert!(parse_baseline("per_calendar_day=1.0\n").is_none(), "without a shipping version nothing can be compared");
    }

    #[test]
    fn calendar_arithmetic_spans_a_month_end() {
        let mut s = String::new();
        s += &line("2026-10-31T09:00:00Z", "v4.4.23", "stop-verdict", "x");
        s += &line("2026-11-02T09:00:00Z", "v4.4.23", "stop-verdict", "x");
        assert_eq!(read(&s, None, Some((4, 4, 23))).window_days(), 3);
    }

    /// The recorded baseline is a file in the repository, embedded in the binary: a rate "against its
    /// baseline" needs the baseline to exist and to read.
    #[test]
    fn the_embedded_baseline_reads() {
        let b = parse_baseline(EMBEDDED_BASELINE).expect("the embedded baseline parses");
        assert!(b.per_calendar_day > 1.0, "a recorded rate: {}", b.per_calendar_day);
        assert!(b.window.starts_with("2026-07-03..2026-11-01"), "{}", b.window);
    }

    #[test]
    fn the_report_states_what_it_cannot_claim() {
        let out = render(&read(&slip("2026-10-30T09:00:00Z", "v4.4.17", None), None, Some((4, 4, 23))), Some(&baseline(10.0)));
        assert!(out.contains("nothing is claimed"), "{out}");
        assert!(out.contains("sessions: none yet"), "{out}");
    }
}
