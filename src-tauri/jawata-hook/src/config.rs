//! `hook_config` — the read side.
//!
//! One binary cannot be ten specialised copies with the endpoint baked in, so
//! endpoint and token live in a file beside the binary: written at deploy,
//! read at fire time.
//!
//! **Concurrency here is measured, not assumed.** Three sessions with a
//! holding hook produced three overlapping pairs — invocations genuinely run
//! in parallel. The deploy side writes temp-file-plus-rename so a reader never
//! sees a torn file; this side must therefore treat a torn or missing file as
//! an ordinary, named outcome rather than a surprise.
//!
//! **Honest scope.** Putting config on disk removes the need for the Studio
//! *process*. It does NOT make the hook independent of the resident JVM, whose
//! lifecycle Studio owns. With the resident down the memory hooks correctly
//! get nothing, and the guard — which never queries — still answers.

use crate::safety::SilenceReason;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// What the deploy writes beside the binary.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct HookConfig {
    /// The resident's MCP endpoint, e.g. `http://127.0.0.1:8800/mcp`.
    pub url: String,
    /// The per-workspace bearer token.
    pub token: String,
    /// Which client this install serves — `"claude-code"` or `"cursor"`.
    ///
    /// Required, and deliberately not defaulted. The same binary ships to both,
    /// and the dialects are not interchangeable: guessing wrong means emitting
    /// Claude's `hookSpecificOutput` wrapper at Cursor, which Cursor ignores —
    /// silently, which is the failure mode this crate exists to end. A default
    /// would make that guess for us.
    pub client: String,
    /// Optional override; the default is the safety module's budget.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Sprint 28b (D4): the workspace's `field/` directory — where the resident
    /// writes the sanitized pile and the two switches. The hook READS it to
    /// decide whether a recurring failure deserves its one-line pointer at
    /// `/report`. Absent (an older deploy, or a client with no workspace) means
    /// no nudge, never a guessed path.
    #[serde(default)]
    pub field_dir: Option<String>,
    /// The recall gate's authority: `off` | `observe` | `block`.
    ///
    /// Absent means `observe` — record what would have been blocked, block
    /// nothing. The DOCUMENTED KILL SWITCH is `off`, and it lives here rather
    /// than in the store so it can be reached without the resident being up.
    #[serde(default)]
    pub recall_gate: Option<String>,
}

impl HookConfig {
    pub fn timeout(&self) -> Duration {
        self.timeout_ms
            .map(Duration::from_millis)
            .unwrap_or(crate::safety::STDIN_DEADLINE)
    }

    /// Resolve the declared client. An unrecognised name is a NAMED failure,
    /// never a fallback to one of them.
    pub fn client(&self) -> Result<crate::roles::Client, SilenceReason> {
        match self.client.trim() {
            "claude-code" => Ok(crate::roles::Client::ClaudeCode),
            "cursor" => Ok(crate::roles::Client::Cursor),
            other => Err(SilenceReason::PayloadUnreadable(format!(
                "hook_config names client {other:?}, which this binary does not know; \
                 guessing would emit one client's dialect at the other, which is ignored \
                 silently"
            ))),
        }
    }
}

/// The config file name, beside the binary.
pub const CONFIG_FILE: &str = "hook_config.json";

/// Where to look: beside the running binary. Not a fixed absolute path,
/// because the same binary ships to five targets and is deployed per client.
pub fn config_path_for(exe: &Path) -> Option<PathBuf> {
    exe.parent().map(|dir| dir.join(CONFIG_FILE))
}

/// Read the config, or say precisely why there is none.
///
/// Every failure is a NAMED reason. "No config" and "a config that will not
/// parse" are different facts about the install, and a user debugging a silent
/// hook needs to know which one they have.
pub fn load_from(path: &Path) -> Result<HookConfig, SilenceReason> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(SilenceReason::NotConfigured)
        }
        Err(e) => {
            return Err(SilenceReason::PayloadUnreadable(format!(
                "{} could not be read: {e}",
                path.display()
            )))
        }
    };
    if text.trim().is_empty() {
        // A zero-length file is what a torn write looks like from here. Named,
        // not silently treated as absent, because it means the DEPLOY is
        // broken rather than absent.
        return Err(SilenceReason::PayloadUnreadable(format!(
            "{} is empty — a deploy wrote it torn, or is writing it now",
            path.display()
        )));
    }
    let config: HookConfig = serde_json::from_str(&text).map_err(|e| {
        SilenceReason::PayloadUnreadable(format!("{} did not parse: {e}", path.display()))
    })?;
    if config.url.trim().is_empty() || config.token.trim().is_empty() {
        return Err(SilenceReason::PayloadUnreadable(format!(
            "{} parsed but carries an empty url or token",
            path.display()
        )));
    }
    Ok(config)
}

/// Load for the currently running binary.
pub fn load() -> Result<HookConfig, SilenceReason> {
    let exe = std::env::current_exe()
        .map_err(|e| SilenceReason::PayloadUnreadable(format!("no current_exe: {e}")))?;
    let path = config_path_for(&exe).ok_or(SilenceReason::NotConfigured)?;
    load_from(&path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jawata-hook-cfg-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(CONFIG_FILE)
    }

    #[test]
    fn a_missing_config_is_not_configured_not_an_error() {
        let p = tmp("missing");
        let _ = std::fs::remove_file(&p);
        assert_eq!(Err(SilenceReason::NotConfigured), load_from(&p));
    }

    #[test]
    fn a_good_config_loads() {
        let p = tmp("good");
        std::fs::write(&p, r#"{"url":"http://127.0.0.1:8800/mcp","token":"abc","client":"claude-code"}"#).unwrap();
        let c = load_from(&p).unwrap();
        assert_eq!("http://127.0.0.1:8800/mcp", c.url);
        assert_eq!("abc", c.token);
        assert_eq!(crate::safety::STDIN_DEADLINE, c.timeout());
    }

    #[test]
    fn a_torn_write_is_told_apart_from_an_absent_config() {
        // THE distinction that matters when debugging a silent hook: nothing
        // deployed here, versus a deploy that half-wrote. Reading a zero-length
        // file as "not configured" would hide a broken deploy forever.
        let p = tmp("torn");
        std::fs::write(&p, "").unwrap();
        match load_from(&p) {
            Err(SilenceReason::PayloadUnreadable(why)) => assert!(why.contains("torn"), "{why}"),
            other => panic!("an empty config must not read as absent: {other:?}"),
        }
    }

    #[test]
    fn a_config_that_parses_but_says_nothing_useful_is_rejected() {
        let p = tmp("blank-fields");
        std::fs::write(&p, r#"{"url":"","token":"","client":"cursor"}"#).unwrap();
        assert!(matches!(load_from(&p), Err(SilenceReason::PayloadUnreadable(_))));
    }

    #[test]
    fn a_malformed_config_names_itself() {
        let p = tmp("malformed");
        std::fs::write(&p, "{not json").unwrap();
        match load_from(&p) {
            Err(SilenceReason::PayloadUnreadable(why)) => assert!(why.contains("did not parse")),
            other => panic!("expected a named parse failure, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_field_does_not_break_a_working_install() {
        // Forward compatibility in the direction that matters: a NEWER studio
        // writing a field this older hook does not know must not silence it.
        let p = tmp("forward");
        std::fs::write(
            &p,
            r#"{"url":"http://x/mcp","token":"t","client":"cursor","somethingNew":{"a":1}}"#,
        )
        .unwrap();
        assert!(load_from(&p).is_ok(), "an unknown field must not disable the hook");
    }

    #[test]
    fn an_unknown_client_is_named_never_guessed() {
        // Guessing would emit Claude's hookSpecificOutput wrapper at Cursor,
        // which Cursor ignores SILENTLY — the failure mode this crate ends.
        let p = tmp("client");
        std::fs::write(&p, r#"{"url":"http://x/mcp","token":"t","client":"windsurf"}"#).unwrap();
        let c = load_from(&p).unwrap();
        match c.client() {
            Err(SilenceReason::PayloadUnreadable(why)) => assert!(why.contains("windsurf"), "{why}"),
            other => panic!("an unknown client must not resolve: {other:?}"),
        }
        assert_eq!(Ok(crate::roles::Client::Cursor),
            HookConfig { url: "u".into(), token: "t".into(), client: "cursor".into(), timeout_ms: None, field_dir: None, recall_gate: None }.client());
    }

    #[test]
    fn a_config_without_a_client_is_refused_rather_than_defaulted() {
        let p = tmp("no-client");
        std::fs::write(&p, r#"{"url":"http://x/mcp","token":"t"}"#).unwrap();
        assert!(matches!(load_from(&p), Err(SilenceReason::PayloadUnreadable(_))),
            "a missing client must not default to one of them");
    }

    #[test]
    fn config_sits_beside_the_binary_whatever_the_platform() {
        assert_eq!(
            Some(PathBuf::from("/opt/jawata/hook_config.json")),
            config_path_for(Path::new("/opt/jawata/jawata-hook-primer"))
        );
    }
}
