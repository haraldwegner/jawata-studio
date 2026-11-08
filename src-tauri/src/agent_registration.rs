//! The three registration rows of D5 (Sprint 28h, S15 slice 2): the places where a client that
//! speaks the Agent Client Protocol is told how to start jawata as its agent.
//!
//! * `jawata-chat` — the 28i chat's one setting, `jawataChat.agent`, in the VS Code user settings.
//! * `zed` — `agent_servers.<name>` in Zed's `settings.json`, with `"type": "custom"`.
//! * `intellij-acp` — `agent_servers.<name>` in `~/.jetbrains/acp.json`.
//!
//! Every row carries the same launch shape, `{command, args, env}`: `java -jar <jawata.jar> --acp`
//! with the resident's address and key in `JAWATA_URL` / `JAWATA_KEY` (the launcher reads them from
//! the environment so the key stays out of any process listing).
//!
//! The merge functions are pure text-in, text-out, so every behaviour is tested without a client.
//! Zed's and VS Code's settings files may hold comments and trailing commas, so those two rows are
//! edited as TEXT (`jsonc_edit`): only the studio's own entry is replaced or inserted and every other
//! byte stays. A file that does not read as JSON with comments is REFUSED, never rewritten: the row
//! says why (the line and column) and prints the snippet to paste. IntelliJ's `acp.json` is strict
//! JSON and keeps the strict rule: a comment there is refused.
//! An entry that is not ours is never overwritten or removed: ours is an entry whose `env` names
//! `JAWATA_URL` and whose `args` carry `--acp`.

use crate::jsonc_edit::{self, Node};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

pub const CHAT_ROW: &str = "jawata-chat";
pub const ZED_ROW: &str = "zed";
pub const INTELLIJ_ROW: &str = "intellij-acp";
/// The ids a deploy may be asked for, besides the MCP clients of the roster.
pub const ROW_IDS: [&str; 3] = [CHAT_ROW, ZED_ROW, INTELLIJ_ROW];

const CHAT_KEY: &str = "jawataChat.agent";
const SERVERS_KEY: &str = "agent_servers";

/// How a client starts jawata as its agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// The BASE address of a resident, from the address the studio keeps for it. The studio's server
/// address is the MCP endpoint (`http://127.0.0.1:<port>/mcp`); the `--acp` launcher takes the base
/// and asks for `<base>/acp`, which is a route at the listener's root, so a `/mcp` left on the end
/// would send it to `/mcp/acp`, where nothing answers. The only place the suffix is removed.
pub fn resident_base_url(server_url: &str) -> String {
    let trimmed = server_url.trim_end_matches('/');
    trimmed.strip_suffix("/mcp").unwrap_or(trimmed).trim_end_matches('/').to_string()
}

impl Launch {
    /// The launcher bridge for one resident: `java -jar <jar> --acp`, address and key in the environment.
    /// `server_url` is the studio's address for the resident (the MCP endpoint); the base is derived here.
    pub fn for_resident(jar: &str, server_url: &str, key: &str) -> Self {
        Launch {
            command: "java".into(),
            args: vec!["-jar".into(), jar.into(), "--acp".into()],
            env: vec![
                ("JAWATA_URL".into(), resident_base_url(server_url)),
                ("JAWATA_KEY".into(), key.into()),
            ],
        }
    }

    /// The same resident reached by the language server's pipe instead of the agent's: `--lsp` for `--acp`,
    /// address and key unchanged (`jawataJava.server`, Sprint 28h Stage 21 (c)).
    pub fn as_lsp(&self) -> Launch {
        Launch {
            command: self.command.clone(),
            args: self.args.iter().map(|a| if a == "--acp" { "--lsp".to_string() } else { a.clone() }).collect(),
            env: self.env.clone(),
        }
    }

    pub(crate) fn value(&self, typed: bool) -> Value {
        let mut entry = Map::new();
        if typed {
            entry.insert("type".into(), json!("custom"));
        }
        entry.insert("command".into(), json!(self.command));
        entry.insert("args".into(), json!(self.args));
        let env: Map<String, Value> =
            self.env.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
        entry.insert("env".into(), Value::Object(env));
        Value::Object(entry)
    }
}

/// The names the agents are listed under: `jawata` for one workspace, `jawata (<workspace>)` for several.
pub fn agent_names(workspaces: &[String]) -> Vec<String> {
    if workspaces.len() == 1 {
        vec!["jawata".to_string()]
    } else {
        workspaces.iter().map(|w| format!("jawata ({w})")).collect()
    }
}

fn is_ours(entry: &Value) -> bool {
    is_ours_launched_with(entry, "--acp")
}

/// An entry of ours: its `env` names `JAWATA_URL` and its `args` carry `flag` (`--acp` for an agent,
/// `--lsp` for the language server).
pub(crate) fn is_ours_launched_with(entry: &Value, flag: &str) -> bool {
    let Some(object) = entry.as_object() else { return false };
    let names_url = object
        .get("env")
        .and_then(Value::as_object)
        .is_some_and(|env| env.contains_key("JAWATA_URL"));
    let launched = object
        .get("args")
        .and_then(Value::as_array)
        .is_some_and(|args| args.iter().any(|a| a.as_str() == Some(flag)));
    names_url && launched
}

pub(crate) fn parse_object(existing: Option<&str>) -> Result<Map<String, Value>, String> {
    let text = existing.unwrap_or("");
    if text.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(object)) => Ok(object),
        Ok(_) => Err("the file is not a JSON object".into()),
        Err(error) => Err(format!(
            "the file is not strict JSON ({error}); a comment or a trailing comma would be lost by a rewrite"
        )),
    }
}

pub(crate) fn render(root: Map<String, Value>) -> String {
    let mut text = serde_json::to_string_pretty(&Value::Object(root)).unwrap_or_else(|_| "{}".into());
    text.push('\n');
    text
}

/// `agent_servers` with every entry of ours dropped and `entries` put in. An entry under one of the
/// new names that is NOT ours stops the merge: it is the user's own and stays theirs.
fn servers_merged(
    existing: Option<&str>,
    entries: &[(String, Launch)],
    typed: bool,
) -> Result<String, String> {
    let mut root = parse_object(existing)?;
    let mut servers = match root.remove(SERVERS_KEY) {
        None => Map::new(),
        Some(Value::Object(servers)) => servers,
        Some(_) => return Err(format!("`{SERVERS_KEY}` is not a JSON object")),
    };
    servers.retain(|_, value| !is_ours(value));
    for (name, launch) in entries {
        if servers.contains_key(name) {
            return Err(format!(
                "`{SERVERS_KEY}` already has an entry named {name:?} that is not jawata's own; rename it or pick another name"
            ));
        }
        servers.insert(name.clone(), launch.value(typed));
    }
    root.insert(SERVERS_KEY.into(), Value::Object(servers));
    Ok(render(root))
}

fn servers_without(existing: &str) -> Result<String, String> {
    let mut root = parse_object(Some(existing))?;
    if let Some(Value::Object(servers)) = root.get_mut(SERVERS_KEY) {
        servers.retain(|_, value| !is_ours(value));
    }
    Ok(render(root))
}

pub(crate) fn is_blank(existing: Option<&str>) -> bool {
    existing.map_or(true, |text| text.trim().is_empty())
}

fn unreadable(reason: String) -> String {
    format!("the file does not read as JSON with comments ({reason}); it is left as it is")
}

pub(crate) fn read_root(text: &str) -> Result<jsonc_edit::ObjectNode, String> {
    jsonc_edit::parse_root_object(text).map_err(|reason| {
        if reason == "the file is not a JSON object" { reason } else { unreadable(reason) }
    })
}

/// The members of `agent_servers` that are ours and are not the last member of one of `keep`.
fn stale_ours(servers: &jsonc_edit::ObjectNode, keep: &[&str]) -> Vec<usize> {
    (0..servers.members.len())
        .filter(|&index| {
            let member = &servers.members[index];
            let kept = keep.contains(&member.key.as_str()) && servers.member_index(&member.key) == Some(index);
            !kept && is_ours(&member.value.to_value())
        })
        .collect()
}

/// `agent_servers` in a file that may hold comments: the same result as `servers_merged`, reached
/// by editing the text. Entries of ours that are no longer wanted are removed, ours that are wanted
/// are synced in place (a comment inside the entry stays), the rest are inserted.
fn servers_merged_in_text(text: &str, entries: &[(String, Launch)], typed: bool) -> Result<String, String> {
    let root = read_root(text)?;
    let style = jsonc_edit::style_of(text, &root);
    let wanted: Vec<(&str, Value)> =
        entries.iter().map(|(name, launch)| (name.as_str(), launch.value(typed))).collect();
    let edits = match root.member(SERVERS_KEY) {
        None => {
            let body: Map<String, Value> =
                wanted.iter().map(|(name, value)| (name.to_string(), value.clone())).collect();
            jsonc_edit::set_member(text, &root, SERVERS_KEY, &Value::Object(body), &style)
        }
        Some(member) => {
            let Node::Object(servers) = &member.value else {
                return Err(format!("`{SERVERS_KEY}` is not a JSON object"));
            };
            for (name, _) in &wanted {
                if let Some(theirs) = servers.member(name) {
                    if !is_ours(&theirs.value.to_value()) {
                        return Err(format!(
                            "`{SERVERS_KEY}` already has an entry named {name:?} that is not jawata's own; rename it or pick another name"
                        ));
                    }
                }
            }
            let names: Vec<&str> = wanted.iter().map(|(name, _)| *name).collect();
            let mut stale = stale_ours(servers, &names);
            let mut edits = Vec::new();
            let mut missing: Vec<(&str, Value)> = Vec::new();
            for (name, value) in &wanted {
                match servers.member(name) {
                    Some(have) => edits.extend(jsonc_edit::sync_value(
                        text,
                        &have.value,
                        value,
                        &style,
                        jsonc_edit::line_indent(text, have.key_span.start),
                    )),
                    // A stale entry of ours is given the new name rather than removed and inserted again.
                    None if !stale.is_empty() => {
                        let recycled = &servers.members[stale.remove(0)];
                        edits.extend(jsonc_edit::recycle_member(text, recycled, name, value, &style));
                    }
                    None => missing.push((name, value.clone())),
                }
            }
            edits.extend(jsonc_edit::remove_members(text, servers, &stale));
            edits.extend(jsonc_edit::insert_members(text, servers, &missing, &style));
            edits
        }
    };
    jsonc_edit::apply(text, edits)
}

fn servers_without_in_text(text: &str) -> Result<String, String> {
    let root = read_root(text)?;
    let edits = match root.member(SERVERS_KEY).map(|member| &member.value) {
        Some(Node::Object(servers)) => jsonc_edit::remove_members(text, servers, &stale_ours(servers, &[])),
        _ => Vec::new(),
    };
    jsonc_edit::apply(text, edits)
}

/// Zed: `agent_servers.<name> = {"type":"custom", command, args, env}`. Comments and trailing
/// commas in the file are kept.
pub fn zed_merged(existing: Option<&str>, entries: &[(String, Launch)]) -> Result<String, String> {
    match existing {
        Some(text) if !is_blank(existing) => servers_merged_in_text(text, entries, true),
        _ => servers_merged(existing, entries, true),
    }
}

pub fn zed_without(existing: &str) -> Result<String, String> {
    if is_blank(Some(existing)) {
        return servers_without(existing);
    }
    servers_without_in_text(existing)
}

/// IntelliJ's `acp.json`: `agent_servers.<name> = {command, args, env}`; other keys (such as
/// `default_mcp_settings`) are kept as they were. Strict JSON: a comment is refused.
pub fn intellij_merged(existing: Option<&str>, entries: &[(String, Launch)]) -> Result<String, String> {
    servers_merged(existing, entries, false)
}

pub fn intellij_without(existing: &str) -> Result<String, String> {
    servers_without(existing)
}

/// The chat takes ONE agent: `jawataChat.agent = {command, args, env}`. Comments and trailing
/// commas in the VS Code settings file are kept.
pub fn chat_merged(existing: Option<&str>, launch: &Launch) -> Result<String, String> {
    let Some(text) = existing.filter(|_| !is_blank(existing)) else {
        let mut root = parse_object(existing)?;
        root.insert(CHAT_KEY.into(), launch.value(false));
        return Ok(render(root));
    };
    let root = read_root(text)?;
    if let Some(current) = root.member(CHAT_KEY) {
        if !is_ours(&current.value.to_value()) {
            return Err(format!(
                "`{CHAT_KEY}` is already set to an agent that is not jawata's own; it is left as it is"
            ));
        }
    }
    let style = jsonc_edit::style_of(text, &root);
    jsonc_edit::apply(text, jsonc_edit::set_member(text, &root, CHAT_KEY, &launch.value(false), &style))
}

pub fn chat_without(existing: &str) -> Result<String, String> {
    if is_blank(Some(existing)) {
        return Ok(render(parse_object(Some(existing))?));
    }
    let root = read_root(existing)?;
    let edits = match root.member_index(CHAT_KEY) {
        Some(index) if is_ours(&root.members[index].value.to_value()) => {
            jsonc_edit::remove_members(existing, &root, &[index])
        }
        _ => Vec::new(),
    };
    jsonc_edit::apply(existing, edits)
}

/// One row's place on this machine: the file it writes and the folder whose presence says the
/// client is installed.
#[derive(Debug, Clone)]
pub struct RowTarget {
    pub id: &'static str,
    pub path: PathBuf,
    pub client_folder: PathBuf,
}

/// The three rows for this machine. `windows` selects Zed's `%APPDATA%\Zed` over `~/.config/zed`;
/// `config_dir` is the OS config directory (`dirs::config_dir()`).
pub fn rows(home: &Path, config_dir: &Path, windows: bool) -> Vec<RowTarget> {
    let zed_folder = if windows { config_dir.join("Zed") } else { home.join(".config").join("zed") };
    let code_user = config_dir.join("Code").join("User");
    let jetbrains = home.join(".jetbrains");
    vec![
        RowTarget {
            id: CHAT_ROW,
            path: code_user.join("settings.json"),
            client_folder: code_user,
        },
        RowTarget { id: ZED_ROW, path: zed_folder.join("settings.json"), client_folder: zed_folder },
        RowTarget {
            id: INTELLIJ_ROW,
            path: jetbrains.join("acp.json"),
            client_folder: jetbrains,
        },
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowAction {
    Write,
    Preview,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowState {
    Success,
    Skipped,
    Failed,
}

#[derive(Debug, Clone)]
pub struct RowOutcome {
    pub client: String,
    pub target_path: String,
    pub state: RowState,
    pub message: String,
    pub backup_path: Option<String>,
    pub changed_sections: Vec<String>,
    pub validation_errors: Vec<String>,
    pub preview_content: Option<String>,
}

impl RowOutcome {
    fn new(target: &RowTarget, state: RowState, message: impl Into<String>) -> Self {
        RowOutcome {
            client: target.id.to_string(),
            target_path: target.path.to_string_lossy().into_owned(),
            state,
            message: message.into(),
            backup_path: None,
            changed_sections: Vec::new(),
            validation_errors: Vec::new(),
            preview_content: None,
        }
    }
}

/// The snippet a person pastes by hand when the file cannot be rewritten.
fn snippet_for(id: &str, entries: &[(String, Launch)]) -> String {
    match id {
        CHAT_ROW => entries
            .first()
            .map(|(_, launch)| format!("\"{CHAT_KEY}\": {}", launch.value(false)))
            .unwrap_or_default(),
        ZED_ROW | INTELLIJ_ROW => {
            let typed = id == ZED_ROW;
            let body: Map<String, Value> =
                entries.iter().map(|(name, launch)| (name.clone(), launch.value(typed))).collect();
            format!("\"{SERVERS_KEY}\": {}", Value::Object(body))
        }
        _ => String::new(),
    }
}

/// Applies one row. `entries` is what to register (an `Err` is the reason there is no engine to
/// launch); `requested` is true when the row was named in the deploy's target list, which creates a
/// file for a client that is not found; `backup` copies a file into the managed backup area.
pub fn apply_row(
    target: &RowTarget,
    entries: &Result<Vec<(String, Launch)>, String>,
    action: RowAction,
    requested: bool,
    backup: &dyn Fn(&Path) -> Result<Option<PathBuf>, String>,
) -> RowOutcome {
    let existing = std::fs::read_to_string(&target.path).ok();
    if action == RowAction::Delete {
        let Some(text) = existing else {
            return RowOutcome::new(target, RowState::Skipped, "Skipped: nothing to remove, the file does not exist.");
        };
        let removed = match target.id {
            CHAT_ROW => chat_without(&text),
            ZED_ROW => zed_without(&text),
            _ => intellij_without(&text),
        };
        return finish(target, action, Some(text), removed, backup, "removed jawata's entry");
    }

    if !target.client_folder.exists() && !requested {
        return RowOutcome::new(
            target,
            RowState::Skipped,
            format!(
                "Skipped: the client was not found ({} does not exist); name this row in the deploy targets to create it.",
                target.client_folder.display()
            ),
        );
    }
    let entries = match entries {
        Ok(entries) if !entries.is_empty() => entries,
        Ok(_) => {
            return RowOutcome::new(target, RowState::Skipped, "Skipped: no workspace has an engine to start.");
        }
        Err(reason) => {
            return RowOutcome::new(target, RowState::Skipped, format!("Skipped: no engine to start ({reason})."));
        }
    };
    let merged = match target.id {
        CHAT_ROW => chat_merged(existing.as_deref(), &entries[0].1),
        ZED_ROW => zed_merged(existing.as_deref(), entries),
        _ => intellij_merged(existing.as_deref(), entries),
    };
    let mut outcome =
        finish(target, action, existing, merged, backup, "registered jawata as an agent");
    if outcome.state == RowState::Failed {
        outcome.message =
            format!("{} Paste this by hand: {}", outcome.message, snippet_for(target.id, entries));
    } else if target.id == CHAT_ROW && entries.len() > 1 && outcome.state == RowState::Success {
        outcome.message = format!(
            "{} (the chat takes one agent: {} of {} workspaces)",
            outcome.message,
            entries[0].0,
            entries.len()
        );
    }
    outcome
}

pub(crate) fn finish(
    target: &RowTarget,
    action: RowAction,
    existing: Option<String>,
    next: Result<String, String>,
    backup: &dyn Fn(&Path) -> Result<Option<PathBuf>, String>,
    done: &str,
) -> RowOutcome {
    let next = match next {
        Ok(text) => text,
        Err(reason) => {
            let mut outcome = RowOutcome::new(target, RowState::Failed, format!("Not written: {reason}."));
            outcome.validation_errors.push(reason);
            return outcome;
        }
    };
    // Same JSON means nothing to write; the file stays byte for byte as it is.
    let same = existing.as_deref().is_some_and(|text| text == next || jsonc_edit::same_json(text, &next));
    if same {
        return RowOutcome::new(target, RowState::Success, "Already up to date; nothing written.");
    }
    if action == RowAction::Preview {
        let mut outcome = RowOutcome::new(target, RowState::Success, "Preview: would write this file.");
        outcome.preview_content = Some(next);
        return outcome;
    }
    let mut backup_path = None;
    if existing.is_some() {
        match backup(&target.path) {
            Ok(path) => backup_path = path.map(|p| p.to_string_lossy().into_owned()),
            Err(error) => {
                return RowOutcome::new(target, RowState::Failed, format!("Not written: backup failed ({error})."));
            }
        }
    }
    if let Some(parent) = target.path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            return RowOutcome::new(target, RowState::Failed, format!("Not written: {error}."));
        }
    }
    if let Err(error) = std::fs::write(&target.path, next) {
        return RowOutcome::new(target, RowState::Failed, format!("Not written: {error}."));
    }
    let mut outcome = RowOutcome::new(target, RowState::Success, format!("Wrote the file: {done}."));
    outcome.backup_path = backup_path;
    outcome.changed_sections.push(
        match target.id {
            CHAT_ROW => CHAT_KEY.to_string(),
            crate::vscode_deploy::SETTINGS_ROW => crate::vscode_deploy::EDITOR_KEYS.join(", "),
            _ => SERVERS_KEY.to_string(),
        },
    );
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn launch(url: &str) -> Launch {
        Launch::for_resident("/e/jawata.jar", url, "k-1")
    }

    fn one(name: &str, url: &str) -> Vec<(String, Launch)> {
        vec![(name.to_string(), launch(url))]
    }

    struct Scratch(PathBuf);
    impl Scratch {
        fn new(label: &str) -> Self {
            static N: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "jawata-studio-regrow-{label}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Scratch(dir)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn no_backup(_: &Path) -> Result<Option<PathBuf>, String> {
        Ok(None)
    }

    #[test]
    fn the_launch_shape_names_the_launcher_and_keeps_the_key_out_of_the_arguments() {
        let value = launch("http://127.0.0.1:7001").value(false);
        assert_eq!(value["command"], "java");
        assert_eq!(value["args"], json!(["-jar", "/e/jawata.jar", "--acp"]));
        assert_eq!(value["env"]["JAWATA_URL"], "http://127.0.0.1:7001");
        assert_eq!(value["env"]["JAWATA_KEY"], "k-1");
        assert!(!value["args"].to_string().contains("k-1"), "the key must not ride on the command line");
    }

    #[test]
    fn the_launcher_is_given_the_base_address_never_the_mcp_endpoint() {
        for given in ["http://127.0.0.1:7001/mcp", "http://127.0.0.1:7001/mcp/", "http://127.0.0.1:7001"] {
            let value = Launch::for_resident("/e/jawata.jar", given, "k-1").value(false);
            assert_eq!(value["env"]["JAWATA_URL"], "http://127.0.0.1:7001", "from {given}");
        }
        assert_eq!(resident_base_url("http://h:1/mcpx/mcp"), "http://h:1/mcpx");
    }

    #[test]
    fn zed_gets_a_custom_agent_entry_and_keeps_what_was_there() {
        let existing = r#"{"theme":"One Dark","agent_servers":{"other":{"type":"custom","command":"x"}}}"#;
        let text = zed_merged(Some(existing), &one("jawata", "http://u")).unwrap();
        let root: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(root["theme"], "One Dark");
        assert_eq!(root["agent_servers"]["other"]["command"], "x");
        assert_eq!(root["agent_servers"]["jawata"]["type"], "custom");
        assert_eq!(root["agent_servers"]["jawata"]["args"][2], "--acp");
    }

    #[test]
    fn intellij_gets_an_entry_without_a_type_and_keeps_its_default_mcp_settings() {
        let existing = r#"{"default_mcp_settings":{"a":1},"agent_servers":{}}"#;
        let text = intellij_merged(Some(existing), &one("jawata", "http://u")).unwrap();
        let root: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(root["default_mcp_settings"]["a"], 1);
        assert!(root["agent_servers"]["jawata"].get("type").is_none());
        assert_eq!(root["agent_servers"]["jawata"]["env"]["JAWATA_URL"], "http://u");
    }

    #[test]
    fn the_chat_gets_its_one_agent_setting() {
        let text = chat_merged(Some(r#"{"editor.fontSize":14}"#), &launch("http://u")).unwrap();
        let root: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(root["editor.fontSize"], 14);
        assert_eq!(root["jawataChat.agent"]["command"], "java");
    }

    #[test]
    fn a_missing_or_empty_file_is_started_from_nothing() {
        assert!(zed_merged(None, &one("jawata", "u")).unwrap().contains("agent_servers"));
        assert!(intellij_merged(Some("  \n"), &one("jawata", "u")).unwrap().contains("agent_servers"));
        assert!(chat_merged(None, &launch("u")).unwrap().contains("jawataChat.agent"));
    }

    #[test]
    fn intellijs_strict_json_file_with_a_comment_is_still_refused_not_rewritten() {
        let jsonc = "{\n  // my theme\n  \"theme\": \"One Dark\"\n}";
        let error = intellij_merged(Some(jsonc), &one("jawata", "u")).expect_err("a commented file must be refused");
        assert!(error.contains("not strict JSON"), "{error}");
        assert!(intellij_without(jsonc).is_err());
    }

    #[test]
    fn an_entry_that_is_not_ours_is_never_overwritten() {
        let theirs = r#"{"agent_servers":{"jawata":{"command":"mine","args":[],"env":{}}}}"#;
        let error = zed_merged(Some(theirs), &one("jawata", "u")).expect_err("their own entry stays theirs");
        assert!(error.contains("not jawata's own"), "{error}");
        let chat_theirs = r#"{"jawataChat.agent":{"command":"mine"}}"#;
        assert!(chat_merged(Some(chat_theirs), &launch("u")).is_err());
    }

    #[test]
    fn a_second_deploy_replaces_our_entry_and_drops_one_of_a_removed_workspace() {
        let first = zed_merged(None, &vec![
            ("jawata (a)".to_string(), launch("http://a")),
            ("jawata (b)".to_string(), launch("http://b")),
        ])
        .unwrap();
        let second = zed_merged(Some(&first), &one("jawata (a)", "http://a2")).unwrap();
        let root: Value = serde_json::from_str(&second).unwrap();
        let servers = root["agent_servers"].as_object().unwrap();
        assert_eq!(servers.len(), 1, "the entry of the removed workspace goes: {servers:?}");
        assert_eq!(servers["jawata (a)"]["env"]["JAWATA_URL"], "http://a2");
    }

    #[test]
    fn delete_removes_only_our_entries() {
        let existing = r#"{"agent_servers":{"keep":{"command":"x"},"jawata":{"command":"java","args":["-jar","j","--acp"],"env":{"JAWATA_URL":"u","JAWATA_KEY":"k"}}},"theme":"t"}"#;
        let root: Value = serde_json::from_str(&zed_without(existing).unwrap()).unwrap();
        assert!(root["agent_servers"].get("jawata").is_none());
        assert_eq!(root["agent_servers"]["keep"]["command"], "x");
        assert_eq!(root["theme"], "t");
        let chat = r#"{"jawataChat.agent":{"command":"java","args":["--acp"],"env":{"JAWATA_URL":"u"}},"a":1}"#;
        let root: Value = serde_json::from_str(&chat_without(chat).unwrap()).unwrap();
        assert!(root.get("jawataChat.agent").is_none());
        assert_eq!(root["a"], 1);
        let theirs = r#"{"jawataChat.agent":{"command":"mine"}}"#;
        let root: Value = serde_json::from_str(&chat_without(theirs).unwrap()).unwrap();
        assert_eq!(root["jawataChat.agent"]["command"], "mine", "an agent that is not ours stays");
    }

    #[test]
    fn names_are_plain_for_one_workspace_and_carry_the_workspace_for_several() {
        assert_eq!(agent_names(&["a".to_string()]), vec!["jawata"]);
        assert_eq!(agent_names(&["a".to_string(), "b".to_string()]), vec!["jawata (a)", "jawata (b)"]);
    }

    #[test]
    fn the_rows_sit_where_each_client_reads_them() {
        let unix = rows(Path::new("/h"), Path::new("/h/.config"), false);
        let paths: Vec<(&str, String)> =
            unix.iter().map(|r| (r.id, r.path.to_string_lossy().into_owned())).collect();
        assert_eq!(paths[0], (CHAT_ROW, "/h/.config/Code/User/settings.json".to_string()));
        assert_eq!(paths[1], (ZED_ROW, "/h/.config/zed/settings.json".to_string()));
        assert_eq!(paths[2], (INTELLIJ_ROW, "/h/.jetbrains/acp.json".to_string()));
        let win = rows(Path::new("/h"), Path::new("/appdata"), true);
        assert_eq!(win[1].path, PathBuf::from("/appdata/Zed/settings.json"));
    }

    #[test]
    fn a_client_that_is_not_installed_is_skipped_unless_the_row_is_requested() {
        let scratch = Scratch::new("absent");
        let targets = rows(&scratch.0, &scratch.0.join("cfg"), false);
        let zed = &targets[1];
        let entries = Ok(one("jawata", "u"));
        let skipped = apply_row(zed, &entries, RowAction::Write, false, &no_backup);
        assert_eq!(skipped.state, RowState::Skipped);
        assert!(!zed.path.exists(), "nothing is created for a client that is not there");
        let created = apply_row(zed, &entries, RowAction::Write, true, &no_backup);
        assert_eq!(created.state, RowState::Success, "{}", created.message);
        assert!(zed.path.exists());
    }

    #[test]
    fn a_write_backs_up_the_old_file_and_a_second_identical_write_changes_nothing() {
        let scratch = Scratch::new("write");
        let targets = rows(&scratch.0, &scratch.0.join("cfg"), false);
        let intellij = &targets[2];
        std::fs::create_dir_all(&intellij.client_folder).unwrap();
        std::fs::write(&intellij.path, r#"{"default_mcp_settings":{}}"#).unwrap();
        let backups = std::cell::RefCell::new(Vec::<PathBuf>::new());
        let backup = |p: &Path| -> Result<Option<PathBuf>, String> {
            backups.borrow_mut().push(p.to_path_buf());
            Ok(Some(p.with_extension("bak-test")))
        };
        let entries = Ok(one("jawata", "u"));
        let first = apply_row(intellij, &entries, RowAction::Write, false, &backup);
        assert_eq!(first.state, RowState::Success, "{}", first.message);
        assert_eq!(backups.borrow().len(), 1, "the old file was backed up before it was replaced");
        assert!(first.backup_path.is_some());
        let second = apply_row(intellij, &entries, RowAction::Write, false, &backup);
        assert_eq!(second.state, RowState::Success);
        assert_eq!(backups.borrow().len(), 1, "an unchanged file is neither backed up nor rewritten");
        assert!(second.message.contains("Already up to date"));
    }

    #[test]
    fn a_file_that_does_not_read_fails_the_row_with_the_reason_and_the_snippet_and_stays_untouched() {
        let scratch = Scratch::new("broken");
        let targets = rows(&scratch.0, &scratch.0.join("cfg"), false);
        let zed = &targets[1];
        std::fs::create_dir_all(&zed.client_folder).unwrap();
        let original = "{\n // keep me\n \"theme\": \n}";
        std::fs::write(&zed.path, original).unwrap();
        let outcome = apply_row(zed, &Ok(one("jawata", "u")), RowAction::Write, false, &no_backup);
        assert_eq!(outcome.state, RowState::Failed);
        assert!(outcome.message.contains("does not read as JSON with comments"), "{}", outcome.message);
        assert!(outcome.message.contains("line 4 column 1"), "the place is named: {}", outcome.message);
        assert!(outcome.message.contains("Paste this by hand"), "{}", outcome.message);
        assert!(outcome.message.contains("\"agent_servers\""));
        assert_eq!(std::fs::read_to_string(&zed.path).unwrap(), original);
    }

    #[test]
    fn preview_writes_nothing_and_delete_removes_our_entry_with_a_backup() {
        let scratch = Scratch::new("modes");
        let targets = rows(&scratch.0, &scratch.0.join("cfg"), false);
        let zed = &targets[1];
        std::fs::create_dir_all(&zed.client_folder).unwrap();
        let entries = Ok(one("jawata", "u"));
        let preview = apply_row(zed, &entries, RowAction::Preview, false, &no_backup);
        assert!(preview.preview_content.is_some());
        assert!(!zed.path.exists(), "a preview writes no file");
        apply_row(zed, &entries, RowAction::Write, false, &no_backup);
        let removed = apply_row(zed, &entries, RowAction::Delete, false, &no_backup);
        assert_eq!(removed.state, RowState::Success, "{}", removed.message);
        let root: Value = serde_json::from_str(&std::fs::read_to_string(&zed.path).unwrap()).unwrap();
        assert!(root["agent_servers"].as_object().unwrap().is_empty());
        let gone = apply_row(&targets[0], &entries, RowAction::Delete, false, &no_backup);
        assert_eq!(gone.state, RowState::Skipped, "a file that does not exist has nothing to remove");
    }

    #[test]
    fn without_an_engine_the_row_is_skipped_and_says_why() {
        let scratch = Scratch::new("noengine");
        let targets = rows(&scratch.0, &scratch.0.join("cfg"), false);
        std::fs::create_dir_all(&targets[1].client_folder).unwrap();
        let outcome = apply_row(
            &targets[1],
            &Err("No managed JAWATA runtime is installed".to_string()),
            RowAction::Write,
            false,
            &no_backup,
        );
        assert_eq!(outcome.state, RowState::Skipped);
        assert!(outcome.message.contains("No managed JAWATA runtime"), "{}", outcome.message);
    }

    #[test]
    fn the_chat_takes_the_first_workspace_and_says_that_it_took_one() {
        let scratch = Scratch::new("chat");
        let targets = rows(&scratch.0, &scratch.0.join("cfg"), false);
        std::fs::create_dir_all(&targets[0].client_folder).unwrap();
        let entries = Ok(vec![
            ("jawata (a)".to_string(), launch("http://a")),
            ("jawata (b)".to_string(), launch("http://b")),
        ]);
        let outcome = apply_row(&targets[0], &entries, RowAction::Write, false, &no_backup);
        assert_eq!(outcome.state, RowState::Success, "{}", outcome.message);
        assert!(outcome.message.contains("takes one agent"), "{}", outcome.message);
        let root: Value =
            serde_json::from_str(&std::fs::read_to_string(&targets[0].path).unwrap()).unwrap();
        assert_eq!(root["jawataChat.agent"]["env"]["JAWATA_URL"], "http://a");
    }

    const ZED_COMMENTED: &str = r#"// my zed settings
{
  /* theme block */
  "theme": "One Dark", // keep
  "agent_servers": {
    // the other agent
    "other": { "type": "custom", "command": "x" },
    // jawata, registered by the studio
    "jawata": {
      "type": "custom", // typed
      "command": "java",
      "args": ["-jar", "/old/jawata.jar", "--acp"],
      /* the address */
      "env": {
        "JAWATA_URL": "http://old", // old address
        "JAWATA_KEY": "k-1"
      }
    }
  },
  "tail": 1 // after
}
// the end
"#;

    fn value_of(text: &str) -> Value {
        crate::jsonc_edit::parse(text).unwrap_or_else(|e| panic!("{e}\n{text}")).to_value()
    }

    const OURS_A: &str = r#"{"type":"custom","command":"java","args":["-jar","/e/jawata.jar","--acp"],"env":{"JAWATA_URL":"http://a","JAWATA_KEY":"k-1"}}"#;
    const OURS_B: &str = r#"{"type":"custom","command":"java","args":["-jar","/e/jawata.jar","--acp"],"env":{"JAWATA_URL":"http://b","JAWATA_KEY":"k-1"}}"#;

    #[test]
    fn comments_before_inside_and_after_the_entry_survive_byte_for_byte() {
        let merged = zed_merged(Some(ZED_COMMENTED), &one("jawata", "http://new")).unwrap();
        let expected = ZED_COMMENTED
            .replace("/old/jawata.jar", "/e/jawata.jar")
            .replace("\"http://old\"", "\"http://new\"");
        assert_eq!(merged, expected, "only the two values that differ are touched");
    }

    #[test]
    fn a_string_with_slashes_is_not_a_comment_and_a_comment_is_not_structure() {
        let text = "{\n  // \"agent_servers\": { \"jawata\": { \"command\": \"fake\" } }\n  \"url\": \"http://x//y /* not a comment */\", /* } , \" { */\n  \"//key\": \"v\"\n}\n";
        let merged = zed_merged(Some(text), &one("jawata", "http://u")).unwrap();
        let head = text.strip_suffix("\n}\n").unwrap();
        assert!(
            merged.starts_with(&format!("{head},\n  \"agent_servers\": {{\n")),
            "the commented-out entry is not the entry, so the real one is inserted after the last member: {merged}"
        );
        let root = value_of(&merged);
        assert_eq!(root["url"], "http://x//y /* not a comment */");
        assert_eq!(root["//key"], "v");
        assert_eq!(root["agent_servers"]["jawata"]["command"], "java");
    }

    #[test]
    fn a_trailing_comma_is_kept_as_the_style_of_the_file() {
        let text = "{\n  \"a\": 1,\n  \"agent_servers\": {\n    \"other\": { \"command\": \"x\", }, // theirs\n  },\n}\n";
        let merged = zed_merged(Some(text), &one("jawata", "http://u")).unwrap();
        assert!(
            merged.starts_with("{\n  \"a\": 1,\n  \"agent_servers\": {\n    \"other\": { \"command\": \"x\", }, // theirs\n    \"jawata\": {\n"),
            "{merged}"
        );
        assert!(merged.ends_with("\n    },\n  },\n}\n"), "{merged}");
        let root = value_of(&merged);
        assert_eq!(root["agent_servers"]["other"]["command"], "x");
        assert_eq!(root["agent_servers"]["jawata"]["type"], "custom");
    }

    #[test]
    fn an_absent_entry_is_inserted_after_the_last_member_and_its_comment() {
        let text = "{\n  \"a\": 1 // note\n}\n";
        let merged = zed_merged(Some(text), &one("jawata", "http://u")).unwrap();
        assert!(merged.starts_with("{\n  \"a\": 1, // note\n  \"agent_servers\": {\n    \"jawata\": {\n"), "{merged}");
        assert_eq!(value_of(&merged)["a"], 1);
        assert_eq!(value_of(&merged)["agent_servers"]["jawata"]["env"]["JAWATA_URL"], "http://u");

        let empty = zed_merged(Some("{}"), &one("jawata", "http://u")).unwrap();
        assert!(empty.starts_with("{\n  \"agent_servers\": {"), "{empty}");
        assert_eq!(value_of(&empty)["agent_servers"]["jawata"]["command"], "java");

        let one_line = zed_merged(Some("{\"a\":1}"), &one("jawata", "http://u")).unwrap();
        assert!(one_line.starts_with("{\"a\":1, \"agent_servers\": {\"jawata\":{"), "a one-line file stays one line: {one_line}");
        assert_eq!(value_of(&one_line)["a"], 1);

        let only_a_comment = "{\n  // nothing yet\n}\n";
        let merged = zed_merged(Some(only_a_comment), &one("jawata", "http://u")).unwrap();
        assert!(merged.contains("// nothing yet"), "{merged}");
        assert_eq!(value_of(&merged)["agent_servers"]["jawata"]["command"], "java");

        let chat = chat_merged(Some("{\n  \"a\": 1,\n}\n"), &launch("http://u")).unwrap();
        assert!(chat.starts_with("{\n  \"a\": 1,\n  \"jawataChat.agent\": {\n"), "{chat}");
    }

    #[test]
    fn a_file_that_does_not_read_keeps_the_refusal_with_its_reason() {
        for broken in [
            "{ \"a\": }",
            "{ \"a\": \"x",
            "{ /* never closed",
            "{} extra",
            "{ \"a\" 1 }",
            "{\"a\":1,,}",
            "{ \"a\": 1 \"b\": 2 }",
        ] {
            for refused in [
                zed_merged(Some(broken), &one("jawata", "u")),
                chat_merged(Some(broken), &launch("u")),
                zed_without(broken),
                chat_without(broken),
            ] {
                let error = refused.expect_err(broken);
                assert!(error.contains("does not read as JSON with comments"), "{broken}: {error}");
                assert!(error.contains("line 1 column"), "{broken}: {error}");
            }
        }
        let error = zed_merged(Some("[1]"), &one("jawata", "u")).expect_err("a list is not a settings object");
        assert!(error.contains("not a JSON object"), "{error}");
    }

    #[test]
    fn running_it_twice_changes_nothing_and_the_row_says_so() {
        let first = zed_merged(Some(ZED_COMMENTED), &one("jawata", "http://new")).unwrap();
        assert_eq!(zed_merged(Some(&first), &one("jawata", "http://new")).unwrap(), first);
        let absent = zed_merged(Some("// c\n{\n  \"a\": 1\n}\n"), &one("jawata", "http://u")).unwrap();
        assert_eq!(zed_merged(Some(&absent), &one("jawata", "http://u")).unwrap(), absent);
        let chat = chat_merged(Some("// c\n{\n  \"a\": 1,\n}\n"), &launch("http://u")).unwrap();
        assert_eq!(chat_merged(Some(&chat), &launch("http://u")).unwrap(), chat);

        let scratch = Scratch::new("twice");
        let targets = rows(&scratch.0, &scratch.0.join("cfg"), false);
        let zed = &targets[1];
        std::fs::create_dir_all(&zed.client_folder).unwrap();
        std::fs::write(&zed.path, ZED_COMMENTED).unwrap();
        let backups = std::cell::RefCell::new(0usize);
        let backup = |p: &Path| -> Result<Option<PathBuf>, String> {
            *backups.borrow_mut() += 1;
            Ok(Some(p.with_extension("bak-test")))
        };
        let entries = Ok(one("jawata", "http://new"));
        let written = apply_row(zed, &entries, RowAction::Write, false, &backup);
        assert_eq!(written.state, RowState::Success, "{}", written.message);
        assert_eq!(*backups.borrow(), 1, "the commented file is backed up before it is edited");
        let on_disk = std::fs::read_to_string(&zed.path).unwrap();
        assert!(on_disk.contains("// jawata, registered by the studio"), "{on_disk}");
        let again = apply_row(zed, &entries, RowAction::Write, false, &backup);
        assert!(again.message.contains("Already up to date"), "{}", again.message);
        assert_eq!(*backups.borrow(), 1, "an unchanged file is neither backed up nor rewritten");
        assert_eq!(std::fs::read_to_string(&zed.path).unwrap(), on_disk);
    }

    #[test]
    fn an_entry_that_is_not_ours_is_refused_in_a_commented_file_too() {
        let theirs = "{\n  // mine\n  \"agent_servers\": { \"jawata\": { \"command\": \"mine\" } } // keep\n}\n";
        let error = zed_merged(Some(theirs), &one("jawata", "u")).expect_err("their entry stays theirs");
        assert!(error.contains("not jawata's own"), "{error}");
        let chat = "{\n  // mine\n  \"jawataChat.agent\": { \"command\": \"mine\" },\n}\n";
        assert!(chat_merged(Some(chat), &launch("u")).is_err());
        assert!(chat_without(chat).unwrap() == chat, "an agent that is not ours stays, byte for byte");
    }

    #[test]
    fn an_entry_of_a_removed_workspace_goes_and_the_comma_goes_with_it() {
        let text = format!(
            "{{\n  \"agent_servers\": {{\n    // first\n    \"jawata (a)\": {OURS_A},\n    // second\n    \"jawata (b)\": {OURS_B} // b's note\n  }}\n}}\n"
        );
        let last_gone = zed_merged(Some(&text), &one("jawata (a)", "http://a")).unwrap();
        assert_eq!(
            last_gone,
            format!("{{\n  \"agent_servers\": {{\n    // first\n    \"jawata (a)\": {OURS_A}\n    // second\n // b's note\n  }}\n}}\n"),
            "the last member's comma is the one before it, and the comments stay"
        );
        let first_gone = zed_merged(Some(&text), &one("jawata (b)", "http://b")).unwrap();
        assert_eq!(
            first_gone,
            format!("{{\n  \"agent_servers\": {{\n    // first\n    // second\n    \"jawata (b)\": {OURS_B} // b's note\n  }}\n}}\n")
        );
        let renamed = zed_merged(Some(&text), &vec![
            ("jawata (a)".to_string(), launch("http://a")),
            ("jawata (c)".to_string(), launch("http://c")),
        ])
        .unwrap();
        let root = value_of(&renamed);
        let servers = root["agent_servers"].as_object().unwrap();
        assert_eq!(servers.keys().collect::<Vec<_>>(), vec!["jawata (a)", "jawata (c)"]);
        assert!(renamed.contains("// second\n    \"jawata (c)\": {"), "the new name takes the old place: {renamed}");
        assert!(renamed.contains("// b's note"), "{renamed}");
    }

    #[test]
    fn delete_takes_only_our_entries_out_of_a_commented_file() {
        let removed = zed_without(ZED_COMMENTED).unwrap();
        let root = value_of(&removed);
        assert_eq!(root["agent_servers"].as_object().unwrap().keys().collect::<Vec<_>>(), vec!["other"]);
        assert!(removed.contains("// jawata, registered by the studio"), "a comment beside the entry stays");
        assert!(removed.contains("// my zed settings") && removed.contains("// the end") && removed.contains("// after"));

        let chat = chat_merged(Some("// c\n{\n  \"a\": 1,\n  \"b\": 2, // last\n}\n"), &launch("http://u")).unwrap();
        assert_eq!(
            chat_without(&chat).unwrap(),
            "// c\n{\n  \"a\": 1,\n  \"b\": 2, // last\n}\n",
            "put in and taken out, the file is as it was"
        );
    }

    #[test]
    fn the_chat_entry_is_edited_in_a_commented_settings_file() {
        let text = "// user settings\n{\n  \"editor.fontFamily\": \"Fira // Code\",\n  \"files.exclude\": { \"**/.git\": true, }, /* kept */\n  \"z\": 1, // last\n}\n";
        let merged = chat_merged(Some(text), &launch("http://u")).unwrap();
        let head = text.strip_suffix("\n}\n").unwrap();
        assert!(merged.starts_with(&format!("{head}\n  \"jawataChat.agent\": {{\n")), "{merged}");
        assert!(merged.ends_with("\n  },\n}\n"), "{merged}");
        let second = chat_merged(Some(&merged), &launch("http://u2")).unwrap();
        assert_eq!(second, merged.replace("\"http://u\"", "\"http://u2\""), "the entry is edited in place");
        assert_eq!(value_of(&second)["jawataChat.agent"]["env"]["JAWATA_URL"], "http://u2");
        assert_eq!(value_of(&second)["editor.fontFamily"], "Fira // Code");
    }

    #[test]
    fn an_inserted_entry_follows_the_files_own_line_endings_and_indentation() {
        let crlf = "{\r\n  \"a\": 1\r\n}\r\n";
        let merged = zed_merged(Some(crlf), &one("jawata", "http://u")).unwrap();
        assert!(!merged.replace("\r\n", "").contains('\n'), "every line break is a CRLF: {merged:?}");
        let tabs = "{\n\t\"a\": 1\n}\n";
        let merged = zed_merged(Some(tabs), &one("jawata", "http://u")).unwrap();
        assert!(merged.contains("\n\t\"agent_servers\": {\n\t\t\"jawata\": {\n\t\t\t\"args\""), "{merged:?}");
    }
}
