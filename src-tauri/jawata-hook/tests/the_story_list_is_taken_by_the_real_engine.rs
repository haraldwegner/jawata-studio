//! 28h S15 (D40, R20) — THE STORY LIST BEFORE A COMPACTION, AGAINST A REAL ENGINE.
//!
//! `the_discussion_is_captured_by_the_real_hooks.rs` drives the real `jawata-hook` binary against a
//! fake resident that answers `{"appended":"stored"}` to anything. That proves the hook sends the
//! story list and consumes its file on an answer, and nothing about whether the ENGINE takes the
//! request the hook builds: the list goes in as a `transcript_append` whose `his` is empty (nobody
//! typed it) and whose `agent` carries the list. The plan said so: "the engine accepts an empty
//! `his` for the list, was read in `ExperienceTool.transcriptAppend`; no run against a real engine
//! has been made". This is that run.
//!
//! It starts the BUILT engine (the `jawata.jar` of a jawata-mcp dist) on a scratch folder with its
//! own scratch experience store (`jawata.experience.shared.dir`), so it never touches a person's
//! store, runs the real hook binary as the stop role on a `PreCompact` payload with a story list
//! waiting, and reads the list back out of the engine's transcript lane. It needs the built jar, so
//! it is ignored by default:
//!
//! `JAWATA_DIST_JAR=<jawata-mcp>/build/dist/target/dist/jawata.jar cargo test -p jawata-hook \
//!   --test the_story_list_is_taken_by_the_real_engine -- --ignored`
//!
//! Whoever starts a thing removes it: the engine is killed and the scratch folder deleted when the
//! test ends, passed or failed.

use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const HOOK: &str = env!("CARGO_BIN_EXE_jawata-hook");
const TOKEN: &str = "story-list-token";

/// The engine and its scratch folder, both gone when this is dropped.
struct Engine {
    child: Child,
    dir: std::path::PathBuf,
    port: u16,
}

impl Engine {
    fn start(jar: &str) -> Engine {
        let dir = std::env::temp_dir().join(format!("jawata-story-list-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let workspace = dir.join("ws");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("workspace.json"), r#"{"version":1,"name":"story-list","projects":[]}"#).unwrap();
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let child = Command::new("java")
            .arg("-Xmx768m")
            .arg(format!("-Djawata.experience.shared.dir={}", dir.join("store").display()))
            .arg("-Djawata.embed.disabled=true")
            .arg("-jar")
            .arg(jar)
            .arg("-data")
            .arg(&workspace)
            .arg("-port")
            .arg(port.to_string())
            .arg("-token")
            .arg(TOKEN)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the built engine with `java`");
        let engine = Engine { child, dir, port };
        let deadline = Instant::now() + Duration::from_secs(180);
        while Instant::now() < deadline {
            if engine.call_raw("tools/list", json!({})).is_some() {
                return engine;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        panic!("the engine did not answer tools/list within 180 s");
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.port)
    }

    fn call_raw(&self, method: &str, params: Value) -> Option<Value> {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        let reply = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .ok()?
            .post(self.url())
            .header("Authorization", format!("Bearer {TOKEN}"))
            .json(&body)
            .send()
            .ok()?;
        if !reply.status().is_success() {
            return None;
        }
        reply.json().ok()
    }

    /// The `experience` tool's answer to `arguments`, the data inside the engine's envelope.
    fn experience(&self, arguments: Value) -> Value {
        let reply = self
            .call_raw("tools/call", json!({"name": "experience", "arguments": arguments}))
            .expect("the engine answers an experience call");
        let text = reply["result"]["content"][0]["text"].as_str().unwrap_or("").to_string();
        serde_json::from_str(&text).unwrap_or_else(|_| panic!("the engine's answer is not JSON: {reply}"))
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A scratch folder that is removed when the test ends, passed or failed.
struct Scratch(std::path::PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The stop role of the built hook, on a scratch HOME, aimed at `url` with the engine's token.
fn run_stop_role(home: &std::path::Path, url: &str, payload: &str) -> String {
    let bin = home.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let exe = bin.join(if cfg!(windows) { "jawata-hook-stop.exe" } else { "jawata-hook-stop" });
    if !exe.exists() {
        std::fs::copy(HOOK, &exe).expect("copy the built binary to its role name");
    }
    std::fs::write(
        bin.join("hook_config.json"),
        json!({"url": url, "token": TOKEN, "client": "claude-code"}).to_string(),
    )
    .unwrap();
    let mut child = Command::new(&exe)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("run the hook");
    child.stdin.take().unwrap().write_all(payload.as_bytes()).unwrap();
    String::from_utf8_lossy(&child.wait_with_output().expect("the hook terminates").stdout).to_string()
}

#[test]
#[ignore = "needs the built dist: set JAWATA_DIST_JAR to its jawata.jar"]
fn the_engine_takes_the_story_list_exactly_as_the_hook_sends_it_and_gives_it_back() {
    let jar = std::env::var("JAWATA_DIST_JAR").expect("JAWATA_DIST_JAR names the built jawata.jar");
    let engine = Engine::start(&jar);

    let home_guard = Scratch(std::env::temp_dir().join(format!("jawata-story-list-e2e-home-{}", std::process::id())));
    let home = home_guard.0.clone();
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let transcript = home.join("session.jsonl");
    let lines = [
        json!({"type": "user", "uuid": "u1", "timestamp": "2026-11-05T09:00:00Z",
               "message": {"role": "user", "content": "the council switch is per project"}}),
        json!({"type": "assistant", "uuid": "a1", "message": {"content": [{"type": "text", "text": "Understood."}]}}),
    ];
    std::fs::write(&transcript, lines.iter().map(Value::to_string).collect::<Vec<_>>().join("\n")).unwrap();
    let stories = home.join(".claude").join("jawata-studio").join("transcript");
    std::fs::create_dir_all(&stories).unwrap();
    let story_file = stories.join("s-real.stories");
    std::fs::write(&story_file, "the zebrafish quota is per project, not per user\n").unwrap();

    let payload = json!({
        "session_id": "s-real",
        "transcript_path": transcript.to_string_lossy(),
        "hook_event_name": "PreCompact",
        "trigger": "auto",
    })
    .to_string();
    let out = run_stop_role(&home, &engine.url(), &payload);

    // The hook consumes the list only on an answer, and holds the compaction with a reason when the
    // engine refuses it: an empty stdout and a missing file are the engine having taken the request.
    assert!(out.trim().is_empty(), "the compaction was held, so the engine did not take the list: {out}");
    assert!(!story_file.exists(), "the list is consumed once the engine took it");

    // And it is in the engine's lane, as the agent's words, found by a word only the list carries.
    let found = engine.experience(json!({"kind": "recall", "lane": "transcript", "symptom": "zebrafish quota"}));
    let rendered = found.to_string();
    assert!(
        rendered.contains("zebrafish") && rendered.contains("STORY LIST"),
        "the engine gives the list back from the transcript lane: {rendered}"
    );
}
