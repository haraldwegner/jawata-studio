//! Sprint 28g S21 — THE DISCUSSION LANE, THROUGH THE REAL BINARY.
//!
//! The unit tests in `transcript.rs` drive the functions. The failure this project has shipped
//! three times is a function that passes its own tests while the pipeline never calls it (see
//! `edit_gate_runs_the_real_binary.rs` and `the_skip_is_seen_end_to_end.rs`, same reasoning, same
//! cure). So this spawns the built executable under the role names a client runs, with a scratch
//! `HOME`, a real socket, and a fake resident that RECORDS every request it is asked:
//!
//! * the **stop** role stores the window that just ended, and names back words he never said;
//! * the **observer** role stores the window so far, once in a gap and not on every tool call;
//! * the **primer** role, on a start after a compaction, stores the windows and asks for his last
//!   words, and on any other start asks the lane nothing.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

const HOOK: &str = env!("CARGO_BIN_EXE_jawata-hook");

/// A resident that answers by what it is asked and remembers every question.
struct Resident {
    url: String,
    asked: Arc<Mutex<Vec<Value>>>,
}

impl Resident {
    fn start(verify: Value) -> Resident {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
        let addr = listener.local_addr().unwrap();
        let asked: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&asked);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let Some(arguments) = read_arguments(&mut stream) else { continue };
                let kind = arguments.get("kind").and_then(Value::as_str).unwrap_or("").to_string();
                log.lock().unwrap().push(arguments);
                let data = match kind.as_str() {
                    "transcript_append" => json!({"appended": "stored"}),
                    "transcript_verify" => verify.clone(),
                    "recall" => json!("[transcript] no size cap, just compression · when he said on 2026-10-17"),
                    _ => json!("[domain_fact] a canned line"),
                };
                let inner = json!({"success": true, "data": data}).to_string();
                let envelope = json!({"result": {"content": [{"type": "text", "text": inner}]}}).to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    envelope.len(),
                    envelope
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        Resident { url: format!("http://{addr}/mcp"), asked }
    }

    fn kinds(&self) -> Vec<String> {
        self.asked
            .lock()
            .unwrap()
            .iter()
            .map(|a| a.get("kind").and_then(Value::as_str).unwrap_or("").to_string())
            .collect()
    }

    fn requests_of(&self, kind: &str) -> Vec<Value> {
        self.asked
            .lock()
            .unwrap()
            .iter()
            .filter(|a| a.get("kind").and_then(Value::as_str) == Some(kind))
            .cloned()
            .collect()
    }
}

/// The `arguments` of the tools/call a hook sent: headers, then a body of Content-Length bytes.
fn read_arguments(stream: &mut std::net::TcpStream) -> Option<Value> {
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    let header_end = loop {
        let n = stream.read(&mut buf).ok()?;
        if n == 0 {
            return None;
        }
        data.extend_from_slice(&buf[..n]);
        if let Some(i) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let head = String::from_utf8_lossy(&data[..header_end]).to_lowercase();
    let length: usize = head
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    while data.len() < header_end + length {
        let n = stream.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
    }
    let body: Value = serde_json::from_slice(&data[header_end..header_end + length.min(data.len() - header_end)]).ok()?;
    body.pointer("/params/arguments").cloned()
}

/// Run the binary under a deployed role name, with a scratch HOME, and return its stdout.
fn run_as(role: &str, home: &std::path::Path, url: &str, payload: &str) -> String {
    let bin = home.join("bin");
    std::fs::create_dir_all(&bin).expect("scratch bin");
    let exe = bin.join(if cfg!(windows) { format!("jawata-hook-{role}.exe") } else { format!("jawata-hook-{role}") });
    if !exe.exists() {
        std::fs::copy(HOOK, &exe).expect("copy the built binary to its role name");
    }
    std::fs::write(
        bin.join("hook_config.json"),
        json!({"url": url, "token": "t", "client": "claude-code"}).to_string(),
    )
    .expect("write the hook config");
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
    let d = std::env::temp_dir().join(format!("jawata-discussion-e2e-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn outcomes(home: &std::path::Path) -> String {
    std::fs::read_to_string(home.join(".claude").join("jawata-studio").join("outcomes.log")).unwrap_or_default()
}

/// A transcript with two windows of his and a final answer.
fn write_transcript(dir: &std::path::Path, final_text: &str) -> String {
    let lines = [
        json!({"type": "user", "uuid": "u1", "timestamp": "2026-10-17T09:00:00Z",
               "message": {"role": "user", "content": "we need the switch for the works council"}}),
        json!({"type": "assistant", "uuid": "a1", "message": {"content": [
            {"type": "text", "text": "I will add it."},
            {"type": "tool_use", "id": "t1", "name": "Read", "input": {"file_path": "/x/notes.md"}}]}}),
        json!({"type": "user", "uuid": "u2", "timestamp": "2026-10-17T09:05:00Z",
               "message": {"role": "user", "content": "no size cap, just compression"}}),
        json!({"type": "assistant", "uuid": "a2", "message": {"content": [{"type": "text", "text": final_text}]}}),
    ];
    let path = dir.join("session.jsonl");
    std::fs::write(&path, lines.iter().map(Value::to_string).collect::<Vec<_>>().join("\n")).unwrap();
    path.to_string_lossy().into_owned()
}

fn payload(path: &str, extra: Value) -> String {
    let mut v = json!({"session_id": "s-e2e", "transcript_path": path});
    if let (Some(m), Some(e)) = (v.as_object_mut(), extra.as_object()) {
        for (k, val) in e {
            m.insert(k.clone(), val.clone());
        }
    }
    v.to_string()
}

#[test]
fn the_stop_role_stores_the_window_that_just_ended() {
    let home = scratch("stop");
    let path = write_transcript(&home, "Understood.");
    let resident = Resident::start(json!({"checked": true, "found": true}));
    run_as("stop", &home, &resident.url, &payload(&path, json!({"stop_hook_active": false})));

    let appends = resident.requests_of("transcript_append");
    assert_eq!(2, appends.len(), "PROOF OF LIFE: the real stop binary stored both windows: {:?}", resident.kinds());
    assert_eq!("u2", appends[1]["key"], "keyed by the record that opened the window");
    assert_eq!("no size cap, just compression", appends[1]["his"]);
    assert_eq!("Understood.", appends[1]["agent"]);
    assert_eq!("turn", appends[1]["capture"]);
}

#[test]
fn words_he_never_said_are_named_back_by_the_stop_role_once() {
    let home = scratch("nameback");
    let path = write_transcript(&home, "He said \"we should rewrite everything in Rust tonight\" earlier.");
    let resident = Resident::start(json!({"checked": true, "found": false}));
    let first = run_as("stop", &home, &resident.url, &payload(&path, json!({"stop_hook_active": false})));
    assert!(first.contains("\"block\"") && first.contains("rewrite everything in Rust"), "named back: {first}");
    assert!(outcomes(&home).contains("transcript-attribution-unfounded"), "and logged: {}", outcomes(&home));

    let second = run_as("stop", &home, &resident.url, &payload(&path, json!({"stop_hook_active": false})));
    assert!(!second.contains("rewrite everything in Rust"), "named once, never twice: {second}");
}

#[test]
fn the_observer_role_stores_the_window_once_in_a_gap_not_on_every_tool_call() {
    let home = scratch("cadence");
    let path = write_transcript(&home, "Working.");
    let resident = Resident::start(json!({}));
    let call = payload(&path, json!({"hook_event_name": "PostToolUse", "tool_name": "Read", "tool_response": {}}));
    for _ in 0..4 {
        run_as("observer", &home, &resident.url, &call);
    }
    assert_eq!(2, resident.requests_of("transcript_append").len(),
        "four tool calls inside one gap capture once, and that capture stores the two windows: {:?}", resident.kinds());
}

#[test]
fn a_start_after_a_compaction_stores_every_window_and_brings_his_last_words_back() {
    let home = scratch("compact");
    let path = write_transcript(&home, "Understood.");
    let resident = Resident::start(json!({}));
    let out = run_as("primer", &home, &resident.url, &payload(&path, json!({"source": "compact"})));

    assert_eq!(2, resident.requests_of("transcript_append").len(), "both windows stored: {:?}", resident.kinds());
    assert!(resident.requests_of("transcript_append").iter().all(|r| r["capture"] == "compaction"));
    let recall = resident.requests_of("recall");
    assert_eq!(1, recall.len(), "and his last words asked for: {:?}", resident.kinds());
    assert_eq!(json!(["transcript"]), recall[0]["lanes"]);
    assert!(out.contains("AFTER A COMPACTION") && out.contains("no size cap, just compression"),
        "and they reach the new context: {out}");
}

#[test]
fn any_other_start_asks_the_discussion_nothing() {
    let home = scratch("startup");
    let path = write_transcript(&home, "Understood.");
    let resident = Resident::start(json!({}));
    run_as("primer", &home, &resident.url, &payload(&path, json!({"source": "startup"})));
    assert!(resident.requests_of("transcript_append").is_empty());
    assert!(resident.requests_of("recall").iter().all(|r| r["lanes"] != json!(["transcript"])),
        "the discussion is asked on a compaction only: {:?}", resident.kinds());
}
