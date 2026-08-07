//! `--explain` and the silence log, through the ACTUAL BINARY.
//!
//! Sprint 28 Stage 8. The unit tests in `silence.rs` drive `append_to`, which
//! is the decidable half. They cannot reach the half that matters here:
//!
//! * `silence::record` — resolves the log from `current_exe()`. Its only
//!   caller is `main`, which is exactly the "called only by tests" shape
//!   inverted: called only by production, therefore covered by nothing. A
//!   process is the only way to give it a real `current_exe()`.
//! * `--explain` — the gate says it must run THE REAL PATH. A test that calls
//!   `dispatch` directly and prints the result proves nothing about the flag,
//!   because it is not the code the flag runs.
//!
//! So this test runs the built binary, under a role name, with no endpoint,
//! and reads what it wrote.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Spawn a freshly-copied executable, retrying ETXTBSY.
///
/// Linux refuses to exec a file any process still holds open for writing, and
/// `fs::copy` in a sibling test thread can hold that handle for a moment after
/// this thread's copy has returned. Caught at ~1 run in 6 as
/// `Os { code: 26, kind: ExecutableFileBusy }`.
///
/// The production deploy path already solved exactly this (`manager_service.rs`
/// unlinks-then-writes and retries the spawn); the test helper never got the
/// same treatment, so the suite carried a flake the product does not have.
fn run_staged(cmd: &mut Command) -> Output {
    for _ in 0..40 {
        match cmd.output() {
            Err(e) if e.raw_os_error() == Some(26) => {
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            other => return other.expect("run the staged hook"),
        }
    }
    panic!("still ETXTBSY after 40 attempts");
}

/// The binary under test, copied to a role name in a scratch directory.
///
/// The copy is the point: the hook resolves its role from `argv[0]` and its
/// config from the executable's own directory, so it must run from where the
/// deploy would place it — not from `target/release`.
fn staged_hook(scratch: &Path, role_binary: &str) -> PathBuf {
    let built = built_binary();
    let dest = scratch.join(role_binary);
    // Unlink before copy: the ETXTBSY hazard the deploy has, for the same
    // reason — a previous run of this test may still be exiting.
    let _ = std::fs::remove_file(&dest);
    std::fs::copy(&built, &dest).expect("stage the hook binary");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755));
    }
    dest
}

fn built_binary() -> PathBuf {
    // `current_exe()` is the test binary: <target>/<profile>/deps/<test>.
    let mut p = std::env::current_exe().expect("test exe");
    p.pop(); // deps
    p.pop(); // profile
    let exe = if cfg!(windows) { "jawata-hook.exe" } else { "jawata-hook" };
    let candidate = p.join(exe);
    assert!(
        candidate.exists(),
        "the hook binary must be built alongside the tests; looked for {}",
        candidate.display()
    );
    candidate
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("jawata-explain-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("scratch dir");
    d
}

/// The gate clause: `--explain` runs the real path.
///
/// Proven by giving the real path a condition only IT can observe — a staged
/// hook with no `hook_config.json` beside it — and requiring `--explain` to
/// name that exact reason. A self-check taking a different route would have to
/// invent this answer; it cannot read a config that was never there.
#[test]
fn explain_reports_the_reason_the_real_path_produced() {
    let dir = scratch("no-config");
    let hook = staged_hook(&dir, if cfg!(windows) { "jawata-hook-primer.exe" } else { "jawata-hook-primer" });

    let out = run_staged(Command::new(&hook).arg("--explain").current_dir(&dir));

    let stderr = String::from_utf8_lossy(&out.stderr);

    // Exit 0 regardless — the fail-safe contract does not bend for a flag.
    assert_eq!(Some(0), out.status.code(), "must exit 0; stderr: {stderr}");
    assert!(
        stderr.contains("primer"),
        "must name the role it resolved from argv[0]: {stderr:?}"
    );
    assert!(
        stderr.contains("SILENT"),
        "with no endpoint it must report silence: {stderr:?}"
    );
    assert!(
        stderr.contains("not-configured"),
        "must name the REAL path's reason — no config beside the binary: {stderr:?}"
    );
}

/// Without `--explain` the hook says nothing on stderr. A hook that chattered
/// into a client's error stream on every prompt would be its own defect.
#[test]
fn without_explain_the_hook_is_quiet() {
    let dir = scratch("quiet");
    let hook = staged_hook(&dir, if cfg!(windows) { "jawata-hook-primer.exe" } else { "jawata-hook-primer" });

    let out = run_staged(Command::new(&hook).current_dir(&dir));
    assert_eq!(Some(0), out.status.code());
    assert!(
        String::from_utf8_lossy(&out.stderr).trim().is_empty(),
        "stderr must be empty without --explain: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `silence::record` is reached ONLY from `main`. This is the test that proves
/// the wire exists: run the binary, then read the log it wrote beside itself.
#[test]
fn the_binary_writes_its_reason_to_the_log_beside_it() {
    let dir = scratch("writes-log");
    let hook = staged_hook(&dir, if cfg!(windows) { "jawata-hook-primer.exe" } else { "jawata-hook-primer" });

    let log = jawata_hook::silence::log_path_for(&hook).expect("a log path");
    let _ = std::fs::remove_file(&log);

    let out = run_staged(Command::new(&hook).current_dir(&dir));
    assert_eq!(Some(0), out.status.code());

    let body = std::fs::read_to_string(&log).unwrap_or_else(|e| {
        panic!("the hook must have written {}: {e}", log.display());
    });
    let line = body.lines().next().expect("one record");
    let cols: Vec<&str> = line.split('\t').collect();
    assert_eq!(4, cols.len(), "record is 4 columns: {line:?}");
    assert_eq!("primer", cols[1], "the role column: {line:?}");
    assert_eq!("not-configured", cols[2], "the reason column: {line:?}");
}

/// Two invocations append rather than overwrite — the log is a history, and a
/// hook that clobbered it would answer "why was it silent" with only the last
/// answer, which is precisely the question that needs the earlier ones.
#[test]
fn a_second_invocation_appends() {
    let dir = scratch("appends");
    let hook = staged_hook(&dir, if cfg!(windows) { "jawata-hook-primer.exe" } else { "jawata-hook-primer" });
    let log = jawata_hook::silence::log_path_for(&hook).expect("a log path");
    let _ = std::fs::remove_file(&log);

    for _ in 0..3 {
        run_staged(Command::new(&hook).current_dir(&dir));
    }
    let body = std::fs::read_to_string(&log).expect("the log");
    assert_eq!(3, body.lines().count(), "three runs, three records: {body:?}");
}

/// An unknown role still records. The two-week outage was a hook that ran and
/// did nothing; a hook whose NAME is wrong is the same silence with a different
/// cause, and the log has to be able to tell them apart.
#[test]
fn an_unknown_role_is_recorded_under_its_own_tag() {
    let dir = scratch("unknown-role");
    let hook = staged_hook(&dir, if cfg!(windows) { "jawata-hook-typo.exe" } else { "jawata-hook-typo" });
    let log = jawata_hook::silence::log_path_for(&hook).expect("a log path");
    let _ = std::fs::remove_file(&log);

    let out = run_staged(Command::new(&hook).arg("--explain").current_dir(&dir));
    assert_eq!(Some(0), out.status.code());

    let body = std::fs::read_to_string(&log).expect("the log");
    let cols: Vec<&str> = body.lines().next().expect("a record").split('\t').collect();
    assert_eq!("unknown", cols[1], "no role resolved: {body:?}");
    assert_eq!("unknown-role", cols[2], "the reason: {body:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("unknown-role"),
        "--explain must say so too"
    );
}
