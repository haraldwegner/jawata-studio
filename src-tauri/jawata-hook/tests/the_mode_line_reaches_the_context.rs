//! THE GRANT'S STATE REACHES THE MODEL'S CONTEXT — through the real binary.
//!
//! # Harald's diagnosis, 2026-08-31, which this implements
//!
//! *"If I say 'autocontinue' then the hook puts the parameter to yes, but you
//! still have it in your context. And this is independent of the hook -> If I
//! restart the communication with a question you move on with the plan."*
//!
//! The grant existed twice: the hook's file, cleared the instant he types, and
//! the agent's remembered instruction, which nothing ever cleared. Measured the
//! night before: the file read `NotGranted` from 20:23 while the agent kept
//! executing the sprint plan for hours on its stale copy. The fix pushes the
//! file's state into the model's context on every prompt event, so re-asserted
//! replaces remembered.
//!
//! # The one dependency that must NOT exist
//!
//! The mode line must arrive even when the recall has nothing to say — the
//! store being down is a fact about the store, not about the grant. Every case
//! below runs with the resident deliberately unreachable, so a line that only
//! rode on recall output would fail all four.

use std::io::{Read, Write};
use std::process::{Command, Stdio};

const HOOK: &str = env!("CARGO_BIN_EXE_jawata-hook");

fn userprompt_says(payload: &str, home: &std::path::Path) -> String {
    let dir = home.join("bin");
    std::fs::create_dir_all(&dir).expect("scratch bin");
    let exe = dir.join(if cfg!(windows) {
        "jawata-hook-userprompt.exe"
    } else {
        "jawata-hook-userprompt"
    });
    // COPY ONCE, and this is the release-red fix, not a nicety: this file's
    // arm-then-clear test calls the helper TWICE with one scratch home, and
    // re-copying over an already-executed binary killed the second exec on
    // macOS with a signal (code signature is cached per inode, so rewriting
    // the inode invalidates the running image's identity) and broke the copy
    // on Windows (sharing violation). Both Linux runners sailed through,
    // which is why the local suite was green while three CI platforms went
    // red at exit 101 — v3.17.5's first attempt, 2026-08-31.
    if !exe.exists() {
        std::fs::copy(HOOK, &exe).expect("copy the built binary to its role name");
    }
    // Claude Code dialect — the client this line ships to first — and a DEAD
    // resident address, which is the point: the grant's state must not depend
    // on the store answering.
    std::fs::write(
        dir.join("hook_config.json"),
        r#"{"url":"http://127.0.0.1:1/mcp","token":"t","client":"claude-code"}"#,
    )
    .expect("write the hook config");

    // ETXTBSY retry — fourth copy of this dance; recorded as extractable in
    // `edit_gate_runs_the_real_binary`.
    let mut attempt = 0;
    let mut child = loop {
        match Command::new(&exe)
            .env("HOME", home)
            .env("USERPROFILE", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(c) => break c,
            Err(e) if e.raw_os_error() == Some(26) && attempt < 40 => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => panic!("the userprompt binary must be executable: {e}"),
        }
    };
    child.stdin.take().expect("piped").write_all(payload.as_bytes()).expect("write");
    let status = child.wait().expect("must terminate");
    assert_eq!(Some(0), status.code(), "a hook must never fail the client");
    let mut out = String::new();
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_string(&mut out);
    }
    out
}

fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("jawata-modeline-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("scratch home");
    d
}

fn payload(session: &str, prompt: &str) -> String {
    serde_json::json!({ "session_id": session, "prompt": prompt }).to_string()
}

#[test]
fn his_word_injects_plan_mode_even_with_the_store_down() {
    let home = scratch("on");

    let out = userprompt_says(&payload("s-on", "carry on and autocontinue"), &home);

    assert!(
        out.contains("AUTOCONTINUE: ON"),
        "his word armed the grant, so the SAME event must say so into context — with \
         the resident unreachable, so this proves the line does not ride on the \
         recall. Got: {out}"
    );
    assert!(
        out.contains("checkpoint"),
        "ON must carry the rule that makes it plan-execution mode: stops only at the \
         plan's own checkpoints. Got: {out}"
    );
}

#[test]
fn his_question_injects_conversation_mode() {
    let home = scratch("off");
    // Arm first, then clear with an ordinary message — the transition the two
    // stale-copy incidents were made of.
    let _ = userprompt_says(&payload("s-off", "work the plan and autocontinue"), &home);

    let out = userprompt_says(&payload("s-off", "why did you stop?"), &home);

    assert!(
        out.contains("AUTOCONTINUE: OFF"),
        "his typing cleared the file; the context must be told in the same event, or \
         the remembered copy keeps running the plan — the 20:23 divergence. Got: {out}"
    );
    assert!(
        out.contains("turn an answer into the start"),
        "OFF must carry the conversation rule — answer what was asked, don't turn \
         around and work. Got: {out}"
    );
}

/// A wake-up is told the STANDING state, not a fabricated transition.
///
/// Harness notifications arrive at this same hook. v3.17.3 made them unable to
/// CLEAR the grant; this asserts they now also RE-ASSERT it — which is exactly
/// what a session resuming from a background job needs to know, and what it
/// previously had to remember across the gap.
#[test]
fn a_harness_notification_reasserts_the_standing_grant() {
    let home = scratch("harness");
    let _ = userprompt_says(&payload("s-h", "carry on and autocontinue"), &home);

    let out = userprompt_says(
        &payload("s-h", "<system-reminder>\n<task-notification>job done</task-notification>\n</system-reminder>"),
        &home,
    );

    assert!(
        out.contains("AUTOCONTINUE: ON"),
        "the machine is not him: a notification must neither clear the grant nor be \
         told OFF — the standing state is ON and the wake-up needs to know it. Got: {out}"
    );
}

/// No session, no line: a mode asserted about a session we cannot identify
/// would be an invented fact placed directly into the model's context.
#[test]
fn no_session_means_no_mode_line() {
    let home = scratch("nosession");

    let out = userprompt_says(
        &serde_json::json!({ "prompt": "carry on and autocontinue" }).to_string(),
        &home,
    );

    assert!(
        !out.contains("AUTOCONTINUE"),
        "without a session id there is no state to assert, and asserting one anyway \
         is fabrication. Got: {out}"
    );
}
