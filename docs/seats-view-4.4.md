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
> | every deployed seat listed with purpose, firing moment, tools, gates, model | **built** — the seats view (studio `70c8154`); every column is read from the seat's own file, with a fallback to the shipped values for an edited seat that lacks them |
> | the stance-editability answer | **written** — editable, an edit wins over a redeploy, and it costs the seat its updates |
> | the deployment side file by file, and what waits for 28h | **written, and REWRITTEN at the C11 audit** — the first version said *nothing new goes now*, which was false once the settings were built. A deploy now removes command files; the section says so |
> | the run panel's place, present and filled at S16 | **written** |
> | per-seat settings, each surviving a redeploy | **three of four built** (studio `3a7be8f`, `70c8154`, `e0554b6`); **the brain is declared UNMET** — nothing consumes a seat's model, so the control was removed rather than shipped inert. Eight further mutations on the fold, each reddening exactly the test aimed at it |
> | a named test proving one engine per workspace | **met by a test that already existed** — `runtime_manager::tests::workspace_two_distinct_workspaces_have_independent_processes`, and it fails when the join ignores the workspace key (measured) |
>
> **Found while building this, and it belongs to Stage 7, not this one:** the `ux` seat that Stage 7
> reported built did not parse (`kind: doc` — the parser accepts only `text` or `code`) and was
> never listed in the embed registry, so it shipped in no binary and ran nowhere. Nothing caught
> it because nothing loaded it. It is fixed (studio `3a7be8f`) and C7 should be read with that
> in mind: its row said BUILT.
>
> Decisions this document raises and does not take: whether `ux` gets a typed command, and what the
> clients setting should mean for a client that reads another client's directory (see the settings
> section). Both are the architect seat's. A third it used to list — sidecar or shadowing for the
> brain — went away with the brain itself, which was removed because nothing consumes it.

## Seats — what each one is for, when it fires, its tools, gates and model

**Fourteen seat files ship. NINE carry a command a person can type; FIVE are driven and have
none** — read off `conductor.rs`'s `COMMAND_MAP`, which is the only thing that decides it, and
pinned by that file's own test asserting `command_for` answers `None` for the driven ones.

Every column below is read from the seat's own file rather than described, so a seat whose model
or tools change makes this table wrong in a way a reader can catch.

**A correction to this document's own first version, which said that of every column when it was
false of two.** *What it is for* and *when it fires* were prose written here; the seat files
carried neither. They are now `purpose:` and `fires:` in each seat's frontmatter (studio
`3a7be8f`), a test holds every shipped seat to declaring both, and the seats view reads them. The
sentence is true now. It was not true when it was written.

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

## The per-seat settings

**THREE OF THE FOUR ARE BUILT. The brain is not, and this section says why rather than listing it
as if it were.** They live in `<config>/seats/.settings.json`, beside the seat files and **never
written by the materialization pass**. That omission is the whole design: surviving a redeploy is a
property of *where the settings live*, not a rule somebody has to remember.

| Setting | What it decides | How it reaches the agents | Proved by |
|---|---|---|---|
| **on / off** | whether the seat's command is written for any client | the deploy **removes** the command file of an off seat — it used to only write. Offered in the view only for a seat that HAS a command; a driven seat writes none, so "off" would remove nothing | `a_seat_turned_off_loses_its_command_and_gets_it_back_when_turned_on`; `the_deploy_seam_reads_the_saved_settings_from_disk` (the seam production runs, with a redeploy between); mutations **B**, **I** |
| **clients** | which client dialects the seat is written for | the same writer; a client dropped from a seat's list loses that client's command and no other's. **The view shows the control only when two independent clients exist** — today it shows none, for the reason in the bullets below | `a_client_dropped_from_a_seats_list_loses_only_that_clients_command`; `only_independent_clients_are_offered_for_a_seats_client_list`; mutations **C**, **O** |
| **brain** | **NOT BUILT.** Built, reviewed, removed | — | see below |
| **reset to shipped** | discard the user's customisation and take this build's | drops the overrides, backs the seat file up as `*.pre-reset`, **re-seeds it at once**, and tolerates a corrupt settings file (keeping its content). Reachable for a hand-edited stance as well as for an override | `a_change_shows_in_the_view_and_a_reset_takes_it_back`, `a_reset_restores_a_stance_the_user_edited`, `reset_recovers_from_a_corrupt_settings_file_instead_of_failing_on_it`, `a_stance_the_user_edited_counts_as_customised_so_reset_is_reachable_for_it`; mutations **F**, **J**, **K** |

Mutations A–F were run against the first build; **H–O** against the fold that followed the audit.
Every one reddened exactly the test aimed at it, on a verified-clean file, reverted by
`git checkout HEAD --`.

**WHY THE BRAIN IS NOT BUILT, and it is the finding that changed this section.** The first version
of this document left *sidecar or shadowing* to the architect seat, then the build took the sidecar
and it worked — the override was merged into the loaded seat and a test proved it. The audit and
the architect watch then converged on the same fact from two directions: **nothing consumes it.**

- `seat.model` is read in `runner.rs`'s hosted-runner half, which `lib.rs` documents as having no
  production caller.
- The deployed commands render no `model:` key, and a conductor test forbids one.
- The hire path in jawata-mcp builds a hire from the seat FILE's text. It never sees this sidecar.

So a user who set a brain saw their own edit echoed in the same field, and an agent ran on whatever
it ran on before. That is the defect the UX seat exists to refuse, and the project already has a
name for its shape: a field that decides nothing. The mutation and the test that "proved" the merge
proved only that the merge happens. **Making it real means a hire reading a resolved seat — a
contract between studio and jawata-mcp, which is 28h's.** C11's clause says four settings; three
are built, and this is a declared shortfall carried to S18 for his signature, not a closed one.

**WHAT THE VIEW DOES NOT SAY, and should be known:**

- **The clients choice offers only clients that write their own seat directory**, derived from the
  roster and the same predicates the deploy asks. Cursor reads Claude's directory when it is
  present (measured), so it is not an independent choice: dropping a seat from Claude removes it
  for both, and listing Cursor alone would let a seat vanish from both while Cursor read ticked.
  The architect watch found the second case by reading the code. That leaves one independent client
  today, so the view shows no Clients control; the writer's capability is real and tested.
- **A change reaches the agents through the dashboard's own Deploy**, which the view requests after
  every successful write. A refused change (no such seat, an unknown client) writes nothing and says
  why.
- **The view shows the brain as text.** It does not mark it as the user's, because the user cannot
  change it here.
- **Two blocks the UX census refused are not on the page**: a `/command` label per seat and a
  "customised" tag. C11's clause asks for neither. Whether the view should show the typed command
  is a small call that adds an element nobody requested, so it is left to the owner.

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

**Something goes now, and it changes what a deploy does.** The first version of this section said
*nothing new goes now* and *a deploy writes exactly what it wrote before, plus nothing*. That was
written before the settings were built and was false by the time they were; the C11 audit caught
it standing in the one section the clause says must be answered plainly.

**What goes now:**

| Goes now | What it changes in a deploy |
|---|---|
| **removal of a seat's command** | a deploy used to only write. A seat turned OFF, or a client dropped from a seat's list, now has that command file REMOVED from that client, and the removal is reported as a change. This is the one behaviour change in the deployment side |
| **the saved settings are read by the deploy** | `deploy_seat_commands` loads `<config>/seats/.settings.json` from disk on every deploy. An unreadable file stops the seat commands for that deploy and is named in its errors; it is never read as "no settings", which would re-enable a seat the user turned off |
| **the fourteenth seat is embedded** | `ux` now ships in the binary. It writes no client command (it is driven), so no client directory changes; its seat file is seeded into `<config>/seats/` like the other thirteen |
| **`purpose:` and `fires:` in every seat file** | fourteen seat files changed. An unedited copy is `refreshed` on the next deploy; one the user edited is `shadowed` and keeps its file, which the view covers by falling back to the shipped values |

**What waits, and where it goes:**

| Waits | Why it cannot go now | Home |
|---|---|---|
| **the brain setting** | nothing in this app consumes a seat's model: the hosted-runner half has no production caller, deployed commands render no `model:` key (a conductor test forbids it), and the hire path in jawata-mcp reads the seat FILE. A control would echo the edit back and change nothing. It was built, reviewed, and removed in this checkpoint | the stage where a hire reads a resolved seat (28h) |
| **a Clients control in the view** | the writer honours a client list, but the only two clients seat commands are written for are Claude and Cursor, and Cursor reads Claude's directory. Offering both let a seat vanish from both while the view read ticked. The view shows the control when two *independent* clients exist | when a second independent client exists, or per-seat inheritance (finding C11-2) |
| **a seat-only redeploy** | a settings change asks for the dashboard's full Deploy, which rewrites every client's managed MCP configuration | finding C11-4 |
| **the scheduler honouring "off"** | `due_seats` and the hire path ignore `enabled`. Invisible today because the scheduler is unwired; once wired, an off `architect` would still fire at 06:00 | 28b, finding C11-6 |
| **one source for a seat's purpose** | a command's picker description (`COMMAND_MAP`) and the seat's `purpose:` say the same thing for nine seats and have already diverged for one. Deriving one from the other rewrites every deployed command | finding C11-7 |
| showing which seats are currently **shadowed** | now partly covered: a hand-edited stance counts as customised, which is what makes Reset reachable. The data per deploy (`shadowed`) is still not shown as its own fact | the view, unrequested |

**One clause of C11 is not written here, deliberately:** *"a named test proves one engine per
workspace and FAILS if two share one."* It is code, and a paragraph about a test makes a document
look like a gate. The test is `workspace_two_distinct_workspaces_have_independent_processes`; it
existed before this stage, and it goes red when the join ignores the workspace key (measured).


## Every studio view: who reads it, and what they do next (C17, 2026-10-21)

Written after his walkthrough of the installed 4.4.7 / engine 4.4.12. His cuts are applied; nothing is argued.
A view opens on one line saying what is true now; everything else is folded.

| View | Who reads it, in what situation | What they do next | What was cut |
|---|---|---|---|
| Dashboard | the person setting up or restarting the workspaces their agents use | start or stop a workspace or one project; add projects; deploy to or remove from their agents | **Dry run** (its preview was returned by the engine and shown nowhere), **Regenerate** (only skips "file already identical"), the four count pills, the hint sentence, the per-project refresh and Delete buttons, the per-card Delete workspace button (right-click keeps it), the "Selected Project Status" strip (its detail is the status badge's mouseover); the two registration forms became one folded "Add projects" panel; **Delete all** moved under "More" |
| Memory | the person keeping the knowledge store healthy, who has just seen a recall miss or wants a copy before a risky action | Load, Back up now, Clean up; under More: export, import, restore, curate rules, wipe | five explanation paragraphs (they are mouseovers now), the 54 area chips and the per-bundle chips (one line), the Settings fields for store backup copies and the story folder moved here |
| Reporting | the person asking "is anything wrong with jawata on this machine?" | open Details, then type `/report`; or switch off the periodic reminder | the five-figure strip and the shell-usage and canary cards (their figures are the opening sentence, the canary shows when a resident is not answering), failure shapes beyond the top three (folded) |
| Seats | the person deciding which seats their agents may use and which brain a hire runs on | On/Off per typed seat; choose a brain; choose which agents get a command | the repeated "Typed as /x" sentence (a mouseover on Brain), the full purpose and When, Brain, Clients (folded under each row), 18 two-column cards (now three groups) |
| Settings | the person changing how the engine updates, which agents deploy writes to, or where the manager keeps its data | choose the update policy; tick the agents; change a path, memory limit or debugging switch; clean up | the tool list of Test Services (a count and a result), the Antigravity row (unsupported; stated in Help), the two memory-store fields (moved to Memory); merge mode and backup-before-write sit under Advanced; Diagnostics paths and the cleanup buttons are folded |
| Help | a person setting up a machine or finding what a tab is for | read one section per tab | the per-button reference and the tool surface (they go stale; hover text carries them), the stale screenshots; a gate (`build/help-gate.sh`) fails when a tab has no section |

**Decisions this overhaul made without asking him, recorded so he can overrule them.**
- The pattern library (D27, his ruling that it is shown as its own thing) stays a block of its own inside "What the store holds": one line with its share and cures, the namespace split in the mouseover.
- The Antigravity row is gone from Settings; its reason is in Help.
- The overhaul is homed in 28g, as the S17 deliverable, because C17 says his cuts are applied here.

**Not proven yet.** Whether the folded views read well on the installed window is judged by him on the next install. A frontend render harness (`build/ux-render/`) is waiting for his word.

