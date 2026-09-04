//! DON'T TURN AROUND AND WORK — through the REAL binary, under its deployed
//! role name.
//!
//! # The failure this refuses
//!
//! Harald, 2026-08-30, after four questions in a row each answered and then
//! used as the opening of an unrelated task: *"in a conversation is the other
//! way round. You are not working on a plan but talking with me -> Hence, don't
//! turn around and work on something."* At 23:01:44 that evening a commit
//! landed in his repository mid-question, while the gate's own record for that
//! window already read `NotGranted` — the signal was present and nothing
//! consulted it.
//!
//! # Why it is a PreToolUse refusal and not a stop-gate report
//!
//! The stop gate runs when the turn is over. It can record that a commit
//! happened mid-conversation; it cannot stop one. A control that must prevent
//! something, placed on a channel that only runs afterwards, is the shape this
//! project's architecture rules name and refuse. So the decision sits before
//! the call.
//!
//! # Why the REAL binary
//!
//! A unit test on the decision function passes whether or not the pipeline ever
//! calls it. This project has shipped exactly that twice — a guard carrying one
//! of its two halves, green the whole time. Every case below spawns the built
//! executable and reads the JSON it prints.

use std::io::{Read, Write};
use std::process::{Command, Stdio};

const HOOK: &str = env!("CARGO_BIN_EXE_jawata-hook");

fn guard_says(payload: &str, home: &std::path::Path) -> String {
    let dir = home.join("bin");
    std::fs::create_dir_all(&dir).expect("scratch bin");
    let exe = dir.join(if cfg!(windows) { "jawata-hook-guard.exe" } else { "jawata-hook-guard" });
    std::fs::copy(HOOK, &exe).expect("copy the built binary to its role name");
    std::fs::write(
        dir.join("hook_config.json"),
        r#"{"url":"http://127.0.0.1:1/mcp","token":"t","client":"cursor"}"#,
    )
    .expect("write the hook config");

    // ETXTBSY: Linux refuses to exec a binary still open for writing, and these
    // integration binaries run concurrently. Third copy of this dance in the
    // suite; the duplication is recorded in `edit_gate_runs_the_real_binary`.
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
            Err(e) => panic!("the guard binary must be executable: {e}"),
        }
    };
    child.stdin.take().expect("piped").write_all(payload.as_bytes()).expect("write");
    let status = child.wait().expect("the guard must terminate");
    assert_eq!(
        Some(0),
        status.code(),
        "a guard must never fail the client — a non-zero exit is itself a block"
    );
    let mut out = String::new();
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_string(&mut out);
    }
    out
}

fn scratch(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("jawata-turnaround-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("scratch home");
    d
}

/// BOTH CLIENT DIALECTS, and the first draft of this file got it wrong in a way
/// worth keeping: it asserted on `permissionDecision`, which is Claude Code's
/// key, while the test config runs the Cursor dialect that says `permission`.
/// The deny case failed loudly — but the four CONTROLS passed VACUOUSLY, because
/// each asserted the ABSENCE of a string that never appears in this dialect. A
/// control that cannot observe the thing it controls for is not a control.
///
/// So the controls below assert ALLOW POSITIVELY. If the output shape moves
/// again, they go red instead of quietly agreeing.
fn is_deny(out: &str) -> bool {
    out.contains("\"permission\":\"deny\"") || out.contains("\"permissionDecision\":\"deny\"")
}

fn is_allow(out: &str) -> bool {
    out.contains("\"permission\":\"allow\"") || out.contains("\"permissionDecision\":\"allow\"")
}

fn human(text: &str) -> String {
    serde_json::json!({"type":"user","message":{"content":text}}).to_string()
}

fn harness(text: &str) -> String {
    // The shape a task notification arrives in — NOT the human.
    serde_json::json!({"type":"user","message":{"content":
        format!("<system-reminder>\n{text}\n</system-reminder>")}})
    .to_string()
}

fn assistant(text: &str) -> String {
    serde_json::json!({"type":"assistant","message":{"content":[{"type":"text","text":text}]}})
        .to_string()
}

/// Long enough to be an ANSWER rather than narration between two calls. The cut
/// is 600 characters and is a declared proxy — see `stop::ANSWER_LENGTH`.
fn an_answer() -> String {
    "x".repeat(700)
}

/// The shape of ordinary narration between two tool calls: short.
fn narration() -> String {
    "Checking the log.".to_string()
}

fn transcript(home: &std::path::Path, name: &str, lines: &[String]) -> std::path::PathBuf {
    let p = home.join(format!("{name}.jsonl"));
    std::fs::write(&p, lines.join("\n") + "\n").expect("write transcript");
    p
}

fn payload(tool: &str, t: &std::path::Path, file: &std::path::Path) -> String {
    serde_json::json!({
        "session_id": "s-turnaround",
        "tool_name": tool,
        "transcript_path": t.to_string_lossy(),
        "tool_input": { "file_path": file.to_string_lossy() }
    })
    .to_string()
}

#[test]
fn a_write_after_answering_him_is_refused() {
    let home = scratch("deny");
    let t = transcript(&home, "w", &[human("why did you stop?"), assistant(&an_answer())]);
    let target = home.join("notes.md");
    std::fs::write(&target, "x").unwrap();

    let out = guard_says(&payload("Write", &t, &target), &home);

    assert!(
        is_deny(&out),
        "his message opened this window and an answer is already out — a WRITE now is the \
         turn-around he named. Got: {out}"
    );
    assert!(
        out.to_lowercase().contains("answer"),
        "the refusal must say what to do instead — work first, answer last. Got: {out}"
    );
}

/// CONTROL 1 — reads are never refused. Answering him often requires them, and
/// a rule that blocked them would make the gate unusable in exactly the
/// conversations it exists to protect.
#[test]
fn a_read_after_answering_him_is_allowed() {
    let home = scratch("read");
    let t = transcript(&home, "r", &[human("why did you stop?"), assistant(&an_answer())]);
    let target = home.join("some.log");
    std::fs::write(&target, "x").unwrap();

    let out = guard_says(&payload("Read", &t, &target), &home);

    assert!(
        is_allow(&out),
        "a READ is how the agent answers him — refusing it would be the rule eating its \
         own purpose. Got: {out}"
    );
}

/// CONTROL 2 — narration between tool calls is not an answer.
///
/// Without this the rule fires on every ordinary working turn, because an agent
/// narrates constantly while it works. This is the case the 600-character cut
/// exists for, and it is the one most likely to break if the number moves.
#[test]
fn short_narration_is_not_an_answer() {
    let home = scratch("short");
    let t = transcript(&home, "s", &[human("fix it"), assistant(&narration())]);
    let target = home.join("notes.md");
    std::fs::write(&target, "x").unwrap();

    let out = guard_says(&payload("Write", &t, &target), &home);

    assert!(
        is_allow(&out),
        "\"Checking the log.\" is not an answer, and a rule that read it as one would \
         block every working turn. Got: {out}"
    );
}

/// CONTROL 3 — the harness is not him.
///
/// A task notification opens a window too. If that counted as his presence, an
/// unattended run would be frozen the moment a background job reported back —
/// which is the defect v3.17.3 fixed on the other side of this same gate.
#[test]
fn a_window_opened_by_the_harness_does_not_freeze_the_work() {
    let home = scratch("harness");
    let t = transcript(
        &home,
        "h",
        &[harness("a background job finished"), assistant(&an_answer())],
    );
    let target = home.join("notes.md");
    std::fs::write(&target, "x").unwrap();

    let out = guard_says(&payload("Write", &t, &target), &home);

    assert!(
        is_allow(&out),
        "he did not open this window — a job did. Freezing here would strand every \
         unattended run at its first wake-up. Got: {out}"
    );
}

/// CONTROL 5 — THE GATE'S OWN PUSH REOPENS WRITING.
///
/// Measured the morning v3.17.4 shipped: one substantial answer froze writes for
/// the REST of the window, because the flag never reset — so under a standing
/// grant, Rule B pushed the agent to work while this guard refused every write.
/// A whole read-only morning ran inside that deadlock. The answer was DELIVERED
/// before the push; the push starts a fresh attempt, and the turn-around rule is
/// about answer-then-work inside ONE attempt.
#[test]
fn the_stop_gates_push_reopens_writing() {
    let home = scratch("push");
    let t = transcript(
        &home,
        "p",
        &[
            human("why did you stop?"),
            assistant(&an_answer()),
            // The stop gate's own bounce, exactly as the client injects it.
            human("Stop hook feedback:\nRULE B: autonomy is granted and this turn armed …"),
        ],
    );
    let target = home.join("notes.md");
    std::fs::write(&target, "x").unwrap();

    let out = guard_says(&payload("Write", &t, &target), &home);

    assert!(
        is_allow(&out),
        "after the gate's own push the previous answer is history — refusing here \
         deadlocks Rule B against this guard, which is exactly what happened live. \
         Got: {out}"
    );
}

/// THE REFUSAL IS ONE-SHOT PER WINDOW (v3.17.6) — after the guard's own denial
/// is on the record, the retry proceeds.
///
/// Why, measured within an hour of v3.17.5 shipping: a dispatched task's own
/// protocol required an answer BEFORE its writes (/memorize's story list), the
/// list tripped the 600-character bar, and every write for the rest of the
/// window was refused — while the only reset lived on a stop-gate push that
/// cannot come with the grant off. "Over-firing costs a reorder" turned out to
/// cost the window. One denial interrupts the turn-around and names the rule;
/// repeating it buys no compliance the first refusal did not, only the
/// deadlock.
#[test]
fn the_refusal_is_one_shot_per_window() {
    let home = scratch("oneshot");
    let denial_result = serde_json::json!({"type":"user","message":{"content":[
        {"type":"tool_result","content":
            "ANSWER FIRST OR WORK FIRST, NOT ANSWER THEN WORK. He is in this window …"}
    ]}})
    .to_string();
    let t = transcript(
        &home,
        "o",
        &[human("why did you stop?"), assistant(&an_answer()), denial_result],
    );
    let target = home.join("notes.md");
    std::fs::write(&target, "x").unwrap();

    let out = guard_says(&payload("Write", &t, &target), &home);

    assert!(
        is_allow(&out),
        "the denial is already on the record in this window — refusing again is the \
         deadlock, not the rule. Got: {out}"
    );
}

/// CONTROL 4 — no transcript means SILENT, never refuse.
///
/// A guard that blocks when it cannot see would take out every session whose
/// transcript it failed to read. The failure direction is chosen deliberately.
#[test]
fn an_unreadable_transcript_refuses_nothing() {
    let home = scratch("blind");
    let target = home.join("notes.md");
    std::fs::write(&target, "x").unwrap();
    let missing = home.join("does-not-exist.jsonl");

    let out = guard_says(&payload("Write", &missing, &target), &home);

    assert!(
        is_allow(&out),
        "unreadable evidence is not evidence of a violation. Got: {out}"
    );
}

/// A SUBAGENT'S WINDOW IS NOT HIS WINDOW — the 2026-09-04 dogfood defect.
///
/// The harness stamps every line of a sidechain `"isSidechain": true`, and this
/// binary read none of it. So a subagent's own prompt looked exactly like the
/// human opening a window, and the rule fired on work nobody was waiting to
/// read.
///
/// The one-shot reset could not end it either, and that is the part worth
/// keeping. A denial comes back carrying `TURNAROUND_MARKER`, which clears
/// `answered_substantially` — one call gets through. Then the subagent writes
/// its next paragraph, the flag is set again, and the next call is refused. In
/// the human's session his next message ends the window; a sidechain has no
/// such message, so the speed bump is a wall.
///
/// MEASURED on the transcript of an architect seat run: SIX refusals in one
/// sidechain, every one on a read-only shell command, and the seat reported
/// its own gates as NOT RUN and therefore NOT passed.
#[test]
fn a_write_inside_a_subagent_is_not_the_turn_around() {
    let home = scratch("sidechain");
    let opened = serde_json::json!({
        "type": "user", "isSidechain": true,
        "message": {"content": "review these three shas and report"}
    })
    .to_string();
    let answered = serde_json::json!({
        "type": "assistant", "isSidechain": true,
        "message": {"content": [{"type": "text", "text": an_answer()}]}
    })
    .to_string();
    let t = transcript(&home, "sub", &[opened, answered]);
    let target = home.join("notes.md");
    std::fs::write(&target, "x").unwrap();

    let out = guard_says(&payload("Write", &t, &target), &home);

    assert!(
        is_allow(&out),
        "the parent agent opened this window, not him — and he is not reading it. \
         `a_write_after_answering_him_is_refused` above is the DISCRIMINATOR: it \
         is this same transcript without the sidechain stamps, and it must still \
         deny. Got: {out}"
    );
}
