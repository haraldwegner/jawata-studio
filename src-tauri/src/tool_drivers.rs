//! D22 (Sprint 28h): the tools nobody drives. A tool the engine publishes and nothing reaches is built
//! and invisible: "wired" means a driver reaches it. This module is the check, and it is TEST-ONLY: it
//! produces nothing the product runs. It keeps true a fact about the seat files, and says which tool
//! turns it false.
//!
//! ## Where the eleven come from
//!
//! D22's eleven were MEASURED, not chosen: the 28f brainstorm, section "The tools nobody drives", counted
//! the tools published by the built v4.1.0 engine that nothing of ours named, matching STRICTLY (a quoted
//! string in code, or a backticked name in `seats/` and `skills/`; plain-word matches were discarded
//! because `move`, `format`, `field`, `data` and `project` are English and Rust words). It found:
//! `analyze`, `dependency`, `find_duplicate_code`, `find_field_writes`, `find_modernization`,
//! `find_pattern_usages`, `find_string_literals`, `go_to_definition`, `hierarchy`, `refresh_workspace`,
//! `validate_syntax`. [`THE_ELEVEN`] is that list. `find_tests`, `get_at_position` and `health_check` are
//! not in it: the first two were added as drivers by this slice because a seat's job needs them, and
//! `health_check` is named by a quoted string in the stop judge. They are additions, not the contract.
//!
//! ## What "driven" means here: the same strict reading, for every tool
//!
//! The project's own strict reader is [`verbs_named`] (a backticked name written as a call or alone).
//! A tool is driven when
//!
//! - a seat file writes it in backticks AND that seat carries it in its own `tools:` list (the seat is
//!   told to reach for it and is allowed to), or its `tools:` list is empty, which makes it a stance the
//!   host agent executes with the host's own tools (a text seat such as `report`); or
//! - a seat that DECIDES which tool an instruction names (the chief engineer, the pm) writes it in
//!   backticks, and the worker, which RUNS the instruction, carries it (the instruction chain); or
//! - a quoted string in the studio's own code names it as a step the product runs ([`ROWS`]).
//!
//! The eleven are held to one more condition: their driver sentence carries the marker `(D22)` and an
//! exact phrase, so a seat that merely says "never use `find_tests`" is not a driver.
//!
//! ## What is true about `load_project`
//!
//! No seat names or carries it. The engine's own messages DO tell an agent to call it
//! (`ErrorInfo.projectNotLoaded`, "No project loaded. Call load_project first.", `ErrorInfo.java:178`),
//! and the engine loads a project at boot only when the project path setting is set
//! (`JawataApplication.autoLoadProjectFromEnv`, `JawataApplication.java:180-183`; otherwise it logs that it
//! is "waiting for load_project call"). Calling it re-installs the workspace, and on a studio-managed
//! resident it dropped four of five loaded projects, so no seat is told to. It is therefore ACCEPTED as
//! undriven by a seat, with the reason below, and the test pins that the accepted set is exactly that.

use std::collections::BTreeSet;

use crate::conductor::EMBEDDED_SEATS;
use crate::engine_floor::{surface_verbs, verbs_named};

/// D22's eleven, as measured at v4.1.0 (see the module doc).
pub const THE_ELEVEN: [&str; 11] = [
    "analyze",
    "dependency",
    "find_duplicate_code",
    "find_field_writes",
    "find_modernization",
    "find_pattern_usages",
    "find_string_literals",
    "go_to_definition",
    "hierarchy",
    "refresh_workspace",
    "validate_syntax",
];

/// A tool a product step drives, named by a quoted string in the studio's own code.
pub struct Row {
    pub tool: &'static str,
    /// The file the string is in, relative to `src-tauri`.
    pub file: &'static str,
    /// The text that must be in that file.
    pub evidence: &'static str,
}

/// Tools driven by a product step rather than a seat.
pub const ROWS: &[Row] = &[Row {
    tool: "health_check",
    file: "jawata-hook/src/judge.rs",
    evidence: r#""steps": [{ "tool": "health_check", "arguments": {} }]"#,
}];

/// Tools no seat drives, by decision, and why. The test pins this set so it cannot grow quietly.
pub const ACCEPTED_UNDRIVEN: &[(&str, &str)] = &[(
    "load_project",
    "calling it re-installs the workspace (on a studio-managed resident it dropped four of five loaded \
     projects), so no seat is told to; the engine loads a project at boot when its project path setting \
     is set (JawataApplication.java:180-183), and the engine's own error text does tell an agent to call \
     it (ErrorInfo.java:178)",
)];

/// The tools a seat file's front matter lists, and whether it has a `tools:` line at all.
fn tools_line(seat_text: &str) -> Option<BTreeSet<String>> {
    seat_text.lines().find_map(|l| l.strip_prefix("tools:")).map(|rest| {
        rest.trim()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect()
    })
}

/// The tools a seat file's front matter carries.
fn tools_of(seat_text: &str) -> BTreeSet<String> {
    tools_line(seat_text).unwrap_or_default()
}

/// Whether the seat can reach `tool`: it lists it, or it lists NOTHING (`tools: []`), which is a stance
/// the host agent executes with the host's own tools (the front-door shape of a text seat such as
/// `report`), so what the stance writes in backticks is what the host agent is told to call.
fn reaches(seat_text: &str, tool: &str) -> bool {
    tools_line(seat_text).is_some_and(|set| set.is_empty() || set.contains(tool))
}

/// The seats whose job is to DECIDE which tool an instruction names; the worker runs what they write.
const INSTRUCTION_GIVERS: &[&str] = &["chief-engineer.md", "pm.md"];
const INSTRUCTION_RUNNER: &str = "developer.md";

/// Who drives `tool` under the strict reading, over `seats` (name, text). `Some((namer, carrier))`.
pub fn strict_driver(seats: &[(&str, &str)], tool: &str, universe: &BTreeSet<String>) -> Option<(String, String)> {
    for (name, text) in seats {
        if !verbs_named(text, universe).contains(tool) {
            continue;
        }
        if reaches(text, tool) {
            return Some((name.to_string(), name.to_string()));
        }
        if INSTRUCTION_GIVERS.contains(name) {
            let runner = seats.iter().find(|(n, _)| *n == INSTRUCTION_RUNNER);
            if runner.is_some_and(|(_, t)| tools_of(t).contains(tool)) {
                return Some((name.to_string(), INSTRUCTION_RUNNER.to_string()));
            }
        }
    }
    None
}

fn embedded() -> Vec<(&'static str, &'static str)> {
    EMBEDDED_SEATS.iter().map(|(n, t)| (*n, *t)).collect()
}

/// The text of a file a [`Row`] cites.
fn text_of(file: &str) -> &'static str {
    match file {
        "jawata-hook/src/judge.rs" => include_str!("../jawata-hook/src/judge.rs"),
        _ => "",
    }
}

/// Published tools nothing reaches under the strict reading, less the product steps and the accepted set.
pub fn undriven() -> BTreeSet<String> {
    let universe = surface_verbs();
    let seats = embedded();
    universe
        .iter()
        .filter(|t| {
            strict_driver(&seats, t, &universe).is_none()
                && !ROWS.iter().any(|r| r.tool == t.as_str())
                && !ACCEPTED_UNDRIVEN.iter().any(|(a, _)| *a == t.as_str())
        })
        .cloned()
        .collect()
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
    fn every_product_row_says_something_the_code_says() {
        for row in ROWS {
            assert!(
                text_of(row.file).contains(row.evidence),
                "the row for {} cites {} but it does not carry: {}",
                row.tool,
                row.file,
                row.evidence
            );
        }
    }

    /// The eleven, each with the seat that writes it and the phrase that makes it a step. A seat that
    /// says "never use `find_tests`" carries none of these phrases, so a negative sentence is no driver.
    const PHRASES: &[(&str, &str, &str)] = &[
        ("analyze", "chief-engineer.md", "change is `analyze(kind=change_impact)`"),
        ("dependency", "chief-engineer.md", "hierarchy, `dependency` for a build file's"),
        ("find_duplicate_code", "architect.md", "(D22): `find_duplicate_code` over the scope a change"),
        ("find_field_writes", "chief-engineer.md", "argument; `find_field_writes` finds every place"),
        ("find_modernization", "architect.md", "read; `find_modernization` names the code that could adopt"),
        ("find_pattern_usages", "chief-engineer.md", "of; `find_pattern_usages` finds where a type"),
        ("find_string_literals", "chief-engineer.md", "spelling: `find_string_literals` finds the code that emits"),
        ("go_to_definition", "debugger.md", "there, and `go_to_definition` follows a call"),
        ("hierarchy", "chief-engineer.md", "state, `hierarchy` for moving a member up or down"),
        ("refresh_workspace", "pm.md", "exists, call `refresh_workspace` once and repeat"),
        ("validate_syntax", "chief-engineer.md", "next instruction, `validate_syntax` on that file answers"),
    ];

    #[test]
    fn the_spec_eleven_are_each_driven_by_a_real_seat_step() {
        let universe = surface_verbs();
        let seats = embedded();
        assert_eq!(PHRASES.len(), THE_ELEVEN.len());
        for tool in THE_ELEVEN {
            let (_, seat, phrase) = PHRASES.iter().find(|(t, _, _)| *t == tool).unwrap_or_else(|| panic!("no phrase for {tool}"));
            let text = seats.iter().find(|(n, _)| n == seat).map(|(_, t)| *t).unwrap_or_default();
            let flat = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
            assert!(flat(text).contains(&flat(phrase)), "{tool}: {seat} must carry the step `{phrase}`");
            assert!(text.contains("(D22)"), "{tool}: {seat} carries no (D22) marker");
            let (namer, carrier) = strict_driver(&seats, tool, &universe).unwrap_or_else(|| panic!("{tool} has no strict driver"));
            assert_eq!(&namer, seat, "{tool}: the seat that writes it");
            // dependency and hierarchy change code: the chief engineer decides, the worker runs.
            let expected_carrier = if matches!(tool, "dependency" | "hierarchy") { "developer.md" } else { *seat };
            assert_eq!(carrier, expected_carrier, "{tool}: the seat that carries it");
        }
    }

    /// Additions this slice made beyond the spec's eleven, said plainly and held to the same reading.
    #[test]
    fn the_additions_are_driven_too_and_are_not_the_contract() {
        let universe = surface_verbs();
        let seats = embedded();
        for (tool, seat) in [("find_tests", "test-writer.md"), ("get_at_position", "debugger.md")] {
            assert!(!THE_ELEVEN.contains(&tool), "{tool} is an addition, not one of the eleven");
            assert_eq!(strict_driver(&seats, tool, &universe).map(|(n, _)| n).as_deref(), Some(seat), "{tool}");
        }
    }

    /// The half-conditions, on fixtures: a seat that writes a tool but does not carry it, or carries it
    /// but does not write it, drives nothing; a negative sentence still "writes" it, which is why the
    /// eleven also need their exact phrase (`the_spec_eleven_are_each_driven_by_a_real_seat_step`).
    #[test]
    fn a_seat_must_both_write_the_tool_and_carry_it() {
        let universe = surface_verbs();
        let writes_only = "---\ntools: [analyze]\n---\nUse `find_tests` first.";
        let carries_only = "---\ntools: [find_tests]\n---\nNothing about tests here.";
        let both = "---\ntools: [find_tests]\n---\nUse `find_tests` first.";
        assert_eq!(strict_driver(&[("a.md", writes_only)], "find_tests", &universe), None, "writes but does not carry");
        assert_eq!(strict_driver(&[("a.md", carries_only)], "find_tests", &universe), None, "carries but does not write");
        assert_eq!(
            strict_driver(&[("a.md", both)], "find_tests", &universe),
            Some(("a.md".to_string(), "a.md".to_string()))
        );
    }

    /// A text seat (`tools: []`) is a stance the host agent executes with its own tools, so the tools it
/// writes in backticks are driven by it; a file with no `tools:` line at all is no seat and drives nothing.
#[test]
fn a_stance_with_an_empty_tools_list_drives_what_it_writes_and_a_file_without_the_line_drives_nothing() {
    let universe = surface_verbs();
    let stance = "---\ntools: []\n---\nCall `find_tests` first.";
    let no_line = "---\nname: x\n---\nCall `find_tests` first.";
    assert_eq!(strict_driver(&[("report.md", stance)], "find_tests", &universe), Some(("report.md".to_string(), "report.md".to_string())));
    assert_eq!(strict_driver(&[("x.md", no_line)], "find_tests", &universe), None);
}

/// An instruction giver that names a change tool the worker does not carry drives nothing; one the
    /// worker carries is driven through the chain.
    #[test]
    fn the_instruction_chain_needs_the_worker_to_carry_the_tool() {
        let universe = surface_verbs();
        let giver = "---\ntools: [analyze]\n---\nName `hierarchy` when a member moves.";
        let worker_without = "---\ntools: [extract]\n---\n";
        let worker_with = "---\ntools: [hierarchy]\n---\n";
        assert_eq!(strict_driver(&[("chief-engineer.md", giver), ("developer.md", worker_without)], "hierarchy", &universe), None);
        assert_eq!(
            strict_driver(&[("chief-engineer.md", giver), ("developer.md", worker_with)], "hierarchy", &universe),
            Some(("chief-engineer.md".to_string(), "developer.md".to_string()))
        );
        // a seat that is not an instruction giver does not drive through the worker
        assert_eq!(strict_driver(&[("debugger.md", giver), ("developer.md", worker_with)], "hierarchy", &universe), None);
    }

    /// `load_project` is accepted undriven, with its reason; the accepted set is exactly that, and no
    /// seat names or carries it.
    #[test]
    fn load_project_is_accepted_undriven_and_no_seat_names_or_carries_it() {
        let accepted: Vec<&str> = ACCEPTED_UNDRIVEN.iter().map(|(t, _)| *t).collect();
        assert_eq!(accepted, vec!["load_project"], "the accepted set must not grow quietly");
        assert!(ACCEPTED_UNDRIVEN[0].1.contains("ErrorInfo.java:178"), "the reason says the engine's own message tells agents to call it");
        let universe = surface_verbs();
        for (name, text) in embedded() {
            assert!(!verbs_named(text, &universe).contains("load_project"), "{name} names load_project");
            assert!(!tools_of(text).contains("load_project"), "{name} carries load_project");
        }
    }
}
