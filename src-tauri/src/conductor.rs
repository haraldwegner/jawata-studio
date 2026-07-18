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

/// Antigravity workflow: `.agent/workflows/<command>.md` — the file NAME is
/// the slash command; YAML frontmatter with `description` is required
/// (verified against antigravity.google/docs/rules-workflows, 2026-07-18).
pub fn render_antigravity_workflow(seat: &SeatDefinition) -> Option<String> {
    let (command, description) = command_for(&seat.name)?;
    Some(format!(
        "---\ndescription: \"{description}\"\n---\n\n\
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

/// The fixed phrase → seat mapping (the IntelliJ substitute and the natural-
/// language entry everywhere). One row per command-bearing seat.
pub const PHRASE_MAP: [(&str, &str); 5] = [
    ("\"document this class\" / \"add javadocs\"", "javadocs"),
    ("\"write tests for this\" / \"improve coverage\"", "cover"),
    ("\"clean this up\" / \"review the architecture\"", "refactor"),
    ("\"find this bug\" / \"why does this fail\"", "debug"),
    ("\"why is this slow\" / \"profile this\"", "profile"),
];

/// The phrase table as markdown rows: phrase → the seat stance to adopt.
pub fn render_phrase_table(seats: &[SeatDefinition]) -> String {
    let mut out = String::from("| You say | Adopt the seat |\n|---|---|\n");
    for (phrase, command) in PHRASE_MAP {
        if let Some((seat_name, _, desc)) = COMMAND_MAP.iter().find(|(_, c, _)| *c == command) {
            // Only rows whose seat actually exists in the loaded set.
            if seats.iter().any(|s| s.name == *seat_name) {
                out.push_str(&format!("| {phrase} | **{seat_name}** — {desc} |\n"));
            }
        }
    }
    out
}

/// The claude.ai / Claude Desktop skill archive: ONE skill (`jawata-seats`),
/// uploaded once — the vendor format requires a single skill folder at the
/// zip root with SKILL.md inside (support.claude.com/articles/12512198,
/// verified 2026-07-18); the five seats ride as `references/<command>.md`,
/// the documented on-demand layout. Deterministic bytes: fixed timestamp,
/// fixed entry order — so redeploy regenerates it unchanged.
pub fn render_claudeai_skill_zip(seats: &[SeatDefinition]) -> Result<Vec<u8>, String> {
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    let mut mapped: Vec<(&SeatDefinition, &str, &str)> = Vec::new();
    for (seat_name, command, desc) in &COMMAND_MAP {
        let seat = seats
            .iter()
            .find(|s| s.name == *seat_name)
            .ok_or_else(|| format!("seat {seat_name} missing from the loaded set"))?;
        mapped.push((seat, command, desc));
    }

    let mut skill = String::from(
        "---\nname: jawata-seats\ndescription: \"Run jawata's engineering seats — \
         javadocs, cover, refactor, debug, profile — with gate discipline: real \
         jawata MCP gate calls, propose-mode, every outcome recorded to the \
         experience store.\"\n---\n\n\
         # jawata seats\n\n\
         > GENERATED by jawata-studio from `seats/*.md` — regenerate by\n\
         > redeploying, then re-upload this skill once.\n\n\
         When the user asks for one of these roles, read the matching reference\n\
         and follow it EXACTLY — the loop contract in it is binding:\n\n",
    );
    for (_, command, desc) in &mapped {
        skill.push_str(&format!("- `references/{command}.md` — {desc}\n"));
    }
    for (cmd, desc) in UTILITY_MAP {
        skill.push_str(&format!("- `references/{cmd}.md` — {desc}\n"));
    }
    skill.push_str(
        "\nThe spec-editor and spec-auditor seats live in the /sprint pipeline,\n\
         not here.\n",
    );

    let opts = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default());
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        zip.start_file("jawata-seats/SKILL.md", opts)
            .and_then(|_| zip.write_all(skill.as_bytes()).map_err(Into::into))
            .map_err(|e| format!("zip SKILL.md: {e}"))?;
        for (seat, command, description) in &mapped {
            let body = format!(
                "# /{command} — the jawata {name} seat\n\n\
                 {description}.\n\n\
                 {contract}\n## Stance (the seat's own, verbatim)\n\n{stance}",
                command = command,
                description = description,
                name = seat.name,
                contract = lane1_contract(seat, command),
                stance = seat.stance,
            );
            zip.start_file(format!("jawata-seats/references/{command}.md"), opts)
                .and_then(|_| zip.write_all(body.as_bytes()).map_err(Into::into))
                .map_err(|e| format!("zip references/{command}.md: {e}"))?;
        }
        for (cmd, desc) in UTILITY_MAP {
            zip.start_file(format!("jawata-seats/references/{cmd}.md"), opts)
                .and_then(|_| zip.write_all(
                    render_cursor_utility(cmd, desc).as_bytes()).map_err(Into::into))
                .map_err(|e| format!("zip references/{cmd}.md: {e}"))?;
        }
        zip.finish().map_err(|e| format!("zip finish: {e}"))?;
    }
    Ok(buf.into_inner())
}


/// Sprint 26 (D6/D7): the utility commands — generated per client like the
/// seats, but not seat-backed (no stance, no gate loop): /memorize writes
/// the shared store FIRST (cross-client by construction), /train runs a
/// learning pass and reports each learner's state.
pub const UTILITY_MAP: [(&str, &str); 2] = [
    ("memorize", "Store a durable decision/lesson/fact in the shared jawata experience store (store-first, cross-client)"),
    ("train", "Run a jawata learning pass now and report each learner's state (events seen, rolling record, deciding or shadow)"),
];

fn utility_body(command: &str) -> &'static str {
    match command {
        "memorize" => "Bare `/memorize`: identify the durable decision, lesson, or fact \
from the current discussion, store it, and echo ONE line of what was stored — no \
approval loop. `/memorize <something>`: store exactly that.\n\nProtocol (binding): \
STORE FIRST — call the jawata `experience` tool with `kind=record`, a fitting `type` \
(domain_fact / lesson / failure_mode / naming_convention), a one-line `summary`, and \
an anchor (`symbol` for Java, `operation`+`language` otherwise). THEN write the \
client's own file memory where one exists. The shared store is the authoritative \
cross-client layer: what one client memorizes, every client recalls.",
        "train" => "Call the jawata `experience` tool with `kind=train` — a forced \
learning pass over every learner — and report the result to the user: per learner, \
the events seen, the rolling record vs the hand rules, and DECIDING or SHADOW. \
`/train status` (or when the user only wants the numbers): call `kind=learner_status` \
instead and report without forcing a pass. Learning is continuous and needs no manual \
step — this command is bring-up, catch-up, and the inspection window.",
        _ => "",
    }
}

fn utility_render(command: &str, description: &str, title_prefix: &str) -> String {
    format!(
        "{title_prefix}# /{command} — jawata utility\n\n{description}.\n\n\
         > GENERATED by jawata-studio — do not edit; redeploy to regenerate.\n\n\
         {body}\n",
        title_prefix = title_prefix,
        command = command,
        description = description,
        body = utility_body(command),
    )
}

pub fn render_claude_utility(command: &str, description: &str) -> String {
    format!("---\nname: {command}\ndescription: \"{description}\"\n---\n\n{rest}",
        command = command, description = description,
        rest = utility_render(command, description, ""))
}

pub fn render_cursor_utility(command: &str, description: &str) -> String {
    utility_render(command, description, "")
}

pub fn render_antigravity_utility(command: &str, description: &str) -> String {
    format!("---\ndescription: \"{description}\"\n---\n\n{rest}",
        description = description,
        rest = utility_render(command, description, ""))
}

/// Line budgets for the rule-block conductor section (the R2 guard — the
/// numbers are FIXED in dossier-25a C0; the build-failing test pins them).
pub const CONDUCTOR_SECTION_BUDGET_UNIVERSAL: usize = 30;
pub const CONDUCTOR_SECTION_BUDGET_INTELLIJ: usize = 60;

/// The rule-block conductor section (Sprint 25a D2): the universal tight
/// summary — seat catalog, when to involve the architect unprompted, the
/// seat discipline, the design-step line — plus a per-client tail: a
/// one-liner where commands are deployed, the FULL phrase table on IntelliJ
/// (its Prompt Library has no file channel). This is the ONE deliberate
/// per-client variation in the rule-block body (the invariant test asserts
/// every other section stays byte-identical across clients).
pub fn render_conductor_section(seats: &[SeatDefinition], client: &str) -> Vec<String> {
    let mut lines = vec![
        "## The jawata seats — narrow engineering roles, gate-disciplined".to_string(),
        String::new(),
        "jawata ships seven SEATS. Five are direct roles; two live in /sprint:".to_string(),
        String::new(),
    ];
    for (seat_name, command, desc) in &COMMAND_MAP {
        if seats.iter().any(|s| s.name == *seat_name) {
            lines.push(format!("- {seat_name} (`/{command}`) — {desc}"));
        }
    }
    lines.push(
        "- spec-editor + spec-auditor — the /sprint two-seat artifact pipeline".to_string(),
    );
    lines.extend([
        String::new(),
        "Involve the ARCHITECT seat unprompted when the ask is a vague \"clean \
         this up\" or you are reviewing a checkpoint diff — design fix or bandage \
         is its call. A sprint's design step: after spec sign-off, before the \
         plan, a design-mode run produces `ARCHITECTURE-<scope>.md`; the plan is \
         written against it."
            .to_string(),
        String::new(),
        "Seat discipline (binding whenever you run a seat):".to_string(),
        "- The gates are real jawata MCP calls you make and read — a gate you \
         could not run has NOT passed."
            .to_string(),
        "- PROPOSE, never auto-apply: present the diff/files, wait for the human \
         yes."
            .to_string(),
        "- Record every outcome: `experience(kind=record, operation=\"seat:<command>\", \
         …)`."
            .to_string(),
        String::new(),
    ]);
    match client {
        "intellij" => {
            lines.push(
                "No command channel in this client — adopt the seat by phrase:".to_string(),
            );
            lines.extend(render_phrase_table(seats).lines().map(String::from));
        }
        "claude_desktop" => {
            lines.push(
                "The `jawata-seats` skill (uploaded once) carries these seats — \
                 invoke it by role."
                    .to_string(),
            );
        }
        _ => {
            lines.push(
                "Seat commands are installed in this client — invoke them directly."
                    .to_string(),
            );
        }
    }
    lines
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
    fn commands_never_leak_runner_only_fields() {
        // Audit observation 1 (25a KEEP round): model/tier/ceilings are the
        // hosted runner's concern — the front-door agent runs as itself. Pin
        // it so a future renderer edit cannot reintroduce a leak silently.
        for seat in seats() {
            for rendered in [
                render_claude_skill(&seat),
                render_cursor_command(&seat),
                render_antigravity_workflow(&seat),
            ]
            .into_iter()
            .flatten()
            {
                for forbidden in ["model:", "effort:", "ttl_secs", "cost_budget_usd"] {
                    assert!(
                        !rendered.contains(forbidden),
                        "{}: runner-only field '{forbidden}' leaked into a command",
                        seat.name
                    );
                }
            }
        }
    }

    #[test]
    fn rendering_is_byte_stable() {
        let all = seats();
        for seat in &all {
            assert_eq!(render_claude_skill(seat), render_claude_skill(seat));
            assert_eq!(render_cursor_command(seat), render_cursor_command(seat));
            assert_eq!(
                render_antigravity_workflow(seat),
                render_antigravity_workflow(seat)
            );
        }
        assert_eq!(
            render_claudeai_skill_zip(&all).unwrap(),
            render_claudeai_skill_zip(&all).unwrap(),
            "the skill zip must be byte-identical across regenerations"
        );
        assert_eq!(render_phrase_table(&all), render_phrase_table(&all));
    }

    #[test]
    fn antigravity_workflow_has_frontmatter_contract_and_stance() {
        let seat = seats().into_iter().find(|s| s.name == "architect").unwrap();
        let wf = render_antigravity_workflow(&seat).expect("mapped");
        assert!(wf.starts_with("---\ndescription: \""), "frontmatter description first");
        assert!(wf.contains("# /refactor"), "slash name in title");
        assert!(wf.contains("1. DETECT") && wf.contains("4. PROPOSE"));
        assert!(wf.contains(&seat.stance), "stance embedded verbatim");
    }

    #[test]
    fn claudeai_zip_is_one_skill_with_a_reference_per_mapped_seat() {
        let bytes = render_claudeai_skill_zip(&seats()).unwrap();
        let mut archive =
            zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip opens");
        let names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();
        assert!(
            names.contains(&"jawata-seats/SKILL.md".to_string()),
            "single skill folder at zip root: {names:?}"
        );
        for (_, command, _) in COMMAND_MAP {
            assert!(
                names.contains(&format!("jawata-seats/references/{command}.md")),
                "reference for /{command} present: {names:?}"
            );
        }
        for (cmd, _) in UTILITY_MAP {
            assert!(names.contains(&format!("jawata-seats/references/{cmd}.md")),
                "utility /{cmd} reference present");
        }
        assert_eq!(names.len(), 8,
            "SKILL.md + five seat references + two utility references");
        // SKILL.md carries the required frontmatter.
        use std::io::Read;
        let mut skill = String::new();
        archive
            .by_name("jawata-seats/SKILL.md")
            .unwrap()
            .read_to_string(&mut skill)
            .unwrap();
        assert!(skill.starts_with("---\nname: jawata-seats\n"));
        assert!(skill.contains("description: \""));
    }

    #[test]
    fn phrase_table_covers_all_five_commands() {
        let table = render_phrase_table(&seats());
        for (_, command, _) in COMMAND_MAP {
            let (seat_name, _, _) = COMMAND_MAP.iter().find(|(_, c, _)| *c == command).unwrap();
            assert!(
                table.contains(&format!("**{seat_name}**")),
                "/{command}'s seat in the table:\n{table}"
            );
        }
        assert_eq!(table.lines().count(), 2 + 5, "header + five rows, nothing more");
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
