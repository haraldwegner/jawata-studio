//! THE CONVERSATION-LOOP COUNTERS RESET AT THE RIGHT BOUNDARIES — through the
//! real binary, because every defect here lived in the pipeline's I/O, which
//! no unit test of the pure `judge` can see.
//!
//! Two live defects, both found 2026-08-29 on Harald's "the conversation loop
//! counter needs to be reset correctly. Double check!":
//!
//! * The audit-fix counter lived on the WINDOW, and every audit verdict
//!   arrives as a background notification that opens a new window — so the
//!   alarm could never fire on the loop it was built for, while firing on
//!   prose that merely discussed refusals.
//! * The review-bounce ceiling was charged by EVERY block: a Rule B push
//!   wrote bounce=1 and the review rule's very first offence then read
//!   "(2 of 3)". Three unrelated pushes and an unjudged ask sails through at
//!   the cap.

use std::io::Write;
use std::process::{Command, Stdio};

const HOOK: &str = env!("CARGO_BIN_EXE_jawata-hook");

fn scratch_home(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "jawata-loopreset-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    let studio = d.join(".claude").join("jawata-studio");
    std::fs::create_dir_all(&studio).unwrap();
    std::fs::write(
        d.join("hook_config.json"),
        serde_json::json!({
            "client": "claude-code",
            "field_dir": studio.join("field").to_string_lossy(),
            "token": "test-token",
            "url": "http://127.0.0.1:9/mcp",
        })
        .to_string(),
    )
    .unwrap();
    d
}

/// One window: an opening user line (harness notice or keyboard) plus a prose
/// assistant turn — no tool calls, the shape every counter rule judges.
fn window_transcript(dir: &std::path::Path, opener: &str, assistant: &str) -> std::path::PathBuf {
    let p = dir.join("transcript.jsonl");
    let body = format!(
        "{}\n{}\n",
        serde_json::json!({"type":"user","message":{"role":"user","content":opener}}),
        serde_json::json!({"type":"assistant","message":{"role":"assistant",
            "content":[{"type":"text","text":assistant}]}})
    );
    std::fs::write(&p, body).unwrap();
    p
}

fn linked(home: &std::path::Path, role: &str) -> std::path::PathBuf {
    let link = home.join(format!("jawata-hook-{role}"));
    if !link.exists() {
        std::fs::copy(HOOK, &link).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&link, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    link
}

fn run(role: &str, home: &std::path::Path, payload: &str) -> String {
    let exe = linked(home, role);
    let mut child = Command::new(&exe)
        .env("HOME", home)
        .env("JAWATA_HOOK_CLIENT", "claude-code")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    for _ in 0..20 {
        match child {
            Err(ref e) if e.raw_os_error() == Some(26) => {
                std::thread::sleep(std::time::Duration::from_millis(25));
                child = Command::new(&exe)
                    .env("HOME", home)
                    .env("JAWATA_HOOK_CLIENT", "claude-code")
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn();
            }
            _ => break,
        }
    }
    let mut child = child.expect("the hook binary runs");
    child.stdin.as_mut().unwrap().write_all(payload.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn stop_payload(session: &str, transcript: &std::path::Path) -> String {
    serde_json::json!({
        "session_id": session,
        "transcript_path": transcript.to_string_lossy(),
        "stop_hook_active": false,
    })
    .to_string()
}

const NOTICE: &str = "<task-notification> <task-id>x</task-id> <result># VERDICT relayed below</result>";
const REFUSAL_TEXT: &str = "# VERDICT: REFUSE\none blocking finding, repairing now";

/// THE ALARM SPANS NOTIFICATION WINDOWS. Three audit refusals, each arriving
/// in its own notice-opened window — the real overnight shape — must trip the
/// AUDIT-FIX alarm on the third, where the per-window count could never
/// exceed one.
#[test]
fn the_audit_alarm_fires_across_notice_windows() {
    let home = scratch_home("spans");
    let t = window_transcript(&home, NOTICE, REFUSAL_TEXT);
    let one = run("stop", &home, &stop_payload("sess-loop", &t));
    assert!(!one.contains("AUDIT-FIX"), "first refusal is not a loop: {one}");
    let two = run("stop", &home, &stop_payload("sess-loop", &t));
    assert!(!two.contains("AUDIT-FIX"), "second refusal is not a loop: {two}");
    let three = run("stop", &home, &stop_payload("sess-loop", &t));
    assert!(
        three.contains("AUDIT-FIX"),
        "the third refusal IS the loop, and before this fix the notice-opened \
         window reset the count to one every round: {three}"
    );
}

/// A relayed SIGN-OFF is the loop converging — the carry resets, and the next
/// loop starts at zero rather than inheriting the last one's near-alarm.
#[test]
fn a_sign_off_resets_the_alarm() {
    let home = scratch_home("signoff");
    let t = window_transcript(&home, NOTICE, REFUSAL_TEXT);
    run("stop", &home, &stop_payload("sess-so", &t));
    run("stop", &home, &stop_payload("sess-so", &t));
    let converged =
        window_transcript(&home, NOTICE, "# VERDICT: SIGN-OFF\nall clauses met");
    run("stop", &home, &stop_payload("sess-so", &converged));
    let after = window_transcript(&home, NOTICE, REFUSAL_TEXT);
    let out = run("stop", &home, &stop_payload("sess-so", &after));
    assert!(
        !out.contains("AUDIT-FIX"),
        "a refusal after a sign-off is round one of a NEW loop: {out}"
    );
}

/// A KEYBOARD window resets the carry: the human is steering, so whatever
/// loop was running is his conversation now, not a runaway.
#[test]
fn a_keyboard_window_resets_the_alarm() {
    let home = scratch_home("keyboard");
    let t = window_transcript(&home, NOTICE, REFUSAL_TEXT);
    run("stop", &home, &stop_payload("sess-kb", &t));
    run("stop", &home, &stop_payload("sess-kb", &t));
    // He types; the reply even relays a refusal — still a fresh count of one.
    let his = window_transcript(&home, "what is the audit saying", REFUSAL_TEXT);
    run("stop", &home, &stop_payload("sess-kb", &his));
    let after = window_transcript(&home, NOTICE, REFUSAL_TEXT);
    let out = run("stop", &home, &stop_payload("sess-kb", &after));
    assert!(
        !out.contains("AUDIT-FIX"),
        "two refusals after his message are not three — the keyboard reset the \
         carry: {out}"
    );
}

/// A `DECISION:` LINE BUYS NOTHING — through the real binary, end to end.
///
/// This test measured a different thing until v4.0.0: that a Rule B push must
/// not spend the REVIEW ceiling, a bound belonging to the reviewer rule that is
/// now retired. Its lesson — one rule's bounce must never charge another rule's
/// ceiling — is not lost; it is asserted against the rule that still has a
/// per-bounce bound, in `a_turn_held_by_another_rule_does_not_spend_a_reseed_chance`.
///
/// What it pins now is the change itself, at the only layer where it matters.
/// Harald, 2026-09-03: *"YOU WANT TO STOP ALL THE TIME AND ARE TRAINED ON THE
/// QUICK RESULT."* The gate used to let a stop through on a line the agent
/// wrote itself, and an agent that wants the turn to end writes that line on
/// the first attempt — no adaptation, nothing to detect. Here the same line is
/// typed and the gate holds.
#[test]
fn a_self_declared_decision_no_longer_ends_the_turn() {
    let home = scratch_home("ceiling");
    let idle = window_transcript(&home, NOTICE, "All gates green. Continuing.");
    run(
        "userprompt",
        &home,
        &serde_json::json!({"session_id": "sess-ceil", "prompt": "work the plan and autocontinue"})
            .to_string(),
    );
    let push = run("stop", &home, &stop_payload("sess-ceil", &idle));
    assert!(push.contains("RULE B"), "precondition: the push fired: {push}");

    let ask = window_transcript(&home, NOTICE, "DECISION: ship it? Say go and I run it.");
    let out = run("stop", &home, &stop_payload("sess-ceil", &ask));
    assert!(
        out.contains("RULE B") && out.contains("autocontinue"),
        "a decision the agent declares about itself must route to the judge, not \
         end the turn: {out}"
    );
}
