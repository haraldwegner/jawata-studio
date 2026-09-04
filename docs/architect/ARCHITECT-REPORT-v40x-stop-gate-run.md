# Architect report — WATCH MODE — the v4.0.0 → v4.0.2 stop-gate run

Produced 2026-09-04 by a fresh-context run of the `/refactor` seat as v4.1.1
deploys it. The seat was given the three shas and the repository, and was told
nothing about which of its rules were new. Whether it would name the
change-of-symptom pattern unprompted was the release's own stated check.

**Scope:** `7630568` (v4.0.0) → `d797a9a` (v4.0.1) → `f6c93d0` (v4.0.2), all on
the stop gate in `src-tauri/jawata-hook/` plus its deploy side in
`src-tauri/src/manager_service.rs`.

**Baseline:** `ARCHITECTURE-hooks-shim-28.md`, whose watch-mode line is *"Any
checkpoint diff that adds a second place to know either of those things, or that
lets a hook fail invisibly, is moving away from this picture."*

**Question asked of the run, before any single diff:** did each fix break
something **different** from the one before it?

**Answer: yes, twice in a row, and the faces share a structure. Rule 12 fires.
The target of this report is the run, not v4.0.2.**

## Gates — what could and could not be run

| Gate | Result |
|---|---|
| `compile_workspace` | **NOT RUN, therefore NOT passed.** The scope is Rust; jawata's compile gate is JDT-backed and Java-only. |
| The Rust test suite | **NOT RUN, therefore NOT passed.** Shell calls in that window were refused by the deployed `ANSWER FIRST OR WORK FIRST` guard — including read-only `git show` and `grep`. |
| Purity | Held trivially. No files changed; the report is the product. |
| History (rule 14) | Derived: `stop.rs` carries 30 commits, 24 on this rule set since 2026-08-19. No prior architect record for this subsystem exists. |

## The run — rule 12 applied first, because it changes what the findings are about

Three releases in **1 h 58 min** (10:53, 12:04, 12:51 on 2026-09-03), all on the
same rule.

| Release | The defect it fixed | Symptom |
|---|---|---|
| v4.0.0 | The stop exemption was a `DECISION:` line **the agent writes in its own prose**. | Two legal stops overnight, correctly formatted, neither a decision. |
| v4.0.1 | The retired reviewer rule had **one surviving reader** — the anti-loop valve still called `owes_a_review()`. | Spawning a retired subagent flipped a verdict. |
| v4.0.2 | The verdict was read from **any tool result after a judge spawn**, so reading the judge's own stance file (which quotes `VERDICT: RESERVED`) stood a stop down. | A file read answered for the judge. |

Three different symptoms; the alarm was reachable at **fix number two** and the
run went to three. The more useful reading is that these are not three
structures but **two**, each with a further face already visible.

### Structure A — the authority is a substring in a file the agent can write

- **Face 1 (v4.0.0):** the agent *authors* the exemption in assistant text.
- **Face 2 (v4.0.2):** the agent *causes* it to appear by reading a file that
  quotes it.
- **Face 3 — already written down in `stop.rs:991`, and open:** *"the transcript
  is writable by the uid the agent runs as, so a forged result line would pass."*

Each release narrows *which slice* of the transcript is trusted — any assistant
text → any tool result after a spawn → the result bearing the judge's own
`tool_use` id. None changes the fact that the gate's authority is a regex over a
file the judged party can write. That is why "this one is different" was true
all three times: the signature of the rule applying, not a reason it does not.

### Structure B — a rule retired in one implementation, left live in a second

v4.0.1's own note names the shape — *"the rule removed from one implementation
and left in a second reader that no test covered"* — then fixed the small
instance and **deferred the large one** as "cleanup rather than a fix". The
large one is a whole second stop gate, in Python, still generated, still tested,
still asserting the retired rule.

## Findings, ranked

### F1 — INCOMPLETE DELEGATION: the stop gate exists twice, and the retired rule is alive and green in the copy nobody looked at

`src-tauri/src/manager_service.rs` generates a complete second stop gate as an
embedded Python script (`STOP_TEMPLATE`), carrying independently of the Rust
crate:

| What it holds | Where | Status in the Rust gate after this run |
|---|---|---|
| `DECISION:` shape detection | `manager_service.rs:7808` | Retired at v4.0.0 as the untrustworthy exemption |
| A 24-phrase ask list, `'SAY THE WORD'` among them | `:7870–7874` | The list whose false positive v4.0.0 cites as the reason `DECISION:` was introduced |
| `UNJUDGED ASK` — the communicator rule | `:7930` | **Retired at v4.0.0** |
| Length budget `2200` / `3500` | `:7883, 7951, 7956` | Different constants from `stop::LENGTH_BUDGET` |

Three things make this a design finding rather than dead code:

1. **It is pinned green.** `the_stop_gate_blocks_an_ask_the_communicator_never_judged`
   (`:9707`) asserts the block fires with reason `UNJUDGED ASK`;
   `the_stop_gate_allows_an_ask_the_communicator_judged` (`:9721`) asserts
   spawning the retired subagent clears it. A test suite is holding the retired
   rule in place.
2. **The contract lies about it, in the one direction nothing checks.**
   `hook-events.json:128` declares `"bash": "absent"`. The template contains it.
   The parity test `every_stop_rule_declared_present_in_bash_has_its_marker_in_the_template`
   (`:12618`) walks only rules declared **present**; nothing walks rules declared
   **absent** and asserts the marker is gone.
3. **It falsifies the release's own claim.** v4.0.0's notes state *"The
   `communicator` subagent is no longer required or consulted by any gate."*
   True of the deployed gate after v4.0.1; not true of the tree.

**Design fix, not the reflex cure.** A shared rule table both generations consult
is refused: a step a source can decline is a step a source will decline, and the
Python generation would decline it the way it declined the retirement. **The
second format stops existing.** It is already unreachable — `settings.json`
invokes `jawata-hook-stop`, no `stop-gate.sh` is on disk — so deletion has no
runtime consumer to break, only the migration rows and the tests holding it.

### F2 — the exemption's channel was never changed, only narrowed

The verdict is not **obtained** by the gate; it is **recovered** by re-reading a
transcript the judged agent can write. `stop.rs` now spends four functions on
that recovery — `tool_result_text`, `answers_one_of`, `verdict_in`,
`Turn::judge_call_ids` — all deciding which substrings in that file may be
authoritative.

Rule 6's mirror says an observe-only channel is correct *for a record*. The
transcript is exactly that, and a good one. It is being used as the **authority
for a control**, which is the inversion the rule names.

**The smallest design-level alternative:** the Stop hook is a running process at
the moment of decision and already reads a config with an endpoint and a token.
Have it **obtain** the verdict — invoke the judge and read its own child's
stdout, or take a harness-signed result — so the verdict never round-trips
through a file the agent can write. All three faces close, and the four recovery
functions collapse to nothing.

**If that is not available** — the client owns subagent dispatch — the honest
move is to **stop narrowing**: state once, beside the fail-safe boundary's
existing bound, that the transcript is a trusted input and forgery is out of
scope, then refuse the next parse-hole patch on that ground. What is not
admissible is the current position, where the code says forgery is out of scope
while three releases in two hours were spent on progressively harder-to-forge
quotations of the same string.

**Rule 5, on the contract this run changed.** v4.0.2 changed what the hook
requires of the transcript shape (`id` on the `tool_use` block, `tool_use_id` on
the result).

- *Producer:* the client harness, not us. Both keys were verified in a live
  transcript — that check is done, on Claude Code.
- *Consumers in this tree:* **two** — `read_turn` in `stop.rs`, and the Python
  `STOP_TEMPLATE`, which string-matches the same JSONL. That second consumer is
  F1. Searched: readers of the transcript file in this repository.
- *The other side:* the degraded direction is stated correctly — a client
  omitting `id` yields no verdict and the turn is held. Safe direction, said out
  loud.
- *Rule 5(c), OPEN and not answerable by the seat:* **is every consumer of this
  transcript shape inside this workspace?** The enumeration is incomplete by
  construction for anything outside this repo — other clients, other tooling, a
  deployed script generation on a user's disk.

### F3 — the judge's stance and the parser's spellings are one fact, kept in step by an assertion

`agents/autocontinue.md` is prose quoting `VERDICT: RESERVED` and
`VERDICT: RESOLVABLE — <…>`. `verdict_in()` parses exactly those spellings, and
`JUDGE_SEAT = "autocontinue"` must equal the stance's front-matter `name:`.
Keeping them in step is a test —
`the_stop_judge_is_deployed_where_the_gate_will_look_for_it` asserts the file
contains those literals.

Who owns the fact? We do, on both sides. The stance is `include_str!`'d into the
same binary as the parser and rewritten from that binary on every deploy — they
cannot ship independently, so the wire-format exception does not apply. This is
rule 13's commonest instance: hand-maintained documentation reconciled against
code by a test.

And the duplication was not merely a drift risk — **it was the attack surface.**
v4.0.2's defect exists *because* the stance carries the parser's own spellings as
literal indented text, so opening the reference implementation of the verdict
line produced a verdict. The copy the guard exists to police is the thing that
broke the gate.

**The answer is generation, not assertion.** Substitute the verdict lines and the
seat name into the stance from the same constants `verdict_in` matches, at build
or deploy. The assertion is then deleted because it cannot fail — and F2's
stronger form becomes available, since a spelling minted per install cannot be
quoted from a checked-in file at all.

## Dispatches

| Finding | Actuator |
|---|---|
| **F1** | An ordered, independently revertible sequence — *not* `refactoring(action=plan)`, which is Java-only. The gate after each step is the Rust suite, run and read. (1) delete the two tests asserting the retired rule (`:9707`, `:9721`); (2) delete the `UNJUDGED ASK` block from `STOP_TEMPLATE` and its marker row; (3) extend the parity test with the missing direction — a rule declared `"absent"` must have **no** marker; (4) decide the template's fate as one call, since after (1)–(3) it is a legacy artifact carrying three more re-decided rules. |
| **F2** | **A decision for the human, before any code.** No plan until the channel question is settled: can the hook obtain the verdict directly, or is the transcript a trusted input we declare and stop defending? Both answers are acceptable. Continuing to narrow the match window is the one that is not. |
| **F3** | A generation change in `manager_service.rs` plus deletion of the string assertions. The *presence* and *restore-on-edit* assertions stay — those are about deployment, not about content agreeing with itself. |
| Debug-build inertness (recorded in v4.0.2) | Owed investigation, unassigned. `/debug` is JVM-only and does not reach a Rust binary. |

## Trend — against `ARCHITECTURE-hooks-shim-28.md`

The baseline forbids two things: a second place to know a rule, and a hook that
fails invisibly. The run touches both.

- **v4.0.0 — mixed, net away.** *Toward:* `rule_b_engaged()` collapses a
  hand-copied condition into one function with two callers. *Away:* a fourth
  concern in `stop.rs` (verdict recovery) in a crate whose declared cut is
  `cue · query · emit · roles-as-a-table`, and a new deploy artifact class
  (`agents/`) with no row in the role table Module 2 makes the single place to
  know what gets deployed where.
- **v4.0.1 — toward.** A retired predicate deleted rather than left unused; a
  dead counter arm removed; a discriminator holding the general shape. The
  release that behaved best.
- **v4.0.2 — toward on safety, sideways on structure.** The hole is closed and
  the control reproduces the defect on demand; the concern count rises again.
- **Against the second clause:** v4.0.2 records that a **debug build of the hook
  is inert in a scratch harness even under `JAWATA_HOOK_SELFTEST=1`**, cause
  unknown, and that two conclusions during the v4.0.1 dogfood were wrong because
  of it. That is a fail-silent in the exact boundary Module 3 specifies, whose own
  note says *"silence from this gate is indistinguishable from allowed."*
- **Size:** `stop.rs` is ~3 400 lines, `judge()` spanning roughly 440 of them.
  `roles.rs` established the table pattern for this crate; the stop rules remain
  a linear run of guard clauses whose *order* is load-bearing — v4.0.0 had to add
  a `Reserved` short-circuit above the review ceiling to stop the ceiling walling
  off its own exit, and pin the ordering with a test. Order-dependent guard
  clauses in a 440-line function is the structural condition that makes "a fix
  taught to one copy" cheap and finding it expensive.

## Reviewed diffs — design fix or bandage

| Diff | Verdict | Smallest design-level alternative |
|---|---|---|
| **v4.0.0**, exemption moves to a third party's verdict | **Design fix** on the *source of truth*; **bandage** on the *channel* | F2: obtain the verdict rather than recover it |
| **v4.0.0**, `rule_b_engaged()` | **Design fix.** Two hand-copies with a comment asking them to agree become one function | — |
| **v4.0.0**, arming requires the word to end a line | **Design fix.** Syntax over intent, derived from his own corpus, tested both directions | — |
| **v4.0.1**, valve term removed, predicate deleted, dead arm removed | **Design fix, incomplete.** Correct within the crate; stopped at the crate boundary and deferred the larger instance of its own defect shape | F1 |
| **v4.0.2**, verdict bound to the judge's `tool_use` id | **Bandage.** Closes one quotation path by narrowing the trusted window; the channel is unchanged and the next face is documented in the same file | F2 |

## Below the fold

- `MAX_UNJUDGED_BOUNCES` (3) and `StopFacts::bounces` survive after v4.0.1 removed
  the only arm that charged them; `bounce_file` is now only ever deleted — a
  counter that can only read zero, which is the thing v4.0.1 deleted one of.
- `Turn::declares_a_decision` and its parser are still populated though no rule
  reads them for an exemption; only tests do.
- `Turn::communicator_ran()` was deliberately kept "as a probe over the parser" —
  an unread predicate beside a retired rule, which is verbatim what v4.0.1's own
  note says caused v4.0.1.
- **Cursor gets none of this.** No subagent facility, so the judge branch is
  unreachable and a granted stop falls back to the 2-empty-turn ceiling alone.
  Declared honestly in the notes — but a **major** release's headline mechanism
  is single-client.
- The **debug-build inertness** deserves its own investigation, not a probe note.
- **The guard that blocked this review's build gate.** The deployed
  `ANSWER FIRST OR WORK FIRST` refusal fired on read-only `git show`, `git grep`,
  `git diff --stat` and `grep`, and kept firing after announcing *"This refusal
  fires ONCE per window"*. Its own text draws the right line — tool-based reads
  are never refused — and its implementation cannot see it, because it classifies
  by transport (shell) rather than by effect (read). Two observed defects in one
  control.
- `hook-events.json` is a hand-maintained inventory of every rule's presence per
  generation, reconciled with the implementations by tests. Same rule-13 shape as
  F3, one level up.

## Skipped by record

Nothing. The repository's existing root `ARCHITECT-REPORT.md` is a 2026-08-29
review of `MemoryView.svelte`; it carries no proposal about the stop gate,
declined or otherwise. **This subsystem has no prior architect record**, which is
itself worth noting given 24 commits since 2026-08-19.

## Verification of F1 by the relaying agent, after the report

Checked against the tree at `4534f28` rather than taken on trust:

- `manager_service.rs:7930` — `UNJUDGED ASK` present in `STOP_TEMPLATE`. **Confirmed.**
- `:9707` and `:9721` — both tests present under those names. **Confirmed.**
- `hook-events.json:128` — `"bash": "absent"` alongside `"rust": "absent"`, the
  row marked `RETIRED v4.0.0`. **Confirmed.**
- The two `UNJUDGED ASK` occurrences in `stop.rs` (`:398`, `:3299`) are both
  comments, so `"rust": "absent"` is **correct**. The contract is false in
  exactly one direction, which is the direction its parity test cannot look.
