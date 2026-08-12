//! The `.java` edit gate, exercised through the REAL binary under its deployed
//! role name — not through `judge_edit` in isolation.
//!
//! # Why this file exists at all
//!
//! This project has shipped half a guard twice. In 3.7.2 the observer's binary
//! carried one of its two jobs; in 3.7.3 the guard's binary carried the shell
//! half and silently dropped this one, and a front-door `Edit` of a `.java`
//! file went through unblocked on a live machine. Both cutovers were decided by
//! *file existence* rather than by behaviour, and both were caught only in
//! dogfood — by a human, after shipping.
//!
//! Unit tests on the decision function cannot catch that class: they pass
//! whether or not the pipeline ever calls them. So this test spawns the built
//! executable, feeds it a real payload on stdin, and reads the JSON it prints.
//! If the wiring is removed, the decision function stays green and this goes
//! red — which is the whole point.
//!
//! It is also the gate on `BINARY_LIVE_ROLES`: the guard may only flip to its
//! binary generation when BOTH halves are proven here.

use std::io::{Read, Write};
use std::process::{Command, Stdio};

/// The binary under its guard role name. `argv[0]` selects the role, so the
/// name is load-bearing — running it as anything else exercises a different
/// role and proves nothing about this one.
const HOOK: &str = env!("CARGO_BIN_EXE_jawata-hook");

/// Feed one payload to the real binary as the guard, and return what it printed.
fn guard_says(payload: &str, home: &std::path::Path) -> String {
    let dir = home.join("bin");
    std::fs::create_dir_all(&dir).expect("scratch bin");
    let exe = dir.join(if cfg!(windows) { "jawata-hook-guard.exe" } else { "jawata-hook-guard" });
    std::fs::copy(HOOK, &exe).expect("copy the built binary to its role name");
    // The config sits beside the binary. Without it the hook is deliberately
    // SILENT (absent config is a fail-safe case, not a crash), so the test
    // writes one.
    //
    // The url and token must be NON-EMPTY even though the guard never calls
    // them: an empty either one is rejected as unreadable and the hook goes
    // silent. The address below is deliberately dead — the guard is a LOCAL
    // decision and must answer with the resident unreachable, which is exactly
    // what this exercises.
    std::fs::write(
        dir.join("hook_config.json"),
        r#"{"url":"http://127.0.0.1:1/mcp","token":"t","client":"cursor"}"#,
    )
    .expect("write the hook config");

    // RETRY THE EXEC, not just the copy. Linux refuses to exec a binary that is
    // still open for writing (ETXTBSY, errno 26), and these tests run
    // concurrently — each copying the same source to its own role name. The
    // race is timing-dependent: it passed locally and on x64 and failed only on
    // ubuntu-22.04-arm, which is precisely how a race hides until it does not.
    //
    // This is the SECOND copy of this dance; `fail_safe_boundary.rs` has the
    // first, with the same errno and the same reasoning. Two copies of one
    // workaround is a duplication worth extracting into a shared test helper —
    // recorded rather than done here, because it spans two integration binaries.
    let mut attempt = 0;
    let mut child = loop {
        let spawned = Command::new(&exe)
            .env("HOME", home)
            .env("USERPROFILE", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        match spawned {
            Ok(c) => break c,
            Err(e) if e.raw_os_error() == Some(26) && attempt < 40 => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => panic!("the guard binary must be executable: {e}"),
        }
    };
    child
        .stdin
        .take()
        .expect("piped")
        .write_all(payload.as_bytes())
        .expect("write the payload");
    let status = child.wait().expect("the guard must terminate");
    assert_eq!(
        Some(0),
        status.code(),
        "a guard must never fail the client — Cursor runs it failClosed, so a non-zero \
         exit is itself a block on the user's command"
    );
    let mut out = String::new();
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_string(&mut out);
    }
    out
}

fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("jawata-editgate-e2e-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("scratch home");
    d
}

#[test]
fn a_java_hand_edit_is_denied_by_the_real_binary() {
    let home = scratch("deny");
    // The file must EXIST — otherwise the new-file allowance would let it
    // through and this test would pass for the wrong reason.
    let target = home.join("Main.java");
    std::fs::write(&target, "class Main {}").unwrap();

    let payload = serde_json::json!({
        "session_id": "s-deny",
        "tool_name": "Edit",
        "tool_input": { "file_path": target.to_string_lossy() }
    })
    .to_string();

    let out = guard_says(&payload, &home);
    let v: serde_json::Value = serde_json::from_str(&out)
        .unwrap_or_else(|e| panic!("the guard must emit JSON, got {out:?}: {e}"));

    let decision = v["hookSpecificOutput"]["permissionDecision"]
        .as_str()
        .or_else(|| v["permission"].as_str())
        .unwrap_or("");
    assert_eq!("deny", decision, "a .java hand-edit must be denied; got {out}");

    let reason = out.to_lowercase();
    assert!(reason.contains("jawata-author"), "the deny must name the authoring escape: {out}");

    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_declared_authoring_window_lets_the_next_java_edit_through() {
    let home = scratch("window");
    let target = home.join("Main.java");
    std::fs::write(&target, "class Main {}").unwrap();

    // 1. Declare the window with a Bash command, exactly as a user would.
    let declare = serde_json::json!({
        "session_id": "s-window",
        "tool_name": "Bash",
        "tool_input": { "command": "echo jawata-author: adding a new class" }
    })
    .to_string();
    let opened = guard_says(&declare, &home);
    assert!(
        !opened.to_lowercase().contains("\"deny\""),
        "declaring an authoring window must not itself be denied: {opened}"
    );

    // 2. The SAME session's .java edit now passes.
    let edit = serde_json::json!({
        "session_id": "s-window",
        "tool_name": "Edit",
        "tool_input": { "file_path": target.to_string_lossy() }
    })
    .to_string();
    let out = guard_says(&edit, &home);
    let v: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    let decision = v["hookSpecificOutput"]["permissionDecision"]
        .as_str()
        .or_else(|| v["permission"].as_str())
        .unwrap_or("");
    assert_ne!("deny", decision, "an edit inside a declared window must pass; got {out}");

    // 3. A DIFFERENT session is not covered — the window is session-scoped, or
    //    it is a standing bypass one declaration away.
    let other = serde_json::json!({
        "session_id": "s-other",
        "tool_name": "Edit",
        "tool_input": { "file_path": target.to_string_lossy() }
    })
    .to_string();
    let out2 = guard_says(&other, &home);
    let v2: serde_json::Value = serde_json::from_str(&out2).expect("JSON");
    let d2 = v2["hookSpecificOutput"]["permissionDecision"]
        .as_str()
        .or_else(|| v2["permission"].as_str())
        .unwrap_or("");
    assert_eq!("deny", d2, "another session must NOT inherit the window; got {out2}");

    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn a_brand_new_java_file_is_not_a_hand_edit() {
    let home = scratch("newfile");
    let target = home.join("Fresh.java"); // deliberately NOT created

    let payload = serde_json::json!({
        "session_id": "s-new",
        "tool_name": "Write",
        "tool_input": { "file_path": target.to_string_lossy() }
    })
    .to_string();

    let out = guard_says(&payload, &home);
    let v: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    let decision = v["hookSpecificOutput"]["permissionDecision"]
        .as_str()
        .or_else(|| v["permission"].as_str())
        .unwrap_or("");
    assert_ne!(
        "deny", decision,
        "writing a file that does not exist yet has nothing to refactor; got {out}"
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn the_shell_half_still_works_after_the_edit_half_was_added() {
    // The regression this file's history demands: adding one half must not
    // quietly cost the other. Both halves, one binary, one test run.
    let home = scratch("shell");
    let payload = serde_json::json!({
        "session_id": "s-shell",
        "tool_name": "Bash",
        "tool_input": { "command": "grep -rn foo src/main/java/Thing.java" }
    })
    .to_string();

    let out = guard_says(&payload, &home);
    let v: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    let decision = v["hookSpecificOutput"]["permissionDecision"]
        .as_str()
        .or_else(|| v["permission"].as_str())
        .unwrap_or("");
    assert_eq!("deny", decision, "a java grep must still be denied; got {out}");

    let _ = std::fs::remove_dir_all(&home);
}
