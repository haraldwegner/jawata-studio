//! The observer's binary arm (Sprint 28b, D8) — the PostToolUse port.
//!
//! Until this module, the binary's observer was a deliberate stub (a silence
//! row, nothing else) while the SCRIPT generation captured tool outcomes and
//! the `jawata-fallback:` audit trail — which is why the observer's live
//! generation stayed the script (the 3.7.2 dogfood froze `outcomes.log` the
//! moment the entry pointed at the stub). This module ports the script's
//! jobs; the generation flips only when the deploy side cuts over.
//!
//! The script's contract, kept verbatim where it is load-bearing:
//! - one line per signal in `~/.claude/jawata-studio/outcomes.log`:
//!   `<iso-ts>\t<jawata-ver>\t<signal>\t<detail>`
//! - signals: `slip` (a declared jawata-fallback the PRE guard allowed),
//!   `read-ungrounded` (a .java Read with no prior jawata lookup this
//!   session), `verify` (a compile/diagnostics/test event)
//! - a slip is BRIDGED into the experience store as a candidate, and answers
//!   with the steering context (the one PostToolUse emission this role makes)
//! - the edit feed: a .java edit's fragments held per session; the session's
//!   next gate outcome (or an undo) labels them and posts each as
//!   `observe_edit(outcome=…)`
//! - judge the REQUEST only: `tool_response` may echo file contents that
//!   merely mention `.java` or the marker (a cat of a hook script once logged
//!   a false slip) — the response is read ONLY to label the edit feed.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::config::HookConfig;
use crate::roles::Client;
use crate::safety::{Outcome, SilenceReason};

/// The slip steering payload — byte-identical to the script's, because the
/// selftest and the real path share it and agents have learned its wording.
pub const SLIP_CONTEXT: &str = "jawata-fallback recorded. Next: verify with compile_workspace + get_diagnostics. A declared fallback is a JAWATA feature request — if a newer JAWATA version can do it, prefer JAWATA next time.";

/// Entry point from the pipeline. `home` is the user's home directory —
/// parameterized (like the edit gate) so tests drive the real path.
pub fn observe(client: Client, payload: &str, config: Option<&HookConfig>) -> Outcome {
    let Some(home) = crate::pipeline::home_dir() else {
        return Outcome::Silent(SilenceReason::NothingToObserve);
    };
    observe_in(&home, client, payload, config)
}

pub fn observe_in(
    home: &Path,
    client: Client,
    payload: &str,
    config: Option<&HookConfig>,
) -> Outcome {
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(payload) else {
        // An unreadable payload observes nothing — never a failure, this role
        // must not disturb the session it watches.
        return Outcome::Silent(SilenceReason::PayloadUnreadable("observer payload".into()));
    };
    let tool = doc["tool_name"].as_str().unwrap_or("");
    let session = doc["session_id"].as_str().unwrap_or("");
    // THE REQUEST, without the response: the slip/read judgements run on this.
    let request_only = {
        let mut d = doc.clone();
        d.as_object_mut().map(|o| o.remove("tool_response"));
        d.to_string()
    };
    let dir = home.join(".claude").join("jawata-studio");

    if tool == "Read" {
        if let Some(path) = doc["tool_input"]["file_path"].as_str() {
            if path.ends_with(".java") && !read_is_grounded(&dir, session, path) {
                emit(&dir, "read-ungrounded", path);
            }
        }
        return Outcome::Silent(SilenceReason::NothingToObserve);
    }
    if ends_with_any(tool, &["compile_workspace", "get_diagnostics", "run_tests"]) {
        emit(&dir, "verify", tool);
        editfeed_resolve(&dir, session, &doc, None, config);
        return Outcome::Silent(SilenceReason::NothingToObserve);
    }
    if ends_with_any(tool, &["find_tests"]) {
        emit(&dir, "verify", tool);
        return Outcome::Silent(SilenceReason::NothingToObserve);
    }
    if ends_with_any(tool, &["refactoring"]) {
        if doc["tool_input"]["action"].as_str().unwrap_or("").starts_with("undo") {
            editfeed_resolve(&dir, session, &doc, Some("failed"), config);
        }
        return Outcome::Silent(SilenceReason::NothingToObserve);
    }
    if matches!(tool, "Edit" | "Write" | "MultiEdit") {
        let path = doc["tool_input"]["file_path"].as_str().unwrap_or("");
        if path.ends_with(".java") {
            editfeed_hold(&dir, session, &doc);
            if request_only.to_lowercase().contains("jawata-fallback:") {
                return slip(&dir, client, tool, &request_only, config);
            }
        }
        return Outcome::Silent(SilenceReason::NothingToObserve);
    }
    if matches!(tool, "Bash" | "Grep") {
        let flat = request_only.replace("\\n", " ").replace("\\t", " ");
        let is_search = tool == "Grep"
            || ["grep", "egrep", "fgrep", "rg", "ripgrep", " ag ", " ack "]
                .iter()
                .any(|t| flat.contains(t));
        if is_search
            && flat.to_lowercase().contains(".java")
            && flat.to_lowercase().contains("jawata-fallback:")
        {
            return slip(&dir, client, tool, &request_only, config);
        }
    }
    Outcome::Silent(SilenceReason::NothingToObserve)
}

/// A slip: log it, bridge it to the store (fail-safe), answer the steering.
fn slip(dir: &Path, client: Client, tool: &str, request: &str, config: Option<&HookConfig>) -> Outcome {
    let reason = reason_after_marker(request);
    emit(dir, "slip", &format!("{tool}\t{reason}"));
    if let Some(cfg) = config {
        let summary = format!("jawata-fallback slip: {tool}: {reason}");
        let args = serde_json::json!({
            "kind": "record", "type": "failure_mode",
            "operation": "jawata-fallback-slip", "summary": summary,
            "symptoms": ["jawata fallback slip"]
        });
        let _ = crate::query::ask(
            &crate::query::Endpoint {
                url: cfg.url.clone(),
                token: cfg.token.clone(),
                timeout: std::time::Duration::from_secs(3),
            },
            args,
        );
    }
    match crate::emit::render(client, &crate::emit::context_for(
        crate::roles::Role::Observer, client, SLIP_CONTEXT.to_string())) {
        Some(rendered) => Outcome::Emitted(rendered),
        // Cursor's afterMCPExecution cannot inject: recorded, not injected —
        // a by-design quiet, never the dead-channel numerator (C2 audit F2).
        None => Outcome::Silent(SilenceReason::RecordedNotInjected),
    }
}

fn reason_after_marker(request: &str) -> String {
    let lower = request.to_lowercase();
    let Some(at) = lower.find("jawata-fallback:") else {
        return String::new();
    };
    request[at + "jawata-fallback:".len()..]
        .chars()
        .take_while(|c| *c != '"' && *c != '\\' && *c != '\n')
        .collect::<String>()
        .trim()
        .chars()
        .take(200)
        .collect()
}

fn ends_with_any(tool: &str, suffixes: &[&str]) -> bool {
    suffixes.iter().any(|s| tool.ends_with(s))
}

/// One outcomes.log line; errors swallowed — observing must never disturb.
fn emit(dir: &Path, signal: &str, detail: &str) {
    let _ = std::fs::create_dir_all(dir);
    let ts = chrono_free_iso();
    let ver = jawata_version().unwrap_or_default();
    let line = format!("{ts}\t{ver}\t{signal}\t{detail}\n");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(dir.join("outcomes.log"))
    {
        let _ = f.write_all(line.as_bytes());
    }
}

/// ISO-8601 UTC without a chrono dependency (second precision is plenty).
fn chrono_free_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Days since epoch → civil date (Howard Hinnant's algorithm).
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

fn jawata_version() -> Option<String> {
    let cache = std::env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .ok()
        .or_else(|| crate::pipeline::home_dir().map(|h| h.join(".cache")))?;
    let current = cache.join("jawata-studio").join("tools").join("jawata").join("current");
    for entry in std::fs::read_dir(current).ok()? {
        let name = entry.ok()?.file_name().to_string_lossy().to_string();
        if let Some(v) = name.strip_prefix("jawata-") {
            return Some(v.to_string());
        }
    }
    None
}

fn read_is_grounded(dir: &Path, session: &str, java_path: &str) -> bool {
    if session.is_empty() {
        return true; // no session, no state — never accuse blindly
    }
    let base = java_path
        .rsplit('/')
        .next()
        .unwrap_or(java_path)
        .trim_end_matches(".java")
        .to_lowercase();
    let Ok(state) = std::fs::read_to_string(dir.join("trygate").join(session)) else {
        return false;
    };
    state
        .lines()
        .filter(|l| l.len() >= 3)
        .any(|token| base.contains(&token.to_lowercase()) || token.to_lowercase().contains(&base))
}

// ---- the edit feed (C7): hold fragments, label on the next gate outcome ----

fn editfeed_path(dir: &Path, session: &str) -> PathBuf {
    dir.join("editfeed").join(session)
}

fn editfeed_hold(dir: &Path, session: &str, doc: &serde_json::Value) {
    if session.is_empty() {
        return;
    }
    let input = &doc["tool_input"];
    let edits: Vec<&serde_json::Value> = match input["edits"].as_array() {
        Some(list) => list.iter().collect(),
        None => vec![input],
    };
    let clip = |s: String| s.chars().take(4000).collect::<String>();
    let before = clip(edits.iter().filter_map(|e| e["old_string"].as_str()).collect::<Vec<_>>().join("\n"));
    let after = clip(edits.iter()
        .filter_map(|e| e["new_string"].as_str().or_else(|| e["content"].as_str()))
        .collect::<Vec<_>>().join("\n"));
    if before.trim().is_empty() && after.trim().is_empty() {
        return;
    }
    let path = editfeed_path(dir, session);
    let _ = std::fs::create_dir_all(path.parent().unwrap_or(dir));
    let mut lines: Vec<String> = std::fs::read_to_string(&path)
        .map(|c| c.lines().map(String::from).collect())
        .unwrap_or_default();
    lines.push(serde_json::json!({"before": before, "after": after}).to_string());
    let keep = lines.len().saturating_sub(32);
    let _ = std::fs::write(&path, lines[keep..].join("\n") + "\n");
}

fn editfeed_resolve(
    dir: &Path,
    session: &str,
    doc: &serde_json::Value,
    forced: Option<&str>,
    config: Option<&HookConfig>,
) {
    if session.is_empty() {
        return;
    }
    let path = editfeed_path(dir, session);
    let Ok(content) = std::fs::read_to_string(&path) else {
        return;
    };
    // Pop FIRST: a lost post is a lost label, never a stale re-label.
    let _ = std::fs::remove_file(&path);
    let pending: Vec<serde_json::Value> = content
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    if pending.is_empty() {
        return;
    }
    let outcome = match forced {
        Some(f) => f.to_string(),
        None => match gate_outcome(doc) {
            Some(o) => o,
            None => return, // unreadable outcome: no label beats a guessed label
        },
    };
    let Some(cfg) = config else {
        return;
    };
    for p in pending {
        let args = serde_json::json!({
            "kind": "observe_edit", "outcome": outcome,
            "before": p["before"].as_str().unwrap_or(""),
            "after": p["after"].as_str().unwrap_or("")
        });
        let _ = crate::query::ask(
            &crate::query::Endpoint {
                url: cfg.url.clone(),
                token: cfg.token.clone(),
                timeout: std::time::Duration::from_secs(3),
            },
            args,
        );
    }
}

/// The gate's verdict from the tool_response — the ONE read of the response.
fn gate_outcome(doc: &serde_json::Value) -> Option<String> {
    let text = doc["tool_response"]["content"][0]["text"].as_str()?;
    let body: serde_json::Value = serde_json::from_str(text).ok()?;
    let ok = body["success"].as_bool().unwrap_or(true);
    let errs = body["data"]["errorCount"]
        .as_i64()
        .or_else(|| body["data"]["failed"].as_i64())
        .unwrap_or(0);
    Some(if ok && errs == 0 { "clean".into() } else { "failed".into() })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The crate's own scratch pattern (see pipeline's tail tests) — no
    /// tempfile dependency: the hook's closure ratchet exists precisely so a
    /// test convenience cannot grow the world a hook links.
    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir()
            .join(format!("jawata-observer-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn dir(home: &Path) -> PathBuf {
        home.join(".claude").join("jawata-studio")
    }

    fn payload(tool: &str, input: serde_json::Value) -> String {
        serde_json::json!({
            "tool_name": tool, "session_id": "s1", "tool_input": input
        })
        .to_string()
    }

    #[test]
    fn a_declared_java_edit_slip_logs_and_answers_the_steering() {
        let home = scratch("slip");
        let p = payload("Edit", serde_json::json!({
            "file_path": "/w/src/A.java",
            "old_string": "x",
            "new_string": "y // jawata-fallback: one narrow reason"
        }));
        let out = observe_in(&home, Client::ClaudeCode, &p, None);
        match out {
            Outcome::Emitted(s) => assert!(s.contains("jawata-fallback recorded"), "{s}"),
            other => panic!("a slip answers the steering, got {other:?}"),
        }
        let log = std::fs::read_to_string(dir(&home).join("outcomes.log")).unwrap();
        assert!(log.contains("\tslip\tEdit\tone narrow reason"), "{log}");
    }

    #[test]
    fn a_response_that_merely_mentions_the_marker_is_not_a_slip() {
        let home = scratch("notslip");
        let p = serde_json::json!({
            "tool_name": "Edit", "session_id": "s1",
            "tool_input": {"file_path": "/w/src/A.java", "old_string": "x", "new_string": "y"},
            "tool_response": {"content": [{"text": "cat of a script: jawata-fallback: not a slip"}]}
        })
        .to_string();
        let out = observe_in(&home, Client::ClaudeCode, &p, None);
        assert!(matches!(out, Outcome::Silent(SilenceReason::NothingToObserve)), "{out:?}");
        assert!(!dir(&home).join("outcomes.log").exists());
    }

    #[test]
    fn a_java_search_slip_needs_tool_target_and_marker_together() {
        let home = scratch("search");
        let hit = payload("Bash", serde_json::json!({
            "command": "# jawata-fallback: why\ngrep -rn foo src/A.java"
        }));
        assert!(matches!(
            observe_in(&home, Client::ClaudeCode, &hit, None),
            Outcome::Emitted(_)
        ));
        let no_marker = payload("Bash", serde_json::json!({"command": "grep foo src/A.java"}));
        assert!(matches!(
            observe_in(&home, Client::ClaudeCode, &no_marker, None),
            Outcome::Silent(SilenceReason::NothingToObserve)
        ));
    }

    #[test]
    fn an_ungrounded_java_read_is_logged_a_grounded_one_is_not() {
        let home = scratch("read");
        let d = dir(&home);
        std::fs::create_dir_all(d.join("trygate")).unwrap();
        std::fs::write(d.join("trygate").join("s1"), "fieldrecorder\n").unwrap();
        let grounded = payload("Read", serde_json::json!({"file_path": "/w/FieldRecorder.java"}));
        observe_in(&home, Client::ClaudeCode, &grounded, None);
        let ungrounded = payload("Read", serde_json::json!({"file_path": "/w/Elsewhere.java"}));
        observe_in(&home, Client::ClaudeCode, &ungrounded, None);
        let log = std::fs::read_to_string(d.join("outcomes.log")).unwrap();
        assert!(!log.contains("FieldRecorder.java"), "{log}");
        assert!(log.contains("\tread-ungrounded\t/w/Elsewhere.java"), "{log}");
    }

    #[test]
    fn a_gate_call_emits_verify_and_labels_the_held_edits() {
        let home = scratch("gate");
        let d = dir(&home);
        // Hold one edit…
        let edit = payload("Edit", serde_json::json!({
            "file_path": "/w/A.java", "old_string": "a", "new_string": "b"
        }));
        observe_in(&home, Client::ClaudeCode, &edit, None);
        assert!(editfeed_path(&d, "s1").exists(), "the edit is held");
        // …then the gate resolves it (no config: the pop still happens — a
        // lost post is a lost label, never a stale re-label).
        let inner = serde_json::json!({"success": true, "data": {"errorCount": 0}}).to_string();
        let gate = serde_json::json!({
            "tool_name": "mcp__jawata__compile_workspace", "session_id": "s1",
            "tool_input": {},
            "tool_response": {"content": [{"text": inner}]}
        })
        .to_string();
        observe_in(&home, Client::ClaudeCode, &gate, None);
        assert!(!editfeed_path(&d, "s1").exists(), "resolve pops the feed");
        let log = std::fs::read_to_string(d.join("outcomes.log")).unwrap();
        assert!(log.contains("\tverify\tmcp__jawata__compile_workspace"), "{log}");
    }

    #[test]
    fn an_undo_forces_the_failed_label_path() {
        let home = scratch("undo");
        let d = dir(&home);
        let edit = payload("Edit", serde_json::json!({
            "file_path": "/w/A.java", "old_string": "a", "new_string": "b"
        }));
        observe_in(&home, Client::ClaudeCode, &edit, None);
        let undo = payload("mcp__jawata__refactoring", serde_json::json!({"action": "undo"}));
        observe_in(&home, Client::ClaudeCode, &undo, None);
        assert!(!editfeed_path(&d, "s1").exists(), "the undo pops the feed");
    }

    #[test]
    fn on_cursor_a_slip_is_recorded_not_injected() {
        let home = scratch("cursor");
        let p = payload("Edit", serde_json::json!({
            "file_path": "/w/src/A.java",
            "old_string": "x",
            "new_string": "y // jawata-fallback: reason"
        }));
        let out = observe_in(&home, Client::Cursor, &p, None);
        assert!(
            matches!(out, Outcome::Silent(SilenceReason::RecordedNotInjected)),
            "Cursor cannot inject PostToolUse context — quiet by design, never dead: {out:?}"
        );
        let log = std::fs::read_to_string(dir(&home).join("outcomes.log")).unwrap();
        assert!(log.contains("\tslip\t"), "the record half still happens: {log}");
    }
}
