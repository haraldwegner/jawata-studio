//! The conductor delivery (Sprint 25a): generates every per-client seat
//! artifact from `seats/*.md` — the single source. Pure renderers, no I/O
//! except `materialize_seats`; the deploy layer in `manager_service.rs` is
//! the only writer into client trees. Architecture: ARCHITECTURE-conductor.md.

use crate::runner::{parse_seat_definition, GateClass, SeatDefinition};
use std::fs;
use std::path::{Path, PathBuf};

/// The seven seat definitions shipped in the binary. Materialized into
/// `<config>/seats/` where absent; the materialized copy wins so a
/// user-edited seat regenerates every channel on redeploy.
pub const EMBEDDED_SEATS: [(&str, &str); 7] = [
    ("architect.md", include_str!("../../seats/architect.md")),
    ("debugger.md", include_str!("../../seats/debugger.md")),
    ("javadoc-writer.md", include_str!("../../seats/javadoc-writer.md")),
    ("profiler.md", include_str!("../../seats/profiler.md")),
    ("spec-auditor.md", include_str!("../../seats/spec-auditor.md")),
    ("spec-editor.md", include_str!("../../seats/spec-editor.md")),
    ("test-writer.md", include_str!("../../seats/test-writer.md")),
];

/// seat name → (command name, one-line description). Exactly the five
/// command-bearing seats; spec-editor/spec-auditor live in /sprint and
/// render NO command (pinned by test).
pub const COMMAND_MAP: [(&str, &str, &str); 5] = [
    (
        "javadoc-writer",
        "javadocs",
        "Document undocumented public Java API from compiler facts (jawata javadoc-writer seat)",
    ),
    (
        "test-writer",
        "cover",
        "Write gate-verified characterization tests for uncovered code (jawata test-writer seat)",
    ),
    (
        "architect",
        "refactor",
        "Architecture review and parity-gated refactoring proposals (jawata architect seat)",
    ),
    (
        "debugger",
        "debug",
        "Disciplined debugging: recall, one discriminating probe, verify (jawata debugger seat)",
    ),
    (
        "profiler",
        "profile",
        "Profile a JVM and name hotspots as compiler-accurate symbols (jawata profiler seat)",
    ),
];

pub fn command_for(seat_name: &str) -> Option<(&'static str, &'static str)> {
    COMMAND_MAP
        .iter()
        .find(|(seat, _, _)| *seat == seat_name)
        .map(|(_, cmd, desc)| (*cmd, *desc))
}

/// Writes the embedded seats into `seats_dir`, absent-only: an existing
/// file is NEVER overwritten (config wins — a user edit survives every
/// redeploy). Returns the paths actually written.
pub fn materialize_seats(seats_dir: &Path) -> Result<Vec<PathBuf>, String> {
    fs::create_dir_all(seats_dir)
        .map_err(|e| format!("cannot create seats dir {}: {e}", seats_dir.display()))?;
    let mut written = Vec::new();
    for (file, body) in EMBEDDED_SEATS {
        let path = seats_dir.join(file);
        if !path.exists() {
            fs::write(&path, body).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
            written.push(path);
        }
    }
    Ok(written)
}

/// The Lane-1 stance-handoff contract embedded in every generated command:
/// the front-door agent runs the seat's loop ITSELF, with real jawata calls
/// as the gates. Propose-mode is taught, not enforced (R3 — enforcement is
/// Sprint 26's injector).
fn lane1_contract(seat: &SeatDefinition, command: &str) -> String {
    let gates = gate_call_lines(seat);
    format!(
        "## The loop (binding — you run the seat yourself)\n\n\
         You are the front-door agent executing the jawata **{name}** seat by\n\
         stance handoff. Work the loop, in order, with REAL jawata MCP calls:\n\n\
         1. DETECT — find the work with the seat's own detectors (jawata\n\
            tools), scoped to what the user named. Never invent targets.\n\
         2. DO — produce the seat's output per the stance below.\n\
         3. VERIFY — the gates are jawata calls you MUST make and read:\n\
         {gates}\
         4. PROPOSE — present the result as a proposal (diff or files) and\n\
            WAIT for the human yes. Never auto-apply, never commit on your\n\
            own. A gate you could not run has NOT passed — say so.\n\
         5. RECORD — after the human verdict, record the outcome to the\n\
            experience store: `experience(kind=record, type=lesson|domain_fact,\n\
            operation=\"seat:{command}\", summary=<what was proposed, the gate\n\
            results, the verdict>)`.\n\n\
         ## Execution context\n\n\
         Unlike the hosted runner, YOU perform detection and gates yourself.\n\
         Where the stance below assumes a hosted harness (e.g. \"you do not\n\
         use tools\"), the loop above wins on MECHANICS; the stance's content\n\
         rules bind unchanged.\n",
        name = seat.name,
        gates = gates,
    )
}

/// Renders the seat's gate classes as named jawata-call bullet lines
/// (indented under VERIFY).
fn gate_call_lines(seat: &SeatDefinition) -> String {
    let mut lines = vec![
        "   - always: `compile_workspace` clean on the touched scope, and the\n\
         change stays inside the named scope (purity — nothing else touched).\n"
            .to_string(),
    ];
    for gate in &seat.gate_classes {
        match gate {
            GateClass::Always => {}
            GateClass::Behavior => lines.push(
                "   - behavior: prove behavior preservation — `run_tests` green on\n\
                 the touched scope; structural changes go through\n\
                 `refactoring(action=plan)` + `apply_plan` (parity-gated).\n"
                    .to_string(),
            ),
            GateClass::Tests => lines.push(
                "   - tests: the new tests compile and pass (`run_tests`, repeated\n\
                 — a flaky pass is a fail), coverage measurably improves\n\
                 (`run_tests coverage=true` before/after), and where mutation\n\
                 support exists at least one previously-surviving mutant dies\n\
                 (`run_tests action=coverage_mutation`).\n"
                    .to_string(),
            ),
            GateClass::Docs => lines.push(
                "   - docs: the documented files compile doclint-clean (javadoc\n\
                 `-Xdoclint:all` on touched files) and `get_diagnostics` reports\n\
                 no new warnings.\n"
                    .to_string(),
            ),
        }
    }
    lines.join("")
}

/// Claude Code skill: `.claude/skills/<command>/SKILL.md` — frontmatter
/// (name, description) + the Lane-1 contract + the seat stance verbatim.
/// `None` for seats without a command mapping.
pub fn render_claude_skill(seat: &SeatDefinition) -> Option<String> {
    let (command, description) = command_for(&seat.name)?;
    Some(format!(
        "---\nname: {command}\ndescription: \"{description}\"\n---\n\n\
         # /{command} — the jawata {name} seat\n\n\
         > GENERATED by jawata-studio from `seats/{name}.md` — do not edit;\n\
         > edit the seat file and redeploy.\n\n\
         {contract}\n## Stance (the seat's own, verbatim)\n\n{stance}",
        command = command,
        description = description,
        name = seat.name,
        contract = lane1_contract(seat, command),
        stance = seat.stance,
    ))
}

/// Cursor command: `.cursor/commands/<command>.md` — plain markdown (no
/// frontmatter; Cursor takes the file name as the command name).
pub fn render_cursor_command(seat: &SeatDefinition) -> Option<String> {
    let (command, description) = command_for(&seat.name)?;
    Some(format!(
        "# /{command} — the jawata {name} seat\n\n\
         {description}.\n\n\
         > GENERATED by jawata-studio from `seats/{name}.md` — do not edit;\n\
         > edit the seat file and redeploy.\n\n\
         {contract}\n## Stance (the seat's own, verbatim)\n\n{stance}",
        command = command,
        description = description,
        name = seat.name,
        contract = lane1_contract(seat, command),
        stance = seat.stance,
    ))
}

/// Parses every embedded seat (compile-time sources). Panics are
/// impossible in practice — the seat files are tested in-repo; a parse
/// error here is a build defect surfaced loudly at the call site.
pub fn embedded_seat_definitions() -> Result<Vec<SeatDefinition>, String> {
    EMBEDDED_SEATS
        .iter()
        .map(|(file, body)| {
            parse_seat_definition(body).map_err(|e| format!("embedded seat {file}: {e}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_tempdir(label: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "jawata-conductor-test-{label}-{}-{}-{}",
            std::process::id(),
            nanos,
            n
        ));
        fs::create_dir_all(&dir).expect("test tempdir");
        dir
    }

    fn seats() -> Vec<SeatDefinition> {
        embedded_seat_definitions().expect("embedded seats parse")
    }

    #[test]
    fn command_map_is_exactly_the_five_pairs() {
        let expected = [
            ("javadoc-writer", "javadocs"),
            ("test-writer", "cover"),
            ("architect", "refactor"),
            ("debugger", "debug"),
            ("profiler", "profile"),
        ];
        assert_eq!(COMMAND_MAP.len(), expected.len());
        for (seat, cmd) in expected {
            assert_eq!(
                command_for(seat).map(|(c, _)| c),
                Some(cmd),
                "seat {seat} must map to /{cmd}"
            );
        }
        // The /sprint pair renders NO command.
        assert_eq!(command_for("spec-editor"), None);
        assert_eq!(command_for("spec-auditor"), None);
    }

    #[test]
    fn unmapped_seats_render_no_command() {
        for seat in seats() {
            if command_for(&seat.name).is_none() {
                assert!(render_claude_skill(&seat).is_none(), "{}", seat.name);
                assert!(render_cursor_command(&seat).is_none(), "{}", seat.name);
            }
        }
    }

    #[test]
    fn claude_skill_embeds_contract_and_stance() {
        let seat = seats()
            .into_iter()
            .find(|s| s.name == "javadoc-writer")
            .unwrap();
        let skill = render_claude_skill(&seat).expect("mapped");
        assert!(skill.starts_with("---\nname: javadocs\n"), "frontmatter name");
        assert!(skill.contains("description: \""), "frontmatter description");
        assert!(skill.contains("GENERATED by jawata-studio"), "provenance marker");
        assert!(skill.contains("1. DETECT"), "loop contract");
        assert!(skill.contains("4. PROPOSE"), "propose-mode taught");
        assert!(skill.contains("experience(kind=record"), "store record taught");
        assert!(skill.contains("compile_workspace"), "always gate named");
        assert!(
            skill.contains("-Xdoclint:all"),
            "docs gate named for the docs-gated seat"
        );
        // The stance itself, verbatim.
        assert!(skill.contains("GROUNDED PROSE ONLY"), "stance embedded");
    }

    #[test]
    fn cursor_command_embeds_contract_and_stance() {
        let seat = seats().into_iter().find(|s| s.name == "test-writer").unwrap();
        let cmd = render_cursor_command(&seat).expect("mapped");
        assert!(cmd.starts_with("# /cover — the jawata test-writer seat"));
        assert!(cmd.contains("1. DETECT") && cmd.contains("4. PROPOSE"));
        assert!(
            cmd.contains("coverage_mutation"),
            "tests gate named for the tests-gated seat"
        );
        assert!(cmd.contains(&seat.stance), "stance embedded verbatim");
    }

    #[test]
    fn rendering_is_byte_stable() {
        for seat in seats() {
            assert_eq!(render_claude_skill(&seat), render_claude_skill(&seat));
            assert_eq!(render_cursor_command(&seat), render_cursor_command(&seat));
        }
    }

    #[test]
    fn materialize_writes_absent_only_and_user_edits_survive() {
        let seats_dir = unique_tempdir("materialize").join("seats");
        let written = materialize_seats(&seats_dir).unwrap();
        assert_eq!(written.len(), 7, "all seven materialized on first run");
        // User edits one seat.
        let edited = seats_dir.join("javadoc-writer.md");
        let custom = fs::read_to_string(&edited).unwrap() + "\nCUSTOM RULE.\n";
        fs::write(&edited, &custom).unwrap();
        // Second run writes nothing, the edit survives.
        let written2 = materialize_seats(&seats_dir).unwrap();
        assert!(written2.is_empty(), "second run writes nothing");
        assert_eq!(fs::read_to_string(&edited).unwrap(), custom);
    }

    #[test]
    fn corrupted_config_seat_is_a_loud_error_via_the_runner_loader() {
        let seats_dir = unique_tempdir("corrupt").join("seats");
        materialize_seats(&seats_dir).unwrap();
        fs::write(seats_dir.join("broken.md"), "not a seat").unwrap();
        let (ok, errors) = crate::runner::load_seat_definitions(&seats_dir);
        assert_eq!(ok.len(), 7, "the seven good seats still load");
        assert_eq!(errors.len(), 1, "the broken file is a loud per-file error");
        assert!(errors[0].0.ends_with("broken.md"));
    }
}
