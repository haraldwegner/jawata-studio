//! D22 (Sprint 28h): the tools nobody drives. A tool the engine publishes and nothing reaches is built
//! and invisible: "wired" means a driver reaches it. A driver is one of three things, and each is read
//! from text that exists, never listed from memory:
//!
//! - **a seat**: a seat file whose body writes the tool in backticks (the same reading the deploy floor
//!   uses to decide what a skill names) AND whose own `tools:` list carries it, so the seat both is told
//!   to reach for it and is allowed to;
//! - **the hook**: a refusal of a shell habit that names the tool that does the job, in the hook's own
//!   text (the loop for lookups, where the agent's own client is the loop);
//! - **the product**: a step the product runs itself (a gate step of a hire it makes), or the engine
//!   itself, which loads the workspace.
//!
//! The eleven tools that had no driver on 2026-11-01 are derived, not remembered: the vendored engine
//! surface (`engine-surface.json`, 44 live tools) against the seat files (32 named in one), less
//! `health_check`, which the stop-judge hire already runs as its gate step — which leaves the eleven the
//! sprint's number names. This module keeps that derivation true: the test `no_published_tool_is_left_
//! without_a_driver` turns red for ANY engine tool, now or when the surface is refreshed, that nothing
//! reaches, and names it.
//!
//! `load_project` is driven by the engine and is deliberately not offered to any seat: calling it
//! re-installs the workspace, and on a studio-managed resident it dropped four of five loaded projects.
//! The tool stays hidden from agents; the workspace is loaded at engine boot and by the studio's managed
//! start. The test pins that no seat names it.

use std::collections::BTreeSet;

use crate::conductor::EMBEDDED_SEATS;
use crate::engine_floor::{surface_verbs, verbs_named};

/// What reaches a tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Driver {
    /// A seat file writes the tool and carries it in its `tools:` list. Holds the seat file's name.
    Seat(&'static str),
    /// The hook's own text names the tool in a refusal that steers an agent to it.
    Hook,
    /// A step the product runs itself (a gate of a hire it makes).
    Product,
    /// The engine itself: it loads the workspace at boot, so no agent is told to.
    Engine,
}

/// A driver that is not a seat: the file that holds its evidence and the sentence that must be in it.
pub struct Row {
    pub tool: &'static str,
    pub driver: Driver,
    /// `""` for the engine, whose evidence is outside this repository and is a decision, not a quotation.
    pub file: &'static str,
    /// The text that must appear in `file`, so the row says something the code does.
    pub evidence: &'static str,
}

/// The drivers that are not a seat naming a tool. Kept short on purpose: a seat is read, not listed.
pub const ROWS: &[Row] = &[
    Row {
        tool: "health_check",
        driver: Driver::Product,
        file: "jawata-hook/src/judge.rs",
        evidence: r#""steps": [{ "tool": "health_check", "arguments": {} }]"#,
    },
    Row {
        tool: "load_project",
        driver: Driver::Engine,
        file: "",
        evidence: "the workspace is loaded at engine boot and by the studio's managed start; the tool stays hidden from agents because it re-installs the workspace",
    },
];

/// The text of a file this module cites as evidence.
fn text_of(file: &str) -> &'static str {
    match file {
        "jawata-hook/src/judge.rs" => include_str!("../jawata-hook/src/judge.rs"),
        _ => "",
    }
}

/// The `tools:` list of a seat file's front matter.
fn seat_tools(seat_text: &str) -> BTreeSet<String> {
    seat_text
        .lines()
        .find_map(|l| l.strip_prefix("tools:"))
        .map(|rest| {
            rest.trim()
                .trim_start_matches('[')
                .trim_end_matches(']')
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// The seat that drives `tool`. First choice: one whose body writes it AND whose own `tools:` list
/// carries it (it is told to reach for it and is allowed to). Otherwise: a seat whose body writes it
/// as the tool of an instruction, when some seat carries it in its `tools:` list (the chief engineer
/// writes `rename_symbol` into an instruction and the worker's list carries it).
fn seat_driving(tool: &str, universe: &BTreeSet<String>) -> Option<&'static str> {
    let namer = |own_tools_too: bool| {
        EMBEDDED_SEATS.iter().find_map(|(name, text)| {
            let named = verbs_named(text, universe).contains(tool);
            (named && (!own_tools_too || seat_tools(text).contains(tool))).then_some(*name)
        })
    };
    if let Some(seat) = namer(true) {
        return Some(seat);
    }
    let carried = EMBEDDED_SEATS.iter().any(|(_, text)| seat_tools(text).contains(tool));
    if carried { namer(false) } else { None }
}

/// The strictest reading: a seat whose body writes `tool` in backticks AND whose own `tools:` list
/// carries it. The eleven of D22 are held to this one, so taking either half away turns them red.
pub fn strict_seat(tool: &str) -> Option<&'static str> {
    let universe = surface_verbs();
    EMBEDDED_SEATS.iter().find_map(|(name, text)| {
        (verbs_named(text, &universe).contains(tool) && seat_tools(text).contains(tool)).then_some(*name)
    })
}

/// Whether `text` carries `tool` as a whole word (not inside a longer identifier).
fn names_as_word(text: &str, tool: &str) -> bool {
    let bytes = text.as_bytes();
    let boundary = |b: u8| !(b.is_ascii_alphanumeric() || b == b'_');
    text.match_indices(tool).any(|(at, _)| {
        let before = at == 0 || boundary(bytes[at - 1]);
        let end = at + tool.len();
        let after = end >= bytes.len() || boundary(bytes[end]);
        before && after
    })
}

/// The first seat whose text names `tool` as a whole word. This is the loosest reading of "a seat
/// reaches it", and it is the one a tool whose name is also a plain English word (`data`, `field`,
/// `project`, `debug`, `hire`) can only be held to: the backticked reading cannot tell the tool from
/// the word. The eleven of D22 all have distinctive names and are held to the strict reading by
/// `the_eleven_each_have_a_named_driver`.
fn seat_naming(tool: &str) -> Option<&'static str> {
    EMBEDDED_SEATS.iter().find(|(_, text)| names_as_word(text, tool)).map(|(name, _)| *name)
}

/// The named driver of `tool`, or `None` when nothing reaches it.
pub fn driver_of(tool: &str) -> Option<Driver> {
    let universe = surface_verbs();
    if let Some(seat) = seat_driving(tool, &universe) {
        return Some(Driver::Seat(seat));
    }
    if let Some(row) = ROWS.iter().find(|r| r.tool == tool) {
        return Some(row.driver.clone());
    }
    seat_naming(tool).map(Driver::Seat)
}

/// The published tools nothing reaches.
pub fn undriven() -> BTreeSet<String> {
    surface_verbs().into_iter().filter(|t| driver_of(t).is_none()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_published_tool_is_left_without_a_driver() {
        let left = undriven();
        assert!(left.is_empty(), "published engine tools that nothing reaches (D22): {left:?}");
    }

    #[test]
    fn every_row_says_something_the_files_say() {
        for row in ROWS {
            if row.driver == Driver::Engine {
                continue;
            }
            assert!(
                text_of(row.file).contains(row.evidence),
                "the driver row for {} cites {} but its text does not carry: {}",
                row.tool,
                row.file,
                row.evidence
            );
        }
    }

    #[test]
    fn the_eleven_each_have_a_named_driver() {
        for (tool, want) in [
            ("find_duplicate_code", "architect.md"),
            ("find_modernization", "architect.md"),
            ("find_tests", "test-writer.md"),
            ("get_at_position", "debugger.md"),
            ("go_to_definition", "debugger.md"),
            ("find_string_literals", "chief-engineer.md"),
            ("find_pattern_usages", "chief-engineer.md"),
            ("find_field_writes", "chief-engineer.md"),
            ("validate_syntax", "chief-engineer.md"),
            ("refresh_workspace", "pm.md"),
        ] {
            assert_eq!(strict_seat(tool), Some(want), "{tool}: a seat must write it in backticks and carry it in its tools: list");
            assert_eq!(driver_of(tool), Some(Driver::Seat(want)), "{tool}");
        }
        assert_eq!(driver_of("load_project"), Some(Driver::Engine));
        assert_eq!(driver_of("health_check"), Some(Driver::Product));
    }

    /// `load_project` re-installs the workspace; a seat that names it would teach an agent to drop the
    /// projects the studio loaded. The decision is that no seat names it.
    #[test]
    fn load_project_is_driven_by_the_engine_and_no_seat_names_it() {
        let universe = surface_verbs();
        for (name, text) in EMBEDDED_SEATS.iter() {
            assert!(!verbs_named(text, &universe).contains("load_project"), "{name} names load_project");
            assert!(!seat_tools(text).contains("load_project"), "{name} offers load_project");
        }
    }

    #[test]
    fn a_tool_a_seat_names_but_is_not_allowed_to_call_is_not_driven_by_that_seat() {
        let text = "---\ntools: [analyze]\n---\nUse `find_tests` first.";
        let universe = surface_verbs();
        assert!(verbs_named(text, &universe).contains("find_tests"));
        assert!(!seat_tools(text).contains("find_tests"));
    }
}
