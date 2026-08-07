//! C6 exit clause 4 — the two runtime states, recorded SEPARATELY.
//!
//! > Hooks fire with Studio's *process* not running (they do), **and** with the
//! > resident service down they emit nothing and exit 0 (they do not answer —
//! > the config is on disk, the store is not). The release ask states this
//! > split; "works with Studio closed" alone would over-claim.
//!
//! The two are different facts and the difference is the whole honesty of the
//! claim. Moving config to disk removes the need for the Studio PROCESS. It
//! does not make the hook independent of the resident JVM, whose lifecycle
//! Studio owns.
//!
//! **STATE A — Studio's process not running: the hook ANSWERS.** No Studio runs
//! anywhere during these tests; the config is a file and the store is reached
//! over HTTP. Proven positively below by a hook that emits real context.
//!
//! **STATE B — the resident down: the hook is SILENT and exits 0.** It does not
//! answer, and it does not block the editor either. Proven in
//! `fail_safe_boundary.rs` (`a_deployed_role_with_an_unreachable_resident_…`)
//! and again here from the config's own side.
//!
//! What the release ask may therefore say: *hooks keep working with the Studio
//! app closed; they go quiet when the resident service is stopped.* Not "hooks
//! work with Studio closed" full stop.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const HOOK: &str = env!("CARGO_BIN_EXE_jawata-hook");

/// A real captured store answer — the same body `sibling_channels.rs` pins.
const RECALL_SYMBOL: &str = include_str!("store-answers/recall-symbol.json");

fn deploy_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Serve one HTTP response, forever, on an ephemeral port.
fn serve(body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut buf = [0u8; 8192];
            let _ = stream.read(&mut buf);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    format!("http://{addr}/mcp")
}

fn deploy_and_run(tag: &str, role: &str, url: &str) -> (Option<i32>, String) {
    let _serial = deploy_lock().lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::env::temp_dir().join(format!("jawata-hook-state-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join(role);

    let mut attempt = 0;
    loop {
        match std::fs::copy(HOOK, &exe) {
            Ok(_) => break,
            Err(e) if e.raw_os_error() == Some(26) && attempt < 20 => {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("copy: {e}"),
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    std::fs::write(
        dir.join("hook_config.json"),
        format!(r#"{{"url":"{url}","token":"t","client":"claude-code"}}"#),
    )
    .unwrap();

    let mut child = Command::new(&exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the deployed hook");
    // A real PreToolUse payload; the recall role reads its cue from it.
    child
        .stdin
        .take()
        .unwrap()
        .write_all(br#"{"prompt":"the importer classifier regression"}"#)
        .unwrap();
    let status = child.wait().expect("terminate");
    let mut out = String::new();
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_string(&mut out);
    }
    let _ = std::fs::remove_dir_all(&dir);
    (status.code(), out)
}

#[test]
fn state_a_with_no_studio_process_anywhere_the_hook_still_answers() {
    // THE positive half. No Studio runs in this test — the config is a file on
    // disk and the store is an HTTP endpoint. If the hook needed the Studio
    // PROCESS, this could not emit.
    let url = serve(RECALL_SYMBOL);
    let (code, out) = deploy_and_run("state-a", "jawata-hook-userprompt", &url);

    assert_eq!(Some(0), code);
    assert!(
        !out.trim().is_empty(),
        "the hook emitted NOTHING with a reachable store — this is the state the whole \
         config-on-disk design exists to support"
    );
    let v: serde_json::Value =
        serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("emitted non-JSON: {e}\n{out}"));
    let ctx = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or_else(|| panic!("no context in the emission: {v}"));
    assert!(ctx.contains("[lesson]"), "the store's answer reached the context: {ctx}");
}

#[test]
fn state_b_with_the_resident_down_the_hook_is_silent_and_exits_zero() {
    // The honest other half. Port 1 on loopback: nothing listens. The hook does
    // NOT answer — and it does not block the editor either.
    let (code, out) = deploy_and_run("state-b", "jawata-hook-userprompt", "http://127.0.0.1:1/mcp");
    assert_eq!(Some(0), code, "a stopped resident must never fail the client");
    assert!(
        out.is_empty(),
        "the hook spoke with no store to speak from — it must stay silent: {out:?}"
    );
}

#[test]
fn the_two_states_are_genuinely_different_outcomes() {
    // The point of recording them separately: if both produced the same thing,
    // the release ask could not honestly distinguish "works with Studio closed"
    // from "works with the resident stopped", and it must.
    let url = serve(RECALL_SYMBOL);
    let (_, with_store) = deploy_and_run("split-a", "jawata-hook-userprompt", &url);
    let (_, without) = deploy_and_run("split-b", "jawata-hook-userprompt", "http://127.0.0.1:1/mcp");
    assert!(
        !with_store.is_empty() && without.is_empty(),
        "the two runtime states must differ — with store: {with_store:?}, without: {without:?}"
    );
}
