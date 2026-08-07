//! C5 exit clause 2, at the level the clause actually states.
//!
//! > Each hazard has a test **that exits 0 and emits nothing**: panicking role,
//! > unreachable endpoint, absent config, malformed response, **and a stdin
//! > that never closes**.
//!
//! The unit tests assert an `Outcome` value in-process, which is not the same
//! claim: `exit_with` — the function that performs the exit and decides
//! whether anything is written — was executed by no test at all. So these
//! spawn the REAL BINARY and read its exit status and its stdout.
//!
//! The distinction is not academic. Stage 8 plans to add logging on exactly
//! the silent path; a diagnostic line written to stdout there is malformed
//! output to a `failClosed` guard, and every in-process test would stay green.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Serialises the deploy-and-run tests.
///
/// C5 audit round 3, N1 — a regression THIS suite introduced. The ETXTBSY
/// retry guarded `fs::copy` (the write) and nothing guarded `spawn` (the exec).
/// `Command::spawn` forks, and a fork inherits every open fd: while test A sits
/// inside `fs::copy` with a write fd open on its freshly-deployed binary, test
/// B forks and B's child inherits that fd and holds it for its whole lifetime —
/// up to four seconds for the watchdog run. A's own `execve` then fails with
/// "Text file busy". Measured at 5 failures in 18 bare `cargo test` runs.
///
/// It was a blocker rather than a nuisance because cargo FAIL-FASTS across test
/// binaries: in a failing run the no-panics, role-events, transport and the
/// 246-test studio suites never executed at all, and the run ended in a red
/// that looks like a flake and teaches a re-run.
///
/// Serialising removes the overlap; the retry below stays as the belt.
fn deploy_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

const HOOK: &str = env!("CARGO_BIN_EXE_jawata-hook");

/// Deploy the binary under a ROLE name, with a config beside it, exactly as
/// the studio's deploy will.
///
/// Needed because the hazard tests are otherwise vacuous: invoked by its build
/// name the binary resolves no role and returns before it ever reads stdin, so
/// a "stdin that never closes" test would pass in microseconds without
/// touching the deadline it claims to exercise. The timing gave that away —
/// four tests in under a second when one of them must take at least 1.5s.
fn deploy(tag: &str, role_binary: &str, config: Option<&str>) -> (std::path::PathBuf, std::path::PathBuf) {
    // The dir is keyed by TEST, not by role name. Two tests deploying the same
    // role into one directory reproduced ETXTBSY ("Text file busy") — the very
    // hazard the design names for re-deploy, since Linux refuses to overwrite
    // a binary that is executing. Tests run in parallel, so they hit it first.
    let dir = std::env::temp_dir().join(format!(
        "jawata-hook-boundary-{}-{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let target = dir.join(role_binary);
    // Retry on ETXTBSY. Copying FROM the freshly-built binary while cargo or
    // another parallel test still holds it executing makes Linux refuse with
    // "Text file busy" — observed once here, and the same hazard the design
    // names for re-deploy. A test that fails this way teaches people to re-run
    // rather than to read, which is worse than the flake.
    let mut attempt = 0;
    loop {
        match std::fs::copy(HOOK, &target) {
            Ok(_) => break,
            Err(e) if e.raw_os_error() == Some(26) && attempt < 20 => {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("copy the binary under its role name: {e}"),
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    if let Some(json) = config {
        std::fs::write(dir.join("hook_config.json"), json).expect("write the config");
    }
    (dir, target)
}

/// Run the binary and return (exit code, stdout). `stdin_never_closes` holds
/// the pipe open so the self-imposed deadline is the only thing that can end
/// the run.
fn run_at(exe: &std::path::Path, stdin_never_closes: bool) -> (Option<i32>, String, Duration) {
    let mut cmd = Command::new(exe);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let started = Instant::now();
    // Retry the EXEC too, not just the copy. A concurrent build or a lingering
    // inherited fd can still make execve report ETXTBSY.
    let mut attempt = 0;
    let mut child = loop {
        match cmd.spawn() {
            Ok(c) => break c,
            Err(e) if e.raw_os_error() == Some(26) && attempt < 40 => {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("the deployed hook must be executable: {e}"),
        }
    };
    let stdin = child.stdin.take().expect("piped");
    if stdin_never_closes {
        std::mem::forget(stdin);
    } else {
        drop(stdin);
    }
    let status = child.wait().expect("the process must terminate");
    let elapsed = started.elapsed();
    let mut out = String::new();
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_string(&mut out);
    }
    (status.code(), out, elapsed)
}

fn run(env: &[(&str, &str)], stdin_never_closes: bool) -> (Option<i32>, String, Duration) {
    let mut cmd = Command::new(HOOK);
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let started = Instant::now();
    let mut child = cmd.spawn().expect("the hook binary must exist — cargo builds it for this test");

    let stdin = child.stdin.take().expect("piped");
    if stdin_never_closes {
        // Deliberately leak: the pipe stays open for the child's lifetime,
        // which is the hazard. The client that does this is not hypothetical —
        // Cursor's guidance is to bound your own read precisely because of it.
        std::mem::forget(stdin);
    } else {
        drop(stdin);
    }

    let status = child.wait().expect("the process must terminate");
    let elapsed = started.elapsed();
    let mut out = String::new();
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_string(&mut out);
    }
    (status.code(), out, elapsed)
}

#[test]
fn an_unowned_role_exits_zero_and_writes_nothing() {
    // Invoked by its own build name, which is not one of the deployed role
    // names, so the role does not resolve. The client must see success and an
    // empty stream — not an error, and not a stray diagnostic.
    let (code, out, _) = run(&[], false);
    assert_eq!(Some(0), code, "a hook must never fail the client");
    assert!(out.is_empty(), "nothing may be written on a silent path, got: {out:?}");
}

#[test]
fn an_absent_config_exits_zero_and_writes_nothing() {
    // C5 audit round 2, R2: this test used to invoke the binary under its BUILD
    // name and set HOME. Both were inert — the build name resolves no role, so
    // the run returned at UnknownRole BEFORE config was ever read (104 ms wall,
    // against 1635 ms for a run that reaches the stdin read), and config is
    // resolved beside current_exe, not from HOME. It was a relabelled duplicate
    // of the unowned-role test, and deploy()'s `None` branch was dead code.
    //
    // Deployed under a REAL role name with NO config beside it, which is the
    // installed-but-not-yet-configured case the clause names.
    let _serial = deploy_lock().lock().unwrap_or_else(|e| e.into_inner());
    let (_dir, exe) = deploy("absent-config", "jawata-hook-primer", None);
    let (code, out, _) = run_at(&exe, false);
    assert_eq!(Some(0), code, "a missing config must not fail the client");
    assert!(out.is_empty(), "got: {out:?}");
}

#[test]
fn a_stdin_that_never_closes_still_exits_zero_and_in_time() {
    // THE hazard the shell generation had no answer to: it blocked in `cat`
    // until the client gave up, which under Cursor's failClosed is a blocked
    // user command.
    //
    // Deployed under a real role name WITH a config, so the run genuinely
    // reaches the stdin read. Pointed at a dead port so that if the deadline
    // ever failed to fire, the test would hang rather than pass quietly.
    let _serial = deploy_lock().lock().unwrap_or_else(|e| e.into_inner());
    let (_dir, exe) = deploy(
        "wedged-stdin",
        "jawata-hook-userprompt",
        Some(r#"{"url":"http://127.0.0.1:1/mcp","token":"t","client":"claude-code"}"#),
    );
    let (code, out, elapsed) = run_at(&exe, true);

    assert_eq!(Some(0), code, "a wedged read must not fail the client");
    assert!(out.is_empty(), "got: {out:?}");
    assert!(
        elapsed >= Duration::from_millis(1_000),
        "the run finished in {elapsed:?} — too fast to have reached the stdin read at all, \
         which is how this test passes without exercising the deadline it names"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "the run took {elapsed:?} — our own deadline must fire before the client's 5s"
    );
}

#[test]
fn a_deployed_role_with_an_unreachable_resident_still_exits_zero_and_writes_nothing() {
    // The unreachable-endpoint hazard at process level: a real role, a real
    // config, a port nothing listens on. The recall cannot succeed, and the
    // client must still see success and an empty stream.
    let _serial = deploy_lock().lock().unwrap_or_else(|e| e.into_inner());
    let (_dir, exe) = deploy(
        "unreachable",
        "jawata-hook-primer",
        Some(r#"{"url":"http://127.0.0.1:1/mcp","token":"t","client":"claude-code"}"#),
    );
    let (code, out, elapsed) = run_at(&exe, false);
    assert_eq!(Some(0), code, "an unreachable resident must not fail the client");
    assert!(out.is_empty(), "nothing may be written when there is nothing to say: {out:?}");
    assert!(elapsed < Duration::from_secs(5), "took {elapsed:?}");
}

#[test]
fn a_deployed_role_with_a_torn_config_still_exits_zero_and_writes_nothing() {
    let _serial = deploy_lock().lock().unwrap_or_else(|e| e.into_inner());
    let (_dir, exe) = deploy("torn-config", "jawata-hook-primer", Some("{ not json"));
    let (code, out, _) = run_at(&exe, false);
    assert_eq!(Some(0), code);
    assert!(out.is_empty(), "got: {out:?}");
}

#[test]
fn the_release_profile_really_unwinds() {
    // The C5 audit proved the previous in-process assertion VACUOUS: it read
    // cfg!(panic = "unwind") from the TEST profile, and Cargo ignores `panic`
    // for test/bench profiles — so it passed even with panic = "abort"
    // declared. This asks rustc for the EFFECTIVE cfg of the release profile,
    // which is the one that ships.
    //
    // It matters exactly as much as the gate says: under abort, every
    // catch_unwind in safety.rs is disarmed, a panic in the guard role exits
    // non-zero, and Cursor's failClosed guard turns that into a blocked shell
    // command.
    let out = Command::new(env!("CARGO"))
        .args([
            "rustc", "-p", "jawata-hook", "--release", "--bin", "jawata-hook",
            "--manifest-path", concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"),
            "--", "--print", "cfg",
        ])
        .output()
        .expect("cargo rustc must run, or this assertion is not made at all");
    let cfg = String::from_utf8_lossy(&out.stdout);
    assert!(
        cfg.lines().any(|l| l == r#"panic="unwind""#),
        "the RELEASE profile does not unwind — catch_unwind is disarmed and the fail-safe \
         boundary is a lie. Effective cfg:\n{cfg}"
    );
}

#[test]
fn the_watchdog_ends_a_run_parked_far_past_its_deadline() {
    // C5 audit B3: the watchdog had no discriminator — `arm_watchdog` could
    // have spawned an empty closure and every test stayed green. It is the
    // layer catch_unwind cannot be (stack overflow, OOM, panic-in-Drop, a
    // transport wedged below its own timeout), so it needs a real one.
    //
    // The route: a listener that ACCEPTS and never answers, with the config's
    // own timeout set to 60s. The main thread then parks inside the transport,
    // far past TOTAL_DEADLINE, and nothing but the watchdog can end the
    // process. Remove the exit(0) from arm_watchdog and this hangs for a
    // minute instead of passing.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        // Accept and hold. Never reply, never close.
        if let Ok((stream, _)) = listener.accept() {
            std::mem::forget(stream);
        }
        std::thread::sleep(Duration::from_secs(120));
    });

    let _serial = deploy_lock().lock().unwrap_or_else(|e| e.into_inner());
    let (_dir, exe) = deploy(
        "watchdog",
        "jawata-hook-primer",
        Some(&format!(
            r#"{{"url":"http://{addr}/mcp","token":"t","client":"claude-code","timeout_ms":60000}}"#
        )),
    );
    let (code, out, elapsed) = run_at(&exe, false);

    assert_eq!(Some(0), code, "the watchdog must end the process with success");
    assert!(out.is_empty(), "got: {out:?}");
    assert!(
        elapsed >= Duration::from_secs(3),
        "finished in {elapsed:?} — too fast to have been parked in the transport at all, \
         so this proves nothing about the watchdog"
    );
    assert!(
        elapsed < Duration::from_secs(20),
        "took {elapsed:?} — the watchdog did not fire; the run was ended by the 60s \
         transport timeout, or not at all"
    );
}
