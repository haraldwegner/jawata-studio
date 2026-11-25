//! THE GUARD IN ANTIGRAVITY'S DIALECT, through the REAL binary under its deployed role name (28h z68).
//!
//! `agy`'s `PreToolUse` hook sends `{"toolCall":{"name","args"},"conversationId","cwd","workspacePaths",…}`. Measured against agy 1.5.17
//! (2026-11-26): `{"decision":"deny","reason":…}` refuses the call; NOTHING on stdout leaves it to agy's own permission rules; `{}` or an
//! empty decision REFUSES it; `"allow"` approves it past those rules; a non-zero exit refuses it. So the one thing this adapter may print is
//! a refusal. Every case below spawns the built executable.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const HOOK: &str = env!("CARGO_BIN_EXE_jawata-hook");

/// A home of the person's, NOT under the platform temp folder (which the containment rule allows as scratch).
struct Machine {
    home: PathBuf,
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

impl Machine {
    fn new(name: &str) -> Self {
        let home = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("jawata-agy-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        for dir in ["project", "private", "bin", ".gemini/config/skills/refactor"] {
            std::fs::create_dir_all(home.join(dir)).expect("scratch home");
        }
        let exe = home.join("bin").join(if cfg!(windows) { "jawata-hook-guard.exe" } else { "jawata-hook-guard" });
        std::fs::copy(HOOK, &exe).expect("copy the built binary to its role name");
        std::fs::write(home.join("bin/hook_config.json"), r#"{"url":"http://127.0.0.1:1/mcp","token":"t","client":"antigravity"}"#)
            .expect("write the hook config");
        Self { home }
    }

    /// One call, as agy puts it to the hook. The hook is started in the folder of `hooks.json`, never in the agent's own.
    fn says(&self, tool: &str, args: serde_json::Value) -> String {
        let project = self.home.join("project");
        let payload = serde_json::json!({
            "hookEventName": "PreToolUse",
            "conversationId": "c-1",
            "cwd": project,
            "workspacePaths": [project],
            "transcriptPath": self.home.join(".gemini/antigravity-cli/brain/c-1/transcript_full.jsonl"),
            "stepIdx": 2,
            "toolCall": { "id": "call_1", "name": tool, "args": args },
        });
        self.raw(&payload.to_string())
    }

    fn raw(&self, payload: &str) -> String {
        let exe = self.home.join("bin").join(if cfg!(windows) { "jawata-hook-guard.exe" } else { "jawata-hook-guard" });
        let mut attempt = 0;
        let mut child = loop {
            let mut command = Command::new(&exe);
            command.current_dir(self.home.join("bin")).env("HOME", &self.home).env("USERPROFILE", &self.home);
            for variable in ["CLAUDE_PROJECT_DIR", "JAWATA_ACCESS_FOLDERS", "JAWATA_ACCESS_FOLDERS_FILE", "JAWATA_EDITOR_FOLDERS_DIR", "JAWATA_ACCESS_FROM_ENGINE"] {
                command.env_remove(variable);
            }
            match command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn() {
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
        assert_eq!(Some(0), status.code(), "agy refuses the call of a hook that exits non-zero");
        let mut out = String::new();
        if let Some(mut o) = child.stdout.take() {
            let _ = o.read_to_string(&mut out);
        }
        out
    }

    fn path(&self, inside: &str) -> String {
        self.home.join(inside).to_string_lossy().into_owned()
    }
}

/// The reason of a refusal, which must be agy's shape and nothing else.
fn refusal(out: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("not agy's refusal ({e}): {out:?}"));
    assert_eq!("deny", v["decision"], "{out}");
    assert!(v.get("hookSpecificOutput").is_none() && v.get("permission").is_none(), "another client's dialect: {out}");
    v["reason"].as_str().expect("a reason").to_string()
}

fn passes(out: &str, what: &str) {
    assert_eq!("", out, "{what}: a passed call prints NOTHING - agy reads `{{}}` as a refusal and `allow` as an approval");
}

#[test]
fn a_java_text_search_in_agys_shell_tool_is_refused_and_a_plain_command_passes() {
    let machine = Machine::new("shell");
    let reason = refusal(&machine.says("run_command", serde_json::json!({ "CommandLine": "grep -rn OrderService src/Main.java" })));
    assert!(reason.to_lowercase().contains("jawata"), "the refusal names the way out: {reason}");
    passes(&machine.says("run_command", serde_json::json!({ "CommandLine": "ls -la" })), "a plain command");
}

#[test]
fn a_hand_edit_of_a_java_file_is_refused_and_a_new_or_other_file_passes() {
    let machine = Machine::new("edit");
    let java = machine.path("project/Main.java");
    std::fs::write(&java, "class Main { int a; }\n").unwrap();
    refusal(&machine.says("edit_file", serde_json::json!({ "Path": java, "Target": "int a;", "Replacement": "long a;" })));
    refusal(&machine.says("write_file", serde_json::json!({ "Path": java, "Content": "class Main {}\n", "Overwrite": true })));
    passes(
        &machine.says("write_file", serde_json::json!({ "Path": machine.path("project/notes.md"), "Content": "x", "Overwrite": false })),
        "a text document",
    );
}

#[test]
fn the_containment_rule_knows_agys_project_and_its_own_folder() {
    let machine = Machine::new("containment");
    passes(&machine.says("read_file", serde_json::json!({ "Path": machine.path("project/A.txt") })), "a file of the project agy was opened in");
    passes(
        &machine.says("read_file", serde_json::json!({ "Path": machine.path(".gemini/config/skills/refactor/SKILL.md") })),
        "agy's own skill, which is where the studio writes the seat commands",
    );
    let reason = refusal(&machine.says("read_file", serde_json::json!({ "Path": machine.path("private/key.txt") })));
    assert!(reason.contains("outside the declared workspace"), "CONTROL: {reason}");
}

#[test]
fn a_tool_server_call_an_unknown_tool_and_an_unreadable_payload_pass() {
    let machine = Machine::new("other");
    passes(
        &machine.says("call_mcp_tool", serde_json::json!({ "ServerName": "jawata", "ToolName": "search_symbols", "Arguments": { "query": "X" } })),
        "a tool server's call",
    );
    passes(&machine.says("ask_question", serde_json::json!({})), "a tool the guard has no rule for");
    passes(&machine.raw("{}"), "a payload with no call");
    passes(&machine.raw("not json"), "a payload that does not parse");
}

#[test]
fn the_shared_contract_names_the_event_the_adapter_answers() {
    let doc: serde_json::Value = serde_json::from_str(include_str!("../../hook-events.json")).unwrap();
    assert_eq!(jawata_hook::antigravity::EVENT, doc["antigravity"]["guard"].as_str().unwrap_or_default());
    assert!(Path::new(HOOK).exists());
}
