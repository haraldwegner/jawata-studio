//! Antigravity's (`agy`) dialect, as an adapter around the SAME guard (28h z68).
//!
//! Only the guard role exists here. Its rules read Claude Code's payload shape, so a call of agy's is restated in that shape, judged by the
//! same code, and the verdict restated in agy's.
//!
//! **Measured against agy 1.5.17, 2026-11-26, and every line below rests on it.** A `PreToolUse` hook receives
//! `{"toolCall":{"id","name","args"},"conversationId","cwd","workspacePaths",…}` before the call runs. `{"decision":"deny","reason":…}`
//! refuses it, in the terminal run and in `agy acp`, also under `--dangerously-skip-permissions`'s auto approval. NOTHING on stdout leaves the
//! call to agy's own permission rules. `{}` and `{"decision":""}` REFUSE it, `"allow"` approves it past those rules, and a non-zero exit or
//! unreadable output refuses it. So this adapter prints a refusal or nothing, never anything else.
//!
//! The tools measured: `run_command{CommandLine}`, `read_file{Path}`, `write_file{Path,Content,Overwrite}`,
//! `edit_file{Path,Target,Replacement}`, `call_mcp_tool{ServerName,ToolName,Arguments}`.
//!
//! agy's transcript is not Claude Code's, so no transcript path is handed on: the rules that read one stay silent here.

use serde_json::{json, Value};

use crate::roles::{Client, Role};
use crate::safety::{Outcome, SilenceReason};

/// The client name the deploy writes into `hook_config.json`.
pub const CLIENT: &str = "antigravity";

/// The event the guard answers; `hook-events.json` holds the same fact for the deploy.
pub const EVENT: &str = "PreToolUse";

/// agy's own folder under the home: its skills, rules and artifacts, which it reads and writes as Claude Code does `.claude`.
const OWN_FOLDER: &str = ".gemini";

/// One call of agy's, restated in the shape the guard's rules read; `None` for a tool they have no rule for.
fn restate(raw: &Value) -> Option<String> {
    let call = raw.get("toolCall")?;
    let name = call.get("name")?.as_str()?;
    let args = call.get("args")?;
    let text = |key: &str| args.get(key).and_then(Value::as_str);
    let (tool, input) = match name {
        "run_command" => ("Bash".to_string(), json!({ "command": text("CommandLine")? })),
        "read_file" => ("Read".to_string(), json!({ "file_path": text("Path")? })),
        "write_file" => ("Write".to_string(), json!({ "file_path": text("Path")?, "content": text("Content").unwrap_or_default() })),
        "edit_file" => (
            "Edit".to_string(),
            json!({
                "file_path": text("Path")?,
                "old_string": text("Target").unwrap_or_default(),
                "new_string": text("Replacement").unwrap_or_default(),
            }),
        ),
        "call_mcp_tool" => (
            format!("mcp__{}__{}", text("ServerName")?, text("ToolName")?),
            args.get("Arguments").cloned().unwrap_or_else(|| json!({})),
        ),
        _ => return None,
    };
    Some(
        json!({
            "tool_name": tool,
            "tool_input": input,
            "session_id": raw.get("conversationId").and_then(Value::as_str).unwrap_or_default(),
            "cwd": raw.get("cwd").and_then(Value::as_str).unwrap_or_default(),
        })
        .to_string(),
    )
}

/// The guard's answer, in agy's shape: a refusal, or nothing at all.
fn answer(outcome: Outcome) -> Outcome {
    let Outcome::Emitted(rendered) = outcome else { return outcome };
    let verdict: Value = serde_json::from_str(&rendered).unwrap_or(Value::Null);
    let decision = &verdict["hookSpecificOutput"];
    if decision["permissionDecision"] == "deny" {
        return Outcome::Emitted(json!({ "decision": "deny", "reason": decision["permissionDecisionReason"] }).to_string());
    }
    Outcome::Silent(SilenceReason::PassedThrough)
}

/// One hook event of agy's.
pub fn run(role: Role, payload: &str) -> Outcome {
    if role != Role::Guard {
        return Outcome::Silent(SilenceReason::RoleAbsentOnClient);
    }
    let raw: Value = match serde_json::from_str(payload) {
        Ok(v) => v,
        Err(e) => return Outcome::Silent(SilenceReason::PayloadUnreadable(e.to_string())),
    };
    match restate(&raw) {
        Some(restated) => answer(crate::pipeline::guard(Client::ClaudeCode, &restated, &[OWN_FOLDER])),
        None => Outcome::Silent(SilenceReason::PassedThrough),
    }
}

/// Takes on the agent's place before the call is judged.
///
/// agy starts a hook in the folder of `hooks.json`, not in the agent's, and sets no project variable: the guard's rules resolve relative
/// paths against the working directory and take the launch project from `CLAUDE_PROJECT_DIR`. Without this the containment rule has no
/// project and refuses the person's own files. For the one-shot hook process only: it changes process-wide state.
pub fn enter(payload: &str) {
    let Ok(raw) = serde_json::from_str::<Value>(payload) else { return };
    if let Some(cwd) = raw.get("cwd").and_then(Value::as_str) {
        let _ = std::env::set_current_dir(cwd);
    }
    if let Some(project) = raw.get("workspacePaths").and_then(|w| w.get(0)).and_then(Value::as_str) {
        std::env::set_var("CLAUDE_PROJECT_DIR", project);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pass_is_never_written_as_a_decision() {
        let allowed = crate::emit::render(Client::ClaudeCode, &crate::emit::Emission::Permission { allowed: true, reason: String::new() });
        assert_eq!(Outcome::Silent(SilenceReason::PassedThrough), answer(Outcome::Emitted(allowed.unwrap())));
        // CONTROL: whatever else was rendered passes too - the adapter writes a refusal or nothing
        assert_eq!(Outcome::Silent(SilenceReason::PassedThrough), answer(Outcome::Emitted("{}".into())));
    }

    #[test]
    fn only_the_guard_exists_for_agy() {
        for role in [Role::Primer, Role::UserPrompt, Role::ToolRecall, Role::Observer, Role::Stop] {
            assert_eq!(Outcome::Silent(SilenceReason::RoleAbsentOnClient), run(role, "{}"), "{role:?}");
        }
    }

    #[test]
    fn the_measured_tools_are_restated_as_the_guard_reads_them() {
        let restated = |tool: &str, args: Value| {
            let raw = json!({ "conversationId": "c-9", "cwd": "/w", "toolCall": { "name": tool, "args": args } });
            serde_json::from_str::<Value>(&restate(&raw).expect(tool)).unwrap()
        };
        assert_eq!("ls", restated("run_command", json!({ "CommandLine": "ls" }))["tool_input"]["command"]);
        let edit = restated("edit_file", json!({ "Path": "/w/A.java", "Target": "a", "Replacement": "b" }));
        assert_eq!(("Edit", "a", "b"), (edit["tool_name"].as_str().unwrap(), "a", "b"));
        assert_eq!("b", edit["tool_input"]["new_string"]);
        assert_eq!("c-9", edit["session_id"], "the authoring window is scoped by agy's conversation");
        assert!(edit.get("transcript_path").is_none(), "agy's transcript is not the one the rules read");
        let call = restated("call_mcp_tool", json!({ "ServerName": "jawata", "ToolName": "experience", "Arguments": { "kind": "recall" } }));
        assert_eq!("mcp__jawata__experience", call["tool_name"]);
        assert_eq!("recall", call["tool_input"]["kind"]);
    }
}
