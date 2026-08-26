//! THE WORD REACHES THE RULE, THROUGH THE REAL BINARY.
//!
//! # What was broken
//!
//! `stop::judge` has had Rule B since Sprint 26 — *"do not stop when autonomy
//! is granted and nothing is armed"* — and production supplied
//! `Autonomy::Unknown` at the single call site in `pipeline.rs`. The rule is
//! gated on `Granted`, so the comparison was false on every stop the product
//! has ever made. `stop.rs` records the consequence in its own comment: the
//! rule *"never fired in 267 recorded stops, and the cause was detection"*.
//!
//! Twenty-odd unit tests construct `Autonomy::Granted` and all of them pass.
//! That is exactly why nothing ever went red: the rule, its bound and its
//! message were all covered, and the one thing missing was an input. The
//! signal — Harald typing `autocontinue` — arrives at `Role::UserPrompt`, which
//! parses the prompt and knows the session. Both ends existed and were never
//! joined.
//!
//! # Why this test spawns the binary
//!
//! Because a unit test cannot see the join. `autonomy::note_prompt` writes
//! through `home_dir()`, the stop role reads through `home_dir()`, and a test
//! of either half passes while the wire between them does not exist. That is
//! the shape this project has shipped repeatedly — a decision function green
//! while the pipeline never calls it. So: two invocations of the real
//! executable, in sequence, sharing one scratch `HOME`.

use std::process::{Command, Stdio};
use std::io::Write;

const HOOK: &str = env!("CARGO_BIN_EXE_jawata-hook");

fn scratch_home(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "jawata-autocontinue-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    // The hook refuses to act unconfigured — `--explain` reports
    // `SILENT [not-configured]`, which on stdout is indistinguishable from a
    // gate that looked and decided to stay quiet. So the scratch HOME gets the
    // same config file the deploy writes. The URL points nowhere on purpose:
    // nothing in this test needs the store, and a gate that cannot reach it
    // must still rule on autonomy.
    let studio = d.join(".claude").join("jawata-studio");
    std::fs::create_dir_all(&studio).unwrap();
    // BESIDE THE EXECUTABLE — `config_path_for` is `exe.parent().join(...)`,
    // not a HOME lookup. The state the gate writes still goes under HOME.
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

/// A transcript whose last assistant turn is PROSE with no tool calls — the
/// exact shape of "answered his question and stopped", which is the behaviour
/// Rule B exists to push past.
fn transcript(dir: &std::path::Path, user: &str, assistant: &str) -> std::path::PathBuf {
    let p = dir.join("transcript.jsonl");
    let body = format!(
        "{}\n{}\n",
        serde_json::json!({"type":"user","message":{"role":"user","content":user}}),
        serde_json::json!({"type":"assistant","message":{"role":"assistant",
            "content":[{"type":"text","text":assistant}]}})
    );
    std::fs::write(&p, body).unwrap();
    p
}

/// The binary picks its role from **argv[0]**, not from an argument — the deploy
/// writes one link per role (`jawata-hook-stop`, `jawata-hook-userprompt`). A
/// test that passes the role as an arg reaches no role at all and gets empty
/// output, which reads exactly like a gate that decided to stay silent.
fn linked(home: &std::path::Path, role: &str) -> std::path::PathBuf {
    let link = home.join(format!("jawata-hook-{role}"));
    if !link.exists() {
        // A COPY, not a symlink. The role comes from argv[0], but the CONFIG is
        // looked up beside `current_exe()` — and on Linux that resolves
        // /proc/self/exe through the symlink to the real target/debug binary,
        // so a linked role would read the build directory's config (there is
        // none) and go silent as `not-configured`. On stdout that is
        // indistinguishable from a gate that looked and chose to say nothing,
        // which is how this test spent three rounds failing for the wrong
        // reason.
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
    let child = Command::new(&exe)
        .env("HOME", home)
        .env("JAWATA_HOOK_CLIENT", "claude-code")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    // ETXTBSY: Linux refuses to exec a file that was just written while any
    // descriptor on it may still be open. Copy-then-exec hits it under parallel
    // test threads, and it is a property of the platform rather than of this
    // gate — so it is retried rather than "fixed", and named so a future reader
    // does not go looking for a bug in the hook.
    let mut child = child;
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
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
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

fn prompt_payload(session: &str, prompt: &str) -> String {
    serde_json::json!({"session_id": session, "prompt": prompt}).to_string()
}

/// THE test. Without the word, a finished prose turn is allowed to end — which
/// is correct, and is also what happened on every single stop for three weeks.
/// With the word, the same turn is refused.
///
/// The pair is asserted together because either half alone passes for the wrong
/// reason: a gate that blocks everything satisfies the second, and today's
/// gate — which cannot see autonomy at all — satisfies the first.
#[test]
fn his_word_turns_rule_b_on_and_its_absence_leaves_it_off() {
    let home = scratch_home("pair");
    let t = transcript(
        &home,
        "Status?",
        "Suite green, 2057 passing. That's the checkpoint done.",
    );

    let without = run("stop", &home, &stop_payload("sess-a", &t));
    assert!(
        !without.contains("RULE B"),
        "with no grant the turn must be free to end; got: {without}"
    );

    run("userprompt", &home, &prompt_payload("sess-a", "work the plan and autocontinue"));

    let with = run("stop", &home, &stop_payload("sess-a", &t));
    assert!(
        with.contains("RULE B"),
        "his word did not reach the rule — this is the wire that has never \
         existed, and the whole point of the test. got: {with}"
    );
    assert!(
        with.contains("\"block\""),
        "Rule B must BLOCK, not merely mention itself: {with}"
    );
}

/// A grant must survive his questions. If it expired on his next turn it would
/// be revoked by the very first thing he asked — and answering a question is
/// precisely where the stopping has been happening.
#[test]
fn the_grant_survives_the_questions_he_asks_afterwards() {
    let home = scratch_home("survives");
    let t = transcript(&home, "why did you stop", "Because I finished that piece.");
    run("userprompt", &home, &prompt_payload("sess-b", "autocontinue"));

    for question in ["Status?", "I want a list of what is left", "what changed?"] {
        run("userprompt", &home, &prompt_payload("sess-b", question));
        let out = run("stop", &home, &stop_payload("sess-b", &t));
        assert!(
            out.contains("RULE B"),
            "answering {question:?} ended the grant; got: {out}"
        );
    }
}

/// The wedge, and the shape of the bound. Two consecutive turns that produce
/// NOTHING is a stuck session, and the gate lets go. A ceiling on BLOCKS would
/// have released a working session just as fast.
#[test]
fn two_empty_turns_release_the_session_rather_than_wedging_it() {
    let home = scratch_home("wedge");
    let t = transcript(&home, "carry on", "I have nothing further here.");
    run("userprompt", &home, &prompt_payload("sess-c", "autocontinue"));

    let first = run("stop", &home, &stop_payload("sess-c", &t));
    assert!(first.contains("RULE B"), "first push: {first}");
    let second = run("stop", &home, &stop_payload("sess-c", &t));
    assert!(second.contains("RULE B"), "second push: {second}");
    let third = run("stop", &home, &stop_payload("sess-c", &t));
    assert!(
        !third.contains("RULE B"),
        "a session that produced nothing twice must be RELEASED, not held — an \
         unbounded hold is the Cursor incident, counter at 11 and still \
         climbing. got: {third}"
    );
}

/// And the revoke he can type. A revoke the gate does not honour is the worse
/// failure of the two: he would believe the leash was off and the session would
/// keep pushing itself.
#[test]
fn he_can_take_it_back() {
    let home = scratch_home("revoke");
    let t = transcript(&home, "ok", "Done with that.");
    run("userprompt", &home, &prompt_payload("sess-d", "autocontinue"));
    assert!(run("stop", &home, &stop_payload("sess-d", &t)).contains("RULE B"));

    run("userprompt", &home, &prompt_payload("sess-d", "stop autocontinue"));
    let after = run("stop", &home, &stop_payload("sess-d", &t));
    assert!(
        !after.contains("RULE B"),
        "the revoke was not honoured — note that it CONTAINS the grant word, so \
         checking the grant first re-grants on the message meant to end it. \
         got: {after}"
    );
}
