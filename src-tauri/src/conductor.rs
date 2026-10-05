//! The conductor delivery (Sprint 25a): generates every per-client seat
//! artifact from `seats/*.md` — the single source. Pure renderers, no I/O
//! except `materialize_seats`; the deploy layer in `manager_service.rs` is
//! the only writer into client trees. Architecture: ARCHITECTURE-conductor.md.

use crate::runner::{parse_seat_definition, GateClass, SeatDefinition};
use std::fs;
use std::path::Path;

/// Every seat definition shipped in the binary — ONE row per `seats/*.md`,
/// with no row left to a human to remember (`every_seat_file_is_embedded…`
/// fails the build on a seat that exists on disk and is registered here).
/// Materialized into `<config>/seats/` where absent; the materialized copy
/// wins so a user-edited seat regenerates every channel on redeploy.
pub const EMBEDDED_SEATS: [(&str, &str); 10] = [
    ("architect.md", include_str!("../../seats/architect.md")),
    ("cataloguer.md", include_str!("../../seats/cataloguer.md")),
    ("debugger.md", include_str!("../../seats/debugger.md")),
    ("javadoc-writer.md", include_str!("../../seats/javadoc-writer.md")),
    ("profiler.md", include_str!("../../seats/profiler.md")),
    ("report.md", include_str!("../../seats/report.md")),
    ("review.md", include_str!("../../seats/review.md")),
    ("spec-auditor.md", include_str!("../../seats/spec-auditor.md")),
    ("spec-editor.md", include_str!("../../seats/spec-editor.md")),
    ("test-writer.md", include_str!("../../seats/test-writer.md")),
];

/// seat name → (command name, one-line description). The command-bearing
/// seats; spec-editor/spec-auditor live in /sprint and render NO command
/// (pinned by test). Every row here must name a seat that is embedded, and
/// every row renders into the deployed command list — both pinned, so the
/// counts in the generated prose and in the deploy inventory are derived
/// from this array rather than typed beside it.
pub const COMMAND_MAP: [(&str, &str, &str); 8] = [
    (
        "cataloguer",
        "catalogue",
        "Describe what each member and each package is FOR, resumably (jawata cataloguer seat)",
    ),
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
    (
        "report",
        "report",
        "Turn the local field recording into a bug report you post from your own GitHub account (jawata report seat)",
    ),
    (
        "review",
        "review",
        "Review what the knowledge store carries that nobody uses, and delete only what you name (jawata review seat)",
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
         0. READ WHAT THE PROJECT SAYS IT IS. Open `{decl}` at the project's\n\
            root and QUOTE its four facts — stack, build system, source roots,\n\
            what counts as green — in your output. They are the project's own\n\
            words: do not contradict them and do not improve on them.\n\
            **If there is no such file, SAY SO** and name every fact you then\n\
            had to assume. Do not infer a stack from the file extensions and\n\
            proceed as though you had been told; `/setup` writes one, and a\n\
            run on a project that has not declared itself is a run whose\n\
            reader needs to know that. If the file is present and does not\n\
            parse, that is a THIRD state — report it broken rather than\n\
            absent, because the two ask different things of whoever reads you.\n\
         1. DETECT — find the work the way the Instrument discipline below\n\
            says this seat finds it, scoped to what the user named. Never\n\
            invent targets.\n\
         2. DO — produce the seat's output per the stance below.\n\
         3. VERIFY — the gates are jawata calls you MUST make and read:\n\
         {gates}\
         4. PROPOSE — present the result as a proposal (diff or files) and\n\
            WAIT for the human yes. Never auto-apply, never commit on your\n\
            own. A gate you could not run has NOT passed — say so.\n\
         5. RECORD — after the human verdict, record the outcome to the\n\
            experience store: `experience(kind=record, type=lesson|domain_fact,\n\
            operation=\"seat:{command}\", summary=<what was proposed, the gate\n\
            results, the verdict>)`.\n\
            A `lesson` is an EXPERIENCE and owes two more fields or the store\n\
            REFUSES it: `situation` — when it applies, phrased as a condition\n\
            (\"when a rename spans two bundles\"), never a path or a symbol —\n\
            and `verdict`: `worked`, `failed_avoid`, or `unproven` while it is\n\
            genuinely still open. A `domain_fact` owes neither; it never turned\n\
            out any way at all, and inventing an outcome for one makes\n\
            retrieval rank on fiction.\n\n\
         {instruments}\n\
         ## Execution context\n\n\
         Unlike the hosted runner, YOU perform detection and gates yourself.\n\
         Where the stance below assumes a hosted harness (e.g. \"you do not\n\
         use tools\"), the loop above wins on MECHANICS; the stance's content\n\
         rules bind unchanged.\n",
        name = seat.name,
        decl = crate::runner::PROJECT_DECLARATION_FILE,
        gates = gates,
        instruments = instrument_discipline(seat),
    )
}

/// The seat's instrument discipline, DERIVED from its `kind` and its own `tools`.
///
/// D12: *"instrument discipline is a rule in each seat's text rather than something an agent is
/// expected to remember."* Derived rather than written ten times, because ten paragraphs are
/// ten chances to disagree with the frontmatter beside them — the defect this sprint spent five
/// audit rounds on. One renderer cannot drift from the fields it reads.
fn instrument_discipline(seat: &SeatDefinition) -> String {
    match seat.kind {
        crate::runner::SeatKind::Code => format!(
            "## Instrument discipline (binding)\n\n\
             This seat reads the CODEBASE, so every question it asks has a compiler-accurate\n\
             answer and reaching for one is not optional. Drive these, by name:\n\n\
             {tools}\n\n\
             A `grep`, `rg`, `sed` or hand-read of a source file is a DECLARED FALLBACK, never a\n\
             shortcut: it carries `jawata-fallback: <why jawata cannot answer THIS>` in the\n\
             command, and the declaration is logged. If an instrument refuses or is unreachable,\n\
             SAY SO and stop — a gate you could not run has not passed, and an absence inferred\n\
             from a failed lookup is not an absence.\n",
            tools = seat
                .tools
                .iter()
                .map(|t| format!("- `{t}`"))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        crate::runner::SeatKind::Text => "## Instrument discipline (binding)\n\n\
             This seat's input is an ARTIFACT — a document, a diff, a report.\n\
             There is no compiler fact to fetch — that is why `tools` is empty — and your\n\
             judgement IS the product. The discipline runs the other way round: every claim is\n\
             the text in front of you, quoted rather than paraphrased, and a claim you cannot\n\
             ground in it does not go in the output. Where the artifact asserts something about\n\
             CODE, you may say the claim is unverified; you may not verify it and you may not\n\
             assume it.\n\n\
             An empty `tools` does NOT mean this seat drives nothing. Where the stance below\n\
             names a verb — a store call, a field read, a command — THAT is this seat's\n\
             instrument, and the same rule binds it: drive it, read the answer it actually\n\
             gave, and if it refuses or is unreachable SAY SO and stop. An absence inferred\n\
             from a failed lookup is not an absence, whichever kind of instrument failed.\n"
            .to_string(),
    }
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

/// Cursor skill: `.cursor/skills/<command>/SKILL.md`.
///
/// studio#44: it used to be a flat `.cursor/commands/<command>.md` with no
/// frontmatter. Cursor's current documentation names neither — it loads skills
/// from its own `.cursor/skills` and `.agents/skills` (and, for compatibility,
/// the Claude and Codex ones), and says existing slash commands are CONVERTED
/// to skills. So the flat file was a location the vendor is retiring, in a
/// format it no longer documents.
///
/// The frontmatter is Cursor's own requirement, read from
/// cursor.com/docs/context/skills on 2026-09-08: `name`, which must match the
/// parent folder, and `description`. `disable-model-invocation` mirrors what
/// Cursor's own converter does to a command — a seat is a thing a person
/// invokes deliberately, not one a model should reach for on a hunch.
///
/// The body is the same as the Claude skill's; only the frontmatter differs,
/// which is why both are generated from `lane1_contract` and the same stance.
pub fn render_cursor_command(seat: &SeatDefinition) -> Option<String> {
    let (command, description) = command_for(&seat.name)?;
    Some(format!(
        "---\nname: {command}\ndescription: \"{description}\"\n\
         disable-model-invocation: true\n---\n\n\
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
/// language entry everywhere). One row per command-bearing seat — that is the
/// rule the table is built on, and `phrase_table_covers_every_command` fails
/// the build on a command that has no way in through plain words.
pub const PHRASE_MAP: [(&str, &str); 8] = [
    ("\"document this class\" / \"add javadocs\"", "javadocs"),
    ("\"what is this package for\" / \"describe this code\"", "catalogue"),
    ("\"write tests for this\" / \"improve coverage\"", "cover"),
    ("\"clean this up\" / \"review the architecture\"", "refactor"),
    ("\"find this bug\" / \"why does this fail\"", "debug"),
    ("\"why is this slow\" / \"profile this\"", "profile"),
    ("\"report this to jawata\" / \"file a jawata bug\"", "report"),
    ("\"clean up my store\" / \"what is nobody using\"", "review"),
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
pub const UTILITY_MAP: [(&str, &str); 5] = [
    // Sprint 28f Stage 6: this line is the ONE every client displays — it is interpolated
    // into each generated artifact's frontmatter — and it said "the knowledge substrate the
    // store is rebuilt from" while the body 68 lines below said the opposite, in the same
    // commit whose subject was that the texts stop telling agents to write the store's
    // files. A description is a shipped claim, not a label.
    ("memorize", "Extract what this session actually learned, review it cold, and record it to the store — which stamps the review and writes the story file when a reader accepts it"),
    // Sprint 28g D16: TWO commands, not one, because they point in opposite
    // directions — /btw takes a thought IN mid-flight and its value is costing
    // nothing, /brainstorm produces options OUT and IS the activity. Merging
    // them would price a capture at the cost of a brainstorm, which is the one
    // thing a capture cannot afford. Neither is a seat: a seat has gates that
    // are real jawata calls, and writing down what you were told has nothing
    // to gate.
    ("brainstorm", "Produce options with their trade-offs for the person who asked — grounded in the store and the code first, read immediately, and never recorded as experience because a generated option is not an observation"),
    ("btw", "Capture a thought mid-task in one or two calls and triage it to where it belongs — a defect to an issue, durable knowledge to /memorize, an idea to the project's append-only capture file that the /sprint editor step must dispose of"),
    // Sprint 28g D14: the project declares what it IS before any seat runs,
    // because every seat's brief opens with these four facts quoted — and a
    // project that has not declared itself makes every brief say so out loud
    // rather than letting the seat infer and proceed.
    ("setup", "Establish what this project is — stack, build system, source roots, and the command that says it is green — prove the green command by running it, and write the declaration every seat's brief quotes"),
    ("sprint", "Run the two-seat EDITOR+AUDITOR pipeline for a sprint doc and/or its actionable plan — the RAW working doc stays the audit baseline, the CLEAN spec is written for the user, a fresh-context auditor can REFUSE and loops until sign-off, and the user signs off LAST"),
];

fn utility_body(command: &str) -> &'static str {
    match command {
        "memorize" => r#"Bare `/memorize`: find what this session learned that is durable and reusable, ROUTE it, and report. `/memorize <something>`: take exactly that as the candidate and start at step 1.

**EXPECT ZERO.** A session of a million tokens commonly yields nothing storable, and *no candidates* is the right answer far more often than it has been given. The store reached ~2,500 entries; routed against these criteria about fifty survive. Whatever this command was doing, it stored roughly fifty times too much. Never produce an entry because producing one feels like working.

A candidate is **durable** (still true next month), **reusable** (someone not in this session could hit it), and **not already known** — recall it first; if the store or a standing rule already has it, it is not a candidate.

## STEP 1 — the story list, and this is the cheap gate

Before writing anything, print ONE LINE per candidate saying what STORY it is:

    1. the amend quantity is the total, not the remainder — from the rejected order
    2. a superseded note does not know it was superseded
    — nothing else worth storing from this session

Then let him look. The expensive correction is the one that arrives after a full entry exists and turns out to be about the wrong thing: in one review session twenty of those landed, and almost every one was visible in a single line. Silence means proceed — this is a chance to redirect, not an approval loop.

## STEP 2 — route it. The store is NOT the default

Ask **when must this fire?** and take the cheapest rung that still fires:

- **hook** — decidable from the artifact, enforceable with no judgement. Propose the diff.
- **seat check** — fires at a known point (a design review, a spec audit, a checkpoint). Propose the wording, naming the FAILURE SHAPE to refuse rather than the outcome to want.
- **standing rule** — must hold always and cannot be mechanised. SCARCE: every rule is read on every task forever. Justify the cost or drop a rung.
- **store** — someone would ask for it, OR it is knowledge applied by judgement that nothing can enforce.

**That last clause matters.** Most knowledge cannot be enforced and that is not a defect in it. You cannot hook *use this pattern*; a reader searches for applicable ones and which to take, or none, is an architecture decision. The store is that knowledge's home, not its consolation prize.

A non-store destination produces a PROPOSAL with a diff and waits. Never a silent edit. Name the file.

## STEP 3 — write it, against `docs/story-template.md`

The rules that carry the most weight, so you do not have to open the file for an ordinary entry:

- **Tell it as a case is argued — the Minto triad: Situation → Complication → Solution, and whether it worked.** Each part has a FIELD, so it is queryable rather than buried: `situation:` (the setting and task), `cause:` (the COMPLICATION — the diagnosis the solution addresses; `complication:` is the same key), the claim + cure (the solution), `verdict:` (worked / failed_avoid / unproven). The cause is NOT the symptom: a symptom is the observable (a fast heartbeat), the cause is the problem behind it (running / a heart attack / a virus) — one symptom, many causes, and the solution binds to the cause. A story whose cause is missing cannot tell a reader whether it transfers to THEIR problem, and the review seat will list it as repair work.
- **Quote the claim from the source; do not restate it.** Every factual error in a fold came from paraphrase. Paraphrasing is confined to the situation.
- **Widen the claim until a reader can no longer answer *is that me?* by inspection, then step back.** *On Alpaca, read the status with a REST GET* is evaluable. *Where acknowledgements go over a websocket with no delivery guarantee, read the status with a GET* is wider and still evaluable — the right rung. *Confirm state before acting* is a tautology that matches everything and helps nobody.
- **Say the situation out loud.** Dictate it before typing it. If nobody would say the sentence, it is assembly.
- **Keep the domain's own nouns and its real word.** Point at each noun and name the thing it replaced. *Cleanup pass, in-flight job, the map* were orders, a broker and our record of them.
- **Keep the incident's numbers OUT of the situation.** 0.28 and forty minutes are evidence for the body.
- **A principle owes a boundary that NAMES a neighbour and does not TEACH it.** If a reader would act on the paragraph, it is a second entry at a different scope.
- **The why is a claim too.** It carries its provenance — the source, a document you can name, a measurement you ran, or the literal word *unsourced*. There is no fifth option, and a humbler invented why is still invented. Abstention means writing no why at all.
- One CLAIM = one entry. A note carrying several claims at several scopes becomes several entries.

## STEP 4 — review it

Call `experience(kind=review, type=…, summary=…, situation=…, verdict=…, details=…, symptoms=[…])`.

It returns what the deterministic gate says and a `prompt`. **Hand that prompt to an agent with NO session context and nothing else** — no background, no explanation, not your reasoning. Read its `VERDICT:` line.

The reader CANNOT check a fact. An entry can be fluent, correctly scoped and false, and it will pass; that is what the provenance rule bounds, not this step.

## STEP 5 — compare, and only one branch reaches him

| you | the reader | what happens |
|---|---|---|
| keep | keep | record it. One line reported. |
| drop | drop | drop it. One line reported. |
| keep | drop | **ASK HIM** — the one-line story, both verdicts, one question |
| drop | keep | **ASK HIM** — same |

Two agents agreeing is the common case and costs him nothing. A disagreement is exactly where a human should look, which is why this gate is conditional and therefore affordable.

## STEP 6 — RECORD IT. The store is the store, and it writes the file itself

**Record to the store. Do not write a markdown file by hand.** This instruction is the REVERSE of what stood here until Sprint 28f, and the reversal is the point: the store used to be rebuilt from a file substrate, so a direct record wrote a row nothing could ever put back, and every agent was told to author a file instead. The store now takes a backup before every destructive verb and writes durably, so the database is the truth and the folder is its export.

1. `experience(kind=record, type=…, summary=…, situation=…, verdict=…, details=…, symptoms=[…])`. It lands as a **candidate** — stored, findable, and vouching for nothing yet.
2. When the cold reader in STEP 4 PASSED it, accept it: `experience(kind=promote, id=<the id>, status=accepted)`. That transition is what moves the row into the lane that vouches for answers.
3. **The store writes the story file at that moment**, if a stories folder is configured, and stamps the row with the date the review happened. You never write that stamp, and that is deliberate — it is the one field `wipe_and_import`'s gate trusts, and a field a gate trusts must not be written by the thing it gates. Nothing you can do at step 6 can forge it.

**`experience(kind=load, path=…, recursive=true)` is for bringing files IN, never for saving.** It merges — it rewrites rows in place and removes nothing — so it is the right verb for a folder of notes somebody wrote by hand, or for a stories folder restored from git. It is not how you save what you just learned.

**NEVER `wipe_and_import` to add one entry.** That verb rebuilds the whole store from a root and retires every file-derived source the root no longer holds; reaching for it to add ONE file is how a routine save becomes a mass deletion, which is the accident the verb was renamed to make visible.

`lesson` and `failure_mode` are experiences: they owe a `situation` and a `verdict` (`worked` / `failed_avoid` / `unproven`), or the gate refuses them. A `domain_fact` owes NEITHER, and nor does an `api_contract`, a `naming_convention` or a `reference` — they never turned out any way at all, and inventing a verdict for one makes retrieval rank on fiction.

**Project state does not go in the store at all.** A sprint phase, a release announcement, "X is executing" — those are true on the day and false a month later, and nothing retires them. They go to the client's own file memory.

## What the run reports

    ── 2 candidates from this session ──
    1. the amend quantity is the total, not the remainder
    2. a superseded note does not know it was superseded

    stored   2  (the reader agreed on both)
    proposed 1  hook: refuse a commit authored by an agent → hooks/pre-commit
    asked    0

**Then echo the FULL TEXT of every story you stored — frontmatter and body, not a summary of it.** This command was pressed for months in the belief that it extracted a useful story, while it stored one compressed line; the only reason that could go unnoticed is that its report never showed what went in. A one-line confirmation of a one-line entry looks exactly like a one-line confirmation of a good one. Print each file's path beside its text, so the claim can be checked against the artifact rather than taken.

An empty run reports `no candidates — nothing in this session was durable and reusable`, and stores nothing. That is a good outcome, not a failed one."#,
        // Single-sourced: the pipeline is far too long to inline, and a second
        // copy would drift from the one the seats' own process runs.
        // Single-sourced for the same reason /sprint is: long enough that a
        // second copy would drift from the file a human reviews.
        "brainstorm" => include_str!("../../skills/brainstorm.md"),
        "btw" => include_str!("../../skills/btw.md"),
        "setup" => include_str!("../../skills/setup.md"),
        "sprint" => include_str!("../../skills/sprint.md"),
        _ => "",
    }
}

/// The command's own title. /sprint is a two-seat pipeline, not a one-liner
/// utility, and its rendered page should not claim otherwise.
fn utility_title(command: &str) -> &'static str {
    match command {
        "brainstorm" => "options, with their trade-offs",
        "btw" => "capture a thought, mid-task",
        "setup" => "the project declares what it is",
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

/// studio#44: a Cursor SKILL, like its seats — the frontmatter Cursor requires
/// (`name` matching the folder, `description`) plus the explicit-invocation flag
/// its own command converter sets. `/memorize` and `/sprint` are typed by a
/// person for the same reason a seat is.
pub fn render_cursor_utility(command: &str, description: &str) -> String {
    format!(
        "---\nname: {command}\ndescription: \"{description}\"\n\
         disable-model-invocation: true\n---\n\n{rest}",
        command = command,
        description = description,
        rest = utility_render(command, description, "")
    )
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
    // DERIVED, never typed: the sentence used to carry the words "seven" and
    // "Five" while the arrays said something else was true — a seat could be
    // added and the prose would keep announcing the old roster. The counts
    // now come from the arrays themselves, and a test pins that they do.
    let total = EMBEDDED_SEATS.len();
    let direct = COMMAND_MAP.len();
    let pipeline = total.saturating_sub(direct);
    let mut lines = vec![
        "## The jawata seats — narrow engineering roles, gate-disciplined".to_string(),
        String::new(),
        format!(
            "jawata ships {total} SEATS. {direct} are direct roles; \
             {pipeline} live in /sprint:"
        ),
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

    /// The registration CONSISTENCY gate — replaces a test that pinned the
    /// roster by hand (`command_map_is_exactly_the_five_pairs`).
    ///
    /// That test asserted the five pairs it already knew, so it pinned the
    /// ABSENCE of every later seat and had to be hand-edited to accept one.
    /// It therefore could not fail on the defect it should have caught:
    /// `seats/report.md` existed for a whole stage, was registered in no
    /// array, and so `/report` was deployed to NO client — the file was
    /// written, shipped in no binary, and nothing said a word.
    ///
    /// This asserts the RELATIONSHIPS instead, so a new seat needs no edit
    /// here and a seat that is only half-registered fails the build:
    ///  - every `seats/*.md` on disk is embedded (by NAME, not by count);
    ///  - every COMMAND_MAP row names an embedded seat and renders into the
    ///    deployed command list;
    ///  - the generated prose's counts are the arrays', not typed beside them.
    /// NO SHIPPED TEXT TELLS AN AGENT TO REBUILD THE STORE FROM FILES.
    ///
    /// Sprint 28f Stage 6 reversed the model: the database is the truth and the story
    /// folder is its export. `load` merges — it rewrites rows in place and removes
    /// nothing — while `wipe_and_import` (once `reseed`) rebuilds a whole store from a
    /// root and retires every file-derived source the root no longer holds. A text that
    /// sends an agent to the second one to save a single entry turns a routine save into
    /// a mass deletion.
    ///
    /// THE SWEEP EXISTS BECAUSE THE SEAT TEXTS WERE OTHERWISE UNGATED. The sibling test
    /// below asserts every `seats/*.md` is EMBEDDED; it never reads a word of their
    /// content. So `seats/review.md` could say — and did say, for three paragraphs —
    /// that a store write "becomes durable only when they say reseed", with nothing able
    /// to notice. The C6 audit found the gap by looking for this test and not finding it.
    ///
    /// It reads the SHIPPED bytes: the embedded seats and the rendered utility bodies,
    /// which is what a client actually receives.
    #[test]
    fn shipped_texts_name_no_reseed() {
        let mut offenders: Vec<String> = Vec::new();
        for (name, body) in EMBEDDED_SEATS {
            if body.contains("reseed") {
                offenders.push(format!("seats/{name}"));
            }
        }
        for (command, _) in UTILITY_MAP {
            let body = utility_body(command);
            if body.contains("reseed") {
                offenders.push(format!("/{command}"));
            }
        }
        assert!(
            offenders.is_empty(),
            "these shipped texts still name `reseed`, which rebuilds a whole store from a \
             root and retires every source that root no longer holds — a text that names \
             it as the way to SAVE turns a routine save into a mass deletion: {offenders:?}"
        );
    }

    /// AND THE INSTRUCTION THAT REPLACED IT IS NAMED, not merely absent.
    ///
    /// The sweep above is a negative, and a negative alone is satisfied by a text that
    /// says nothing at all about how knowledge is brought in. This asserts the positive
    /// half the plan asks for: the substrate rule instructs `load`.
    #[test]
    fn the_substrate_rule_instructs_load() {
        let memorize = utility_body("memorize");
        assert!(
            memorize.contains("kind=load"),
            "/memorize must name `load` as the way files are brought IN — without it the \
             no-reseed sweep is satisfied by a text that explains nothing"
        );
        let review = EMBEDDED_SEATS
            .iter()
            .find(|(n, _)| *n == "review.md")
            .map(|(_, b)| *b)
            .expect("review.md is embedded");
        assert!(
            review.contains("merge") || review.contains("merges"),
            "the review seat must say that bringing files in MERGES — its repair advice \
             is what a reader follows when a stored row and its source file disagree"
        );
    }

    #[test]
    /// D12: *"instrument discipline is a rule in each seat's text rather than something an
    /// agent is expected to remember."*
    ///
    /// DERIVED FROM `kind` AND `tools`, not written ten times. Ten hand-written paragraphs
    /// would be ten chances to disagree with the frontmatter beside them, which is the defect
    /// this sprint has been paying for all week — a claim in prose that the code next to it
    /// falsifies. One renderer cannot drift from the fields it reads.
    #[test]
    fn every_seat_is_told_its_instrument_discipline_and_it_matches_its_kind() {
        for seat in seats() {
            // THE RULE IS ASSERTED ON THE SEAT, not only on a channel. Two seats — spec-editor
            // and spec-auditor — live inside `/sprint` and render NO command of their own, so a
            // test that only read `render_claude_skill` would skip them silently and D12's
            // "each seat's text" would hold for eight of ten. Their copy reaches a reader
            // through the `/sprint` skill.
            let rule = instrument_discipline(&seat);
            assert!(
                rule.contains("## Instrument discipline"),
                "{}: no instrument-discipline rule — the agent is left to remember it",
                seat.name
            );
            // For the eight that DO render a command, the rule must actually be in the rendered
            // text — a derived paragraph nothing emits is the hollow shape this repo keeps
            // finding.
            let rendered = match render_claude_skill(&seat) {
                Some(r) => {
                    assert!(
                        r.contains("## Instrument discipline"),
                        "{}: the rule exists and the rendered skill does not carry it",
                        seat.name
                    );
                    r
                }
                None => rule,
            };
            match seat.kind {
                crate::runner::SeatKind::Code => {
                    // It must name the seat's OWN tools, so the rule is actionable rather than
                    // an exhortation to "use jawata".
                    for tool in &seat.tools {
                        assert!(
                            rendered.contains(tool.as_str()),
                            "{}: declares tool '{tool}' and the rendered rule never names it",
                            seat.name
                        );
                    }
                    assert!(
                        rendered.contains("jawata-fallback:"),
                        "{}: a code seat must be told the declaration a text search costs",
                        seat.name
                    );
                }
                crate::runner::SeatKind::Text => {
                    // AND THE TEXT SEATS MUST NOT BE TOLD TO FETCH FACTS. Before this, step 1
                    // of the shared loop told every seat to "find the work with the seat's own
                    // detectors (jawata tools)" — false for a seat whose input is an artifact,
                    // and the kind of instruction that sends an auditor looking for a tool.
                    assert!(
                        rendered.contains("no compiler fact to fetch")
                            && rendered.contains("judgement IS the product"),
                        "{}: a text seat must be told its judgement IS the product",
                        seat.name
                    );
                    // AND AN EMPTY `tools` MUST NOT READ AS "drives nothing", which is
                    // FALSE for two of the four text seats — review drives six store
                    // verbs and report drives `field` and `gh`, while spec-auditor and
                    // spec-editor genuinely drive none. Found by READING a rendered
                    // skill rather than by any gate: the first version of this
                    // paragraph told all four there was "no instrument to reach for".
                    assert!(
                        rendered.contains("does NOT mean this seat drives nothing"),
                        "{}: the empty-tools clause does not say what it does NOT mean, so a \
                         seat whose stance names store verbs is told it has no instruments",
                        seat.name
                    );
                    assert!(
                        !rendered.contains("jawata-fallback:"),
                        "{}: a text seat has nothing to fall back FROM",
                        seat.name
                    );
                }
            }
        }
    }

    /// D12's MEASURE, over the SHIPPED seats rather than a fixture: *"each seat file carries the
    /// task kind and the tools field."*
    ///
    /// The parser's own test proves the pairing refuses the two impossible shapes. It proves
    /// nothing about the ten files that ship — a fixture passing is what this repository means
    /// by a gate that cannot fail. This drives every embedded seat through the real parser and
    /// asserts the population, so a seat added without a kind cannot reach a release.
    #[test]
    fn every_shipped_seat_declares_a_kind_that_agrees_with_its_tools() {
        let mut text = Vec::new();
        let mut code = Vec::new();
        for (file, body) in EMBEDDED_SEATS {
            let seat = crate::runner::parse_seat_definition(body)
                .unwrap_or_else(|e| panic!("{file} does not parse: {e}"));
            match seat.kind {
                crate::runner::SeatKind::Text => {
                    // The parser already refuses tools here; asserted anyway, because this is
                    // the claim a reader of the seat files is relying on.
                    assert!(seat.tools.is_empty(), "{file} is text and names tools");
                    text.push(seat.name);
                }
                crate::runner::SeatKind::Code => {
                    assert!(!seat.tools.is_empty(), "{file} is code and names none");
                    code.push(seat.name);
                }
            }
        }

        // THE POPULATION, by his own ruling rather than by whatever the files happen to say:
        // *"the auditor and the communicator read text and use none; the architect must use
        // them; the production seats already do."* The architect is named in that sentence, so
        // its membership is asserted by name — if it ever drifts to `text` this fails loudly
        // rather than reporting a tidy count.
        text.sort();
        code.sort();
        assert_eq!(
            text,
            vec!["report", "review", "spec-auditor", "spec-editor"],
            "the text seats are the ones whose input is an ARTIFACT"
        );
        assert!(
            code.contains(&"architect".to_string()),
            "his ruling names the architect as a seat that MUST use the tools; it is {code:?}"
        );
        assert_eq!(text.len() + code.len(), EMBEDDED_SEATS.len(), "every seat classified");
    }

    /// D16: `/btw` and `/brainstorm` ship as TWO commands, and what `/btw`
    /// writes has a reader with a moment that fires.
    ///
    /// The second half is the one this product has failed three times — a
    /// capability employed by nothing — so it is asserted in BOTH places the
    /// rule lives: the `/sprint` skill's editor step (which the front-door
    /// agent follows) and the spec-auditor seat's own disposition check (which
    /// runs as the auditor). One without the other is a reader that fires in
    /// the orchestrator and an auditor that never asks, or the reverse.
    #[test]
    fn btw_ships_beside_brainstorm_and_its_capture_file_has_a_reader() {
        let commands: Vec<&str> = UTILITY_MAP.iter().map(|(c, _)| *c).collect();
        for needed in ["btw", "brainstorm"] {
            assert!(
                commands.contains(&needed),
                "/{needed} does not ship; UTILITY_MAP holds {commands:?}"
            );
            assert!(
                !utility_body(needed).is_empty(),
                "/{needed} ships with an empty body"
            );
        }

        // TWO, not one: each must say what the other is for, or a later reader
        // merges them and the capture inherits the brainstorm's cost.
        let btw = utility_body("btw");
        let brainstorm = utility_body("brainstorm");
        assert!(
            btw.contains("INTERJECTION") && brainstorm.contains("opposite way from `/btw`"),
            "the two commands do not distinguish themselves from each other"
        );

        // The capture must NEVER reach the store directly — the one hard rule of
        // the triage, because the store refuses by gate what a capture carries.
        assert!(
            btw.contains("NEVER write to the experience store directly"),
            "/btw does not forbid the direct store write"
        );
        assert!(
            btw.contains("/memorize"),
            "/btw names no forwarding path for durable knowledge"
        );

        // THE READER, AND THE MOMENT IT FIRES — in both copies of the rule.
        let sprint = utility_body("sprint");
        assert!(
            sprint.contains("docs/btw/<project>.md"),
            "the /sprint editor step does not read the capture file, so nothing consumes \
             a capture and /btw writes to a file with no reader"
        );
        assert!(
            sprint.contains("consumed:"),
            "the /sprint step names no consumed marker, so every pass re-offers every \
             capture and the disposition cannot be checked"
        );
        let auditor = EMBEDDED_SEATS
            .iter()
            .find(|(f, _)| *f == "spec-auditor.md")
            .map(|(_, b)| *b)
            .expect("spec-auditor.md is embedded");
        assert!(
            auditor.contains("THE BASELINE INCLUDES THE CAPTURE FILE"),
            "the auditor's disposition check does not cover captures, so the editor's \
             reading of them is enforced by nobody"
        );
    }

    /// D17: the review seat's housekeeping runs unattended on a declared
    /// cadence, reports what it DID, and asks the user to rule on no list.
    ///
    /// The clause carrying the risk is the SPLIT, so that is what this asserts
    /// from both sides: a scheduled run must not reach the deletion screen, and
    /// a hand run must still have it. Asserting only the first would pass a
    /// seat that had quietly lost the user's say over their own store; only the
    /// second, a seat that still stops a timer run to ask.
    #[test]
    fn the_review_seat_runs_unattended_and_rules_on_no_list() {
        let (_, body) = EMBEDDED_SEATS
            .iter()
            .find(|(f, _)| *f == "review.md")
            .expect("review.md is embedded");
        let seat = crate::runner::parse_seat_definition(body).expect("review.md parses");

        // A declared cadence, parsed — not merely a line in the file.
        let schedule = seat
            .schedule
            .as_deref()
            .expect("the review seat declares no schedule, so its housekeeping is hand-run only");
        crate::runner::Schedule::parse(schedule)
            .unwrap_or_else(|e| panic!("review.md's schedule '{schedule}' does not parse: {e}"));

        // The scheduled half: no list goes up, and steps 4 and 5 are named as
        // not running — a scheduled run that reached them would be a timer
        // blocking on a human who is not there.
        assert!(
            body.contains("Nothing you produce asks him to rule on a list of entries"),
            "the scheduled mode does not forbid the list, which is D17's own clause"
        );
        assert!(
            body.contains("Steps 4 and 5 do not run"),
            "the scheduled mode does not say which steps it skips"
        );

        // The hand half: the user keeps the disposal of their own knowledge.
        assert!(
            body.contains("**BY HAND.** Every step, unchanged"),
            "the hand mode no longer keeps step 4's screen — a scheduled seat must not \
             cost the user their say over their own store"
        );

        // The bound an unattended run needs, and the number that bounds the
        // deletion list's meaning.
        assert!(
            body.contains("At most 25\n   repairs per run") || body.contains("At most 25 repairs per run"),
            "the unattended run declares no repair bound"
        );
        assert!(
            body.contains("conformance") && body.contains("2.3%"),
            "the seat does not carry the measured reason a scheduled run declines to delete"
        );

        // D17's third clause: kept as data, not rendered.
        assert!(
            body.contains("PER-TRIGGER SPLIT IS DATA, AND YOU DO NOT RENDER IT"),
            "the per-trigger ruling is not in the seat's text"
        );

        // And the firing is named rather than assumed — nothing in studio runs
        // a timer, which this seat must not let a reader take on trust.
        assert!(
            body.contains("the client's own cron"),
            "the seat does not say what fires its schedule, so a declared cadence reads \
             as a running timer"
        );
    }

    /// D14: every seat is told to read the project's own declaration before it
    /// starts, and to NAME its absence rather than infer the facts.
    ///
    /// THE RENDERED SKILL IS THE BRIEF THAT ACTUALLY SHIPS, which is why this
    /// gate reads it rather than the hosted runner's prompt. Measured while
    /// building D14: `run_seat`, `run_javadoc_writer` and `run_test_writer` have
    /// no caller outside runner.rs and all three sit in the hollow-wiring
    /// baseline, so the hosted brief is carried by nothing today. Every seat
    /// that runs, runs by stance handoff from one of these texts.
    #[test]
    fn every_rendered_seat_is_told_to_read_the_projects_declaration() {
        let decl = crate::runner::PROJECT_DECLARATION_FILE;
        for (file, body) in EMBEDDED_SEATS {
            let seat = crate::runner::parse_seat_definition(body)
                .unwrap_or_else(|e| panic!("{file} does not parse: {e}"));
            // All ten, including the two that render no command of their own —
            // asserting only the rendered channel would skip them silently, and
            // D14 is about the brief EVERY seat gets.
            let contract = lane1_contract(&seat, &seat.name);
            assert!(
                contract.contains(decl),
                "{file}: the loop does not name the declaration file"
            );
            assert!(
                contract.contains("QUOTE its four facts"),
                "{file}: the loop does not say to QUOTE it — a seat told to 'consider' \
                 the project's facts summarises them, and a summary is where a guess hides"
            );
            assert!(
                contract.contains("If there is no such file, SAY SO"),
                "{file}: the loop does not require the ABSENCE to be named, which is \
                 the half D14 measures — an unnamed absence is indistinguishable from \
                 a project whose facts the seat quietly invented"
            );
            assert!(
                contract.contains("/setup"),
                "{file}: the loop names no way to create one"
            );
        }

        // And it reaches the rendered artifact, not only the contract that
        // builds it — the eight seats that render a command of their own, on
        // ALL THREE CHANNELS. The three renderers share one `lane1_contract`
        // call today, so asserting one channel is nearly a tautology over the
        // other two — nearly, and that gap is the point: a renderer that
        // stopped calling it and hand-built its own preamble would be caught
        // by the Claude gate alone and by nothing on the client the user
        // actually runs.
        for (channel, render) in [
            ("claude", render_claude_skill as fn(&crate::runner::SeatDefinition) -> Option<String>),
            ("cursor", render_cursor_command),
            ("antigravity", render_antigravity_workflow),
        ] {
            let rendered: Vec<String> = EMBEDDED_SEATS
                .iter()
                .filter_map(|(_, body)| {
                    let seat = crate::runner::parse_seat_definition(body).ok()?;
                    render(&seat)
                })
                .collect();
            assert_eq!(
                rendered.len(),
                8,
                "{channel}: the eight seats that render their own command \
                 (spec-editor and spec-auditor live inside /sprint)"
            );
            for skill in &rendered {
                assert!(
                    skill.contains(decl),
                    "{channel}: a rendered artifact omits the declaration file"
                );
            }
        }
    }

    /// D12's three remaining clauses, each a claim about a NAMED seat's own
    /// text: the audit round cap is a STOP in the spec-auditor's text, the
    /// architect's watch mode says a "no" is his decision rather than a logged
    /// finding, and the cataloguer carries its two burned steps.
    ///
    /// A text claim is checked as text — but not as a bare `contains`, which
    /// passes on a phrase and says nothing about whether the rule bites. Each
    /// clause is therefore checked at the point where it would be useless if
    /// absent: the cap at its stop-is-not-sign-off distinction, the "no" at
    /// its slot in the report structure, and the cataloguer's ask-step at its
    /// POSITION — a step that asks the lane AFTER the write is no step at all.
    #[test]
    fn three_named_seats_carry_the_clauses_d12_assigns_them() {
        let seat = |name: &str| -> &str {
            EMBEDDED_SEATS
                .iter()
                .find(|(f, _)| *f == format!("{name}.md"))
                .map(|(_, body)| *body)
                .unwrap_or_else(|| panic!("{name}.md is not embedded"))
        };

        // --- the spec-auditor: the cap is a STOP in its own text ---
        let auditor = seat("spec-auditor");
        assert!(
            auditor.contains("ROUND CAP IS YOURS TO STOP AT"),
            "the spec-auditor does not carry the round cap; it would live only in \
             the orchestrator's text, which is where it was forgotten"
        );
        assert!(
            auditor.contains("STOPPING IS NOT SIGNING OFF"),
            "the cap without that distinction is its dangerous half — an auditor \
             that stops by blessing an artifact it still has findings against"
        );

        // --- the architect: a "no" is a decision, and it has somewhere to go ---
        let architect = seat("architect");
        assert!(
            architect.contains("A \"NO\" IS HIS DECISION, NOT A FINDING YOU LOG"),
            "the architect does not carry the no-is-a-decision rule"
        );
        assert!(
            architect.contains("Decisions for you"),
            "the rule exists and the report structure gives its output no section \
             — a rule whose output has nowhere to go is dropped at the first run"
        );

        // --- the cataloguer: the ask comes BEFORE the write ---
        let cataloguer = seat("cataloguer");
        let ask = cataloguer
            .find("ASK THE LANE BEFORE YOU WRITE")
            .expect("the cataloguer does not carry the ask-the-lane step");
        let write = cataloguer
            .find("RECORD A JOB PER MEMBER")
            .expect("the cataloguer's write step moved or was renamed");
        assert!(
            ask < write,
            "the cataloguer asks the lane at byte {ask} and writes at {write} — an \
             ask after the write spends the budget it exists to protect"
        );
        assert!(
            cataloguer.contains("THE SECOND BURNED STEP"),
            "the cataloguer does not name its close-time record as the burned step \
             it is"
        );
    }

    /// D12: the run limits must bound TOOL STEPS, not only model turns.
    ///
    /// The bound exists in the runner with a default, so a seat that said
    /// nothing would still be bounded — which is exactly why this gate reads
    /// the seat's own FRONTMATTER TEXT rather than the parsed value. Every
    /// other ceiling is declared on all ten seats; a step ceiling visible
    /// only as a constant in Rust is the one limit a reader of the seat file
    /// cannot see, and "it has a default" is how it would stay that way.
    #[test]
    fn every_shipped_seat_declares_its_tool_step_ceiling_in_its_own_frontmatter() {
        for (file, body) in EMBEDDED_SEATS {
            let frontmatter = body
                .split("\n---")
                .next()
                .unwrap_or_else(|| panic!("{file} has no frontmatter"));
            assert!(
                frontmatter.contains("max_tool_steps:"),
                "{file} does not declare max_tool_steps — it would be bounded by the \
                 runner's default, which no reader of the seat file can see"
            );
            let seat = crate::runner::parse_seat_definition(body)
                .unwrap_or_else(|e| panic!("{file} does not parse: {e}"));
            assert!(
                seat.ceilings.max_tool_steps > 0,
                "{file} declares a zero step ceiling, which reaps before the seat acts"
            );
            // A code seat reaches for compiler facts and a text seat reads one
            // artifact, so the budgets are not interchangeable. Asserted as an
            // ORDERING rather than per-seat numbers: the numbers are a
            // judgement that may move, the ordering is the claim.
            let bound = seat.ceilings.max_tool_steps;
            match seat.kind {
                crate::runner::SeatKind::Code => assert!(
                    bound >= 40,
                    "{file} is a code seat with only {bound} tool steps"
                ),
                crate::runner::SeatKind::Text => assert!(
                    bound <= 30,
                    "{file} is a text seat budgeted {bound} tool steps — its input is an artifact"
                ),
            }
        }
    }

    fn every_seat_file_is_embedded_and_the_prose_counts_match() {
        let seats_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../seats");
        let mut on_disk: Vec<String> = fs::read_dir(&seats_dir)
            .expect("seats/ is readable from the crate root")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".md"))
            .collect();
        on_disk.sort();
        let mut embedded: Vec<String> =
            EMBEDDED_SEATS.iter().map(|(f, _)| (*f).to_string()).collect();
        embedded.sort();
        assert_eq!(
            embedded, on_disk,
            "every seats/*.md must be embedded — a seat definition registered \
             nowhere is shipped in no binary and reaches no client, however \
             finished the file itself looks"
        );

        // A command may only name a seat that actually exists…
        let definitions = seats();
        for (seat_name, command, _) in COMMAND_MAP {
            assert!(
                definitions.iter().any(|s| s.name == seat_name),
                "/{command} maps to seat '{seat_name}', which is not an embedded seat"
            );
        }
        // …and every command actually renders into the deployed list.
        let rendered = render_conductor_section(&definitions, "claude").join("\n");
        for (seat_name, command, _) in COMMAND_MAP {
            assert!(
                rendered.contains(&format!("- {seat_name} (`/{command}`)")),
                "/{command} is registered but renders into no client's command list:\n{rendered}"
            );
        }

        // The prose counts are DERIVED. A hand-typed "seven" is how the
        // deployed rule block kept announcing a roster that had changed.
        assert!(
            rendered.contains(&format!("jawata ships {} SEATS.", EMBEDDED_SEATS.len())),
            "the generated prose must count the seats it actually ships:\n{rendered}"
        );
        assert!(
            rendered.contains(&format!("{} are direct roles", COMMAND_MAP.len())),
            "the generated prose must count the direct roles it actually renders:\n{rendered}"
        );

        // The /sprint pair renders NO command — a rule, not a roster entry.
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

    /// Every seat's RECORD step must teach what an experience owes, because the
    /// engine's form gate REFUSES a `lesson` that carries no situation and no
    /// verdict.
    ///
    /// This is the producer side of a contract the engine tightened. A seat
    /// whose instructions still say "record a lesson with a summary" sends a
    /// payload the store rejects, and the seat run ends with its outcome
    /// unrecorded — silently, because a seat has no reason to re-read the
    /// response it discards. The engine-side gate cannot see this, and neither
    /// can any engine test: the producer lives in another repository.
    /// The architect is the FIRST PRODUCTION CONSUMER of the anchorless lane, and
    /// a consumer that exists only in a plan is not a consumer.
    ///
    /// A design question carries no symbol, package or operation, so ordinary
    /// recall cannot serve it — the two-step nominate/decide path is the only way
    /// the store can answer one. If the seat's own text does not tell it to make
    /// both calls, the engine ships a lane nothing walks down, which is the
    /// wired-not-built failure this codebase has now recorded five times.
    ///
    /// The abstention half is asserted separately and deliberately: a seat told to
    /// nominate but not told that selecting NOTHING is a real answer will pick the
    /// closest candidate, and a design built on a past experience that does not
    /// transfer is worse than one built on none.
    #[test]
    fn the_architect_seat_consults_the_store_and_is_told_it_may_choose_nothing() {
        let architect = seats()
            .into_iter()
            .find(|s| s.name == "architect")
            .expect("the architect seat ships, under its own name (the /refactor command \
                     is how it is invoked, not what it is called)");
        let skill = render_claude_skill(&architect).expect("it renders as a skill");

        assert!(
            skill.contains("kind=nominate") && skill.contains("kind=decide"),
            "the architect must call BOTH operations: ranking is not an answer, and a \
             nomination nobody decides on leaves the question unanswered"
        );
        assert!(
            skill.contains("SELECTING NOTHING"),
            "and it must be told that choosing none is a real answer — otherwise it \
             takes the closest candidate, and closest is not applicable"
        );
    }

    #[test]
    fn every_seat_teaches_what_an_experience_owes() {
        for seat in seats() {
            let Some(skill) = render_claude_skill(&seat) else {
                continue;
            };
            assert!(
                skill.contains("`situation`") && skill.contains("`verdict`"),
                "seat '{}' tells agents to record a lesson without naming the two \
                 fields the store requires — every outcome it records is refused",
                seat.name
            );
            assert!(
                skill.contains("failed_avoid"),
                "seat '{}' names no verdict vocabulary, so an agent must guess one \
                 and the store refuses whatever it guesses",
                seat.name
            );
        }
    }

    /// studio#44: a Cursor SKILL now, so it opens with the frontmatter Cursor
    /// requires rather than with the heading. `name` MUST match the folder the
    /// deploy writes it into — assert that here, because the folder is chosen
    /// in a different module and a mismatch is a skill Cursor silently ignores.
    #[test]
    fn cursor_command_embeds_contract_and_stance() {
        let seat = seats().into_iter().find(|s| s.name == "test-writer").unwrap();
        let cmd = render_cursor_command(&seat).expect("mapped");
        assert!(cmd.starts_with("---\nname: cover\n"), "{cmd}");
        assert!(cmd.contains("description: \""), "Cursor requires a description");
        assert!(
            cmd.contains("disable-model-invocation: true"),
            "a seat is invoked deliberately, which is what Cursor's own converter              records for a command"
        );
        assert!(cmd.contains("# /cover — the jawata test-writer seat"));
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

    
    /// Sprint 28f Stage 7 deliverable 4 — the /catalogue command RENDERS FROM the seat
    /// file, rather than from prose typed beside it.
    ///
    /// The seat's binding half is its FAILURE SHAPE — the three bad summaries it tells the
    /// agent to refuse in its own draft. A command that carried the loop and dropped that
    /// text would look complete and would let through exactly the output the store's form
    /// gate then rejects, which is how a whole batch's budget gets spent on refusals. So the
    /// assertion is on the stance reaching the command verbatim, not on the command merely
    /// existing.
    /// EVERY RECALL A SEAT INSTRUCTS MUST NAME A CUE — the guard whose absence let a
    /// dead detection call ship.
    ///
    /// The architect seat's D-SIX told the agent to run
    /// `experience(kind="recall", lane="code", population=true)`. Neither of those two
    /// selects anything: `population` is not a parameter at all, and `lane` is a FILTER
    /// that narrows what the cues found. So the call named no CUE, and the store's whole
    /// answer to it was the sentence "No cue — provide symbol / package / operation /
    /// symptom." Half of D-SIX's detection had never run, and nothing could say so,
    /// because a seat's text is prose and prose compiles.
    ///
    /// This reads the EMBEDDED seat text — the same bytes the deployed skill carries —
    /// and asks one structural question of every recall in it. It cannot check that the
    /// engine answers well; it checks that the call can select anything at all, which is
    /// the half that was false.
    #[test]
    fn every_recall_a_seat_instructs_names_a_cue() {
        // The cues recall dispatches on. `lane` is deliberately absent: it is a filter
        // over what the cues found, so a call carrying only `lane` still selects nothing.
        const CUES: [&str; 5] = ["symbol=", "package=", "operation=", "symptom=",
                                 "external_system="];
        // BOTH SPELLINGS, because these files use both: architect writes
        // `kind="recall"` and `kind=nominate` in the same document. A matcher on one of
        // them is blind exactly where the corpus varies, which is how a guard reads green
        // over the case it was written for.
        let calls: Vec<(String, String)> = seats()
            .into_iter()
            .flat_map(|seat| {
                let stance = seat.stance.clone();
                let name = seat.name.clone();
                ["kind=\"recall\"", "kind=recall"]
                    .iter()
                    .flat_map(|spelling| {
                        stance
                            .match_indices(spelling)
                            .map(|(at, _)| {
                                let tail = &stance[at..];
                                let end =
                                    tail.find(')').map(|e| e + 1).unwrap_or(tail.len().min(240));
                                (name.clone(), tail[..end].to_string())
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        for (seat_name, call) in &calls {
            assert!(
                CUES.iter().any(|c| call.contains(c)),
                "seat `{seat_name}` instructs a recall naming no cue, so the store answers \
                 \"No cue — provide symbol / package / operation / symptom.\" and the step \
                 is one round-trip to a refusal that reads like detection:\n{call}"
            );
        }
        assert!(
            !calls.is_empty(),
            "PROOF OF LIFE: no seat instructs a recall at all, so the loop above asserted \
             nothing. If a seat's call was reworded out of this shape the guard went blind \
             rather than green."
        );
    }

    #[test]
    fn the_catalogue_command_renders_from_the_cataloguer_seat() {
        let seat = seats()
            .into_iter()
            .find(|s| s.name == "cataloguer")
            .expect("the cataloguer seat must be embedded");
        let cmd = render_claude_skill(&seat).expect("cataloguer maps to a command");

        assert!(cmd.contains("name: catalogue"), "the slash name is catalogue:\n{cmd}");
        assert!(
            cmd.contains(&seat.stance),
            "the seat's stance is embedded VERBATIM — a paraphrase is a second copy that \
             drifts from the file it came from"
        );
        assert!(
            seat.stance.contains("Parses a compilation unit."),
            "the stance must carry the FAILURE SHAPE by example; without it the seat \
             describes the wanted output and leaves the wrong one unnamed, which is this \
             project's recorded reason a seat drifts"
        );
        assert!(
            seat.stance.contains("action=done") && seat.stance.contains("contentHash"),
            "and the loop must name the verb that closes a unit, with the hash it needs — \
             a run that never closes a unit re-describes it next time"
        );
    }

    #[test]
    fn phrase_table_covers_every_command() {
        let table = render_phrase_table(&seats());
        for (_, command, _) in COMMAND_MAP {
            let (seat_name, _, _) = COMMAND_MAP.iter().find(|(_, c, _)| *c == command).unwrap();
            assert!(
                table.contains(&format!("**{seat_name}**")),
                "/{command}'s seat in the table:\n{table}"
            );
        }
        assert_eq!(
            table.lines().count(),
            2 + PHRASE_MAP.len(),
            "header + one row per phrase, nothing more"
        );
    }

    #[test]
    fn materialize_seeds_refreshes_unedited_and_preserves_edits() {
        let seats_dir = unique_tempdir("materialize").join("seats");
        let first = materialize_seats(&seats_dir).unwrap();
        assert_eq!(
            first.seeded.len(),
            EMBEDDED_SEATS.len(),
            "every embedded seat materialized on first run"
        );

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
            "the other identical files are adopted silently: {report:?}");
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
        assert_eq!(
            ok.len(),
            EMBEDDED_SEATS.len(),
            "every good seat still loads beside the broken file"
        );
        assert_eq!(errors.len(), 1, "the broken file is a loud per-file error");
        assert!(errors[0].0.ends_with("broken.md"));
    }
}
