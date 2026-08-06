# ARCHITECTURE — the hook shim and the seam between the products (Sprint 28)

> **Version 2**, 2026-08-06. Design-mode artifact for Sprint 28, produced after GATE 1
> sign-off and before the plan. The plan is written against this document, and the
> Phase-B auditor checks that it matches.
>
> Spec: `jawata-enterprise/docs/sprints/jawata-mcp/sprint-28-prove-the-engine.md`.
> Vendor contract + measurements: `jawata-enterprise/docs/cursor-hooks-open-questions.md`.
>
> **Version 1 was reviewed NOT SOUND** — five blocking design findings and nine factual
> errors, one of which contradicted its own table. What changed, and why, is recorded
> in "How Version 1 was wrong" at the end. Read that section before trusting any
> reasoning that looks familiar from v1.

## The subject

Sprint 28 is not six unrelated deliverables. It is **one seam** — the boundary between
jawata-studio (Rust/Tauri, owns the client's hooks) and jawata-mcp (Java/JDT, owns the
store and the tools) — plus the instruments that make a break in that seam loud.

Every defect the sprint exists for lived at a boundary where **one side changed and the
other side's assumption was never re-checked**:

| Defect | The boundary | What crossed it unchecked |
|---|---|---|
| the hook outage | studio's bash script ↔ the store's answer shape | 21c said "one fact or silence"; 27a made multi-answer normal |
| the v3.4.0 inert release | production wiring ↔ the capability | every production site built the engine without its embedding part |
| `jawata-mcp#9` | the JDT project model ↔ a name-shaped assumption | the code asks for a *project* named `*.tests`; jawata makes one project per workspace root, suffixed with a session id |

## Current structure, as measured

Every number below was read from the code or a live tool call on 2026-08-06. Where v1
guessed, it is corrected.

### studio side

`src-tauri/src/manager_service.rs` is **9 124 lines** and holds all ten shipped hook
scripts as bash text, written at deploy time. `#[cfg(test)] mod tests` begins at line
6334; the three shebangs after it are test fixtures, not shipped scripts.

**The real role × client matrix** (v1 had two cells in the wrong rows):

| Role | Claude event | Claude | Cursor event | Cursor |
|---|---|---|---|---|
| Guard | `PreToolUse` | `build_guard_script()` — a `format!`, **no token** | `beforeShellExecution` | `CURSOR_GUARD_TEMPLATE` — **no placeholders**, `failClosed: true` |
| Observer | `PostToolUse` | `OBSERVER_TEMPLATE` | `postToolUse` | `CURSOR_OBSERVER_TEMPLATE` — **no placeholders** |
| Primer | `SessionStart` | `PRIMER_TEMPLATE` | `sessionStart` | `CURSOR_PRIMER_TEMPLATE` |
| Tool-time recall | `PreToolUse` (matcher `mcp__jawata.*`) | `RECALL_TEMPLATE` | — | **absent** |
| Prompt-time recall | `UserPromptSubmit` | `USERPROMPT_TEMPLATE` | `beforeSubmitPrompt` | `CURSOR_RECALL_TEMPLATE` — **side-effect only** |
| Stop gate | `Stop` | `STOP_TEMPLATE` | — | absent |

Ten scripts across **twelve** cells, **two** empty. Cursor has no tool-time recall at
all, and its prompt-time recall cannot inject — confirmed against Cursor's current docs
2026-08-06: `beforeSubmitPrompt` returns only `{continue, user_message}`;
`additional_context` exists solely on `sessionStart` and `postToolUse`.

**Configuration is baked in for 7 of the 10**, by substituting `__MCP_URL__` and
`__TOKEN__`. Three carry neither: the Claude guard takes a health URL through `format!`
and no token at all; both Cursor guard and observer have no placeholders.

**Two client config dialects, not one:**

- Claude: `{"type":"command","command":"<absolute path>"}` under `~/.claude/jawata-studio/`.
- Cursor: `{"command":"./hooks/jawata-guard.sh","timeout":5,"failClosed":true,"matcher":…}`
  — **relative** path, **no `type` field**, extra keys.

**Hook identity is the per-role filename.** Five sentinels
(`jawata-studio/pretooluse-guard.sh`, `…/posttooluse-observer.sh`,
`…/sessionstart-primer.sh`, `…/pretooluse-recall.sh`,
`…/userpromptsubmit-recall.sh`) plus `hooks/jawata-` for Cursor drive every
`is_*_entry` predicate, the replace-in-place merge and the undeploy removal.
`legacy_sentinel()` covers exactly one prior generation, `goja-*` → `jawata-*`.

**No Windows handling exists**: all four `set_permissions(0o755)` calls are inside
`#[cfg(unix)]`, and `manager_service.rs` contains no `cfg(windows)`, no PowerShell, no
`.cmd`. (`runtime_manager.rs` has Windows branches, but for the resident JVM, not hooks.)

**JSON is parsed by regex**, with a comment at 5781 explaining why the pattern must be
`[^"]*` rather than `.*`. That parser cannot know what a JSON string is; it works until
the payload shape moves and then fails silently.

**No `[[bin]]` target and no `[workspace]`** exist. `tauri.conf.json` has neither
`externalBin` nor `resources` — there is no mechanism today for shipping a second
executable.

### mcp side

- `QualityDetectors.builtins(Supplier<IJdtService>) → DetectorCatalog` at
  `org.jawata.mcp.tools.QualityDetectors:27` — the detector registration seam.
- `LoadedProject` (record) carries `IJavaProject javaProject` **and a single
  `BuildSystem` enum for the whole workspace**.
- `build/end-to-end-test.sh` + `build/e2e-fixture/entries.json` (48 invented entries,
  hash-compared). **No workflow references the script.** It contains **no tool-count
  assertion** — "the count is still 45" is one of the spec's *unchecked* promises, not
  an existing check.

### The mcp#9 mechanism, proven

`CompileWorkspaceTool.matchesScope(String,String,String)`:

```java
boolean mavenTest  = pathStr.contains("/src/test/") || pathStr.contains("\\src\\test\\");
boolean testBundle = projectName.endsWith(".tests");
boolean inTest     = mavenTest || testBundle;
boolean inMain     = !inTest && (mavenMain || pathStr.endsWith(".java"));
```

`JdtServiceImpl:276` builds the name as `"jawata-" + rootDirectory`, and
`WorkspaceManager:128` appends a session id — measured here,
`jawata-jawata-mcp-8dbe9351`. **A name ending in a session id can never end in
`.tests`, on any workspace**: `testBundle` is unreachable code. PDE bundles keep
sources under `src/`, so `mavenTest` is false too — and then **the `.java` catch-all in
`inMain` sweeps every unclassified file into main.** Live: `scope=main` 133 warnings
including both test bundles, `scope=test` 0.

**Both halves must change.** v1 listed the Maven branch as untouchable and named only
the project-name branch; the `.java` fallback is what actually produces the wrong
answer.

## Target architecture

```
        ┌──────────────────────────── THE SEAM ────────────────────────────┐
   ╔════╧═════════════════════════╗                    ╔══════════════════╧════════╗
   ║        jawata-studio         ║                    ║        jawata-mcp         ║
   ╠══════════════════════════════╣                    ╠═══════════════════════════╣
   ║  crate: jawata-studio        ║                    ║  bundle org.jawata.mcp    ║
   ║   manager_service            ║                    ║   ToolRegistry (45 tools) ║
   ║    · deploy / undeploy       ║                    ║   QualityDetectors        ║
   ║    · merge client config     ║                    ║        │ registers        ║
   ║    · writes role-named files ║                    ║        ▼                  ║
   ║    · writes hook_config      ║                    ║   smell/                  ║
   ║           │                  ║                    ║    TestOnlyCallerDetector ║
   ║           │ (data only)      ║                    ║           │ reads         ║
   ║           ▼                  ║                    ║           ▼               ║
   ║  ~/.claude/jawata-studio/    ║                    ║   SourceRootClassifier ◄──╫── CompileWorkspaceTool
   ║   jawata-hook-guard          ║                    ║           │ reads         ║      (scope=)
   ║   jawata-hook-observer       ║                    ║           ▼               ║
   ║   jawata-hook-primer         ║                    ║  ╔═════════════════════╗  ║
   ║   jawata-hook-recall         ║                    ║  ║ bundle org.jawata.  ║  ║
   ║   jawata-hook-userprompt     ║                    ║  ║        core         ║  ║
   ║   jawata-hook-stop           ║                    ║  ║  LoadedProject      ║  ║
   ║   config.json  (atomic)      ║                    ║  ║   + per-root TEST   ║  ║
   ║           ▲                  ║                    ║  ║     tag (NEW)       ║  ║
   ║           │ argv[0] dispatch ║                    ║  ║  ProjectImporter    ║  ║
   ║  ╔════════╧═══════════════╗  ║   HTTP + serde     ║  ║   TAGS AT IMPORT    ║  ║
   ║  ║ crate: jawata-hook     ║  ║ ═════════════════► ║  ║   (per build system)║  ║
   ║  ║  cue · query · emit    ║  ║ ◄═════════════════ ║  ╚═════════════════════╝  ║
   ║  ║  + FAIL-SAFE exit      ║  ║                    ╚═══════════════════════════╝
   ║  ║  deps: serde, reqwest  ║  ║                       ▲ compiler-enforced:
   ║  ║  NO tauri, NO studio   ║  ║                         core cannot see mcp
   ║  ╚════════▲═══════════════╝  ║
   ╚═══════════╪══════════════════╝
               │ exec (role = the name it was invoked as)
        ┌──────┴───────┐
        │  the CLIENT  │
        └──────▲───────┘
               │  the ONE path nothing tested
   ┌───────────┴──────────────────────────────────────────────────┐
   │ D-E2E, as a RELEASE GATE: real prompt → hook → store → back  │
   └──────────────────────────────────────────────────────────────┘
```

### Module 1 — `jawata-hook`, a separate workspace crate

**Not a module inside the app.** `src-tauri/` gains a `[workspace]`; `jawata-hook`
becomes a member crate depending only on `serde`, `serde_json` and an HTTP client.

**This is the only way the constraints are real.** v1 placed the hook logic in the same
crate as `manager_service`, in a package that already depends on Tauri, and then listed
`hook_core ⇏ tauri` as something "to watch in review" — which is an admission that
nothing enforces it. As a separate crate, both forbidden edges become **compile
errors**, and the hook binary stops linking Tauri, the HTTP server, and the archive
libraries it has no use for. That also delivers the start-up cost the design claims,
rather than asserting it.

A workspace member is driven by `cargo test` exactly as a module is, so nothing is lost.

### Module 2 — three concerns, and role as data

v1 cut this as "6 roles × 2 dialects" with Strategy over the role. That merges three
genuinely different shapes — query-and-inject (primer, both recalls), local policy
decision (guard: fail-closed, emits a permission), and fire-and-forget (observer) — and
it hides the only piece with real algorithmic content.

**The cut is by concern:**

```
  cue.rs     extract cues from the client's event payload
             (~45 lines of stopword filtering, n-gram tiering, rarity marking —
              THE PLACE THE HOOK OUTAGE LIVED. Client-independent. Property-tested.)
  query.rs   ask the store; parse the envelope with serde_json
  emit.rs    encode for the client — Claude and Cursor dialects
  roles.rs   a TABLE: role → {event name per client, which concerns apply, can it inject}
```

Role and dialect become **data in a table**, not two Strategy axes. The two empty cells
and Cursor's inability to inject are then rows in that table — an explicit absence
rather than four files nobody notices.

- **Pattern:** pipeline of three stages, driven by a declarative table.
- **Smell it prevents:** the shotgun surgery that let one of ten scripts keep a retired
  contract. A change to the answer shape has **one** place to be wrong — in `query`.
- **`serde_json` replaces the regex**: a payload-shape change becomes a typed error.

### Module 3 — the fail-safe boundary

**One function is the only exit**, and its rule outranks correctness:

> Any error, panic, timeout, missing config or malformed response → **emit nothing,
> exit 0.**

v1 specified `catch_unwind` plus a transport timeout. Measured and reviewed, that is
insufficient. The boundary must carry all of:

| Hazard | Mitigation | Status |
|---|---|---|
| Blocking stdin read | **Our own deadline on the stdin read**, never the client's timeout | Measured on Claude Code: EOF at 4.3 ms. Cursor unmeasured, and Cursor's own guidance is "bound your own read" |
| `panic = "abort"` disarms `catch_unwind` | **Pin `panic = "unwind"`** in the crate's release profile, with a test asserting it | No `[profile]` exists today, so the default holds — and nothing guards it |
| Stack overflow, OOM, panic-in-`Drop` | A **watchdog thread with a total deadline** that `exit(0)`s regardless of where the main thread is parked | Not covered by `catch_unwind` |
| Missing / non-executable binary | No code of ours runs — this is the client's behaviour | **Measured**: Claude Code proceeds normally (fail-open). **Cursor's guard is `failClosed: true`** and would likely block; unmeasured, a 28a probe |
| No published client timeout default | **Set `timeout` explicitly on every entry we write** | Cursor documents the default only as "platform default" |

### Module 4 — `hook_config`, written atomically, read concurrently

One binary cannot be ten specialised copies, so endpoint and token move to a file
beside the binary, written at deploy and read at fire time.

**Concurrency is measured, not assumed.** Three sessions with a holding hook produced
**three overlapping pairs** — invocations run in parallel. Therefore:

- config is written **temp-file + rename**, so a reader never sees a torn file;
- **any file a hook writes** (decision log, counters) is append-under-lock or
  per-process — never read-modify-write;
- re-deploy **unlinks before writing** the binary, or Linux returns `ETXTBSY` while a
  hook is executing. This hazard is *introduced* by the change: overwriting a `.sh` has
  no such problem.

**Honest scope of "works with Studio closed":** moving config to disk removes the need
for Studio's *process*. It does not make the hooks independent of the resident JVM,
whose lifecycle Studio owns — with the resident down, the memory hooks correctly get
nothing, and the guard already tells the model to start it. v1 asserted a stronger
requirement than the system meets.

### Module 5 — `argv[0]` dispatch, and the deploy contract

**The role is the name the binary was invoked as** — `jawata-hook-recall`,
`jawata-hook-guard`, … — not an argument. Symlinks or hardlinks on Unix, copies on
Windows.

Three reasons, and the first is decisive:

1. **Windows tokenization of the `command` string is unspecified.** Cursor documents it
   only as "a shell string"; whether it runs through `cmd.exe`, PowerShell, or a direct
   spawn is not published, staff guidance recommends forcing PowerShell explicitly, and
   there are known launcher bugs under Git Bash. A design that needs
   `"<path> <role>"` to tokenize our way rests on unspecified behaviour **on the one
   platform D-SHIM exists to serve.** With no argument, that risk disappears; what
   remains is "can the client execute a path", which is what ships today.
2. **Per-role path identity is preserved**, so the sentinel/merge/undeploy machinery
   stays close to as-is instead of being rewritten.
3. **No launcher wrapper**, so we inherit none of the PowerShell bootstrap bugs.

**The deploy contract still changes in one way v1 denied.** The filenames change
(`userpromptsubmit-recall.sh` → `jawata-hook-userprompt`), so:

- every sentinel becomes the new role-named file, **and the `.sh` generation joins the
  legacy-removal set**. `legacy_sentinel()` handles exactly one prior generation
  (`goja-*`); without a third, an existing install's old entries match no sentinel, are
  classified as *the user's own hook*, and are **preserved forever** — the retired
  script and the new binary both firing. That is the hook-outage shape, shipped by the
  fix for the hook outage.
- a test asserts that an install carrying the **old** entries converges to exactly one
  managed entry per event, with the user's own entries untouched. The existing
  `goja` → `jawata` migration and its test are the working precedent.

### Module 6 — test-ness is tagged AT IMPORT

**v1's central error.** It claimed *"the project model already distinguishes"* the
source roots and printed a table with a `TEST` column. The model does **not**:
`inspect(kind=classpath)` returns every root as `{"kind":"source"}` with no test
attribute. The column was an annotation derived from the linked-folder **name** — the
name convention that has already failed twice here. A classifier reading that would be
a *third* derivation of build-system layout knowledge, which the module's own rule
forbids, and would reproduce mcp#9.

**The knowledge exists and is thrown away.** `ProjectImporter.readPomSourceDirs`
returns `SourceDirs(srcMain, srcTest)` — the importer *knows*.
`addSourcePathsFromDirectory` then flattens both into one `List<Path>`, and
`addSourceEntries` emits `JavaCore.newSourceEntry(path, …, null)` with no
`IClasspathAttribute.TEST`, though Eclipse `.classpath` supports exactly that
attribute.

**The fix is at the producer:**

> `ProjectImporter` **tags each source root's test-ness where it already determines
> it** — Maven `srcTest`, Gradle test source sets, Tycho `eclipse-test-plugin`
> packaging, the `.classpath` `test` attribute — and carries the tag into the model.
> `SourceRootClassifier` **reads the tag**. It derives nothing.

This also repairs the justification for sharing. v1 argued D-IMPORTER and D-UNWIRED
"need the same knowledge"; D-IMPORTER's own measure never mentions test-vs-main. The
true statement is narrower and stronger: **the importer is the only place that knows,
it must record what it knows, and every consumer reads that record.** Two consumers
today — `compile_workspace(scope=)` and the new detector.

`LoadedProject.buildSystem` is a **single enum for the whole workspace** (`MAVEN` here,
because of a root `pom.xml`, while roots 5 and 6 are PDE bundles), so per-root
provenance must live on the root, not be inferred from the project.

- **Dependency direction:** `smell/` → `SourceRootClassifier` → `LoadedProject`
  (bundle `org.jawata.core`). Core cannot see mcp without a Require-Bundle cycle, so
  this edge **is** compiler-enforced — unlike v1's studio-side edges.
- **Gate:** a **live tool call**, never a unit test. A unit test hand-feeding
  `"org.jawata.mcp.tests"` is what closed mcp#9.
- The `.java` catch-all in `inMain` changes with it, or unclassified files silently
  stay "main".

### Module 7 — `TestOnlyCallerDetector`, and what runs it

Lands in `org.jawata.mcp/src/org/jawata/mcp/tools/smell/`, registered in
`QualityDetectors.builtins` so it appears in the standard `find_quality_issue` sweep.

**But registration is not a runner.** v1 stopped there, which made the instrument for
"built but not connected" itself a capability nobody invokes. **The release gate runs
`find_quality_issue` over jawata's own repository and fails on a new finding**, against
a committed baseline — the tool already supports `baseline: save` / `diff`.

**Bounded cost:** the check is a reference search per public member. It declares a
scope default and a measured runtime, and runs through the async sweep path. A detector
too slow to run is a detector nobody runs, which is the same failure again.

**Stated limit:** a caller can exist while the capability is dead — a production site
passing an empty dependency keeps a caller and leaves the wire dead. D-E2E carries the
weight; this is necessary, not sufficient.

### Module 8 — the hook says why it stayed silent

**The strongest argument against v1, which v1 did not make:** it replaces ten scripts a
human can `cat` with an opaque binary whose only specified failure behaviour is to be
**indistinguishable from success** — while deferring D-REACH, the counter that would say
"fired 400 times, injected 0", to 28d. Observability would arrive strictly after
opacity. The fail-safe rule, uninstrumented, is the *mechanism* of the outage it
repairs, institutionalised one level lower where nobody can read it.

**Therefore, in this sprint, not 28d:**

- the fail-safe exit **records why it emitted nothing** — a bounded, append-under-lock
  line: role, timestamp, and one of `no-cues` / `store-said-nothing` / `unreachable` /
  `timeout` / `bad-response` / `no-config` / `panic`;
- the binary has a **`--explain` mode** that runs the real path and prints the decision
  instead of a canned string. The spec already condemns the predecessor: the old
  self-check *"emitted a canned string and exited before any call to the store"*, so it
  proved the script could print, never that it printed a real answer. A binary makes
  the honest version cheap.

This is the minimum that keeps "silence because nothing to say" distinguishable from
"silence because broken". 28d still owns the aggregate reach metric; this owns the
per-invocation reason.

### Module 9 — D-E2E as a release gate

The test and its 48-entry fixture exist. Four changes, no new module:

1. **A workflow runs it** and fails the build — today nothing does.
2. **Checks renamed** from `27a-D1a`/`D5`/`D6` to the promise each protects.
3. **The fixture gains what import cannot produce**: rows at an older schema and a mix
   of vectored and unvectored rows, so the *upgrade* path is exercised. Import writes
   current-schema rows, which is why three of the four v3.4.0 defects still have no
   live check.
4. **The tool-count check is written** — it does not exist today.

The genuinely new check is the **cross-product** one: a real prompt reaches the hook
binary, the hook reaches the store, context comes back. It exercises the hook crate
through its deployed role-named entry point against a running resident.

### Module 10 — shipping a second executable

**Unaddressed in v1, and it falsifies v1's sizing.** `tauri.conf.json` has no
`externalBin` and no `resources`. Shipping a binary alongside the app needs:

- an `externalBin` entry with Tauri's target-triple suffix convention
  (`jawata-hook-x86_64-unknown-linux-gnu`, …), and a per-platform build step;
- a copy from the bundle into the deploy directory, creating the role-named links;
- **macOS notarization of a second Mach-O** — the subject of studio#2, which took two
  release rounds to get right for one binary;
- Windows: an unsigned executable that runs on every prompt, with the SmartScreen and
  Defender consequences that implies (signing is Sprint 33).

## Dependency direction — and which edges are actually enforced

```
  jawata-hook (crate) ──► serde, serde_json, http          ENFORCED (crate deps)
  jawata-hook         ──X── tauri                          ENFORCED (compile error)
  jawata-hook         ──X── jawata-studio crate            ENFORCED (compile error)
  manager_service     ──► hook_config file                 data only, no code edge
  manager_service     ──X── jawata-hook                    ENFORCED (deploy never runs hooks)

  smell/ ──► SourceRootClassifier ──► LoadedProject        review-enforced within mcp
  ProjectImporter ──► the TEST tag on each root            the producer
  LoadedProject ──X── smell/                               ENFORCED (bundle boundary)
```

**Every `MUST NOT` in this version is compiler-enforced**, by a crate boundary or a
bundle boundary. v1 had one real constraint and two decorative ones and did not say
which was which.

## What must NOT be touched

| Area | Why |
|---|---|
| `ToolRegistry`'s 45-tool surface and the fact gate | Out of scope. The e2e gains an assertion that the count is still 45 — it has none today |
| The 48 committed fixture entries as they stand | Additive only: invented, non-corpus, hash-compared pristine per run |
| Cursor's inability to inject on `beforeSubmitPrompt` | A platform fact, confirmed against current vendor docs. It becomes a row in the role table |
| The experience store's schema and its import/restore paths | Not this sprint |
| `matchesScope`'s Maven-path branch **as a rule** | The `/src/test/` convention is correct where it applies. But the `.java` catch-all beside it **does change** — v1 wrongly protected the whole expression |

**Removed from v1's list, because they are load-bearing and must change:** the
deploy/undeploy merge semantics (filenames change, so sentinels and the legacy set
change with them), and the `inMain` fallback above.

## Open, and deliberately not decided here

- **Windows execution of a role-named binary** at an absolute path, with a space in the
  path, and missing-under-`failClosed` — unspecified by Cursor's contract, needs the VM,
  **28a**. `argv[0]` dispatch is chosen precisely to minimise what those answers can
  break.
- **A version handshake across the seam.** `serde_json` turns a *shape* change into an
  error, but a *semantic* change — 21c's "one fact or silence" becoming 27a's eleven
  nominees — parses fine. That is exactly what happened. Whether the hook and store
  exchange a contract version is a real question this sprint does not answer, and it is
  the most likely site of the next instance.

## How Version 1 was wrong

Recorded because the sprint is about inherited claims that were never measured, and the
design document committed the same fault.

**Design (five blocking):** the classifier rested on a `TEST` column I wrote by hand and
presented as tool output; the deploy contract was said to change "in exactly one way"
when hook identity is the filename and no third-generation migration existed; the
fail-safe covered neither a blocking stdin read nor `panic="abort"` nor a missing
binary; nothing ran the new detector; and the two `MUST NOT` edges were unenforceable
because there is no workspace.

**Facts (nine):** 8 400 lines (9 124); "two other shebangs" (three); Cursor's recall
filed under the wrong event; "each template carries `__MCP_URL__` and `__TOKEN__`"
(7 of 10); one config dialect (two); "fourteen-way product … four missing files" (12
cells, two empty — contradicting its own table); "the e2e asserts the count is still
45" (no such assertion exists); "works with Studio closed" asserted as met; and the
`.java` fallback protected as correct.

**The pattern, stated so it can be checked next time:** every factual error was on the
studio side. The mcp-side claims — the detector seam, `LoadedProject`, the six build
systems, the mcp#9 chain, the seven roots, the 133/0 — were all true, because they came
from compiler-accurate tools. The Rust claims came from reading greps. **Where there was
no precise instrument, there was no precision.**

## The one-line summary for watch mode

**Ten specialised bash copies become one small crate with three concerns and a role
table, dispatched by the name it is invoked as, reading its config from disk and
recording why it stayed silent; and the importer tags test-ness where it already knows
it, so the scope filter and the new detector both read one record instead of deriving a
third.** Any checkpoint diff that adds a second place to know either of those things,
or that lets a hook fail invisibly, is moving away from this picture.
