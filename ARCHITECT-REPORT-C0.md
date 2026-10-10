# ARCHITECT-REPORT-C0 — watch mode over the S0 edit classifier

Seat: architect, watch mode, read-only. Slice: studio `373f8b5` (the classifier, the one-use exception and
the pipeline wiring). Verdict: **ARCHITECT: MIXED**. The run read the code and the replay reports and ran
nothing.

## Verdict by question

1. **Layer.** A text classifier as the FIRST line is sound and its header admits the limit; as the ONLY
   layer it is a bandage, and the requirement ("every one of the 47 refused by name") is not met: 24 of 47.
   The test asserts `named >= 24`, a regression floor, not the contract. What text cannot see: a declaration
   rename versus a use-site swap; a signature change versus a call that gained an argument. The engine layer
   (asked by the hook through the resident, as `recallgate`/`dupgate` already do) parses before and after and
   compares declaration sets, so it separates those, and sees a call that is already a compile error after an
   earlier signature edit. **Inference, not measured:** the 12 call-site ripple edits exist only because the
   first signature edit was not refused, so per-edit 47-of-47 is unreachable at the text layer and the honest
   ceiling at the engine layer is "roughly 40 or more", and only if "refused" may mean "refused at the
   causing edit". **Recommendation:** keep the structural text rules (imports, visibility, parameter list on a
   declaration, private-member deletion); move rename, extract and move to the engine.
2. **Tightening or relaxation.** A tightening: a refusal is added inside windows ahead of `judge_edit`; the
   `.java` window rule is untouched. Two edges noted: `jawata-fallback:` on the Edit call no longer covers a
   shaped edit; the exception is bound to the tool name, not the file or the edit, so a `MultiEdit`'s first
   pair can spend it and the second then refuses the whole call, and it is burned if the edit then fails to
   apply.
3. **Consumers/producers.** The binary reads the window directory; nothing enumerates `*.once`. The
   script-generation guard in `manager_service.rs` writes the window and has no shape check or `.once` — dormant
   while `BINARY_RETIRED_ROLES` is empty, but a role retirement would silently drop D1/D2. **A bypass:**
   `guard.rs` returns Allow for any Bash command containing `jawata-author:`, so
   `jawata-author: x; sed -i … Foo.java` skips D1 (that path predates this slice). The failure text was
   returned and discarded — no ledger. The generated instructions taught `jawata-author: <narrow reason>`.
   **The question it could not answer:** is D1 scoped to Edit/Write/MultiEdit only?
4. **No production caller:** none. Everything is reached from `pipeline.rs`.
5. **Overfit.** The rules were tuned against the 312-edit census they are measured on, so 24 of 47 is a fit
   figure. The held-out look (2,229 other edits, no labels) refused 223 at the time of the review, and the
   reviewer's listing named plainly-authoring refusals under `rename_symbol` (`Wait.SPIN`→`Wait.BLOCK`,
   `System.err`→`System.out`, `HashMap`→`ConcurrentHashMap`, comment-word edits). A real held-out check is a
   labelled sample of about 100 refusals scored on the false-refusal rate.

## What was folded (one round)

- **Rename narrowed** to a change in the name of a DECLARATION (the old identifier follows a type or a
  declaring keyword and is followed by something a declaration continues with). Held-out refusals fell from
  223 to 160, `rename_symbol` from 65 to 2; the census figures did not move (24 of 47, 0 false refusals).
  Four of the reviewer's named false positives are now unit tests.
- **The declared failure is logged** (`edit-exception-declared` in `outcomes.log`).
- **The generated instructions state the rule** (`manager_service.rs`).

## Carried, not done

- The engine-layer classifier for rename/extract/move and the cross-edit cases — a new MCP tool and a hook
  question to the resident; outside C0's written clauses.
- A labelled held-out sample (≈100) to put a false-refusal RATE on the text rules.
- The Bash `jawata-author:` early-allow (also opens the shell-write route) — pre-existing, unchanged here.
- The script-generation guard has no shape check (dormant).
- The exception's binding to a tool name and not an edit.

## D4 — what was asked of the experience store and the pattern catalogue, and what was selected

**Recorded after the fact (2026-10-11), not before the design.** The consultation was run after the
classifier and this report were committed, so it informed the record and not the build. Stage 6's exit
(`ARCHITECT-REPORT-C<n>.md` names what was asked and what was selected) was not met at the time C0
closed; this section is the repair, and the lateness is the finding.

`experience(kind=nominate)` was asked three questions in the report's own words:

| Asked | Selected | What it changes |
|---|---|---|
| A guard classifies a hand edit from the edit's old and new text alone and must refuse the shapes a refactoring tool already performs without refusing new-code authoring | `6794c8ce` (a fixture written to exercise an operation is written in the shape it handles) | It is the mechanism behind §5: the census the rules were tuned on cannot show their false refusals, so 24 of 47 is a fit figure. It supports the carried action, a labelled held-out sample of about 100 refusals, and nothing else |
| A grant ended by the human's typed message, including a typed slash command, but not by machine events | `8df56474` | Nothing for C0. It belongs to S1/C1 and is the entry recorded from that very fix, so it contributed nothing to the design |
| Classify the tool calls of an unattended run as code-changing or not | **none** | The store had nothing for it. Nothing selected |

No pattern from the catalogue was nominated or selected: the classifier is a rule set over text, and
no catalogue entry names that situation.
