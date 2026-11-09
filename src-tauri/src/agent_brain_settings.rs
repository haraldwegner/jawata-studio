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

/// The settings for what [`crate::brains::installed_program_path`] found.
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
                let registry = serde_json::json!({ CURSOR: one.clone() });
                AgentBrainSettings {
                    env: vec![
                        (BRAIN_ENV.to_string(), one.to_string()),
                        (HIRE_BRAINS_ENV.to_string(), registry.to_string()),
                        (REVIEWER_ENV.to_string(), one.to_string()),
                    ],
                    note: None,
                }
            }
        },
    }
}

/// The ACP adapters a person may have installed, by the brain they start: an adapter that is on the path can run the principal
/// or btw, and one that is not is never offered. None is downloaded here.
const ADAPTERS: [(&str, &str); 3] = [("claude-code", "claude-agent-acp"), ("claude-code", "claude-code-acp"), ("codex", "codex-acp")];

/// The settings for this machine, read from `path_var` (the `PATH` value).
pub fn detect(path_var: Option<&OsStr>) -> AgentBrainSettings {
    let mut settings = settings_for(crate::brains::installed_program_path(CURSOR, path_var));
    let Some((_, registry)) = settings.env.iter_mut().find(|(name, _)| name == HIRE_BRAINS_ENV) else {
        return settings;
    };
    let mut programs: serde_json::Value = serde_json::from_str(registry).unwrap_or_else(|_| serde_json::json!({}));
    for (brain, adapter) in ADAPTERS {
        if programs.get(brain).is_none() {
            if let Some(path) = crate::brains::program_on_path(adapter, path_var).and_then(|p| p.to_str().map(str::to_string)) {
                programs[brain] = serde_json::json!({ "command": [path], "env": {} });
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
        for name in ["agent", "codex-acp"] {
            let file = dir.join(name);
            std::fs::write(&file, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let settings = detect(Some(dir.as_os_str()));
        let mut brains = acp_brains(&settings);
        brains.sort();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(brains, vec!["codex".to_string(), "cursor".to_string()]);
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
            assert_eq!(keys, vec!["command", "env"], "exactly the engine's shape: {program}");
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
