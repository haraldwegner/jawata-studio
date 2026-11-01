//! D22: the fallback-rate report, through the REAL binary: `jawata-hook fallback-rate --log <file>
//! --baseline <file>`. A report nothing can invoke is a report nobody reads, and a decision function
//! that is green while the subcommand is not dispatched is the failure this file exists to catch.

use std::process::Command;

const HOOK: &str = env!("CARGO_BIN_EXE_jawata-hook");

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("jawata-fallback-rate-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

fn run(log: &std::path::Path, baseline: &std::path::Path) -> (i32, String) {
    let out = Command::new(HOOK)
        .args(["fallback-rate", "--log"])
        .arg(log)
        .arg("--baseline")
        .arg(baseline)
        .output()
        .expect("run the binary");
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).to_string())
}

#[test]
fn the_report_is_dispatched_and_says_falls_only_for_a_window_that_carries_sessions() {
    let dir = scratch("dispatch");
    let baseline = dir.join("baseline.txt");
    std::fs::write(&baseline, "per_active_day=5.0\nwindow=2026-07-03..2026-11-01\n").unwrap();

    // Before the drivers: lines with no session. Whatever their count, they prove no fall.
    let old = dir.join("old.log");
    std::fs::write(&old, "2026-10-30T09:00:00Z\tv4.4.17\tslip\tBash\tx\n").unwrap();
    let (code, text) = run(&old, &baseline);
    assert_eq!(0, code, "{text}");
    assert!(text.contains("nothing is claimed"), "an old window proves no fall: {text}");

    // After: lines that carry a session, fewer per active day than the baseline.
    let new = dir.join("new.log");
    std::fs::write(
        &new,
        "2026-11-02T09:00:00Z\tv4.4.23\tslip\tBash\tx\tsession=a\n2026-11-02T09:01:00Z\tv4.4.23\tslip\tBash\ty\tsession=a\n",
    )
    .unwrap();
    let (code, text) = run(&new, &baseline);
    assert_eq!(0, code, "{text}");
    assert!(text.contains("FALLS"), "2.0 per active day against 5.0: {text}");

    // A log that cannot be read is an error, never a zero.
    let (code, _) = run(&dir.join("absent.log"), &baseline);
    assert_eq!(2, code, "a missing log is not a quiet window");
    let _ = std::fs::remove_dir_all(&dir);
}
