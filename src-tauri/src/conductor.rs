//! The conductor delivery (Sprint 25a): generates every per-client seat
//! artifact from `seats/*.md` — the single source. Pure renderers, no I/O
//! except `materialize_seats`; the deploy layer in `manager_service.rs` is
//! the only writer into client trees. Architecture: ARCHITECTURE-conductor.md.

use crate::runner::{parse_seat_definition, GateClass, SeatDefinition};
use std::fs;
use std::path::Path;

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

/// What one materialization pass did, per seat — surfaced into the deploy
/// result so nothing about the user's instruments changes silently.
#[derive(Debug, Default)]
pub struct SeatMaterialization {
    /// Freshly seeded (file was absent).
    pub seeded: Vec<String>,
    /// UNEDITED copy refreshed to this build's content.
    pub refreshed: Vec<String>,
    /// User-EDITED copy left in place while this build ships different
    /// content — the user's version wins, and the deploy says so.
    pub shadowed: Vec<String>,
    /// Pre-manifest copy that differed from this build: backed up beside
    /// itself and refreshed (we cannot prove it was unedited, so nothing is
    /// destroyed — but a silently stale instrument is the worse failure).
    pub migrated: Vec<String>,
}

/// The manifest of content hashes THIS APP wrote, per seat file. A copy whose
/// hash still matches is an unedited seed; one that differs was edited by the
/// user. Lives beside the seats, dot-named, never rendered.
const SEED_MANIFEST: &str = ".seeded.json";

fn content_hash(body: &str) -> String {
    // Not cryptographic — an identity check between bytes we wrote and bytes
    // on disk. FNV-1a over the content, hex-rendered.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in body.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Materialize the embedded seats into `seats_dir` — and keep UNEDITED copies
/// CURRENT.
///
/// The original contract was materialize-if-absent with "config wins"
/// absolutely. The 3.7.6 dogfood showed what that costs: every install's
/// seats freeze at their first materialization, so a seat improvement shipped
/// in an update exists in the binary and never reaches the deployed
/// instrument — the invisible-stale-deploy defect class, on the files that
/// define the roles. Ruled a major bug (Harald, 2026-08-09).
///
/// The refined contract, per file:
/// - absent                          → seed it, record its hash;
/// - hash matches the manifest       → an unedited seed: refresh it whenever
///   this build's content differs, and re-record;
/// - hash differs from the manifest  → the user edited it: PRESERVE it, and
///   report the shadowing loudly when this build ships different content;
/// - no manifest entry (pre-manifest install): identical to this build →
///   adopt silently; different → back it up beside itself and refresh,
///   reported — provenance is unknowable there, so nothing is destroyed,
///   and the instrument does not stay silently stale.
pub fn materialize_seats(seats_dir: &Path) -> Result<SeatMaterialization, String> {
    fs::create_dir_all(seats_dir)
        .map_err(|e| format!("cannot create seats dir {}: {e}", seats_dir.display()))?;
    let manifest_path = seats_dir.join(SEED_MANIFEST);
    let mut manifest: std::collections::BTreeMap<String, String> =
        fs::read_to_string(&manifest_path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();

    let mut report = SeatMaterialization::default();
    for (file, body) in EMBEDDED_SEATS {
        let path = seats_dir.join(file);
        let write_current = |report_bucket: &mut Vec<String>,
                             manifest: &mut std::collections::BTreeMap<String, String>|
         -> Result<(), String> {
            fs::write(&path, body).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
            manifest.insert(file.to_string(), content_hash(body));
            report_bucket.push(file.to_string());
            Ok(())
        };

        let Ok(existing) = fs::read_to_string(&path) else {
            write_current(&mut report.seeded, &mut manifest)?;
            continue;
        };
        match manifest.get(file) {
            Some(seeded_hash) if *seeded_hash == content_hash(&existing) => {
                // Unedited seed — keep it current.
                if existing != body {
                    write_current(&mut report.refreshed, &mut manifest)?;
                }
            }
            Some(_) => {
                // User-edited: config wins, loudly.
                if existing != body {
                    report.shadowed.push(file.to_string());
                }
            }
            None => {
                if existing == body {
                    // Pre-manifest but identical: adopt.
                    manifest.insert(file.to_string(), content_hash(body));
                } else {
                    // Pre-manifest and different: provenance unknowable.
                    // Back up, refresh, say so.
                    let backup = seats_dir.join(format!("{file}.pre-refresh"));
                    fs::copy(&path, &backup)
                        .map_err(|e| format!("cannot back up {}: {e}", path.display()))?;
                    write_current(&mut report.migrated, &mut manifest)?;
                }
            }
        }
    }
    let rendered = serde_json::to_string_pretty(&manifest)
        .map_err(|e| format!("cannot render the seed manifest: {e}"))?;
    fs::write(&manifest_path, rendered)
        .map_err(|e| format!("cannot write {}: {e}", manifest_path.display()))?;
    Ok(report)
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



/// Sprint 26 (D6/D7): the utility commands — generated per client like the
/// seats, but not seat-backed (no stance, no gate loop): /memorize writes
/// the shared store FIRST (cross-client by construction), /sprint runs the
/// two-seat spec + plan pipeline.
///
/// v3.3.1 — two changes, both about a command matching reality:
/// - `/train` REMOVED. Sprint 26a (D4) retired the edit-ML and deleted every
///   model class, so the command had nothing left to train: its entire output
///   was a "retired" tombstone. A command whose whole content is a tombstone is
///   noise in every client that ships it.
/// - `/sprint` ADDED. It was the ONE command jawata never shipped — hand-placed
///   in a single machine's `~/.claude/skills/`, so the pipeline the whole sprint
///   process depends on was simply absent everywhere else. Its body is now
///   single-sourced from `skills/sprint.md` and rides the same deploy as the rest.
pub const UTILITY_MAP: [(&str, &str); 2] = [
    ("memorize", "Store a durable decision/lesson/fact in the shared jawata experience store (store-first, cross-client)"),
    ("sprint", "Run the two-seat EDITOR+AUDITOR pipeline for a sprint doc and/or its actionable plan — the RAW working doc stays the audit baseline, the CLEAN spec is written for the user, a fresh-context auditor can REFUSE and loops until sign-off, and the user signs off LAST"),
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
        // Single-sourced: the pipeline is far too long to inline, and a second
        // copy would drift from the one the seats' own process runs.
        "sprint" => include_str!("../../skills/sprint.md"),
        _ => "",
    }
}

/// The command's own title. /sprint is a two-seat pipeline, not a one-liner
/// utility, and its rendered page should not claim otherwise.
fn utility_title(command: &str) -> &'static str {
    match command {
        "sprint" => "the two-seat artifact pipeline",
        _ => "jawata utility",
    }
}

fn utility_render(command: &str, description: &str, title_prefix: &str) -> String {
    format!(
        "{title_prefix}# /{command} — {title}\n\n{description}.\n\n\
         > GENERATED by jawata-studio — do not edit; redeploy to regenerate.\n\n\
         {body}\n",
        title_prefix = title_prefix,
        command = command,
        title = utility_title(command),
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
///
/// Sprint 28 Stage 4 (D-UNWIRED): `#[cfg(test)]` because that is the truth.
/// Nothing in the shipped binary reads these — the render path does not
/// enforce a budget at runtime, the guard test does, by measuring what
/// `render_conductor_section` produced. They were public production constants
/// whose only consumers were assertions, which is the shape this sprint
/// exists to stop; scoping them to the test build says so in the type system
/// instead of in a comment.
#[cfg(test)]
pub const CONDUCTOR_SECTION_BUDGET_UNIVERSAL: usize = 30;
#[cfg(test)]
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
        // Sprint 26a D3a: the seat WORKFLOW PLACEMENT — coded here and deployed to
        // every client so the seats fire at their dev-process points by RULE, not
        // because the agent remembers. Deterministic points (no heuristic
        // event-detection — that is the fuzzy, hollow mechanism 26a rejected).
        "When each seat fires in the dev process — run it at these points, do not wait to remember:"
            .to_string(),
        "- architect (`/refactor`) — at the sprint DESIGN step (produces \
         `ARCHITECTURE-<scope>.md`), AND at EVERY checkpoint (a watch-diff of the \
         checkpoint's changes against that architecture: design fix or bandage?)."
            .to_string(),
        "- test-writer (`/cover`) — at the COVERAGE gate: before a change is called \
         done, if it added behaviour, cover it (`run_tests coverage=true` before/after)."
            .to_string(),
        "- javadoc-writer (`/javadocs`) — at the DOC gate: undocumented public API a \
         change touched is documented before the change is done."
            .to_string(),
        "- debugger (`/debug`) — at the runtime reflex: a bug / bad value / NPE → \
         attach and probe; do NOT hand-add logging (the guard surfaces this)."
            .to_string(),
        "- profiler (`/profile`) — at the runtime reflex: performance / a hotspot → \
         sample the JVM; do NOT hand-roll a stopwatch (the guard surfaces this)."
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

    fn unique_tempdir(label: &str) -> std::path::PathBuf {
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
    fn materialize_seeds_refreshes_unedited_and_preserves_edits() {
        let seats_dir = unique_tempdir("materialize").join("seats");
        let first = materialize_seats(&seats_dir).unwrap();
        assert_eq!(first.seeded.len(), 7, "all seven materialized on first run");

        // A user edit survives every later run — config wins for EDITED files.
        let edited = seats_dir.join("javadoc-writer.md");
        let custom = fs::read_to_string(&edited).unwrap() + "\nCUSTOM RULE.\n";
        fs::write(&edited, &custom).unwrap();
        let second = materialize_seats(&seats_dir).unwrap();
        assert!(second.seeded.is_empty() && second.refreshed.is_empty() && second.migrated.is_empty(),
            "nothing rewritten on a converged run: {second:?}");
        assert_eq!(fs::read_to_string(&edited).unwrap(), custom, "the edit survives");
        assert!(second.shadowed.contains(&"javadoc-writer.md".to_string()),
            "an edited seat that differs from this build is REPORTED, never silent: {second:?}");

        // An UNEDITED seed from an older build refreshes. Simulated by writing
        // old content AND recording its hash as the seeded one — exactly what
        // an older app version would have left behind.
        let stale = seats_dir.join("architect.md");
        let old_body = "OLD SEAT BODY from a previous release\n";
        fs::write(&stale, old_body).unwrap();
        let manifest_path = seats_dir.join(".seeded.json");
        let mut manifest: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
        manifest.insert("architect.md".into(), content_hash(old_body));
        fs::write(&manifest_path, serde_json::to_string(&manifest).unwrap()).unwrap();

        let third = materialize_seats(&seats_dir).unwrap();
        assert!(third.refreshed.contains(&"architect.md".to_string()),
            "the 3.7.6 dogfood defect: an unedited stale seed must refresh: {third:?}");
        let refreshed = fs::read_to_string(&stale).unwrap();
        let embedded = EMBEDDED_SEATS.iter().find(|(f, _)| *f == "architect.md").unwrap().1;
        assert_eq!(refreshed, embedded, "the file now carries this build's content");
    }

    /// The install this defect was found on: seats seeded by a pre-manifest
    /// build, no record of what was written. Identical files are adopted;
    /// differing ones are backed up and refreshed — provenance is unknowable
    /// there, so nothing is destroyed, and nothing stays silently stale.
    #[test]
    fn pre_manifest_installs_adopt_matching_and_migrate_differing_with_backup() {
        let seats_dir = unique_tempdir("premanifest").join("seats");
        materialize_seats(&seats_dir).unwrap();
        fs::remove_file(seats_dir.join(".seeded.json")).unwrap();
        let stale = seats_dir.join("architect.md");
        let old_body = "SEAT BODY as an older release shipped it\n";
        fs::write(&stale, old_body).unwrap();

        let report = materialize_seats(&seats_dir).unwrap();
        assert!(report.migrated.contains(&"architect.md".to_string()), "{report:?}");
        assert!(report.shadowed.is_empty() && report.seeded.is_empty(),
            "the six identical files are adopted silently: {report:?}");
        let embedded = EMBEDDED_SEATS.iter().find(|(f, _)| *f == "architect.md").unwrap().1;
        assert_eq!(fs::read_to_string(&stale).unwrap(), embedded, "refreshed to this build");
        assert_eq!(fs::read_to_string(seats_dir.join("architect.md.pre-refresh")).unwrap(),
            old_body, "the displaced content is preserved beside the file");
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
