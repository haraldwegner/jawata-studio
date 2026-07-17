# ARCHITECTURE-runner.md — target architecture for the studio runner/seat subsystem (v3.0.0)

## Scope

The reusable layer that spawns and drives a narrow agent (a "seat") through a
bounded, gated, journaled task loop: detect the work → do it → verify it →
stop → record the outcome. This document is the baseline WATCH MODE diffs
against — it fixes module boundaries, seams, and dependency direction before
Sprint 29's pipeline work and Sprint 33's non-Claude adapters land.

## Module diagram

```
                         +-------------------+
                         |     scheduler     |   Schedule
                         |                   |   journal_nothing_to_do
                         +---------+---------+
                                   | triggers
                                   v
       +----------------+   +-----------+     +------------------+
       |    harness     |-->|    run    |<--   |      store       |
       | run_javadoc_*  |   |           |      | StoreRecorder    |
       | run_test_writer|   | run_seat  |----->| ResidentStoreRec.|
       | TestWriter*    |   | RunReport |      +------------------+
       +--------+-------+   | JournalEnt|
                |           | Verdict   |
                |           +-----+-----+
                |                 |  uses (down only)
                |     +-----------+-----------+-----------+
                |     v           v           v           v
                |  +------+   +-------+   +--------+  +------------+
                +->| seat |   |adapter|   |  gate  |  |containment |
                   +------+   +-------+   +---+----+  +-----+------+
                   SeatDef.   CliAdapter  GateExecutor  ShadowWorkdir
                   Ceilings   ClaudeCode  Resident/         ^
                   GateClass  Script      ShadowJava        | shadow-verify
                                             |  depends on --+
                                             v
                                    +-----------------+
                                    |    proposal     |
                                    | Proposal/File   |
                                    | Proposal::diff  |
                                    +-----------------+
```

Arrows = "depends on / calls into". `seat` and `proposal` are leaf modules —
pure data + parser, no outbound dependency. `adapter` is a leaf on the I/O
boundary: it spawns a process and returns text, and knows nothing of
proposals, gates, or journaling. `gate` depends on `containment` for the
Java shadow-verify path; `containment` never depends back on `gate`.

## Module responsibilities

1. **seat** — `SeatDefinition`, `Ceilings`, `GateClass`, `parse_seat_definition`.
   Parses a seat's `.md` prompt (`architect.md`, `debugger.md`, `javadoc-writer.md`,
   `profiler.md`, `spec-auditor.md`, `spec-editor.md`, `test-writer.md`) into a
   structured definition: which `GateClass` applies, which `Ceilings` bound it.
   No side effects, no knowledge of adapters or gates.

2. **proposal** — `Proposal`, `ProposalFile`, `parse_proposal`, diffing. The only
   module that understands the wire format between an agent's propose-mode
   text (the `JAWATA-PROPOSAL-BEGIN/END` block) and a set of file edits.

3. **adapter** — `CliAdapter` trait, `ClaudeCodeAdapter`, `ScriptAdapter`. The
   pluggable boundary to whatever drives the model — headless Claude Code
   today, enterprise models via Sprint 33. Stops at "here is the agent's raw
   text output."

4. **gate** — `GateOutcome`, `purity_check`, `GateExecutor` trait,
   `ResidentGateExecutor`, `ShadowJavaGateExecutor`. Verifies a proposal
   before it lands: compile-verify + purity always; parity + coverage for
   behavior-touching changes; pass + coverage-delta + mutation-bite for test
   proposals; doclint for docs. A `GateExecutor` answers exactly one
   question — does this proposal pass — never partially.

5. **containment** — `shadow_apply`, `shadow_stage_files`, `Ceilings`
   enforcement. Owns the disposable shadow copy a proposal is materialized
   into for gate-checking, and the reaping of a run that exceeds its time
   limit / step limit / cost ceiling — the same reap contract a debug
   session uses for a wedged JVM.

6. **run** — `Verdict`, `RunRequest`, `PreDetected`, `JournalEntry`,
   `RunReport`, `StoreRecorder` trait, `ResidentStoreRecorder`, `run_seat`,
   `record_human_verdict`. The control loop: detect → drive the adapter →
   parse the proposal → run the gates → apply or refuse → record to the
   experience store → journal. **The only module that sequences the
   others** — every other module is a service `run` calls, never the
   reverse.

7. **store** — `StoreRecorder` trait, `ResidentStoreRecorder`. Every run is a
   producer of lessons, not just a consumer of the primer. Kept behind a
   trait so `run_seat` never touches the store's wire format directly.

8. **scheduler** — `Schedule`, `journal_nothing_to_do`. Decides *when* a seat
   runs, and honestly journals the common case where a seat finds nothing to
   do — "ran, found nothing" must stay a distinct, auditable state, never
   collapsed into silence.

9. **harness** — `run_javadoc_writer`, `run_test_writer`, `TestWriterBaseline`,
   `TestWriterGates`, `proposed_test_classes`. Thin wrappers over `run_seat`
   for seats whose gates need pre-run state (test-writer needs a coverage
   baseline before it can compute a delta). A seat with no such need —
   architect, debugger, profiler — gets **no** harness function; `run_seat`
   drives it straight from its `SeatDefinition`.

## Seams for future work

- **New adapter** (Sprint 33 enterprise models) → implement `CliAdapter`.
  Nothing else changes; `RunRequest` carries it as a trait object. Pattern:
  **Strategy**. Prevents: an if/else ladder over "which CLI am I" spreading
  into `run_seat`'s body as adapters accumulate.

- **New gate executor** (e.g. a linter gate) → implement `GateExecutor`,
  register it against a `GateClass` in the seat definition. Pattern:
  **Strategy + Chain of Responsibility** (gates run in sequence, first
  refusal stops the chain). Prevents: `run_seat` accreting gate-specific
  conditionals instead of gates staying uniform, swappable units.

- **New seat** → add a seat `.md`; `parse_seat_definition` reads it. Add a
  harness function only if it needs baseline/pre-detection state the generic
  loop doesn't carry. Pattern: **Template Method** (`run_seat` is the
  invariant skeleton; `SeatDefinition` + the harness supply the variable
  parts). Prevents: a bespoke run loop per seat — the exact failure mode
  this subsystem exists to avoid.

- **Sprint-29 pipeline integration** (spec-editor → spec-auditor → architect
  → … → test-writer) → a **new** module, `pipeline`, built *on top of* `run`:
  it sequences `run_seat` calls, passing each `RunReport`/`JournalEntry`
  forward as the next seat's `PreDetected` input. It must compose only
  through `run_seat`'s existing `RunRequest`/`RunReport` contract — never
  reach into `run`'s or `gate`'s internals. Pattern: **Pipeline over
  Template Method**, not a rewrite. Prevents: `run` growing
  pipeline-specific branches and becoming two subsystems wearing one name.

## Dependency direction

```
seat, proposal  <-  adapter, gate, containment, store  <-  run  <-  harness, scheduler  <-  (Sprint 29) pipeline
```

Nothing below `run` may depend on `run`, `harness`, `scheduler`, or the
future `pipeline` — the loop is assembled from services; services never
reach back up to ask the loop for anything.

## Encapsulation direction (give the object its behaviour)

The current shape leans on free functions taking their subject as the first
parameter. Three moves that put behaviour where its data lives, ranked by
leverage:

1. **`ShadowWorkdir` owns `shadow_apply`/`shadow_stage_files`.** Introduce a
   `ShadowWorkdir` type (workdir path + shadow copy path) in `containment`;
   make `apply`/`stage_files` methods on it instead of free functions that
   re-thread `workdir: &Path` through every call. `ShadowJavaGateExecutor`
   then holds a `ShadowWorkdir`, not a path plus two function calls.

2. **`Proposal::diff(&self, workdir)` replaces `compute_files_diff`.** The
   diff is a property of the proposal against a workdir, not a standalone
   utility — folding it into `Proposal` stops call sites from having to know
   the file list lives on `Proposal.files`.

3. **`SeatDefinition::check_purity(&self, touched_files)` absorbs
   `purity_check`.** `scope_prefixes` is already a property of the seat's
   definition; passing it in separately at every call site is the tell that
   the function belongs on the struct that owns the scope.

## What must not be touched

- **The propose-mode invariant.** Every adapter emits text; every proposal is
  parsed by `proposal`, gated by `gate`, and only then applied to the real
  workdir by `run`. An adapter must never gain direct write access to the
  workdir — that collapses propose/verify/apply into "trust the agent," the
  exact failure mode gates exist to prevent. A new adapter that writes files
  directly is a bug, not a feature.

- **Gate-refusal honesty.** `GateOutcome` must never be synthesized as a pass
  when a check didn't run (unavailable tool, timeout, ambiguous result) — an
  inconclusive gate refuses, it does not default to green. Ceilings-triggered
  reaping is a refusal, not a silent drop. `journal_nothing_to_do` exists so
  "ran, found nothing" and "did not run" are never conflated either — both
  invariants protect the same property: the journal is never more
  optimistic than what actually happened.

## Adoption notes (dispatch)

- The three encapsulation moves above are a parity-gated refactoring plan
  (`refactoring action=plan`, kind=compose_method-style extraction onto the
  named structs) — not a redesign; land them before Sprint 29 so `pipeline`
  is built against the cleaner surface, not the free-function one.
- `ShadowWorkdir` is a prerequisite for any second `Shadow*GateExecutor`
  (e.g. a future non-Java shadow gate) — introduce it once, in `containment`,
  before a second language-specific gate executor is written against the
  current ad hoc path-passing.
