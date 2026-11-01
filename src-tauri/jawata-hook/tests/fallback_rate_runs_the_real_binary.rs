//! D22: the fallback-rate report, through the REAL binary: `jawata-hook fallback-rate --log <file>
//! [--baseline <file>]`. A report nothing can invoke is a report nobody reads, and a decision function
//! that is green while the subcommand is not dispatched is the failure this file exists to catch.

use std::process::Command;

const HOOK: &str = env!("CARGO_BIN_EXE_jawata-hook");

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("jawata-fallback-rate-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

fn run_with(log: &std::path::Path, baseline: Option<&std::path::Path>) -> (i32, String) {
    let mut cmd = Command::new(HOOK);
    cmd.arg("fallback-rate").arg("--log").arg(log);
    if let Some(b) = baseline {
        cmd.arg("--baseline").arg(b);
    }
    let out = cmd.output().expect("run the binary");
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).to_string())
}

const BASELINE: &str = "per_calendar_day=5.0\nshipped_in=4.4.23\nwindow=2026-07-03..2026-11-01\n";

fn lines(rows: &[(&str, &str, &str, &str)]) -> String {
    rows.iter().map(|(when, ver, sig, tail)| format!("{when}\t{ver}\t{sig}\t{tail}\n")).collect()
}

#[test]
fn the_report_is_dispatched_and_claims_a_fall_only_for_a_window_written_after_the_drivers_shipped() {
    let dir = scratch("dispatch");
    let baseline = dir.join("baseline.txt");
    std::fs::write(&baseline, BASELINE).unwrap();

    // Before the drivers: an old window, however quiet, proves no fall.
    let old = dir.join("old.log");
    std::fs::write(&old, lines(&[("2026-10-30T09:00:00Z", "v4.4.17", "slip", "Bash\tx")])).unwrap();
    let (code, text) = run_with(&old, Some(&baseline));
    assert_eq!(0, code, "{text}");
    assert!(text.contains("nothing is claimed"), "an old window proves no fall: {text}");

    // After, with slips: two slips over three covered calendar days is 0.67 a day, below 5.0.
    let new = dir.join("new.log");
    std::fs::write(
        &new,
        lines(&[
            ("2026-11-02T09:00:00Z", "v4.4.23", "slip", "Bash\tx\tsession=a"),
            ("2026-11-02T09:01:00Z", "v4.4.23", "slip", "Bash\ty\tsession=a"),
            ("2026-11-03T09:00:00Z", "v4.4.23", "stop-verdict", "z"),
            ("2026-11-04T09:00:00Z", "v4.4.23", "stop-verdict", "z"),
        ]),
    )
    .unwrap();
    let (code, text) = run_with(&new, Some(&baseline));
    assert_eq!(0, code, "{text}");
    assert!(text.contains("FALLS"), "{text}");

    // After, with NO slip at all over three days: the best outcome still registers as a fall.
    let quiet = dir.join("quiet.log");
    std::fs::write(
        &quiet,
        lines(&[
            ("2026-11-02T09:00:00Z", "v4.4.23", "stop-verdict", "z"),
            ("2026-11-03T09:00:00Z", "v4.4.23", "stop-verdict", "z"),
            ("2026-11-04T09:00:00Z", "v4.4.23", "stop-verdict", "z"),
        ]),
    )
    .unwrap();
    let (code, text) = run_with(&quiet, Some(&baseline));
    assert_eq!(0, code, "{text}");
    assert!(text.contains("FALLS") && text.contains("0.00 per calendar day"), "{text}");

    // A window of one day claims nothing.
    let short = dir.join("short.log");
    std::fs::write(&short, lines(&[("2026-11-02T09:00:00Z", "v4.4.23", "stop-verdict", "z")])).unwrap();
    let (_, text) = run_with(&short, Some(&baseline));
    assert!(text.contains("fewer than 3"), "{text}");

    // No --baseline: the one embedded in the binary is used, and the report still names it.
    let (code, text) = run_with(&quiet, None);
    assert_eq!(0, code, "{text}");
    assert!(text.contains("baseline: 28.24 per calendar day"), "the embedded baseline is the default: {text}");

    // A log that cannot be read is an error, never a zero.
    let (code, _) = run_with(&dir.join("absent.log"), Some(&baseline));
    assert_eq!(2, code, "a missing log is not a quiet window");
    let _ = std::fs::remove_dir_all(&dir);
}
