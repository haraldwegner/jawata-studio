//! The role table.
//!
//! Role and client dialect are DATA here, not two axes of Strategy classes.
//! Ten generated shell scripts were the alternative, and their cost was
//! precise: when a contract changed, one of the ten kept the retired one and
//! nothing said so. A table has one place to be wrong.
//!
//! The cells that do NOT exist are rows too. Cursor has no equivalent of
//! Claude's tool-call recall and no stop gate, and its prompt hook cannot
//! inject context at all — those are [`Availability::Absent`] and
//! `can_inject: false` entries, visible in the same list as everything else,
//! rather than four files nobody notices are missing.

/// Which client we are speaking to. The dialects differ in more than wording:
/// Cursor wants a relative command path, no `type` field, an explicit
/// `timeout`, `failClosed`, and a `matcher`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Client {
    ClaudeCode,
    Cursor,
}

/// What this binary is being asked to do, taken from `argv[0]`.
///
/// The role is the NAME the binary was invoked as, never an argument. On
/// Windows the tokenization of a client's `command` string is unspecified —
/// whether it goes through `cmd.exe`, PowerShell or a direct spawn is not
/// published — so a design needing `"<path> <role>"` to split our way rests on
/// unspecified behaviour on the one platform D-SHIM exists to serve. With no
/// argument, what remains is "can the client execute a path", which is what
/// ships today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Session start: inject the domain primer.
    Primer,
    /// A prompt was submitted: recall against its cues.
    UserPrompt,
    /// A tool call is about to run: recall against its target.
    ToolRecall,
    /// A shell command is about to run: allow or deny (local policy, no store).
    Guard,
    /// A tool call finished: record, emit nothing.
    Observer,
    /// The agent is about to stop: the stop gate.
    Stop,
}

/// Whether a role exists for a client at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    /// The client fires this event and we handle it.
    Handled { event: &'static str },
    /// The client has no equivalent event. Recorded so the gap is READABLE:
    /// an absent capability that no one wrote down becomes an absent
    /// capability no one remembers.
    Absent { because: &'static str },
}

/// Which of the three concerns a role uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Concerns {
    /// Extract cues from the payload (only the prompt-shaped roles).
    pub cue: bool,
    /// Ask the store.
    pub query: bool,
    /// Write something the client will read back.
    pub emit: bool,
}

/// One row: a role, for a client.
#[derive(Debug, Clone, Copy)]
pub struct RoleSpec {
    pub role: Role,
    pub client: Client,
    pub availability: Availability,
    pub concerns: Concerns,
    /// Whether this hook can put text into the model's context. Cursor's
    /// prompt hook CANNOT — it is side-effect only — which is a property of
    /// the client, not of our code, and the reason a Cursor recall records
    /// without injecting.
    pub can_inject: bool,
    /// The filename the deploy writes, which is also how `argv[0]` names the
    /// role. Sprint 28 renames these from the `*.sh` generation, so the old
    /// names join the legacy-removal set.
    pub binary_name: &'static str,
}

const fn c(cue: bool, query: bool, emit: bool) -> Concerns {
    Concerns { cue, query, emit }
}

/// THE TABLE. Ten rows: six Claude Code, four Cursor — matching exactly what
/// the deploy writes — plus the two Cursor absences.
pub const ROLES: &[RoleSpec] = &[
    // ---- Claude Code ----
    RoleSpec {
        role: Role::Primer,
        client: Client::ClaudeCode,
        availability: Availability::Handled { event: "SessionStart" },
        concerns: c(false, true, true),
        can_inject: true,
        binary_name: "jawata-hook-primer",
    },
    RoleSpec {
        role: Role::UserPrompt,
        client: Client::ClaudeCode,
        availability: Availability::Handled { event: "UserPromptSubmit" },
        concerns: c(true, true, true),
        can_inject: true,
        binary_name: "jawata-hook-userprompt",
    },
    RoleSpec {
        role: Role::ToolRecall,
        client: Client::ClaudeCode,
        availability: Availability::Handled { event: "PreToolUse" },
        concerns: c(true, true, true),
        can_inject: true,
        binary_name: "jawata-hook-recall",
    },
    RoleSpec {
        role: Role::Guard,
        client: Client::ClaudeCode,
        availability: Availability::Handled { event: "PreToolUse" },
        // No query: the guard is a LOCAL policy decision. It must answer while
        // the resident is down, so it never asks anyone.
        concerns: c(false, false, true),
        can_inject: true,
        binary_name: "jawata-hook-guard",
    },
    RoleSpec {
        role: Role::Observer,
        client: Client::ClaudeCode,
        availability: Availability::Handled { event: "PostToolUse" },
        concerns: c(false, true, false),
        can_inject: false,
        binary_name: "jawata-hook-observer",
    },
    RoleSpec {
        role: Role::Stop,
        client: Client::ClaudeCode,
        availability: Availability::Handled { event: "Stop" },
        concerns: c(false, false, true),
        can_inject: true,
        binary_name: "jawata-hook-stop",
    },
    // ---- Cursor ----
    RoleSpec {
        role: Role::Primer,
        client: Client::Cursor,
        availability: Availability::Handled { event: "sessionStart" },
        concerns: c(false, true, true),
        can_inject: true,
        binary_name: "jawata-hook-primer",
    },
    RoleSpec {
        role: Role::UserPrompt,
        client: Client::Cursor,
        availability: Availability::Handled { event: "beforeSubmitPrompt" },
        // Queries, records, emits NOTHING back into context.
        concerns: c(true, true, false),
        can_inject: false,
        binary_name: "jawata-hook-userprompt",
    },
    RoleSpec {
        role: Role::Guard,
        client: Client::Cursor,
        availability: Availability::Handled { event: "beforeShellExecution" },
        concerns: c(false, false, true),
        can_inject: true,
        binary_name: "jawata-hook-guard",
    },
    RoleSpec {
        role: Role::Observer,
        client: Client::Cursor,
        availability: Availability::Handled { event: "afterMCPExecution" },
        concerns: c(false, true, false),
        can_inject: false,
        binary_name: "jawata-hook-observer",
    },
    // ---- Cursor's two empty cells, written down ----
    RoleSpec {
        role: Role::ToolRecall,
        client: Client::Cursor,
        availability: Availability::Absent {
            because: "Cursor fires no before-a-tool-call event; afterMCPExecution is the \
                      only tool-shaped hook and it runs too late to recall INTO the call",
        },
        concerns: c(false, false, false),
        can_inject: false,
        binary_name: "jawata-hook-recall",
    },
    RoleSpec {
        role: Role::Stop,
        client: Client::Cursor,
        availability: Availability::Absent {
            because: "Cursor has no agent-stop event, so the stop gate cannot exist there; \
                      the discipline it enforces is carried by the rule block instead",
        },
        concerns: c(false, false, false),
        can_inject: false,
        binary_name: "jawata-hook-stop",
    },
];

/// Resolve `argv[0]` to a role. Returns `None` for a name we do not own —
/// which is a fact worth recording, not a reason to guess.
impl Role {
    /// The stable log/diagnostic tag for this role.
    ///
    /// A separate accessor rather than `Debug`, for the reason the silence
    /// tags are separate too: a `Debug` rendering ties the on-disk format to
    /// the variant's Rust identifier, so a rename silently rewrites every
    /// record and breaks every grep that reads them. The match is exhaustive,
    /// so a new role cannot be added without naming it here.
    pub fn name(self) -> &'static str {
        match self {
            Role::Primer => "primer",
            Role::UserPrompt => "user-prompt",
            Role::ToolRecall => "tool-recall",
            Role::Guard => "guard",
            Role::Observer => "observer",
            Role::Stop => "stop",
        }
    }
}

pub fn role_for_binary(argv0: &str) -> Option<Role> {
    // BOTH separators, on every platform, deliberately — not
    // `Path::file_name`. That resolves per HOST: on Linux a backslash is an
    // ordinary character, so `C:\...\jawata-hook-primer.exe` comes back whole
    // and resolves to nothing. Since role dispatch is the mechanism the
    // Windows leg depends on, host-dependent parsing would make it exactly the
    // behaviour our Linux CI cannot test — and Windows is the platform D-SHIM
    // exists to serve.
    let name = argv0.rsplit(['/', '\\']).next().unwrap_or(argv0);
    // Windows deploys copies (which carry .exe); Unix uses symlinks/hardlinks.
    let name = name.strip_suffix(".exe").unwrap_or(name);
    ROLES.iter().find(|r| r.binary_name == name).map(|r| r.role)
}

/// The row for a role on a client. Every (role, client) pair has one — the
/// absences included.
pub fn spec(role: Role, client: Client) -> Option<&'static RoleSpec> {
    ROLES.iter().find(|r| r.role == role && r.client == client)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_matches_what_the_deploy_writes() {
        // Six Claude entries and four Cursor entries are HANDLED — the exact
        // count the deploy writes today. If a role is added to the deploy and
        // not here, or here and not there, this is where it shows.
        let handled = |client: Client| {
            ROLES
                .iter()
                .filter(|r| r.client == client)
                .filter(|r| matches!(r.availability, Availability::Handled { .. }))
                .count()
        };
        assert_eq!(6, handled(Client::ClaudeCode), "Claude Code entries");
        assert_eq!(4, handled(Client::Cursor), "Cursor entries");
    }

    #[test]
    fn every_role_has_a_row_for_every_client_including_the_absences() {
        // The point of the table: a missing capability is a ROW, so nobody has
        // to notice a file that was never written.
        for role in [
            Role::Primer,
            Role::UserPrompt,
            Role::ToolRecall,
            Role::Guard,
            Role::Observer,
            Role::Stop,
        ] {
            for client in [Client::ClaudeCode, Client::Cursor] {
                assert!(
                    spec(role, client).is_some(),
                    "no row for {role:?} on {client:?} — an unwritten cell is how a gap \
                     stops being visible"
                );
            }
        }
    }

    #[test]
    fn an_absence_carries_its_reason() {
        for r in ROLES {
            if let Availability::Absent { because } = r.availability {
                // A length check would accept twenty spaces (C5 audit F7). The
                // obligation is that the reason names the CLIENT's limitation,
                // so a reader can tell "they have no such event" from "we have
                // not built it yet".
                assert!(
                    because.contains("Cursor") || because.contains("no "),
                    "{:?}/{:?} must say what the client lacks, got: {because:?}",
                    r.role,
                    r.client
                );
                assert!(!r.concerns.cue && !r.concerns.query && !r.concerns.emit,
                    "{:?}/{:?} is absent but claims concerns", r.role, r.client);
                assert!(!r.can_inject, "{:?}/{:?} is absent but claims injection", r.role, r.client);
            }
        }
    }

    #[test]
    fn cursors_prompt_hook_queries_but_cannot_inject() {
        // The one asymmetry that is a property of the CLIENT, not of our code,
        // and the reason a Cursor recall records without injecting.
        let cursor = spec(Role::UserPrompt, Client::Cursor).unwrap();
        assert!(cursor.concerns.query, "it still asks the store");
        assert!(!cursor.can_inject, "beforeSubmitPrompt cannot put text into context");
        assert!(!cursor.concerns.emit, "so it must not try to emit context");

        let claude = spec(Role::UserPrompt, Client::ClaudeCode).unwrap();
        assert!(claude.can_inject, "Claude's UserPromptSubmit can");
        assert!(claude.concerns.emit);
    }

    #[test]
    fn the_guard_never_queries_the_store() {
        // It must answer while the resident is down. A guard that asked and
        // failed open would leak exactly the calls it exists to deny.
        for client in [Client::ClaudeCode, Client::Cursor] {
            let g = spec(Role::Guard, client).unwrap();
            assert!(!g.concerns.query, "{client:?} guard must decide locally");
            assert!(g.concerns.emit, "{client:?} guard emits a permission");
        }
    }

    #[test]
    fn every_deployed_role_name_resolves_its_own_role() {
        // C6 exit clause 3: "argv[0] dispatch is asserted — a table-driven test
        // over all six role-named entry points, each resolving its role from
        // the name it was invoked as. This is the design's decisive decision;
        // ungated, an argument scheme could ship instead."
        //
        // Six DISTINCT names, six DISTINCT roles, checked both ways: every name
        // resolves, and no two names resolve to the same role. A table where
        // two entries collided would dispatch one role's work on another's
        // event, silently.
        let deployed: Vec<&str> = ROLES
            .iter()
            .filter(|r| r.client == Client::ClaudeCode)
            .map(|r| r.binary_name)
            .collect();
        assert_eq!(6, deployed.len(), "six role-named entry points: {deployed:?}");

        let mut seen = std::collections::HashSet::new();
        for name in &deployed {
            let role = role_for_binary(name)
                .unwrap_or_else(|| panic!("{name} is deployed but resolves no role"));
            assert!(
                seen.insert(format!("{role:?}")),
                "{name} resolves to {role:?}, which another deployed name already claims — \
                 one role's work would run on another's event"
            );
            // And through the shapes a client actually invokes it with.
            for shape in [
                format!("/home/u/.claude/hooks/{name}"),
                format!("./hooks/{name}"),
                format!(r"C:\Users\h\.claude\hooks\{name}.exe"),
            ] {
                assert_eq!(
                    Some(role),
                    role_for_binary(&shape),
                    "{shape} did not resolve to {role:?}"
                );
            }
        }
        assert_eq!(6, seen.len(), "six distinct roles: {seen:?}");
    }

    #[test]
    fn argv0_resolves_the_role_across_platform_shapes() {
        assert_eq!(Some(Role::UserPrompt), role_for_binary("jawata-hook-userprompt"));
        assert_eq!(Some(Role::Guard), role_for_binary("/usr/local/bin/jawata-hook-guard"));
        assert_eq!(Some(Role::Primer), role_for_binary(r"C:\Users\h\jawata-hook-primer.exe"));
        assert_eq!(None, role_for_binary("jawata-hook-nonsense"));
        assert_eq!(None, role_for_binary(""));
    }

    #[test]
    fn a_role_that_can_inject_nothing_does_not_claim_to_emit_context() {
        for r in ROLES {
            if r.concerns.emit && !r.can_inject {
                // The guard is the deliberate exception: it emits a PERMISSION
                // decision, which is not context injection.
                assert_eq!(
                    Role::Guard,
                    r.role,
                    "{:?}/{:?} emits without being able to inject",
                    r.role,
                    r.client
                );
            }
        }
    }
}
