# The seats view — 4.4

> **This document is Stage 11's and its WRITTEN ANSWERS are now here.** It opened with one section
> — the run panel, which C16 required to be filled at Stage 16 — and the rest as named gaps. The
> gaps were closed on 2026-10-08, after Stage 7 shipped the UX seat this view's additions are
> meant to be guarded by; the plan says of that stage *"Blocks S11"*, so writing these sections
> before it would have been building the seats view with its guard unbuilt.
>
> **What is written and what is not, so a reader need not infer it:**
>
> | C11 clause | State |
> |---|---|
> | every deployed seat listed with purpose, firing moment, tools, gates, model | **written** — 14 rows, every column read from the seat's own frontmatter |
> | the stance-editability answer | **written** — editable, an edit wins over a redeploy, and it costs the seat its updates |
> | the deployment side file by file, and what waits for 28h | **written** — and the answer to *what goes now* is **nothing new**, said plainly because the clause asks for it plainly |
> | the run panel's place, present and filled at S16 | **written** |
> | four per-seat settings, each surviving a redeploy | **UNMET, and not papered over** — none of the four exists in code; the section specifies them and names the design question that blocks the brain one |
> | a named test proving one engine per workspace | **open as CODE.** Deliberately not written about here: a paragraph about a test would make a document look like a gate |
>
> Two decisions this document raises and does not take: whether `ux` gets a typed command, and
> whether a per-seat brain lives in the seat file (shadowing it) or in a sidecar. Both are the
> architect seat's.

## Seats — what each one is for, when it fires, its tools, gates and model

**Fourteen seat files ship. NINE carry a command a person can type; FIVE are driven and have
none** — read off `conductor.rs`'s `COMMAND_MAP`, which is the only thing that decides it, and
pinned by that file's own test asserting `command_for` answers `None` for the driven ones.

Every column below is read from the seat's own frontmatter rather than described, so a seat whose
model or tools change makes this table wrong in a way a reader can catch.

| Seat | Typed as | What it is for | When it fires | Model | Gates | Tools |
|---|---|---|---|---|---|---|
| **main** | `/main` | Owns the workflow: declares the steps, drives them one at a time, refuses a step whose gate did not run | whenever the work has more than one step | sonnet-5 | — | compile_workspace, get_diagnostics, run_tests, find_quality_issue, experience |
| **cataloguer** | `/catalogue` | Says what each member and each package is FOR, resumably | on a package nobody has described; resumes where it stopped | sonnet-5 | — | inspect, search_symbols, analyze, experience |
| **javadoc-writer** | `/javadocs` | Documents undocumented public Java API from compiler facts | the DOC gate — public API a change touched, before that change is done | haiku-4.5 | `docs` | inspect, analyze, compile_workspace, get_diagnostics |
| **test-writer** | `/cover` | Writes gate-verified characterization tests for uncovered code | the COVERAGE gate — a change that added behaviour, before it is called done | haiku-4.5 | `tests` | run_tests, find_quality_issue, inspect, compile_workspace |
| **architect** | `/refactor` | Architecture review and parity-gated refactoring proposals | the sprint's DESIGN step, and EVERY checkpoint as a watch-diff. **Also daily at 06:00** | sonnet-5 | — | find_quality_issue, find_references, inspect, analyze, compile_workspace, experience, refactoring |
| **debugger** | `/debug` | Recall, one discriminating probe, verify | the runtime reflex: a bug, a bad value, an NPE — instead of hand-adding log lines | sonnet-5 | — | debug, experience, analyze, find_references |
| **profiler** | `/profile` | Names hotspots as compiler-accurate symbols | the runtime reflex: performance or latency — instead of a hand-rolled stopwatch | haiku-4.5 | — | profile, search_symbols, inspect |
| **report** | `/report` | Turns the local field recording into a bug report posted from the user's own account | when a failure shape has recurred and nobody has filed it | sonnet-5 | — | — (its instrument is the field recording) |
| **review** | `/review` | Shows what the store carries that nobody uses, and deletes only what the user names | **Mondays at 05:00**, and on request | sonnet-5 | — | — |
| **pm** | *driven* | Hires another seat: issues the brief, runs the gates, journals the result | when a seat's work needs a brain that is not this one | sonnet-5 | — | compile_workspace, get_diagnostics, search_symbols, find_references, inspect, analyze, find_quality_issue, … |
| **test-manager** | *driven* | Decides which tests a change actually needs run | at a gate, in place of running everything | haiku-4.5 | — | run_tests, compile_workspace, get_diagnostics, find_references, get_call_hierarchy, inspect, experience |
| **spec-editor** | *driven* | Derives the clean spec from the raw working document | inside `/sprint`, before the audit | sonnet-5 | — | — |
| **spec-auditor** | *driven* | The gate: refuses a spec that narrowed the raw | inside `/sprint`, after the editor, looping until sign-off | sonnet-5 | — | — |
| **ux** | *driven* | Asks the question no gate asks: was the present REQUESTED | on a diff that changes a view | sonnet-5 | — (a census discipline instead; see its file) | experience |

**TWO THINGS THIS TABLE EXPOSES, and neither is settled here.**

The **ux** seat has no command, so a person cannot ask for it. That is defensible — it fires on a
view diff the way the architect fires on a checkpoint — but the architect ALSO has `/refactor`,
and the asymmetry was not decided, it fell out of `COMMAND_MAP` not being touched when the seat
was added. **Open: does `ux` get a command?**

Only **two seats declare a gate** (`docs`, `tests`). Eleven declare `gates: []` and carry their
discipline in their stance instead, and the fourteenth — `ux` — says in writing why a jawata gate
cannot reach its subject. That is a real pattern and not an oversight per seat, but nothing in the
view says it, so a reader comparing two rows would read an empty gate list as a missing one.

## The four per-seat settings

**NONE OF THE FOUR IS BUILT, and this section says so rather than describing them as if they
were.** Searched `conductor.rs` and `runner.rs` for any of `enabled`, `reset_to_shipped`,
`seat_settings`, `SeatSetting`: **zero hits.** There is no per-seat settings store anywhere in
`src-tauri/src`. C11's clause — *"four settings changeable per seat, each surviving a redeploy"* —
is **UNMET**, and a view that listed four toggles would be the clearest possible instance of the
defect the UX seat exists to catch.

What they are, and what each would have to mean, so the work is specified rather than vague:

| Setting | What it decides | Why "surviving a redeploy" is the hard half |
|---|---|---|
| **on / off** | whether this seat deploys to a client at all | a deploy writes every seat it knows; an off seat must be *removed* from clients it was previously written to, or turning it off does nothing visible |
| **clients** | which of the client dialects it is written for | `conductor.rs` already writes per dialect (`.claude/skills/…`, `.cursor/skills/…`); the setting narrows that list, so a removed client has the same removal problem as off |
| **brain** | which CLI answers when this seat is hired | the seat FILE's frontmatter carries `model`, and the hire reads it from there — so this setting is a frontmatter edit, which makes it collide directly with the shadowing rule above: set the brain and the seat becomes shadowed, and stops tracking the product |
| **reset to shipped** | discard the user's edits and take this build's content | the one setting the manifest already supports — it is `refreshed` forced, and the backup path `migrated` uses is the safety it needs |

**The brain row is the one that needs a decision before any of this is built.** A per-seat brain
setting and the shadowing rule are the same mechanism pointed two ways: writing the setting into
the frontmatter marks the seat edited, so changing a brain through the view would silently stop
that seat receiving product updates. Either the setting lives OUTSIDE the seat file — a sidecar the
deploy merges, which keeps the stance unshadowed — or changing a brain is accepted as shadowing and
the view says so at the moment of the change. **Not decided here; it is a design question and the
architect seat's, not this document's.**

C16 is worth naming beside it: *changing a seat's brain is a settings change, not a product edit*
is one of that checkpoint's met clauses, and it is met **by the seat file's frontmatter being the
hire** — `SeatHire.read(markdown)` takes brain, model, tools and ceilings from it. So the hire path
already treats a brain as settings. What is missing is a way to change it that does not cost the
seat its updates.

## Is a stance editable?

**YES, AND AN EDIT WINS OVER A REDEPLOY — which is read off the deploy code, not decided here.**

Every deployed seat opens with *"GENERATED by jawata-studio from `seats/<name>.md` — do not edit;
edit the seat file and redeploy."* Taken alone that reads as a prohibition. It is not one, and the
mechanism underneath it is better than the sentence suggests.

`conductor.rs` keeps `.seeded.json`, a manifest of the content hash **this app wrote** for each
seat file. A deploy compares it and reports one of four outcomes per seat:

| outcome | what happened | what the deploy does |
|---|---|---|
| **seeded** | the file was absent | writes it |
| **refreshed** | the copy still hashes to what we wrote, so nobody edited it | overwrites with this build's content |
| **shadowed** | the copy DIFFERS, so the user edited it | **leaves the user's version in place and says so** |
| **migrated** | a pre-manifest copy that differs, so we cannot prove it was unedited | backs it up beside itself, then refreshes |

So the answer a reader needs is three sentences. **A stance is editable.** **An edit survives a
redeploy** — the user's version wins and the deploy names it as shadowed rather than reverting it.
**And an edit stops the seat tracking the product**: a shadowed seat keeps the user's wording while
this build ships different content, which is the cost of editing and the thing the header's "do
not edit" is actually warning about.

The `migrated` case is the one worth reading twice. It exists because a copy written before the
manifest cannot be proved unedited — so nothing is destroyed, and a backup is left beside the file.
Its own comment gives the reason for choosing refresh over leave: *"a silently stale instrument is
the worse failure."*

**What the view owes a reader here, and does not yet have:** which seats are currently shadowed.
The data exists — a deploy already computes it per seat — and the panel does not show it, so a
user whose edited seat has silently stopped tracking the product has no way to see that from the
view. **Named as the gap it is**, and it is the one addition to this view that would pass the UX
seat's own test: the reader is the person who edited a stance weeks ago, and the next action is
re-reconciling it against the shipped version.

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
| **tools attached** | the attachment report, verbatim | **A brain that cannot be told which tools to attach is REPORTED here, never omitted.** Cursor's row now reads *"CANNOT be told which tools to attach; containment available instead: --mode plan plus --sandbox enabled, both passed by this hire. MEASURED: NEITHER BOUNDS A WRITE … Treat a cursor hire as UNCONTAINED for writes."* **THIS CELL QUOTED THE OLD STRING, and the old string was measured false** — *"read-only, so it analyses and proposes and cannot edit or run shell commands"*, which the dialect stopped emitting at C16 after cursor wrote a file under exactly those flags. The panel renders the report VERBATIM, so it inherits whatever the dialect says: the fix belongs there and did land there, and this cell was the last copy of the retracted wording THAT A PHRASE SWEEP OVER THE THREE TREES COULD FIND — which is what was measured, and not the universal negative an earlier version of this sentence asserted. That sweep had already missed a wrapped occurrence once. |
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

**What a deploy writes, read from `conductor.rs` rather than described.** Per seat that has a
command, and per client dialect:

| File written | Dialect | Content |
|---|---|---|
| `.claude/skills/<command>/SKILL.md` | Claude Code | frontmatter plus the seat's stance verbatim, under the GENERATED header naming `seats/<name>.md` |
| `.cursor/skills/<command>/SKILL.md` | Cursor | the same stance, same header |
| the third dialect's path | per `client_dialect.rs` | same again — the header is written at three sites in that file |
| `.seeded.json`, beside the seats | — | the manifest of content hashes this app wrote. Dot-named and never rendered; it is what makes `shadowed` distinguishable from `refreshed` |

Four utility skills ride the same deploy and are NOT seats — `brainstorm`, `btw`, `setup`,
`sprint`, each `include_str!`'d from `skills/<name>.md`. `sprint.md` is single-sourced for the
reason its own comment gives: the two-seat pipeline lives in one machine's `~/.claude/skills/`, so
the spec-editor and spec-auditor reach a client through that file rather than through
`COMMAND_MAP`.

**The five driven seats write NOTHING to a client** — `pm`, `test-manager`, `spec-editor`,
`spec-auditor`, `ux`. `command_for` answers `None` for each, and the materialization pass skips
them, which `conductor.rs`'s own test asserts. They are read by the runner, not typed by a person.

### What goes now, and what waits for 28h — plainly, because C11 asks for it plainly

**NOTHING NEW GOES NOW. The deployment side needs no change for Stage 11, and saying that is the
answer rather than a deferral.** The reason is that the mechanism is complete for what exists:
`ux` is the fourteenth seat and the fifth driven one, so it needs no command, no skill file and no
dialect entry — it joins a path that already handles seats with no command, and the existing test
covers it. A deploy run today writes exactly what it wrote before this stage, plus nothing.

**What waits for 28h is the part that needs the settings store that does not exist:**

| Waits | Why it cannot go now |
|---|---|
| removing a seat from a client when it is turned OFF | there is no off. The removal path does not exist either — a deploy writes and refreshes, and nothing deletes a previously written skill |
| narrowing which clients a seat reaches | same: the setting has no store, and a narrowed list needs the same removal path |
| the brain setting | blocked on the design question in the settings section above — a sidecar, or shadowing accepted and disclosed |
| showing which seats are currently **shadowed** | the data exists per deploy and the view does not render it. This is the one item that is a VIEW change rather than a store change, and the one the UX seat would pass |

**And one clause of C11 is not in this document at all, deliberately:** *"a named test proves one
engine per workspace and FAILS if two share one."* That is a test, not a written answer, and
writing a paragraph about it here would make a document look like a gate. It stays open as code.
