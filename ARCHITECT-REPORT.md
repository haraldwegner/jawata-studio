# Architect — watch-diff `6004e4a..b07c16a`, jawata-studio

**Mode:** watch. **Trigger:** the design alarm — two consecutive fixing commits
each introduced a fresh defect of its own.

**Gates I could run, and the one I could not.** `compile_workspace`,
`find_quality_issue` and the smell detectors are JDT-backed and see Java only;
this scope is Rust and Svelte, so **the jawata detectors did not run and their
absence is not a clean result**. What did run: `cargo test --no-fail-fast`
(664 passed, 0 failed, 7 ignored) and a direct probe of the defect below, which
I reproduced myself rather than taking on report.

---

## Findings, ranked

There is **no incomplete-delegation finding** in this diff. Rule 1 ranks that
class first when it is present; manufacturing one to fill the slot would be
worse than saying so.

### F1 — One field, two jobs: the ranking key is also the rendered value

`src-tauri/src/field_view.rs::store_health`

`report.slowest_millis` is read on the left of the tie-break **and** written as
the displayed number:

```rust
if state > report.health || (state == report.health && r.recall_millis > report.slowest_millis)
{
    report.worst_workspace = r.workspace.clone();
    report.slowest_millis = if state == StoreHealth::Unavailable { 0 } else { r.recall_millis };
}
```

Forcing `0` for presentation reasons made the comparison read `> 0` forever, so
among several unreadable stores the tie-break degraded from **slowest-wins** to
**last-iterated-wins**. Measured directly (three dead readings at 50 / 3 / 10 ms):
`worst_workspace = mid-dead`. The doc comment two lines above — *"ties go to the
slower reading, so the workspace named is always the one a human should look at
first"* — is false for that case.

The commit's own new test uses a **single** reading: the one shape in which the
changed line cannot touch the tie-break it feeds.

### F2 — The parser answers a rule question

`src-tauri/jawata-hook/src/stop.rs::read_turn` / `Turn::narration`

`narration` now accumulates **only while `degraded_consumed > 0`**. That is the
rule "a mention must come after the stamp" implemented inside the transcript
parser. `Turn` is otherwise a record of what the window contained; one of its
fields is now a partial verdict, and the only place the ordering constraint is
written down is an `if` in a loop that no rule-level reader can see.

Both fixes in this family — round 1 widening `final_text` → all blocks, round 2
narrowing to after-the-stamp — were adjustments to *which derived view the
parser builds*, made from the rule's side. That is why each one moved the
behaviour somewhere new.

### F3 — The same shape produced all three defects

B1, B1-REGRESSION and F1 are one thing: **a fold computes its verdict and its
presentation in a single pass over one mutable accumulator, so a change made for
one reader silently redefines the other.**

- `store_health`: selection (which reading is worst) and rendering (what number
  to show) share `slowest_millis`.
- `read_turn` + `judge`: extraction (what the window contained) and adjudication
  (was it reported in time) share `narration`.

Three rounds of audit found three defects because each fix touched the shared
field, and the shared field is the design.

---

## The target: separate the reduction from the rendering

One coherent shape, applied to both sites. Not two patterns — one rule:
**reduce to a WINNER first, over each item's own facts; derive the report from
the winner afterwards.** A presentation choice then cannot reach the comparison,
because by the time it runs the comparison is over.

```
                       BEFORE (both sites)                 AFTER
                  ┌────────────────────────┐      ┌─────────────────────┐
   readings ─────►│  one loop:             │      │  select_worst()     │  pure, total
   / transcript   │   compare ─┐           │      │   compares readings │  order over
                  │            ├─ SAME     │ ───► │   by their OWN facts│  items
                  │   render  ─┘  FIELD    │      └──────────┬──────────┘
                  └────────────────────────┘                 │ Option<&CanaryResult>
                                                             ▼
                                                  ┌─────────────────────┐
                                                  │  report_for(winner) │  presentation
                                                  │   suppresses, words,│  only — reads
                                                  │   formats           │  nothing back
                                                  └─────────────────────┘

   stop.rs, same rule:
        read_turn ──► Turn { blocks: Vec<Block> }      facts, in order, no verdicts
                              │
                              ▼
        judge ──► surfaced_after_stamp(&turn.blocks)   the rule, where rules live
```

**Pattern per seam.** `select_worst` is a *total order over items* (the
`max_by` idiom) — it prevents the accumulator smell that F1 is an instance of.
`report_for` is a *presenter*: it may drop, round or reword anything, and
nothing downstream of it feeds back. `Turn.blocks` is a *sequence of events*,
and `surfaced_after_stamp` is a predicate over it — which prevents F2 by making
the ordering question askable at the rule layer instead of encoded as a parser
side effect.

**Dependency direction.** `report_for` may read the winner; the winner never
reads the report. `judge` may read `Turn`; `read_turn` never reads a rule.

**What must NOT be touched:** `STORE_SLOW_MILLIS` and its `hook-events.json`
binding (round 2 closed it and both directions bind), the nine stop-rule
markers, `RecallSignals`, and the `DEGRADED:` line-start match. All four are
pinned by mutations that were verified red.

---

## Migration — ordered, parity-gated, each step reversible

The jawata refactoring engine cannot drive these (Rust), so each step is a
manual edit with its own gate. They are still independently verifiable and
independently revertible, which is what the parity contract is for.

| # | Step | Gate |
|---|---|---|
| 1 | Extract `fn worst_reading(&[CanaryResult]) -> Option<&CanaryResult>` — the existing comparison, verbatim, reading `r.recall_millis` only. `store_health` calls it and keeps its current rendering. | `cargo test` green; the three existing store-health tests unchanged and passing |
| 2 | Add the missing case: two unreadable readings, slowest named. RED first — it fails at step 1's code, which still forces `0`. | the new test is red, then green after step 3 |
| 3 | Move the `Unavailable ⇒ 0` suppression out of the loop into the report construction. `worst_reading` no longer knows about presentation. | step 2 green; `an_unreadable_store_reports_no_latency` still green |
| 4 | Delete the Svelte guard `store.health !== "unavailable"` (`FieldView.svelte:157`) — one invariant must not be enforced twice in two languages, and the JS half is unreachable. | `svelte-check` + `vite build` green; the binding test extended to cover `slowestMillis` |
| 5 | `Turn` gains `blocks: Vec<Block>` (ordered text/tool-result events); `narration` and its accumulation guard are removed. | `cargo test` green — the two narration tests must pass unchanged |
| 6 | `surfaced_degradation` becomes a predicate over `blocks` asking the ordering question at the rule layer. | both narration tests green; re-run round 3's 13 transcript probes and require no BLOCK→ALLOW movement |

Steps 1–4 and 5–6 are independent; either half can land alone.

---

## The end-state test surface

- **Environment-independent, run anywhere:** `worst_reading` and
  `report_for` are pure over owned data — the two-dead-store case, the
  Healthy+Slow ordering, and the suppression become plain unit tests with no
  canary, no socket, no clock. Today the ordering case cannot be written without
  also asserting rendering.
- **Owned by the boundary:** `canary_probe` keeps the round-trip measurement and
  is tested against a stub resident (it already is), because what it measures is
  a property of the transport, not of the judge.
- **Only reality can verify:** that a genuinely wedged store crosses the
  threshold on a live fleet. That is a dogfood observation, not a suite — and it
  is what `#37`'s own reproduction was.

---

## Reviewed diffs — design fix or bandage

| Change | Verdict |
|---|---|
| `hook_budget` in `hook-events.json`, asserted from both sides | **Design fix.** One fact, two readers, neither linking the other; it respects the existing forbidden edge rather than relaxing it |
| Rule names moved into the emitted `reason` strings | **Design fix.** The marker now lives in the artefact the rule produces, so the parity scan binds to behaviour rather than to prose |
| Parity scan cut to `judge`'s body | **Design fix.** Nine of nine mutations red, verified |
| `StoreHealth` ordering asserted explicitly | **Design fix**, and it caught F1's sibling |
| `slowest_millis = 0` for `Unavailable` | **BANDAGE** — it fixed a rendering complaint by writing to a comparison input. Smallest design-level alternative: steps 1–3 above |
| `narration` accumulation guarded by `degraded_consumed` | **BANDAGE** — correct behaviour, encoded where no rule reader can find it. Smallest design-level alternative: steps 5–6 above |

## Below the fold

- `hook_budget.store_slow_millis` has exactly one reader (the hook-side test); the studio derives the same number by subtraction, so the published field is asserted only against itself.
- `todo` remains an unguarded opt-out in the parity contract for any rule with `bash: "absent"`.
- `the_block_reason_is_not_full_of_whitespace_runs` covers one of nine reason strings.
- A cold `cargo test` in a fresh worktree failed 8 tests across 3 targets and passed on re-run; unreproduced, and CI builds cold.
- The `DEGRADED:` rule's residual false pass (a mention after the stamp but about something else) is inherent to keyword matching and pre-dates this diff.

## Skipped by record

None — no previously-declined proposal in this scope.

---

## The contract question I cannot answer

`StoreHealthReport` is a **response payload**: its clients are the TypeScript
mirror in `src/lib/api/tauri.ts` and `FieldView.svelte`. I derived those two by
reading the serialization path, and steps 1–4 do not change the payload's shape
— only which workspace `worstWorkspace` names in one case. **I cannot see
consumers outside this workspace.** If anything else reads the studio's
`field_status` output, that set is incomplete by construction.
