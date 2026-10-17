//! The engine's tool surface, as the studio vendors it - the one place the hook learns which of the
//! engine's calls change code.
//!
//! The hook kept three hand-written lists of those names, and they disagreed (REL44-37): two named
//! tools the engine had retired and omitted four it publishes, and the third was a different list for a
//! different question. Worse, they were matched against the name AS THE CLIENT WRITES IT, and Claude Code
//! writes an MCP tool as `mcp__<server>__<tool>`, so no engine tool ever matched in a transcript at all.
//!
//! `engine-surface.json` (beside `Cargo.toml` of the studio crate) is taken FROM the published engine by
//! `build/engine-surface.sh`, which asks a running engine for `tools/list` and reads the flag it publishes
//! per tool, `_meta."jawata/changesCode"` (the engine's own `Tool.changesCode()`). The studio release job
//! asks the engine again and fails when the committed file differs. This module only reads the file.

use std::collections::HashSet;
use std::sync::OnceLock;

/// The vendored file, compiled in: a hook has no network and no engine to ask at the moment it fires.
const SURFACE_JSON: &str = include_str!("../../engine-surface.json");

/// Names of the engine's live tools whose own flag says they change code.
fn changing() -> &'static HashSet<String> {
    static CHANGING: OnceLock<HashSet<String>> = OnceLock::new();
    CHANGING.get_or_init(|| {
        let json: serde_json::Value = serde_json::from_str(SURFACE_JSON).unwrap_or_default();
        json["live"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .filter(|row| row["changesCode"].as_bool() == Some(true))
                    .filter_map(|row| row["name"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// The tool's own name, without the prefix a client puts in front of it: Claude Code's
/// `mcp__<server>__<tool>`, or the bare name another client passes. Engine tool names hold no double
/// underscore, so the last segment is the name.
pub(crate) fn bare_name(raw: &str) -> &str {
    raw.rsplit("__").next().unwrap_or(raw)
}

/// Whether this call is to one of the engine's tools that changes code. A retired name is not: the engine
/// answers it not-found, so it changed nothing. The host's own editing tools (`Edit`, `Write`) are not the
/// engine's and are the caller's to name.
pub(crate) fn changes_code(raw_tool_name: &str) -> bool {
    changing().contains(bare_name(raw_tool_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vendored_surface_is_the_engines_and_not_empty() {
        let json: serde_json::Value = serde_json::from_str(SURFACE_JSON).expect("engine-surface.json is JSON");
        let live = json["live"].as_array().expect("a live array").len();
        let retired = json["retired"].as_array().expect("a retired array").len();
        assert!(live >= 30, "{live} live tools: the file was truncated");
        assert!(retired >= 60, "{retired} retired names: the file was truncated");
        assert!(changing().len() >= 10, "{} tools flagged as changing code", changing().len());
    }

    #[test]
    fn the_name_is_read_under_every_spelling_a_client_uses() {
        assert!(changes_code("rename_symbol"));
        assert!(changes_code("mcp__jawata-javata-dev__rename_symbol"));
        assert!(changes_code("mcp__jawata-patterns__hierarchy"));
        assert!(!changes_code("mcp__jawata-javata-dev__compile_workspace"));
        assert!(!changes_code("mcp__jawata-javata-dev__experience"));
        assert!(!changes_code("move_method"), "retired: the engine answers it not-found");
        assert!(!changes_code("Edit"), "the host's tools are the caller's to name");
        assert!(!changes_code(""));
    }

    #[test]
    fn a_tool_that_changes_code_is_never_also_retired() {
        let json: serde_json::Value = serde_json::from_str(SURFACE_JSON).unwrap();
        let retired: HashSet<&str> = json["retired"].as_array().unwrap().iter().filter_map(|n| n.as_str()).collect();
        for name in changing() {
            assert!(!retired.contains(name.as_str()), "{name} is both live and retired in the vendored file");
        }
    }
}
