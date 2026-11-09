//! The brains the engine's own agent starts, written from what is installed on this machine.
//!
//! The engine reads three host settings, each as an environment variable (jawata-mcp
//! `DefaultEngineSettings`): `jawata.agent.brain` (`JAWATA_AGENT_BRAIN`, the brain main hires),
//! `jawata.agent.hireBrains` (`JAWATA_AGENT_HIRE_BRAINS`, a registry from a brain's own name to its
//! program) and `jawata.agent.reviewer` (`JAWATA_AGENT_REVIEWER`, the brain that reviews main's text
//! before it is sent). Each names a program as `{"command":[...],"env":{...}}`, which the engine's
//! `AcpBrainSession.parseProgram` reads. This studio writes them as it writes `JAWATA_AGENT_SEATS_DIR`:
//! into the environment of every engine it starts.
//!
//! What is written is what this machine HAS, never a guess (the ruling of 28h S15, open to the person's
//! veto): when Cursor's agent program is on the path, all three settings name it with the argument `acp`
//! (the form the real-brain runs of S7a and S12 used); when it is not, nothing is written and the engine
//! answers its existing no-brain sentence. Nothing is written for the Claude or Codex adapters (each needs
//! a download the person has not approved) or for Copilot (its ACP start fails). A credential is never
//! written: the `env` of every program is empty, the program uses its own login.

use std::ffi::OsStr;
use std::path::PathBuf;

/// The engine's environment name for main's brain (`jawata.agent.brain`).
pub const BRAIN_ENV: &str = "JAWATA_AGENT_BRAIN";
/// The engine's environment name for the registry of hired brains (`jawata.agent.hireBrains`).
pub const HIRE_BRAINS_ENV: &str = "JAWATA_AGENT_HIRE_BRAINS";
/// The engine's environment name for the reviewer (`jawata.agent.reviewer`).
pub const REVIEWER_ENV: &str = "JAWATA_AGENT_REVIEWER";

const ALL_ENV: [&str; 3] = [BRAIN_ENV, HIRE_BRAINS_ENV, REVIEWER_ENV];

/// The one brain this studio writes a program for, named the way the engine's dialect names itself.
const CURSOR: &str = "cursor";

/// What the studio will put in an engine's environment, and why nothing when it puts nothing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AgentBrainSettings {
    /// The variables to set: empty or exactly the three of [`ALL_ENV`].
    pub env: Vec<(String, String)>,
    /// The reason nothing was written, in words a log line or a status can show. `None` when written.
    pub note: Option<String>,
}

/// The model the principal's Cursor session is moved to once it is open (D54 (a), (b)), named the way Cursor's own session names it: it offers
/// 47 models in a `model` option, and this is the `name` of the one whose value is `grok-4.5[effort=high,fast=true]`. The engine matches it against
/// what the session offers and, when the account has no such model, leaves the session on Cursor's own Auto. It is chosen because Auto names no
/// model and a row that must say which model is answering cannot say it for Auto; speed does not choose it (not measured: a first comparison on
/// 2026-11-09 passed the model as a command-line argument, which Cursor's ACP mode ignores, so every run it timed was on Auto).
pub const PRINCIPAL_CURSOR_MODEL: &str = "grok-4.5";

/// The settings for what [`crate::brains::installed_program_path`] found. Only the principal names a model: the registry's Cursor entry, which
/// hires and the reviewer start from, keeps Cursor's own default, because a model chosen for a conversation is not the model a hired architect
/// should run on.
pub fn settings_for(found: Result<Option<PathBuf>, String>) -> AgentBrainSettings {

    let nothing = |note: String| AgentBrainSettings { env: Vec::new(), note: Some(note) };
    match found {
        Err(reason) => nothing(format!("no agent brain was configured: {reason}")),
        Ok(None) => nothing("no agent brain was configured: Cursor's agent program is not installed on this machine".into()),
        Ok(Some(path)) => match path.to_str() {
            None => nothing(format!(
                "no agent brain was configured: the path of Cursor's agent program is not text ({})",
                path.display()
            )),
            Some(program) => {
                let one = serde_json::json!({ "command": [program, "acp"], "env": {} });
                let principal = serde_json::json!({ "command": [program, "acp"], "env": {}, "model": PRINCIPAL_CURSOR_MODEL });
                let registry = serde_json::json!({ CURSOR: one.clone() });
                AgentBrainSettings {
                    env: vec![
                        (BRAIN_ENV.to_string(), principal.to_string()),
                        (HIRE_BRAINS_ENV.to_string(), registry.to_string()),
                        (REVIEWER_ENV.to_string(), one.to_string()),
                    ],
                    note: None,
                }
            }
        },
    }
}

/// A program that speaks ACP on behalf of one brain, and what a person runs to get it. An adapter that is on the path can run the principal
/// or btw, and one that is not is never offered. None is downloaded here: the person is told the `install` line and runs it.
pub struct Adapter {
    /// The brain it starts, named the way the engine's dialect names itself.
    pub brain: &'static str,
    /// The program looked for on the path.
    pub program: &'static str,
    /// The arguments that make that program speak ACP (Copilot's CLI does so only with `--acp`; the others are adapters that speak nothing else).
    pub args: &'static [&'static str],
    /// The command that installs it. Each package name was read from the package registry on 2026-11-09 and its `bin` name matches `program`.
    pub install: &'static str,
    /// Whether the program must be started with the person's own deployed jawata servers switched off. Copilot's CLI loads every server of its own
    /// configuration into each turn, and the chat already gives the brain the engine itself: measured on 2026-11-09, one jawata server adds about
    /// 51,000 input tokens to a turn, and with the three deployed here Copilot answered "Static system messages and tool definitions exceed the
    /// model's usable context budget" and no answer. With them off it answered in 4 s.
    pub without_own_servers: bool,
}

/// The adapters in the order they are tried: for a brain with two, the first found is used and the first is the one a person is told to install.
pub const ADAPTERS: [Adapter; 4] = [
    Adapter { brain: "claude-code", program: "claude-agent-acp", args: &[], install: "npm install -g @agentclientprotocol/claude-agent-acp", without_own_servers: false },
    Adapter { brain: "claude-code", program: "claude-code-acp", args: &[], install: "npm install -g @zed-industries/claude-code-acp", without_own_servers: false },
    Adapter { brain: "codex", program: "codex-acp", args: &[], install: "npm install -g @zed-industries/codex-acp", without_own_servers: false },
    Adapter { brain: "copilot", program: "copilot", args: &["--acp"], install: "npm install -g @github/copilot", without_own_servers: true },
];

/// A brain a person can run the chat on whose adapter is not on this machine, with the one command that installs it.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct MissingAdapter {
    pub brain: String,
    pub install: String,
}

/// The brains no adapter of which is on `path_var`, each with the install command of its first adapter, in [`ADAPTERS`] order. It reads the same
/// list [`detect`] reads, so what is offered and what is said to be missing cannot disagree.
pub fn missing_adapters(path_var: Option<&OsStr>) -> Vec<MissingAdapter> {
    let mut missing: Vec<MissingAdapter> = Vec::new();
    for adapter in &ADAPTERS {
        let already = missing.iter().any(|m| m.brain == adapter.brain);
        let found = ADAPTERS
            .iter()
            .filter(|other| other.brain == adapter.brain)
            .any(|other| crate::brains::program_on_path(other.program, path_var).is_some());
        if !already && !found {
            missing.push(MissingAdapter { brain: adapter.brain.to_string(), install: adapter.install.to_string() });
        }
    }
    missing
}

/// The settings for this machine, read from `path_var` (the `PATH` value), when the studio has deployed no server of its own.
pub fn detect(path_var: Option<&OsStr>) -> AgentBrainSettings {
    detect_with(path_var, &[])
}

/// The settings for this machine, read from `path_var`, for a machine on which `deployed_servers` are the names of the jawata servers the
/// studio has written into the person's agents. An adapter that loads the person's own servers is told, by name, to leave them off.
pub fn detect_with(path_var: Option<&OsStr>, deployed_servers: &[String]) -> AgentBrainSettings {
    let mut settings = settings_for(crate::brains::installed_program_path(CURSOR, path_var));
    let Some((_, registry)) = settings.env.iter_mut().find(|(name, _)| name == HIRE_BRAINS_ENV) else {
        return settings;
    };
    let mut programs: serde_json::Value = serde_json::from_str(registry).unwrap_or_else(|_| serde_json::json!({}));
    for adapter in &ADAPTERS {
        if programs.get(adapter.brain).is_none() {
            if let Some(path) = crate::brains::program_on_path(adapter.program, path_var).and_then(|p| p.to_str().map(str::to_string)) {
                let mut command = vec![path];
                command.extend(adapter.args.iter().map(|arg| arg.to_string()));
                if adapter.without_own_servers {
                    for server in deployed_servers {
                        command.push("--disable-mcp-server".to_string());
                        command.push(server.clone());
                    }
                }
                programs[adapter.brain] = serde_json::json!({ "command": command, "env": {} });
            }
        }
    }
    *registry = programs.to_string();
    settings
}

/// The brains the engine has a program for on this machine, which is what the principal and btw can run on.
pub fn acp_brains(settings: &AgentBrainSettings) -> Vec<String> {
    settings
        .env
        .iter()
        .find(|(name, _)| name == HIRE_BRAINS_ENV)
        .and_then(|(_, registry)| serde_json::from_str::<serde_json::Value>(registry).ok())
        .and_then(|programs| programs.as_object().map(|map| map.keys().cloned().collect()))
        .unwrap_or_default()
}

/// `base` with the agent settings set: an entry of the same name already in `base` is replaced, never
/// kept beside the new one, so a changed program path rewrites the value and a repeat adds nothing.
pub fn applied_to(base: Vec<(String, String)>, settings: &AgentBrainSettings) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = base
        .into_iter()
        .filter(|(name, _)| !(ALL_ENV.contains(&name.as_str()) && settings.env.iter().any(|(set, _)| set == name)))
        .collect();
    env.extend(settings.env.iter().cloned());
    env
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn found(path: &str) -> Result<Option<PathBuf>, String> {
        Ok(Some(PathBuf::from(path)))
    }

    fn value_of<'a>(settings: &'a AgentBrainSettings, name: &str) -> &'a str {
        settings
            .env
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
            .unwrap_or_else(|| panic!("{name} not written: {:?}", settings.env))
    }

    #[test]
    fn an_installed_agent_program_is_written_as_main_the_hired_cursor_and_the_reviewer() {
        let settings = settings_for(found("/home/me/.local/bin/agent"));

        let names: Vec<&str> = settings.env.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, vec!["JAWATA_AGENT_BRAIN", "JAWATA_AGENT_HIRE_BRAINS", "JAWATA_AGENT_REVIEWER"]);
        assert!(settings.note.is_none(), "written, so nothing to explain: {:?}", settings.note);

        let brain: Value = serde_json::from_str(value_of(&settings, BRAIN_ENV)).unwrap();
        assert_eq!(brain["command"], serde_json::json!(["/home/me/.local/bin/agent", "acp"]));
        let reviewer: Value = serde_json::from_str(value_of(&settings, REVIEWER_ENV)).unwrap();
        assert_eq!(reviewer["command"], serde_json::json!(["/home/me/.local/bin/agent", "acp"]));
        let registry: Value = serde_json::from_str(value_of(&settings, HIRE_BRAINS_ENV)).unwrap();
        assert_eq!(registry["cursor"]["command"], serde_json::json!(["/home/me/.local/bin/agent", "acp"]));
    }

    #[cfg(unix)]
    #[test]
    fn an_adapter_on_the_path_joins_the_registry_and_an_absent_one_does_not() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("jw-adapters-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["agent", "codex-acp", "copilot"] {
            let file = dir.join(name);
            std::fs::write(&file, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let settings = detect(Some(dir.as_os_str()));
        let mut brains = acp_brains(&settings);
        brains.sort();
        let registry: Value = serde_json::from_str(value_of(&settings, HIRE_BRAINS_ENV)).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(brains, vec!["codex".to_string(), "copilot".to_string(), "cursor".to_string()]);
        // D54 (f): Copilot's CLI speaks ACP only with its own argument, so the registry carries the argument, and an adapter that needs none carries none.
        let copilot = registry["copilot"]["command"].as_array().unwrap();
        assert_eq!(copilot.len(), 2, "{copilot:?}");
        assert!(copilot[0].as_str().unwrap().ends_with("copilot"), "{copilot:?}");
        assert_eq!(copilot[1], "--acp");
        assert_eq!(registry["codex"]["command"].as_array().unwrap().len(), 1, "codex-acp takes no argument");
    }

    #[cfg(unix)]
    fn path_holding(names: &[&str], tag: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("jw-missing-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in names {
            let file = dir.join(name);
            std::fs::write(&file, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        dir
    }

    /// D54 (f): Copilot's CLI loads the person's own servers into every turn, and three jawata servers made it answer "Static system messages
    /// and tool definitions exceed the model's usable context budget" and nothing else (measured 2026-11-09). So it is started with each
    /// deployed server named off, and the adapters that read no such list are started as they were.
    #[cfg(unix)]
    #[test]
    fn copilot_is_started_with_the_deployed_servers_off_and_the_others_are_not() {
        let dir = path_holding(&["agent", "copilot", "codex-acp"], "c");
        let deployed = vec!["jawata-alpha".to_string(), "jawata-beta".to_string()];
        let with = detect_with(Some(dir.as_os_str()), &deployed);
        let without = detect_with(Some(dir.as_os_str()), &[]);
        let _ = std::fs::remove_dir_all(&dir);
        let words = |settings: &AgentBrainSettings, brain: &str| -> Vec<String> {
            let registry: Value = serde_json::from_str(value_of(settings, HIRE_BRAINS_ENV)).unwrap();
            registry[brain]["command"].as_array().unwrap().iter().skip(1).map(|w| w.as_str().unwrap().to_string()).collect()
        };
        assert_eq!(
            words(&with, "copilot"),
            vec!["--acp", "--disable-mcp-server", "jawata-alpha", "--disable-mcp-server", "jawata-beta"],
            "each deployed server named off, after the argument that makes Copilot speak ACP"
        );
        assert_eq!(words(&without, "copilot"), vec!["--acp"], "no deployed server, nothing to switch off");
        assert!(words(&with, "codex").is_empty(), "an adapter that loads no list of the person's servers is started bare");
        assert!(words(&with, "cursor").iter().all(|w| w == "acp"), "Cursor's agent is started as before");
    }

    /// D54 (g): the Seats page names each brain whose adapter is not here with the command that installs it, read from the list that decides
    /// what is offered. His words: "I can see your please install message".
    #[cfg(unix)]
    #[test]
    fn a_brain_whose_adapter_is_absent_is_named_with_its_install_command_and_one_that_is_present_is_not() {
        let dir = path_holding(&["agent", "claude-code-acp"], "a");
        let missing = missing_adapters(Some(dir.as_os_str()));
        let _ = std::fs::remove_dir_all(&dir);
        let brains: Vec<&str> = missing.iter().map(|m| m.brain.as_str()).collect();
        assert_eq!(brains, vec!["codex", "copilot"], "the older Claude adapter alone counts as Claude being here: {missing:?}");
        assert_eq!(missing[0].install, "npm install -g @zed-industries/codex-acp");
        assert_eq!(missing[1].install, "npm install -g @github/copilot");

        let none = path_holding(&["agent"], "b");
        let missing = missing_adapters(Some(none.as_os_str()));
        let _ = std::fs::remove_dir_all(&none);
        assert_eq!(missing.iter().map(|m| m.brain.as_str()).collect::<Vec<_>>(), vec!["claude-code", "codex", "copilot"]);
        assert_eq!(missing[0].install, "npm install -g @agentclientprotocol/claude-agent-acp", "the current adapter is the one a person is told to install");
    }

    #[test]
    fn what_is_said_to_be_missing_and_what_is_offered_come_from_one_list() {
        let offered: std::collections::BTreeSet<&str> = ADAPTERS.iter().map(|a| a.brain).collect();
        let empty = std::env::temp_dir().join(format!("jw-empty-{}", std::process::id()));
        std::fs::create_dir_all(&empty).unwrap();
        let missing: std::collections::BTreeSet<String> = missing_adapters(Some(empty.as_os_str())).into_iter().map(|m| m.brain).collect();
        let _ = std::fs::remove_dir_all(&empty);
        assert_eq!(missing.iter().map(String::as_str).collect::<std::collections::BTreeSet<_>>(), offered, "an empty path misses every brain the list offers");
    }

    /// D54 (a), (b): the principal's setting names the model its Cursor session is moved to, so a row can say which model is answering; the registry's
    /// Cursor entry, which hires start from, and the reviewer name none and keep Cursor's own default. The model is not a command-line argument:
    /// Cursor's ACP mode ignores `--model`, which is how a first version of this change named a model that never ran.
    #[test]
    fn the_principal_names_its_model_and_hires_and_the_reviewer_do_not() {
        let settings = settings_for(found("/home/me/.local/bin/agent"));

        let brain: Value = serde_json::from_str(value_of(&settings, BRAIN_ENV)).unwrap();
        assert_eq!(brain["model"], PRINCIPAL_CURSOR_MODEL);
        assert_eq!(brain["command"], serde_json::json!(["/home/me/.local/bin/agent", "acp"]), "no model argument: Cursor's ACP mode ignores it");
        let reviewer: Value = serde_json::from_str(value_of(&settings, REVIEWER_ENV)).unwrap();
        assert!(reviewer.get("model").is_none(), "the reviewer is not moved: {reviewer}");
        let registry: Value = serde_json::from_str(value_of(&settings, HIRE_BRAINS_ENV)).unwrap();
        assert!(registry["cursor"].get("model").is_none(), "a hired Cursor keeps Cursor's default: {registry}");
    }

    #[test]
    fn the_variable_names_are_the_ones_the_engine_maps() {
        // Spelled out on purpose, as `SEATS_DIR_ENV` is: a rename on either side would fail silently.
        assert_eq!(BRAIN_ENV, "JAWATA_AGENT_BRAIN");
        assert_eq!(HIRE_BRAINS_ENV, "JAWATA_AGENT_HIRE_BRAINS");
        assert_eq!(REVIEWER_ENV, "JAWATA_AGENT_REVIEWER");
    }

    #[test]
    fn the_values_are_the_shape_the_engine_reads_and_carry_no_credential() {
        let settings = settings_for(found("/opt/cursor/agent"));
        let brain: Value = serde_json::from_str(value_of(&settings, BRAIN_ENV)).unwrap();
        let reviewer: Value = serde_json::from_str(value_of(&settings, REVIEWER_ENV)).unwrap();
        let registry: Value = serde_json::from_str(value_of(&settings, HIRE_BRAINS_ENV)).unwrap();
        for program in [&brain, &reviewer, &registry["cursor"]] {
            assert!(program["command"].is_array());
            assert_eq!(program["env"], serde_json::json!({}), "the program uses its own login: {program}");
            let keys: Vec<&String> = program.as_object().unwrap().keys().collect();
            let expected: Vec<&str> = if program == &brain { vec!["command", "env", "model"] } else { vec!["command", "env"] };
            assert_eq!(keys, expected, "exactly the engine's shape: {program}");
        }
        assert_eq!(registry.as_object().unwrap().len(), 1, "no entry for a brain that was not asked for: {registry}");
    }

    #[test]
    fn a_path_with_a_quote_or_a_backslash_is_still_valid_json_that_names_that_path() {
        let awkward = r#"C:\Users\me\"quoted"\agent.exe"#;
        let settings = settings_for(found(awkward));
        let brain: Value = serde_json::from_str(value_of(&settings, BRAIN_ENV)).unwrap();
        assert_eq!(brain["command"][0], awkward);
    }

    #[test]
    fn a_machine_without_the_program_gets_no_setting_and_the_reason_says_so() {
        let settings = settings_for(Ok(None));
        assert!(settings.env.is_empty(), "a guess is never written: {:?}", settings.env);
        let note = settings.note.expect("the reason is reportable");
        assert!(note.contains("not installed"), "{note}");
    }

    #[test]
    fn a_path_that_could_not_be_read_gets_no_setting_and_the_reason_is_carried() {
        let settings = settings_for(Err("PATH is not set, so no program could be looked for".into()));
        assert!(settings.env.is_empty());
        let note = settings.note.expect("the reason is reportable");
        assert!(note.contains("PATH is not set"), "{note}");
    }

    #[cfg(unix)]
    #[test]
    fn a_program_path_that_is_not_text_is_not_written_and_says_why() {
        use std::os::unix::ffi::OsStringExt;
        let raw = std::ffi::OsString::from_vec(b"/bin/ag\xFFent".to_vec());
        let settings = settings_for(Ok(Some(PathBuf::from(raw))));
        assert!(settings.env.is_empty());
        assert!(settings.note.expect("reportable").contains("not text"));
    }

    #[test]
    fn a_changed_program_path_rewrites_the_value_and_a_repeat_adds_nothing() {
        let base = vec![("JAWATA_AGENT_SEATS_DIR".to_string(), "/cfg/seats".to_string())];

        let first = applied_to(base.clone(), &settings_for(found("/old/agent")));
        assert_eq!(first.len(), 4, "the seats folder and the three settings: {first:?}");

        // The same environment passed again with a new path: each name once, the new value.
        let second = applied_to(first.clone(), &settings_for(found("/new/agent")));
        assert_eq!(second.len(), 4, "never doubled: {second:?}");
        for name in ALL_ENV {
            assert_eq!(second.iter().filter(|(k, _)| k == name).count(), 1, "{name} once");
        }
        let brain = second.iter().find(|(k, _)| k == BRAIN_ENV).unwrap();
        assert!(brain.1.contains("/new/agent") && !brain.1.contains("/old/agent"), "{}", brain.1);
        assert!(second.iter().any(|(k, v)| k == "JAWATA_AGENT_SEATS_DIR" && v == "/cfg/seats"), "the seats folder is kept");

        // The same settings again is the same environment.
        assert_eq!(applied_to(second.clone(), &settings_for(found("/new/agent"))), second);
    }

    #[test]
    fn no_settings_leave_the_environment_as_it_was() {
        let base = vec![("JAWATA_AGENT_SEATS_DIR".to_string(), "/cfg/seats".to_string())];
        assert_eq!(applied_to(base.clone(), &AgentBrainSettings::default()), base);
        let empty: Vec<(String, String)> = Vec::new();
        assert!(applied_to(empty, &settings_for(Ok(None))).is_empty());
    }
}
