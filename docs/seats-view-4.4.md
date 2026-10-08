# The seats view — 4.4

> **This document is Stage 11's and it is NOT written.** Stage 11 owns D19 and D20 — every
> deployed seat listed with what it is for, the four per-seat settings surviving a redeploy, the
> stance-editability answer, and the deployment side listed file by file. None of that is here.
>
> What is here is **one section**, the run panel, which C16 requires to be FILLED at Stage 16 and
> which therefore needed somewhere to be. Writing the rest now would be Stage 11 done quietly by
> the wrong stage and against a checkpoint that has not opened. The headings below it are left as
> named gaps so a reader can tell an unwritten section from a forgotten one.

## Seats — what each one is for, when it fires, its tools, gates and model

NOT WRITTEN — Stage 11, D19.

## The four per-seat settings

NOT WRITTEN — Stage 11, D19. (On/off, clients, brain, reset-to-shipped; each must survive a
redeploy, which is C11's own clause.)

## Is a stance editable?

NOT WRITTEN — Stage 11, D20.

## The run panel — what a reader sees after a seat has been hired, and what they do next

**Who reads this and what do they do next.** A human who hired a seat, or who arrived to find that
a seat fired unattended. They do exactly one of three things: accept the proposal, reject it, or
re-hire with a changed brief. The panel exists to make that choice without opening a terminal, so
every row below is a fact the panel must show rather than a fact it could show.

**Nothing in a hired run is applied.** The record's own first line says so — *"NOTHING HERE WAS
APPLIED. A hired run proposes; a human decides."* The panel is a reading surface, and the only
actions on it are the three above.

### What the panel shows, and why each row is there

| Row | What it says | Why a reader needs it |
|---|---|---|
| **hirer → hired** | `pm → cataloguer` | A run with no attribution cannot be argued with. Both names are on the record; neither is inferred from the other. |
| **brain** | `claude-code`, `cursor`, `copilot`, `codex` | Which vendor answered. It is the dialect's own label, not the executable, so a run pointed at a double is still attributable to the brain whose reader parsed it. |
| **tools attached** | the attachment report, verbatim | **A brain that cannot be told which tools to attach is REPORTED here, never omitted.** Cursor's row now reads *"CANNOT be told which tools to attach; containment available instead: --mode plan plus --sandbox enabled, both passed by this hire. MEASURED: NEITHER BOUNDS A WRITE … Treat a cursor hire as UNCONTAINED for writes."* **THIS CELL QUOTED THE OLD STRING, and the old string was measured false** — *"read-only, so it analyses and proposes and cannot edit or run shell commands"*, which the dialect stopped emitting at C16 after cursor wrote a file under exactly those flags. The panel renders the report VERBATIM, so it inherits whatever the dialect says: the fix belongs there and did land there, and this cell was the last copy of the retracted wording anywhere in the three trees. |
| **is the containment a MECHANISM or an INSTRUCTION** | the word, per brain | Measured at C16 and the distinction a reader cannot do without: **codex** and **copilot** are stopped by something outside the model — a sandbox rejecting an attempted patch, a `permission_denied` naming the denied rule — while **claude** and **cursor** are only ever told. Claude's instruction held for the working directory in 5 of 5 runs and still wrote its own plan file outside it every time; cursor's fails about one run in three. A panel showing only the flag would put all four in one column, which is the error the C16 record spent four audit rounds removing. |
| **did it exit cleanly** | yes, or the reason it did not | Three distinct failures and the panel must not merge them: it timed out, a ceiling stopped it, or it crashed. |
| **tool steps** | a count | How much the brain actually did. Zero against a brief that expected work is a finding; zero against a brief that forbade tools is correct. |
| **cost** | the amount, **or that this brain publishes none** | Measured: one brain of the four publishes a dollar cost. The other three must read as an ABSENCE and never as zero — a brain that reported nothing is not a brain that cost nothing. |
| **ceiling fired** | which one, and at what value | `cost: 2.26402 over the ceiling of 0.25` is a real row from a real run. It is the difference between a run that finished and a run that was stopped. |
| **gates** | one row per declared gate: the tool, whether it **ANSWERED**, and what it counted | **Two fields, never the count alone.** A gate that did not answer reports zero errors exactly as a clean one does, so a panel showing only the number would let four distinct failures read as a pass. |
| **steps past their gates** | `1 of 1` | The ledger's own answer, not a number the panel computes. |
| **the record** | a link to the file | The panel shows a summary; the record is the artifact. They must not drift, so the panel links to it rather than restating it. |
| **what it said** | the brain's answer | The thing a reader is actually judging. |

### What the panel must NOT do

- **It must not say WHY a gate did not answer**, because the product does not know. Four causes
  collapse into one reported state — the tool was gone, it threw, it refused, or it answered in a
  shape nothing reads — and the distinction survives only in the resident's log. Filed as S16-10.
  A panel inventing a reason here would be the most convincing wrong thing on the screen.
- **It must not present an absent cost as zero.** See the cost row.
- **It must not offer "apply".** The three actions are accept, reject, re-hire. Applying is a
  separate decision with its own surface.

### Measured rows, so the panel is designed against real output and not an imagined shape

From `build/m12-four-brains.sh` — one hired run per brain, same brief, same ceilings:

| brain | exit | tool steps | cost | ceiling fired | tools settable |
|---|---|---|---|---|---|
| claude-code | not clean | 0 | `2.26402` | **yes** — over `0.25` | yes |
| cursor | **clean** | 0 | not published | no | **no** |
| copilot | not clean | 0 | not published | no | yes |
| codex | **timed out** | 0 | not published | no | no |

Copilot's answer row is what makes the case for showing the brain's own words verbatim: it is not
a model reply but the CLI's own warning, carried through labelled —
`[copilot warning: compaction_static_context_blocked] Static context is using 125% of available
input tokens…`. A panel that showed only "no answer" would send a reader looking for a prompt
problem. The cause is configuration rather than the brain, and it is filed as S16-17.

### What this section does not answer

Where the panel LIVES in the studio UI, and how a reader reaches it. That is Stage 11's layout
work, against Stage 7's UX guard, and it is deliberately not decided here.

## The deployment side, file by file

NOT WRITTEN — Stage 11, C11's own clause, including the "if nothing goes now, say so plainly" half.
