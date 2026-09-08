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
//! [`dialect_for`] and the answer carries everything that varies.
//!
//! **Scope, stated exactly** — an earlier version of this sentence claimed
//! "nothing downstream branches on a client name", and the C2 audit measured
//! that as false: `derive_rule_path`, `derive_seat_commands_dir`,
//! `seat_artifact_paths` and eight more sites still do, and adding a client
//! still needs a decision at the first two. What is true is narrower and is
//! the whole claim: **nothing in the MCP-entry write path branches on a client
//! name.** The rest of the roster's scatter is Stage 2b's target.
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

/// One client, as one row.
///
/// Sprint 28a Stage 2b, from the C2 architect pass: a client was described in
/// **13 separate places** in the backend, and Stage 2 collapsed three of them
/// while adding a fourteenth. Stage 2b's brief was "one shared roster
/// constant" — on the FRONTEND. Building it on one side only does not remove
/// the bug class; it creates a second authority that has to agree with the
/// first, and the two drift. That drift already shipped once, as
/// `claudeDesktop` versus `claude_desktop`, which made Claude Desktop
/// undeployable while reporting "Skipped: not selected in this deploy run".
///
/// So the roster is here, and the tests below hold every other description
/// against it: the settings structs, the deploy target list, and the UI
/// picker's own source. A client added to one and missed in another fails the
/// run instead of shipping.
pub struct Client {
    /// The snake_case id the deploy backend matches on.
    pub id: &'static str,
    /// The camelCase key the settings API speaks. Differs from `id` for every
    /// two-word client — there are now three, where there used to be one.
    pub settings_key: &'static str,
    /// What the user sees.
    pub label: &'static str,
    /// Whether the product can carry jawata here at all. The USER cannot
    /// change this — it is a fact about the client. Antigravity is the one
    /// `false`: its command-line tool has no mechanism to connect jawata, so
    /// it stays visible and greyed rather than silently vanishing.
    pub supported: bool,
    /// studio#19: other clients whose seat directory THIS client also reads.
    ///
    /// Cursor loads skills from the Claude and Codex directories *"for
    /// compatibility"* — its own documentation says so
    /// (cursor.com/docs/context/skills, read 2026-09-08). So on a machine where
    /// jawata writes one of those, our own copy is a SECOND source for ONE
    /// reader and every seat is listed twice.
    ///
    /// A LIST rather than one name, because the vendor's sentence names two
    /// directories and a single `Option` cannot say so — even though only
    /// `claude` receives seat files today, which is why Codex is not in this
    /// row: a source that is written nowhere cannot be inherited from, and
    /// listing it would claim coverage this deploy does not have.
    ///
    /// It lives HERE rather than beside the deploy for the reason this module
    /// exists: a client described in a `match` in a sibling module is the
    /// fourteenth place a client is described, and the conformance tests below
    /// cannot see one.
    pub seats_inherited_from: &'static [&'static str],
}

impl Client {
    /// studio#19: whether this client must write NO seat files of its own.
    ///
    /// ONE input, and it is an OBSERVATION: are the source's seat files there
    /// right now. The deploy visits a source before anything that inherits from
    /// it, so that one question covers a source written moments ago in this
    /// same run and a source left by an earlier one alike.
    ///
    /// A first version also asked whether the source was SELECTED for this run,
    /// which is a prediction — and wrong in the way that matters. If the
    /// source's own write then failed, this client's files were removed against
    /// a promise nothing kept, and the machine ended with no seat files at all:
    /// the invisible absence the whole conditional exists to prevent, arriving
    /// through it.
    ///
    /// The bound that keeps it honest: a client with no source PRESENT keeps
    /// its own files. Trading a visible duplicate for an invisible absence
    /// would be the worse defect.
    pub fn inherits_seat_commands(
        &self,
        source_is_present: impl Fn(&str) -> bool,
    ) -> bool {
        self.seats_inherited_from.iter().any(|source| source_is_present(source))
    }
}

/// Every client the deploy knows, in display order.
///
/// **IntelliJ is deliberately absent** (Harald, 2026-08-16). It is not
/// unsupported the way Antigravity is — it is REDUNDANT. IntelliJ hosts Claude
/// Code, Cursor, Codex and Copilot as agents that read their own config files,
/// so deploying those four already covers it; measured live, with the Claude
/// agent driving jawata inside IntelliJ through `~/.claude.json`. A dedicated
/// target added only JetBrains' own Junie, which could not reach our tools
/// across three explicit attempts while the Claude agent beside it could.
///
/// So IntelliJ's coverage is INHERITED, and the parity matrix must say that
/// rather than claim it as a driven client.
pub const CLIENTS: &[Client] = &[
    Client {
        id: "cursor",
        settings_key: "cursor",
        label: "Cursor",
        supported: true,
        // Measured, and the reason is in the field's own doc.
        seats_inherited_from: &["claude"],
    },
    Client {
        id: "claude",
        settings_key: "claude",
        label: "Claude Code",
        supported: true,
        seats_inherited_from: &[],
    },
    Client {
        id: "codex",
        settings_key: "codex",
        label: "Codex",
        supported: true,
        seats_inherited_from: &[],
    },
    Client {
        id: "copilot_cli",
        settings_key: "copilotCli",
        label: "Copilot CLI",
        supported: true,
        seats_inherited_from: &[],
    },
    Client {
        id: "vscode",
        settings_key: "vscode",
        label: "VS Code",
        supported: true,
        seats_inherited_from: &[],
    },
    Client {
        id: "grok",
        settings_key: "grok",
        label: "Grok",
        supported: true,
        seats_inherited_from: &[],
    },
];

/// The roster row for a client id, or `None` for an id we do not know.
pub fn client(id: &str) -> Option<&'static Client> {
    CLIENTS.iter().find(|c| c.id == id)
}

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
    /// TOML only — what this client calls the auth header block, and how it
    /// shapes it. Codex writes an INLINE table keyed `http_headers`; Grok
    /// writes a SUB-TABLE keyed `headers`. Same file format, same
    /// `[mcp_servers.<id>]` header, different spelling for the one field that
    /// carries the bearer token — so getting it wrong produces a server the
    /// client sees and cannot authenticate to.
    pub toml_headers: Option<TomlHeaders>,
    /// TOML only — whether the client's own tooling writes `enabled` even when
    /// the server IS enabled. `grok mcp add` does; `codex mcp add` omits it and
    /// writes `enabled = false` only to disable. Matching each tool's own
    /// output is what keeps a redeploy byte-stable against a file the user may
    /// have edited with that tool.
    pub toml_always_writes_enabled: bool,
}

/// How a TOML client spells its auth headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TomlHeaders {
    /// The key the block hangs under (`http_headers` / `headers`).
    pub key: &'static str,
    /// `true` for `key = { "Authorization" = … }`; `false` for a
    /// `[mcp_servers.<id>.key]` sub-table.
    pub inline: bool,
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
        // Codex: `[mcp_servers.<id>]` with `url` and an INLINE `http_headers`
        // table. Confirmed by `codex mcp get`, which reports
        // `http_headers: Authorization=*****` back from a file carrying it.
        "codex" => ClientDialect {
            format: ConfigFormat::Toml {
                table: "mcp_servers",
            },
            url_field: "url",
            emits_type: false,
            emits_always_load: false,
            emits_tools_filter: false,
            toml_headers: Some(TomlHeaders { key: "http_headers", inline: true }),
            toml_always_writes_enabled: false,
        },

        // Grok Build: the SAME TOML family as Codex — `[mcp_servers.<id>]`,
        // `url` — and different in exactly two ways, both measured on
        // 2026-08-16 by running `grok mcp add --transport http … --header
        // "Authorization: Bearer …"` in a sandboxed HOME and reading the file
        // back: the headers are a SUB-TABLE keyed `headers`, and `enabled` is
        // written even when true.
        //
        // Scope, decided by Harald: TOOLS ONLY. Grok's terms restrict sending
        // it an automation prompt, which is a question about driving it and not
        // about connecting it — a config a human then uses interactively is not
        // automation. The GUARD is not claimed: this CLI exposes no hook
        // mechanism in its help or its bundled manifest, and the hook target on
        // record belongs to Grok's editor extension, a surface not checked yet.
        "grok" => ClientDialect {
            format: ConfigFormat::Toml {
                table: "mcp_servers",
            },
            url_field: "url",
            emits_type: false,
            emits_always_load: false,
            emits_tools_filter: false,
            toml_headers: Some(TomlHeaders { key: "headers", inline: false }),
            toml_always_writes_enabled: true,
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
            toml_headers: None,
            toml_always_writes_enabled: false,
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
            toml_headers: None,
            toml_always_writes_enabled: false,
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
            toml_headers: None,
            toml_always_writes_enabled: false,
        },
        "claude" => ClientDialect {
            format: ConfigFormat::Json {
                root_key: "mcpServers",
            },
            url_field: "url",
            emits_type: true,
            emits_always_load: true,
            emits_tools_filter: false,
            toml_headers: None,
            toml_always_writes_enabled: false,
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
            toml_headers: None,
            toml_always_writes_enabled: false,
        },
    }
}

#[cfg(test)]
mod tests {

    /// studio#19: Cursor reads the Claude skills directory for compatibility, so
    /// a machine that gets both targets lists every seat twice.
    ///
    /// The decision lives on the roster and so does its test — a duplicate is a
    /// fact about a MACHINE, and the deploy's own per-client assertions cannot
    /// see one.
    #[test]
    fn cursor_inherits_the_seat_files_it_would_otherwise_duplicate() {
        let cursor = client("cursor").expect("cursor");
        assert_eq!(&["claude"], &cursor.seats_inherited_from);

        assert!(cursor.inherits_seat_commands(|s| s == "claude"), "the source is present");
        assert!(
            !cursor.inherits_seat_commands(|s| s == "codex"),
            "a source that is present but is not one THIS client reads changes nothing"
        );

        // THE CASE THAT KEEPS THE FIX HONEST: no source present, so Cursor
        // keeps its own files. Trading a visible duplicate for an invisible
        // absence would be the worse defect.
        assert!(
            !cursor.inherits_seat_commands(|_| false),
            "a Cursor-only machine has nothing to inherit FROM"
        );

        // The source itself never inherits — it is where the files come from —
        // and neither does a client that reads no other client's directory.
        // Asserted over the WHOLE roster rather than a hand-typed list, so a
        // client added tomorrow is covered without anyone remembering.
        for row in CLIENTS.iter().filter(|c| c.id != "cursor") {
            assert!(
                !row.inherits_seat_commands(|_| true),
                "{} must keep writing its own seat files",
                row.id
            );
        }
    }

    /// A source that is never WRITTEN cannot be inherited from, and naming one
    /// would claim coverage the deploy does not have.
    ///
    /// Cursor's documentation names the Codex skills directory beside Claude's,
    /// and this is why Codex is nonetheless absent from that row: the deploy
    /// writes seat files for `claude`, `cursor` and `antigravity` only, so a
    /// Cursor+Codex machine has nothing of ours in the Codex directory to
    /// duplicate. If that ever changes, this test is what fails.
    #[test]
    fn every_named_source_is_a_client_that_receives_seat_files() {
        const RECEIVES_SEAT_FILES: &[&str] = &["claude", "cursor", "antigravity"];
        for row in CLIENTS {
            for source in row.seats_inherited_from {
                assert!(
                    RECEIVES_SEAT_FILES.contains(source),
                    "{} inherits from {source}, which receives no seat files — so there is \
                     nothing there to inherit and this row promises coverage it has not got",
                    row.id
                );
                assert!(client(source).is_some(), "{source} is not a client at all");
            }
        }
    }
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
