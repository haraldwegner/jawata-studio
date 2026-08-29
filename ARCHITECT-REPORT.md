# Architect report — WATCH MODE

**Scope:** the change I made on 2026-08-29 to `src/lib/components/MemoryView.svelte`
(retry backoff), and the defect it was made in response to.
**Question:** design fix or bandage?
**Verdict on the reviewed diff: BANDAGE.** Named below, with the smallest
design-level alternative.

---

## Gates

| Gate | Result |
|---|---|
| `compile_workspace` (jawata-mcp) | **0 errors, 0 warnings** |
| `svelte-check` (jawata-studio) | **0 errors**, 114 files (3 pre-existing a11y warnings, in files I did not touch) |
| Purity | The reviewed change touches ONE file. No Java was edited, so the engine tree is byte-unchanged. |

Every finding below is derived from a jawata call or a file read, never from memory.
The reference counts are `find_references` / `get_call_hierarchy` output.

---

## Findings, ranked

### F1 — There is no cheap way to ask whether the resident is alive, so the liveness probe is a full analytical report

`ExperienceTool#stats` has **18 callees** and composes five blocks:
`store.stats()` · `quality.statsBlock()` · `catalogueBlock()` · `substrateBlock()` ·
an embedding-coverage block. One of those five does unbounded filesystem I/O.

`manager_service.rs:923 knowledge_status_for` calls `experience(kind=stats)` with a
**5-second timeout** for one purpose: to set a boolean, `reachable`. The Studio needs
liveness. The only call available returns analytics. There is no seam between them.

**This is why bounding the walk is not the fix.** It makes *today's* expensive block
cheap. The next block added to `stats()` — a bigger catalogue scan, a fuller embedding
count, anything — re-creates the outage, and nothing in the design says it must not.
A probe on a 5 s budget that transitively calls whatever `stats` grows into is an
open-ended cost with a fixed deadline.

**Give the object its behaviour:** the resident owns its own cost. A liveness answer
should be one the resident can always give inside a fixed budget, and the client should
not be metering the resident's expense from outside.

### F2 — `addDrift` ignores the bound its own subsystem already uses, and its javadoc claims the bound it does not have

`ExperienceTool.java:647` — `Files.walk(root)`, no depth limit, no file cap.

Its javadoc four lines above (`:632`) reads: *"Read-only and **bounded**: it lists the
ROOT's own markdown files"*. That sentence describes `Files.list` — depth 1. The code is
`Files.walk` — depth unlimited. A reviewer asking whether the walk is bounded reads the
comment and stops. (Same shape as the miscounted ceiling: the artifact says *bounded*
and the mechanism is not.)

**And the correct pattern already exists, in the loader this check exists to report on:**

    ExperienceMaintenance.java:195
    try (Stream<Path> s = recursive ? Files.walk(root, Math.max(1, maxDepth))
                                    : Files.list(root)) {

`ProjectImporter` caps at `Files.walk(projectPath, 8)` in two places. Across the engine,
`addDrift` is the ONE walk with no cap. Nothing had to be invented; it had to be reused.

### F3 — `substrateBlock` publishes a derived common ancestor as an actionable scan root

`commonPrefix` (`:673`) climbs until one path contains the other. With entries ingested
from `~/.claude/…`, `~/CursorProjects/…` and `~/Projects/…`, it returns **`/home/harald`**.

The block then hands that value out as `root`, with `howToAdd` telling the caller to run
`reseed(path=<root>, recursive=true)`. A derived ancestor is a *location hint*. Published
as a scan root, every consumer inherits a home-directory traversal — `addDrift` on every
`stats`, and any agent that follows `howToAdd` literally.

Not hypothetical: during `/memorize` this session that step sent me to
`reseed(path=substrate.root)`, I read 13,430 unloaded files under it, and refused. The
outage is the same defect arriving from the other side.

---

## Reviewed diff — `MemoryView.svelte` retry backoff: BANDAGE

**What it does:** 5 s → 10 → 20 → 40 → 60 s cap, reset on success. Measured effect: the
accumulation rate of abandoned server-side walks drops from ~6/min to ~1/min.

**Why it is a bandage, by this seat's own rule 6** — *a control must sit on a channel
that can act in time*. The backoff throttles a client that **cannot cancel the work it
is throttling**. Abandoning the HTTP request at 5 s does not stop the resident's walk. So
the control can only reduce how fast new unbounded jobs ARRIVE; it can never subtract
one. That is precisely the refused shape — a limit on a channel whose only power is to
add — and it arrived from my own hand, one message after I wrote the story about it.

**Smallest design-level alternative: F1.** With a liveness call the resident can always
answer inside its budget, the probe succeeds, `autoRetrying` goes false, and there is
nothing left to back off from.

**Disposition — keep the mechanism, cut the justification.** A client backing off an
unresponsive server is defensible hygiene on its own terms, independent of this bug. But
the fifteen-line incident narrative I attached to it is the tell that I wrote it AS this
bug's fix. Trim that comment to the general reason; the incident belongs at F2 and F3,
where the defect is.

---

## Dispatches

| Finding | Actuator |
|---|---|
| F1 | Design work — a liveness answer separate from the analytics report. Needs a ruling before a plan: a new tool kind, or a scope argument on `stats`. |
| F2 | One line, and `refactoring(action=plan)` would be overkill: match `ExperienceMaintenance:195` (depth cap + non-recursive mode), and make the javadoc true. |
| F3 | Design work, coupled to F1: `substrate.root` becomes a hint carrying an explicit scan scope, or the derivation refuses to return a directory above the entries' own tree. |

---

## Below the fold

- **A javadoc asserts wiring that does not exist.** `addDrift`'s comment (`:628-630`) says
  the drift number *"rides every `stats` and every `review_sweep`"*. The reference graph
  says otherwise: `addDrift` ← `substrateBlock` ← `stats` ← `execute` case `"stats"`, one
  caller at each hop. `review_sweep` never reaches it. This is the Sprint-27 shape — a
  claim about wiring that no test can contradict, because the tests do the wiring.
- The `guard-gate-output.sh` hook blocks any command whose text contains `check`,
  including `tail -8 /tmp/svelte-check.txt`. It matches the filename, not the gate. Cost:
  three wasted calls this session. Not worth a fix on its own; worth knowing.

## Skipped by record

None — no previously-declined proposal in this scope.

---

## The pattern, stated once

Three defects this week, one shape: **an unbounded cost driven by a retry loop that
nobody metered.** The stop-gate valve, its miscounted ceiling, and this. In each, the
mitigation that suggested itself was another layer on the outside of the thing that was
too expensive. Bounding cost at its source is the only one of those that does not
accrete.
