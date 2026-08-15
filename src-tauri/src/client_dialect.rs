//! Sprint 28a (Stage 2) — how each agent client SPELLS an MCP server entry.
//!
//! Every client stores the same three facts (a URL, a bearer header, an
//! enabled flag) and every one of them spells it differently: a different file
//! format, a different root key, a different name for the URL field, a
//! different set of extras. Before this module those differences lived as bare
//! string literals scattered across the writer, the remover and the validator —
//! `"mcpServers"` appeared in three places, and `client == "antigravity"`
//! appeared in two. Adding a client meant finding all of them.
//!
//! So the dialect becomes a VALUE, in one place, the same move
//! `org.jawata.core.host` made for the operating system in 1b: ask
//! [`dialect_for`] and the answer carries everything that varies. A new client
//! is one arm here plus its path; nothing downstream branches on a client name.
//!
//! **Every fact in the table below was measured by making the client's own
//! tooling write a config**, in a sandboxed HOME, on 2026-08-15 — not recalled
//! and not read from documentation:
//!
//! | client         | file                            | how it was measured        |
//! |----------------|---------------------------------|----------------------------|
//! | codex          | `~/.codex/config.toml`          | `codex mcp add --url …`    |
//! | copilot_cli    | `~/.copilot/mcp-config.json`    | `copilot mcp add …`        |
//! | vscode         | `<user-data>/User/mcp.json`     | `code --add-mcp …`         |
//! | cursor/claude… | `~/.cursor/mcp.json` etc.       | shipped since Sprint 15    |

/// The file format a client's MCP configuration is written in, and the key its
/// server map hangs under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFormat {
    /// A JSON object whose server map lives under `root_key`.
    ///
    /// `root_key` is `"mcpServers"` for every client EXCEPT VS Code, which uses
    /// `"servers"` — measured by running `code --add-mcp`, which wrote
    /// `{"servers": {…}, "inputs": []}`.
    Json { root_key: &'static str },
    /// TOML, with each server as its own `[<table>.<id>]` table (Codex).
    Toml { table: &'static str },
}

/// Everything that varies, per client, about writing one managed MCP entry.
#[derive(Debug, Clone, Copy)]
pub struct ClientDialect {
    pub format: ConfigFormat,
    /// What the client calls the URL field. Antigravity's Windsurf lineage
    /// reads `serverUrl` and rejects `url`; Codex's TOML also uses `url`.
    pub url_field: &'static str,
    /// Whether the client accepts a `type: "http"` discriminator. Antigravity
    /// rejects `type` + `url` together ("serverURL or command must be
    /// specified", verified 2026-06-10); Codex's TOML has no such field.
    pub emits_type: bool,
    /// Claude Code (CLI, v2.1.121+) honours a per-server `alwaysLoad` flag, so
    /// jawata's tool surface loads upfront instead of deferring behind MCP
    /// tool-search. No other client has an equivalent.
    pub emits_always_load: bool,
    /// Copilot CLI writes a `tools` filter beside every server; its own
    /// `copilot mcp add` emits `"tools": ["*"]`, and matching what the client
    /// writes for itself is what keeps a round-trip byte-stable.
    pub emits_tools_filter: bool,
}

impl ClientDialect {
    /// The JSON root key, or `None` when this client is not JSON at all.
    pub fn json_root_key(&self) -> Option<&'static str> {
        match self.format {
            ConfigFormat::Json { root_key } => Some(root_key),
            ConfigFormat::Toml { .. } => None,
        }
    }

    /// The TOML table prefix, or `None` when this client is not TOML.
    pub fn toml_table(&self) -> Option<&'static str> {
        match self.format {
            ConfigFormat::Toml { table } => Some(table),
            ConfigFormat::Json { .. } => None,
        }
    }

    pub fn is_toml(&self) -> bool {
        matches!(self.format, ConfigFormat::Toml { .. })
    }
}

/// The dialect a client's configuration file is written in.
///
/// An unknown client falls through to the majority shape rather than failing:
/// the deploy path REFUSES unknown ids up front (see
/// `KNOWN_DEPLOY_CLIENT_IDS`), so by the time a name reaches here it is known,
/// and a default that matches five of the eight clients is the least surprising
/// answer for a caller that reaches it in a test.
pub fn dialect_for(client: &str) -> ClientDialect {
    match client {
        // Codex: `[mcp_servers.<id>]` with `url` and `http_headers`. Confirmed
        // by `codex mcp get`, which reports `http_headers: Authorization=*****`
        // back from a file carrying exactly that key.
        "codex" => ClientDialect {
            format: ConfigFormat::Toml {
                table: "mcp_servers",
            },
            url_field: "url",
            emits_type: false,
            emits_always_load: false,
            emits_tools_filter: false,
        },
        // VS Code: `servers`, NOT `mcpServers` — the one client that differs.
        "vscode" => ClientDialect {
            format: ConfigFormat::Json {
                root_key: "servers",
            },
            url_field: "url",
            emits_type: true,
            emits_always_load: false,
            emits_tools_filter: false,
        },
        // Copilot CLI: `mcpServers` with the standard http entry plus a tools
        // filter, exactly as `copilot mcp add` writes it.
        "copilot_cli" => ClientDialect {
            format: ConfigFormat::Json {
                root_key: "mcpServers",
            },
            url_field: "url",
            emits_type: true,
            emits_always_load: false,
            emits_tools_filter: true,
        },
        // Antigravity (Windsurf lineage): `serverUrl`, and no `type`.
        "antigravity" => ClientDialect {
            format: ConfigFormat::Json {
                root_key: "mcpServers",
            },
            url_field: "serverUrl",
            emits_type: false,
            emits_always_load: false,
            emits_tools_filter: false,
        },
        "claude" => ClientDialect {
            format: ConfigFormat::Json {
                root_key: "mcpServers",
            },
            url_field: "url",
            emits_type: true,
            emits_always_load: true,
            emits_tools_filter: false,
        },
        // cursor, claude_desktop, intellij and anything else.
        _ => ClientDialect {
            format: ConfigFormat::Json {
                root_key: "mcpServers",
            },
            url_field: "url",
            emits_type: true,
            emits_always_load: false,
            emits_tools_filter: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vscode_is_the_only_client_whose_root_key_is_not_mcpservers() {
        // The whole reason this module exists. `code --add-mcp` wrote
        // {"servers": …}; writing our entry under "mcpServers" would land in a
        // key VS Code never reads — a deploy that reports success and does
        // nothing, which is this project's recorded deepest bug class.
        assert_eq!(dialect_for("vscode").json_root_key(), Some("servers"));
        for client in [
            "cursor",
            "claude",
            "claude_desktop",
            "antigravity",
            "intellij",
            "copilot_cli",
        ] {
            assert_eq!(
                dialect_for(client).json_root_key(),
                Some("mcpServers"),
                "{client} must keep the majority root key"
            );
        }
    }

    #[test]
    fn codex_is_toml_and_carries_no_json_root_key() {
        let codex = dialect_for("codex");
        assert!(codex.is_toml());
        assert_eq!(codex.toml_table(), Some("mcp_servers"));
        assert_eq!(
            codex.json_root_key(),
            None,
            "a TOML client must not answer a JSON question — a caller that \
             asks the wrong one gets None, not a plausible-looking key"
        );
    }

    #[test]
    fn antigravity_keeps_its_serverurl_and_refuses_type() {
        let ag = dialect_for("antigravity");
        assert_eq!(ag.url_field, "serverUrl");
        assert!(
            !ag.emits_type,
            "Windsurf lineage rejects type+url together (verified 2026-06-10)"
        );
    }

    #[test]
    fn always_load_is_claude_only() {
        assert!(dialect_for("claude").emits_always_load);
        for client in [
            "cursor",
            "claude_desktop",
            "antigravity",
            "intellij",
            "codex",
            "vscode",
            "copilot_cli",
        ] {
            assert!(
                !dialect_for(client).emits_always_load,
                "{client} has no alwaysLoad equivalent; emitting one writes a \
                 key the client does not understand"
            );
        }
    }

    #[test]
    fn tools_filter_is_copilot_only() {
        assert!(dialect_for("copilot_cli").emits_tools_filter);
        for client in ["cursor", "claude", "codex", "vscode", "antigravity"] {
            assert!(!dialect_for(client).emits_tools_filter);
        }
    }
}
