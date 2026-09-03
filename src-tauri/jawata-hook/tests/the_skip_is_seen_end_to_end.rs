//! THE INCIDENT, REPLAYED THROUGH THE REAL BINARY.
//!
//! # What happened, and why a unit test cannot see it
//!
//! The failure this whole stage exists for: the store handed over entry
//! `1f67feb7` — Sprint 23's bundle-pool approach, for the exact problem being
//! diagnosed — the agent read it, reasoned past it, and **nothing observed the
//! miss**. That record is anchored at a PACKAGE, so the recall gate cannot
//! fire on it by design (gating on `org.jawata.core` would fire constantly and
//! turn `recall-rejected: not relevant` into a reflex token). The observer's
//! ledger is the half that covers it.
//!
//! Both halves of that ledger already had unit tests, and both were green
//! while the WIRE between them was untested: `note_injection` writes through
//! `home_dir()`, the stop role reads through `home_dir()`, and nothing drove
//! one into the other. That is the shape this project has shipped twice — a
//! decision function passing its own tests while the pipeline never calls it
//! (see `edit_gate_runs_the_real_binary.rs`, same reasoning, same cure).
//!
//! So this test spawns the built executable under two role names in sequence,
//! with a scratch `HOME` and a real socket:
//!
//! 1. **userprompt** — a package-anchored record is offered and INJECTED.
//! 2. **stop** — the session ends having said nothing about it.
//!
//! The skip signal must appear in `outcomes.log`, and it must appear as a LOG
//! LINE rather than a stop decision: a skip is a measurement, and bouncing the
//! agent back into a finished turn over one would wedge the session.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};

const HOOK: &str = env!("CARGO_BIN_EXE_jawata-hook");

/// The record from the incident, in the text form the store returns. Anchored
/// at a PACKAGE — which is the whole point: the gate stands down here and only
/// the ledger can see what happens next.
const PACKAGE_ANCHORED: &str = "[lesson] Sprint 23 already shipped the bundle-pool approach for \
    this: ExternalBundlePool.index(defaultPoolDirs()) is called from the method under study \
    (scope: org.jawata.core, org.jawata.mcp)";

/// Serve recall answers for as long as the test runs. Multi-request, because
/// the recall path tries symbol cues and then symptoms — a `serve_once` server
/// would make the outcome depend on which cue happened to be asked first.
fn serve_recalls(data: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    let addr = listener.local_addr().unwrap();
    let inner = serde_json::json!({ "success": true, "data": data }).to_string();
    let envelope =
        serde_json::json!({ "result": { "content": [ { "type": "text", "text": inner } ] } })
            .to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        envelope.len(),
        envelope
    );
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut buf = [0u8; 8192];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    format!("http://{addr}/mcp")
}

/// Run the binary under one deployed role name and return its stdout.
///
/// `argv[0]` selects the role, so the name is load-bearing — running it as
/// anything else exercises a different role and proves nothing about this one.
fn run_as(role: &str, home: &std::path::Path, url: &str, payload: &str) -> String {
    let bin = home.join("bin");
    std::fs::create_dir_all(&bin).expect("scratch bin");
    let exe = bin.join(if cfg!(windows) {
        format!("jawata-hook-{role}.exe")
    } else {
        format!("jawata-hook-{role}")
    });
    std::fs::copy(HOOK, &exe).expect("copy the built binary to its role name");
    std::fs::write(
        bin.join("hook_config.json"),
        serde_json::json!({ "url": url, "token": "t", "client": "claude-code" }).to_string(),
    )
    .expect("write the hook config");

    // RETRY THE EXEC. Linux refuses to exec a binary still open for writing
    // (ETXTBSY); the sibling integration tests carry the same dance for the
    // same errno, and it failed only on one CI architecture — which is how a
    // race hides until it does not.
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
            Err(e) if attempt < 20 => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(25));
                if attempt == 20 {
                    panic!("could not exec {exe:?}: {e}");
                }
            }
            Err(e) => panic!("could not exec {exe:?}: {e}"),
        }
    };
    child.stdin.take().unwrap().write_all(payload.as_bytes()).expect("feed stdin");
    let out = child.wait_with_output().expect("the hook must terminate");
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("jawata-skip-e2e-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn outcomes(home: &std::path::Path) -> String {
    std::fs::read_to_string(home.join(".claude").join("jawata-studio").join("outcomes.log"))
        .unwrap_or_default()
}

#[test]
fn knowledge_injected_and_never_answered_raises_the_skip_at_session_end() {
    let home = scratch("skip");
    let url = serve_recalls(PACKAGE_ANCHORED);
    let session = "s-incident";

    // 1. THE OFFER. A prompt naming the symbol under diagnosis; the store
    //    answers with the package-anchored record.
    let prompt = serde_json::json!({
        "session_id": session,
        "prompt": "why does ProjectImporter#addDependencyEntries resolve nothing here?"
    })
    .to_string();
    let emitted = run_as("userprompt", &home, &url, &prompt);
    assert!(
        emitted.contains("additionalContext"),
        "the record must actually REACH the session — a skip charged to a client that \
         cannot inject would be a false accusation, so this half is load-bearing: {emitted}"
    );
    assert!(
        emitted.contains("bundle-pool"),
        "and it must be the incident's own record that arrived: {emitted}"
    );

    // The ledger keys on INJECTED, so the emission above is what makes the
    // next step's verdict possible at all.
    let ledger = home.join(".claude").join("jawata-studio").join("recallledger").join(session);
    assert!(ledger.exists(), "the injection must have been recorded at {ledger:?}");

    // 2. THE SILENCE. The turn ends with no `recall-applied` and no
    //    `recall-rejected: <reason>` anywhere in it.
    let transcript = home.join("transcript.jsonl");
    std::fs::write(
        &transcript,
        // The communicator pass is part of the FIXTURE, not the subject: since
        // 2026-08-20 every stop needs one, and without it this turn would bounce
        // for that unrelated reason — which would leave the assertion below
        // reading "no decision" when the truth is "a decision about something
        // else". The subject here is that a SKIP produces no decision.
        "{\"type\":\"user\",\"message\":{\"content\":\"why does it resolve nothing?\"}}\n\
         {\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"tool_use\",\
         \"name\":\"Agent\",\"input\":{\"subagent_type\":\"communicator\"}}]}}\n\
         {\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\
         \"text\":\"The importer never populates projectDependencies here.\"}]}}\n",
    )
    .unwrap();
    let stop = serde_json::json!({
        "session_id": session,
        "transcript_path": transcript.to_string_lossy(),
        "stop_hook_active": false
    })
    .to_string();
    let stop_out = run_as("stop", &home, &url, &stop);

    let log = outcomes(&home);
    assert!(
        log.contains("recall-skipped"),
        "the incident must now be SEEN — knowledge injected, nothing said, session over.\n\
         outcomes.log: {log:?}"
    );
    assert!(
        log.contains("injected=1"),
        "and the signal must carry how much was ignored: {log:?}"
    );

    // RECORDED, NEVER BLOCKED. The Stop role's only injection shape is a block
    // decision; a first draft of this reached for it, which would have bounced
    // the agent back into a turn it had finished — over a measurement.
    assert!(
        !stop_out.contains("\"decision\""),
        "the skip is an observation, not a stop decision: {stop_out:?}"
    );

    let _ = std::fs::remove_dir_all(&home);
}

/// THE FALSE-ACCUSATION GUARD, end to end. One word from the agent closes it,
/// and the same session then produces no signal at all.
/// THE DISCRIMINATOR for decoupling the skip from the verdict.
///
/// The skip used to be emitted inside the ALLOW arm, which tied a measurement
/// to a ruling. That was harmless only while the gate almost never blocked;
/// the moment the communicator rule became unconditional, every unjudged turn
/// blocked and the skip stopped being recorded at all — the signal Stage 5
/// exists to produce, disabled by the commit that was strengthening the gate.
///
/// Here the turn is BLOCKED (no communicator pass) and the skip must be
/// recorded anyway. Move the emission back inside the Allow arm and this fails.
#[test]
fn a_blocked_turn_still_records_the_skip() {
    let home = scratch("skip-blocked");
    let url = serve_recalls(PACKAGE_ANCHORED);
    let session = "s-blocked";

    let prompt = serde_json::json!({
        "session_id": session,
        "prompt": "why does ProjectImporter#addDependencyEntries resolve nothing here?"
    })
    .to_string();
    let emitted = run_as("userprompt", &home, &url, &prompt);
    assert!(emitted.contains("additionalContext"), "the offer must reach the session: {emitted}");

    // A BLOCKED TURN IS THIS TEST'S SUBJECT, and which rule blocks it is not.
    // It used to be the reviewer rule, retired in v4.0.0; the length rule
    // serves exactly as well and is better suited — it needs no autonomy grant,
    // so this test stays about the skip observation rather than about Rule B.
    let transcript = home.join("transcript.jsonl");
    let long_answer = "The importer never populates projectDependencies here. ".repeat(60);
    std::fs::write(
        &transcript,
        format!(
            "{}\n{}\n",
            serde_json::json!({"type":"user","message":{"content":"look at the importer"}}),
            serde_json::json!({"type":"assistant","message":{"content":[
                {"type":"text","text": long_answer}]}}),
        ),
    )
    .unwrap();
    let stop = serde_json::json!({
        "session_id": session,
        "transcript_path": transcript.to_string_lossy(),
        "stop_hook_active": false
    })
    .to_string();
    let stop_out = run_as("stop", &home, &url, &stop);

    assert!(
        stop_out.contains("TOO LONG"),
        "this turn must indeed be blocked, or the test proves nothing: {stop_out:?}"
    );
    let log = outcomes(&home);
    assert!(
        log.contains("recall-skipped"),
        "a turn that ignored its recalled knowledge AND got bounced for another reason is \
         STILL a turn that ignored its recalled knowledge — the observation must not \
         depend on the verdict.\noutcomes.log: {log:?}"
    );
    assert!(log.contains("injected=1"), "and it must carry how much was ignored: {log:?}");
}

#[test]
fn one_disposition_closes_the_session_and_nothing_is_raised() {
    let home = scratch("answered");
    let url = serve_recalls(PACKAGE_ANCHORED);
    let session = "s-answered";
    let dir = home.join(".claude").join("jawata-studio");

    let prompt = serde_json::json!({
        "session_id": session,
        "prompt": "why does ProjectImporter#addDependencyEntries resolve nothing here?"
    })
    .to_string();
    assert!(run_as("userprompt", &home, &url, &prompt).contains("additionalContext"));

    // The agent judged it — which is all the ledger ever asks for.
    std::fs::write(
        dir.join("recallledger").join(session),
        "injected\ndisposed\trecall-rejected: that pool is indexed, the failure is upstream\n",
    )
    .unwrap();

    let transcript = home.join("transcript.jsonl");
    std::fs::write(
        &transcript,
        "{\"type\":\"user\",\"message\":{\"content\":\"why?\"}}\n\
         {\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\
         \"text\":\"Upstream of the pool.\"}]}}\n",
    )
    .unwrap();
    let stop = serde_json::json!({
        "session_id": session,
        "transcript_path": transcript.to_string_lossy(),
        "stop_hook_active": false
    })
    .to_string();
    run_as("stop", &home, &url, &stop);

    assert!(
        !outcomes(&home).contains("recall-skipped"),
        "a session that answered is not a skip: {:?}",
        outcomes(&home)
    );

    let _ = std::fs::remove_dir_all(&home);
}
