You produce a SPRINT SPEC and/or an ACTIONABLE PLAN through two separated roles and a
**gated audit LOOP**. The user signs off **last** — after the auditor. Shaping failures:
the 2026-07-09 goja-rebrand scope-drop (an advisory audit certified a quiet scope
reduction as diligence) and the 2026-07-07 book-flatten (plan validated against its doc
while dropping the doc's goal).

## TWO documents, by reader — never three

- **RAW** (`<name>-raw.md`) — the **organic working document**: written on the fly as the
  project progresses (agile) — feature ideas, discussion outcomes, decisions, spec
  drafts, even implementation details. Nothing is filtered out and nothing is descoped:
  *in the raw we just talk* — difficulty is not allowed to touch it. When `/sprint` runs,
  the existing working doc **is renamed with the `-raw` suffix** and becomes the
  immutable audit baseline. **On the user's sign-off of the clean spec, the raw is
  DELETED** — the clean supersedes it.
- **CLEAN** (the canonical `<name>.md`) — the sprint spec the EDITOR derives from the
  raw. **Reader: the USER.** Plain language throughout. MAY contain high-level design:
  ASCII architecture, design decisions, short snippets for new tech — **high level
  only**. A hard requirement becomes a **risk for the user to decide + an honest plain
  measure**, never a dropped or narrowed goal.
- **Detailed mechanics extracted while cleaning** (recipes, token lists, exact gates,
  command lines) go to the **editor's own notes** — memory, scratch, wherever
  retrievable at plan-creation time — **NOT a third document in the sprint tree.** They
  resurface as the plan's stage contracts.

## The reader split (binding on content and language)

| | **SPRINT SPEC** | **PLAN** |
|---|---|---|
| Reader | The USER | The AGENT (implementer) |
| Answers | WHAT · WHICH approach · WHICH risks we accept | HOW exactly · in WHAT order · PROVEN how |
| Contains | Deliverables with one plain sentence of measure each · approach/architecture decisions (design level) · **risks as decisions** · deferrals each with its home · the user's recorded decisions · audit trail | The mechanical contract in sequenced stages · exact recipes/gates/tests/numbers · **a management summary up front** (see Phase B) |
| Language | Plain. "goja is completely replaced by jawata." No unexplained tech-speak (no bare grep/curl/HTTP codes) | Technical, exhaustive, measurable |

**Binding rules:**
1. **RISKS LIVE IN THE SPEC, NEVER IN THE PLAN.** A risk is a decision the user makes
   BEFORE planning (accept / mitigate / change approach). The plan has NO risk section:
   the agent does it or fails; failure or needed deviation = **STOP + report + the user's
   new decision** — never a pre-written mitigation to hide behind. Implementation
   creativity is the agent's; risk decisions are the user's, early.
2. Solution/architecture at decision level → spec. Detail → plan.
3. A spec measure is one plain sentence the user can verify by reading/looking; the
   mechanical form (recipes, counts, exit codes) is the plan's contract.
4. **NOT A NOVEL** (Harald, 2026-07-18): deliverables are terse statements
   of what ships and how it's verified — no narrative connective prose, no
   flowery language ("it is what X has been waiting for" is noise). If a
   sentence neither states a commitment nor defines a term, cut it.
5. **Write for a skilled reader — do not condescend.** Section names are bare
   ("Executive Summary", "Deliverables", "Approach", "Risks", "Deferred", "Audit
   trail") — no explanatory parentheticals ("what you get and how we know it's done"),
   no reading instructions ("READ THIS IN FULL"), no defining terms the user obviously
   knows. The WHOLE clean doc is the spec; the Executive Summary is merely its first
   section (task sentence + context), not a wrapper that demotes the rest to reference.
6. Context/history that conditions nothing in the sprint is cut (one line max; the rest
   goes to memory).

## The two roles + the gated loop

- **EDITOR** (main context): maintains the raw faithfully; derives the clean per the
  split; on refusal revises the CLEAN (never the raw); parks mechanics in own notes.
- **AUDITOR** (a FRESH-CONTEXT subagent — author-blindness is a context property; a
  same-context re-read is NOT an audit): the **GATE**. Receives the **RAW + the CLEAN +
  the relevant chat** — never the editor's reasoning. Adversarial: assume the clean has
  shrunk the raw until proven otherwise. **Verdict = SIGN-OFF or REFUSE (blocking).**
- **The loop:** REFUSE → editor revises the clean → re-audit → repeat until SIGN-OFF.
  Editor-auditor disagreement that cannot converge is **ESCALATED TO THE USER as a named
  decision** — never resolved silently between the seats.
- **The user signs off LAST (GATE 1)**, ratifying a gate that already did the forensics
  against their own words. On sign-off: the raw is deleted.
- **The audit trail records verdicts AFTER they are given — never pre-written** (writing
  "signed off" before the verdict exists is pre-claiming; caught live 2026-07-10).

## The auditor's checks (all blocking)

1. **Measurable** — every deliverable has a measure (plain sentence in the spec;
   mechanical in the plan).
2. **Consistent** — internally coherent. Necessary, NOT sufficient.
3. **No scope change vs the RAW** — per-requirement traceability (matrix below); silent
   absence/narrowing/softening = REFUSE.
4. **No deferral without an agreed home** — named destination AND the user's recorded
   decision; otherwise REFUSE ("defer as hygiene / later" with no home = a DROP in a
   deferral's clothes).
5. **Achievable WITHOUT narrowing** — not "is the measure hittable?" but "**was it made
   hittable by shrinking the requirement?**" Hard requirement → honest ugly measure + a
   risk decision, never a redefined goal. The auditor polices the goal-translation
   force, never serves it.
6. **Every RAW item DISPOSED** — in-scope / deferred-with-home / skipped-by-user-decision.
   Surfaced-but-undecided findings = REFUSE (they resurface later as negative surprises).
7. **Reader-fit** — spec in user language, no implementation contracts, no condescension
   (rule 4); plan mechanical with NO risk section. Wrong-level or wrong-tone material =
   finding.
8. **THE DECISION TEST (reader-meaning, blocking)** — for the artifact AND for
   the sign-off ask that presents it: *can the reader make the decision from
   this text alone — no interpretation, no guessing, every term defined,
   meaning preserved rather than merely shortened?* A summary that condenses
   tech/process detail but loses the meaning fails; a gate result reported as
   how it ran instead of what it proves fails. (Harald, 2026-07-18: "I cannot
   make decisions if I have to interpret and guess.")
9. **WIRED, NOT JUST BUILT** (Harald, 2026-07-18, third occurrence of the
   pattern: debug/profile built in 24 but employed only by 25's seats; seats
   built in 25 but front-door-wired only in 25a; an "intelligent injector"
   specced with no automatic event supply) — every capability deliverable
   names WHAT EMPLOYS IT in the live process and carries a measure proving
   it operates there UNPROMPTED, as a side effect of normal use. A
   capability whose activation depends on someone remembering to run it is
   not shipped.
10. **ENDS SHIPPED, NOT RECOMMENDED** — every terminal path of the sprint (including a
   spike's success path) must end in a SHIPPED STATE: "we switched" or "we stayed" —
   never "adopt, and a follow-up sprint does the switching". A success verdict whose
   completion is bounded work belongs IN the sprint; "X works (proven partially) →
   migrate later" is a deferral wearing a verdict's clothes. THIS CHECK APPLIES TO THE
   RAW'S OWN FRAMING TOO — a deferral the raw contains is still a finding (escalate to
   the user as a named decision, don't inherit it). Caught live 2026-07-12 (22d: three
   audit rounds faithfully verified a spec whose success path ended in "a migration
   sprint follows"; Harald: "Every sprint should have an end result — either we stick
   with the old or we implement this immediately").

11. **CRITICAL PATH DERIVED, NOT ASSUMED** (plan only) — every stage declares
   `DEPENDS ON` and `RESOURCE`; the plan states the critical path and names the
   long pole. **REFUSE a stage sequenced without a real input dependency** — the
   auditor's question is "what does stage N consume that stage M produces?", and
   "it reads more tidily in this order" is a REFUSE. Also REFUSE a plan that
   schedules `token` work (audits, seat runs) strictly after the work it reviews
   when it could start on commit, or that leaves the local machine idle across a
   `remote` wait. Conversely REFUSE unsafe concurrency: two stages marked parallel
   that share a `cpu` saturation point or an `artifact` path, without the plan
   naming why they do not collide.

## The traceability matrix — anchored to the RAW

| RAW requirement (verbatim) | Where the CLEAN satisfies it + its measure | kept / deferred-with-home / **DROPPED → refuse** |

Left column = the raw items verbatim, one row each — never the clean's self-declared
goals. Principle-shaped requirements get a mechanical form (in the plan), never demotion
to narrative. Clean elements mapping to no raw item = scope creep, flag.

## Phase A — the sprint spec

0. The organic working doc gets the `-raw` suffix (or, if none exists, capture the
   requirements + discussion into one now). Immutable from here.
1. **EDITOR → CLEAN**: Executive Summary (task sentence + context) · Deliverables (one
   plain measure each) · Approach (decisions, high-level design/ASCII where useful) ·
   Risks (as user decisions) · Deferred (each with home) · Recorded decisions · Audit
   trail. Mechanics → editor's notes for the plan.
2. **AUDITOR** (raw + clean + chat): checks + matrix → SIGN-OFF / REFUSE + findings in
   plain language (each names the raw item, how the clean failed it, minimal fix).
3. **LOOP** until auditor sign-off; unresolved disagreements → the user.
4. **GATE 1 — user sign-off.** STOP and wait.
5. On sign-off: **delete the raw**; the clean is canonical.

## The design-mode step (mandatory, between GATE 1 and Phase B)

After spec sign-off and BEFORE the plan, run the architect's DESIGN MODE on
the sprint's scope: produce the target architecture — modules, seams,
dependency direction, where new code lands, what must not be touched — as the
versioned artifact `ARCHITECTURE-<scope>.md` WITH a picture (ASCII/mermaid),
committed to the affected repo. The plan is then WRITTEN AGAINST this
artifact, and **plan promotion requires it: the Phase-B auditor checks the
artifact exists and the plan's design section matches it** — a plan without
its design-mode artifact is refused. (Shipped Sprint 25a D3; the architect's
watch mode diffs checkpoint changes against the same picture. The mechanized
end-to-end pipeline remains Sprint 29.)

**And AGAIN at the END of the sprint, over the COMPLETE deliverable set —
before the release ask** (Harald 2026-07-22, Sprint 27). The checkpoint watch
mode diffs the checkpoint's CHANGES against the design, and that cannot see a
connection that was never made: **an unwired capability is a NON-EDIT.** It
lives in a line no commit in the sprint touched, so it appears in no diff,
breaks no assertion, and is invisible to every diff-scoped review. Sprint 27's
own missing connection sat in a line last modified twelve days before the
sprint began.

So the architect runs once more, AS BUILT, over all deliverables at once — not
per stage, not per diff. The check is mechanical and uses tools that exist:

> **For each capability the sprint delivers, name the code that CALLS it —
> file and line, in a main source root.** Resolve the capability to a symbol,
> run the incoming call hierarchy, and classify every caller. A capability
> whose callers are all test code is NOT DELIVERED, however green its tests.

Two limits, stated so the run is not mistaken for proof it cannot give. A
caller can exist and the capability still be dead at runtime — a dependency
that every production site passes as empty. And the main-vs-test classification
must come from the project's source roots, not a path convention, because
jawata's own `compile_workspace(scope=)` gets this wrong on jawata's own
repository. Those residuals are what the end-to-end gate below is for.

## The architect runs at EVERY checkpoint, not only at design time

The design-mode step above produces the artifact. **Every checkpoint then diffs
its changes against that artifact** — the architect's watch mode, asking one
question the code auditors never ask: **design fix or bandage?**

**Every stage's checkpoint definition carries this line**, in the plan, not only
here. Sprint 28 C8 is the evidence: the plan mentioned the architect at the
release ask and at one stage, and nothing at execution time required it. Six
adversarial code audits ran, all six REFUSED, and every one correctly answered
*"does this meet the gate?"* Not one asked *"is this the right shape?"* — that
is not their brief and they will not volunteer it. The architect is the only
seat whose whole mandate is that question, and it was run zero times.

### The escalation trigger (binding)

**A defect introduced by the fixing commit, TWICE IN A ROW, is a DESIGN ALARM.**
Stop fixing findings. Run the architect against the artifact before writing the
next fix.

C8 hit that condition at round two and ran to round six — five more fixing
commits, each generating the next round's defect, inside a design with no fixed
point. The cure, when it finally came, DELETED 186 lines and moved the work to a
process that may block. It was available at round two for the cost of one
question.

### Two cheap reads that beat six audits

Both are available at INTRODUCTION, before any defect exists:

1. **When a module's own doc states two properties, ask whether both can hold at
   once.** C8's `silence.rs` header said, in adjacent bullets, *"no lock file,
   because a lock introduces a way to block"* and *"capped and truncated from
   the front"* — while its append path opened with a trim. Unsynchronised
   read-modify-write over a concurrently-appended file cannot preserve records.
   That is arithmetic, not an emergent bug, and it was written on day one.
2. **Grep the architecture artifact for the file the module writes.** The same
   artifact already said: *"any file a hook writes (decision log, counters) is
   append-under-lock or per-process — never read-modify-write."* The prohibition
   was in writing before the stage began. Six rounds of code audits read the
   code and never read the artifact against it.

## Phase B — the actionable plan

The signed-off spec = the baseline; the plan = the clean, in AGENT language, built from
the spec + the editor's parked mechanics. Structure per the Collaboration-spec template —
with two changes:
- **§1 is a MANAGEMENT SUMMARY the user reads** — because the spec deliberately lacks
  steps and sequence, this is where the user sees them: stages-at-a-glance, the
  **critical path**, the user's own manual steps and when they land, expected
  checkpoints. Plus the traceability matrix anchored to the spec's deliverables.
- **NO risk section.** Deviation or failure = STOP + the user's decision.
- **EVERY STAGE DECLARES ITS DEPENDENCY AND ITS RESOURCE CLASS** — see below.

### The critical path is derived, not assumed (binding)

A plan that sequences stages by habit wastes calendar time the user is paying for
in wall-clock, not tokens. **Sequential is a claim that must be justified per
stage**; parallel is the default. Every stage carries two declarations:

**DEPENDS ON: <stage ids, or "nothing">** — a real input dependency: stage N
consumes an artifact, a measurement, or a decision that stage M produces. "It
feels tidier to finish M first" is NOT a dependency. The **critical path** is the
longest chain of these; the plan states it explicitly and names the long pole.
Everything not on it is marked **PARALLEL WITH: <stage ids>**.

**RESOURCE: <class>** — because independence is necessary but not sufficient; two
independent stages still collide if they want the same scarce thing. The classes:

| Class | Scarce thing | Concurrency rule |
|---|---|---|
| `token` | model tokens; subagents, fresh-context audits, seat runs | **Fan out freely.** No local CPU. Split one audit into concurrent per-dimension audits rather than one generalist doing them serially. |
| `remote` | a CI runner, a hosted job | **Free locally — always fill the wait.** Never idle while a remote job runs. |
| `cpu` | the local machine's cores and RAM | **One at a time against each other.** A full test suite or release build saturates the box. |
| `artifact` | a shared file a build writes and a test reads (a dist jar, a shard dir, an install tree) | **Exclusive.** Two stages touching the same path serialize even when both are `cpu`-cheap. |
| `human` | the user's attention and decisions | **Blocks by definition.** Everything schedulable BEFORE the ask is done before the ask. |

**Oversubscription is not free, and the caution is earned, not theoretical.**
Running `cpu` work concurrently has repeatedly manufactured FALSE REDS — a
timing-sensitive race that appears only under contention, a shared build artifact
rewritten mid-read, two runs corrupting each other's output directory. A false red
costs a full debugging cycle plus trust in the gate, which is strictly worse than
a serial wait. So `token` and `remote` parallelism is taken greedily; `cpu` and
`artifact` parallelism is taken only where the plan can name why they do not
collide.

**The shape the plan must make possible:** an audit of stage N running while
stage N+1 is being built, and a remote job in flight while local work continues.
If the plan's stage order forbids that, the order is wrong.
Auditor refuse-loop as in Phase A (baseline = the signed-off spec). **GATE 2 — plan-mode
approval, after auditor sign-off.** STOP.

## The communication audit (ENFORCED — not the agent's choice)

**Every SELF-INITIATED upward message — decision ask, checkpoint summary,
sprint result, unprompted status/alarm — passes through the COMMUNICATOR AGENT
before it is sent.** Replies to the user's OWN questions are DIRECT, fast,
bottom-line-first — never routed through the agent (ruled 2026-08-07: gating
conversation triples his waiting time for a failure mode conversation barely
has). Invoke the agent named `communicator` (defined at
`~/.claude/agents/communicator.md`; a missing definition is re-created from
this section's rules, never skipped) with the draft, a one-paragraph
true-state statement, and — for any ask — the one-sentence answer to "what
does this achieve?". Send its PASS or its REWRITE — never the refused draft,
never an ask it returns as DROP-THE-ASK. This is an executed step with the
invocation visible in the transcript; "I kept it in mind" is the recorded
failure it exists to end.

The agent judges by THREE TESTS (Harald's criteria, ruled 2026-08-07): the
IMPRESSION TEST — what does a zero-context reader conclude (broken? blocked?
am I needed?), refused when impression ≠ reality in EITHER direction, so false
alarms are refused AND muffled real showstoppers are escalated; the NECESSITY
TEST — an ask lives only if its purpose names a real consequence, and dies if
it lies inside granted authority (autocontinue, tightenings), if every answer
leads to the same action, or if the sender could answer it itself; the
SEVERITY MATCH — the agent assigns showstopper / needs-a-ruling /
progress-note / not-worth-sending from the reader's side, and the draft's
urgency must match. Fresh context is the instrument: the agent knows only what
the reader knows, plus the true-state facts it verifies against.

Two rulings bind it (both Harald's, verbatim anchors in the store):
2026-07-18 "This needs to be enforced. I don't want you to decide if you do or
leave" — and 2026-08-07, after an unintelligible ask stalled an authorized
full-night sprint while every fix was already green: "I wanted a communication
agent." The Sprint-26 stop-gate hook is SHAPE enforcement only (trigger words,
length, abbreviations) and is never an argument against the communicator pass;
the mechanical block-until-communicator-pass lands in the hook binary (Sprint
28 D-SHIM), and the product-side communicator seat for all clients is Sprint
29.

Core content rules the communicator applies: bottom line first (green/red,
moving/blocked-on-what, in the first two lines); the decision test — what is
broken (or that NOTHING is, explicitly), what yes changes, what no costs, one
recommendation; no loop interior (same-session found-and-fixed is never
reported as an open problem); every abbreviation defined; a gate reported as
what it proves, never as how it ran.

## The communicator is ENFORCED, not remembered

Every self-initiated upward message — decision ask, checkpoint summary,
unprompted status — passes the fresh-context communicator agent BEFORE sending.
His own questions are answered directly and fast; the gate is for what the agent
initiates.

**This is not a rule the agent keeps by reading it.** It lived as prose for
months and was skipped three times in one session, the third an hour after being
recorded as a lesson. Harald: *"A rule in claude.md is optional and will not be
applied anywhere else. Have I told you to leave it optional?"*

It is now enforced by the deployed Stop hook: a message asking for a word, a
ruling, a decision or a sign-off BLOCKS unless a communicator subagent ran since
the human's last turn. The transcript is written by the HARNESS, so that is a
fact the agent cannot fake by writing a marker — which is the whole reason the
check can exist at all.

**The general form, and the reason it keeps mattering:** a fix is GENERAL only
in a BINDING layer. The experience store carries knowledge and is a recall
NOMINEE. Skill text binds the agent that reads it. Only a hook enforces. Every
rule broken during Sprint 28's night — the turn boundary, the communicator, the
architect at every checkpoint — lived in exactly one of the first two layers.

## Execution discipline (after GATE 2)

Stages sequential; STOP at every checkpoint with the summary format; every number
annotated expected-vs-actual; never advance after a failure and never deviate from the
plan without the user's decision; commits per checkpoint; push/tag/release only on the
user's explicit word; update the plan file when a user-approved change lands.

**AUTOCONTINUE is an instruction, not a scheduler — mind the turn boundary**
(Harald 2026-08-07, Sprint 28: the sprint halted silently at a checkpoint
summary and resumed only because an unrelated command woke the session). The
agent runs only while a turn is active; a turn is started only by user input or
by a background job's completion. Therefore, when the user has granted
autocontinue: a checkpoint summary is MID-TURN text, never a turn's end; the
STOP-at-checkpoint rule above is REPLACED by summarize-and-continue; and a turn
may end only (a) genuinely blocked on the user's decision or (b) with a
background job RUNNING whose completion re-invokes the agent. Writing "work
continues" while nothing is armed to wake the session is describing machinery
that does not exist.

**The mechanic, because "verify before ending" failed twice in the session that
wrote this rule:** ARM FIRST, THEN SPEAK. Under granted autocontinue, launch the
next unit of work as a background job — the next stage's suite run, the
checkpoint's fresh-context audit, the next long build — *before* composing any
message, and let its completion carry the session forward. A turn that ends with
no job armed is a stopped sprint whatever the message says. "Check which of the
two holds" asks the agent to remember at the exact moment it is least likely to;
"never speak without a job running" is one glance.

**And a skill edit does NOT reach the session that is already running.** The
rule above was committed and deployed at 09:56 on 2026-08-07 and the same
session halted twice more afterwards, because its copy of this skill was loaded
hours earlier. A fix landed here binds every FUTURE session; for the current one
the only layers that reach are the agent's own attention and a hook. So when a
process fix is made mid-sprint, say plainly which sessions it binds — claiming
it is fixed "in general" while the session that needed it carries the old copy
is the same over-claim in a new place.
**Every release in a plan is followed by a PLANNED dogfood-in-anger + re-release
stage** (Harald 2026-07-13, Sprint 24 GATE 2: "I cannot imagine that we don't have
fixes here" — the record agrees: v2.7.1/v2.8.1/v2.9.1–.2 were all dogfood patches):
work the released features in anger on real targets; findings → fix → vX.Y.1 on the
word; a genuinely clean dogfood is recorded as "clean, no patch" WITH the probes that
prove it — the stage ends shipped either way, and the plan auditor checks such a stage
exists for every release the plan contains.
**Sixth rule (Harald 2026-08-07, Sprint 28 C8; evidence: six audit rounds, every
one refusing a defect introduced by the previous round's fix, inside a design
the baseline artifact had already prohibited):** at EVERY checkpoint, run the
ARCHITECT's watch mode against the design artifact — *design fix or bandage?* —
in addition to the fresh-context code audit. And **two consecutive
defect-in-the-fixing-commit rounds STOP the fixing** and force that pass. A code
audit answers whether the change meets the gate; only the architect asks whether
the shape is right, and a loop of correct answers to the first question can run
indefinitely inside a wrong answer to the second.

**Three rules from the Sprint-24 post-close audit (Harald 2026-07-15, Sprint 25 C0;
evidence: 4×REFUSE on a sprint whose every checkpoint was green and whose close-out
claimed "no narrowing"):**
1. **Every release is followed by a fresh-context IMPLEMENTATION AUDIT** — the released
   code against the spec AND the plan, checking deliverable BODIES, never their
   one-line "Measure:" summaries. The plan auditor checks such a stage exists for
   every release, alongside the dogfood stage.
2. **Checkpoint gates enumerate deliverable BODY clauses** — a checkpoint that gates on
   the Measure line alone is how three capabilities were silently lost in Sprint 24.
3. **The close-out's "no narrowing" verdict is produced by a context that did not write
   the stages** — a same-author close-out cannot certify itself; author-blindness is a
   context property at close-out exactly as it is at the spec and plan gates.

**Fourth rule (Harald 2026-07-19, Sprint 26; evidence: 5 of 7 learners shipped HOLLOW —
constructed, listed, persisted, machinery unit-tested, but never wired to observe/serve —
through the spec audit, the plan audit, AND stepwise checkpoints):** a **FRESH-CONTEXT
ADVERSARIAL AUDITOR runs at EVERY checkpoint (Cn)**, performing a **code + tests review
against the plan's gate clauses** for that stage — not only at spec-time, plan-time, and
close-out. **Green is never the checkpoint.** Two failure surfaces that neither "compiles"
nor "tests green" exposes: (a) functionality present-but-HOLLOW (every green signal
present except the wire from a real event to `observe()`), and (b) TESTS SCOPED NARROWER
THAN THE GATE (Sprint-26's zero-manual-step test claimed "every learner advances" in its
Javadoc while asserting 2 of 7; the plan's D7 gate literally said "every learner's count
advanced, asserted numerically" — the gate existed, was correct, and was replaced by
"the test is green"). The checkpoint is "the plan's gate, verified against the code AND
tests by a context that did not write them," ASSUMING the functionality and the tests
each UNDER-cover the plan until the code proves otherwise. Corollary for dogfood: **work
the deliverable's FULL claim, enumerated** — "N learners" means read all N liveness rows,
every dogfood, not the one component you touched.
**Fifth rule (Harald 2026-07-22, Sprint 27; evidence: v3.4.0 was RELEASED with its
central deliverable completely INERT — every one of the three production sites built
the recall engine through the constructor that takes no embedding index, the enabling
overload had 15 callers and all 15 were tests, and the self-heal that gives an existing
store its vectors had 4 callers, all tests. The suite was 1591/1591 four ways. Coverage
ROSE, all three ratchets. The user's first dogfood probe found it in three calls):**

**Nothing that only tests exercise counts as shipped, and the proof runs BEFORE the
sign-off ask — never after it, and never in CI.**

1. **END-TO-END BEFORE THE WORD.** Before any release sign-off ask, every deliverable is
   exercised **through the product's own front door, against the BUILT ARTIFACT** — one
   live assertion each — and **the results go IN the ask**. A deliverable with no
   front-door assertion is reported as **unproven**, never as done. Two properties do the
   work, and both are why unit tests cannot: the harness **has only the front door**, so
   it cannot construct an object and hand it its own wiring; and it runs against **the
   thing being published**, not a build tree. Placing this gate in the release CI is NOT
   compliance — CI runs after the user has already said yes, so it protects the artifact
   while leaving the user to underwrite a claim nobody verified.
2. **"CALLED ONLY BY TESTS" IS A SMELL, and it is checked.** For every public member the
   sprint adds or is meant to employ: incoming call hierarchy; **callers > 0 and every
   caller in a test source root → finding.** (Zero callers is the ordinary unused check;
   "called, but only by tests" is the one that catches hollowness.) Run it manually via
   the call-hierarchy tools until the detector ships; the finding blocks the release ask
   either way. It is necessary, not sufficient — a production site passing an explicit
   `null` keeps a caller and still leaves the wire dead — which is why rule 1 stands
   above it.
3. **A CAPABILITY DECLARES ITS OWN REACH.** Where the product has a health/status
   surface, wiring reports which surfaces a capability actually reaches, not merely that
   the component loaded. "Component available: true" was true and misleading for the
   whole of v3.4.0.

**Why the existing rules did not catch it, so none of them is mistaken for cover:** the
per-checkpoint audit (rule 4) reviews a stage's CHANGES, and this defect is a non-edit;
coverage says a line executed, never who executed it — `backfill` was covered by the
four tests that were its only callers; and the implementation audit (rule 1 above) is
the one instrument shaped to find it, but the plan schedules it AFTER the release.
**A sprint must not schedule its only claim-first check after shipping.**

**Sixth rule (Harald 2026-08-07, Sprint 28 C1; evidence: THREE consecutive checkpoint
audits were briefed against a ONE-LINE condensation of the exit criterion, written by
the implementer whose work was being audited. It dropped four of the criterion's
clauses. All four turned out to be satisfied — but they were satisfied WITHOUT BEING
EXAMINED, which is not the same thing, and nobody could have known which it was):**

**THE AUDITOR'S BRIEF IS ITSELF AN AUDITED ARTIFACT.**

1. **QUOTE, NEVER SUMMARISE.** The brief carries the governing text VERBATIM — the
   deliverable body from the spec and the stage's full exit criterion from the plan.
   Not a restatement, not a distillation, however faithful it feels while writing it.
   Condensation is lossy by construction, and the condenser has a structural conflict
   of interest: authoring the acceptance criterion for the audit of their own work.
   Where the plan lives outside the repository the reader can open, it is REPRODUCED
   in the brief, not linked.
2. **THE VERDICT MAPS EVERY CLAUSE TO ITS EVIDENCE.** Not a narrative and not a list of
   findings: one row per clause of the exit criterion, each naming the test, commit or
   retained output that satisfies it. A clause with no named evidence is NOT DONE,
   however green everything else is. "No auditor objected" is not evidence about a
   clause nobody read.
3. **CLOSE A DEFECT CLASS, NOT ITS INSTANCES.** When a checkpoint finds a defect that
   is a repeated CODE PATTERN, record the query that enumerates the population
   (`find_references`, a structural search) and show `fixed == population`. Fixing the
   instances an auditor happened to open is how the same class was found in round 1,
   round 2 and round 3 of the same checkpoint, each time in a file the previous round
   had not read.
4. **EVERY FIX CARRIES A CONTROL.** Revert the fix; a test must go red. A fix whose
   removal changes no test result has not been shown to do anything — this is the
   discriminators-not-green-counts rule applied per fix rather than per suite, and it
   is how a checkpoint's widest-blast-radius fix reached its third audit round with no
   test that could detect its removal.
5. **STOPPING RULE.** A checkpoint closes on: every clause mapped to evidence · every
   found defect class enumerated to zero · every fix controlled · the whole thing
   reproduced from a FRESH CLONE of the commit, not the working tree · and ONE
   full-criterion audit briefed per rule 1. Repeat rounds of whole-stage audit are for
   NEW CLASSES of defect only. The loop is a detector, not a converger; more rounds of
   it do not converge it.

Close-out
ticks the SPEC's deliverables against tool evidence, flips the spec to ✅ with as-built
actuals, updates the cascade row, memorizes + syncs the experience store.

## The honest limit

The auditor is the same kind of model with the same softening drive: the raw baseline +
concrete criteria + refuse-loop make a drop harder and louder, not impossible. Keep
criteria mechanical where possible; "achievable" is judgment — the weakest link. The
user stays final sign-off, now cheap. Any narrowing actually made appears as **one blunt
user-facing sentence** ("NOT doing X you asked, because Y") in the Executive Summary —
surfaced, never buried (vagueness is the hiding mechanism).

## Provenance

Interim, user-level harness of Sprint 29 G5 — GOJA + ORB shared; productized with jawata
in Sprints 25 + 29. Redesigns: 2026-07-09 (raw≠clean, auditor gates against raw,
refuse-loop, raw-anchored matrix, user-last) after the rebrand scope-drop; 2026-07-10
(reader split · risks-in-spec-only · disposition check · two-docs-only with -raw suffix
and delete-on-sign-off · mechanics to editor notes · bare section names, no condescension
· plan management summary with critical path · no pre-written verdicts) after Harald's
process review; 2026-07-22 (end-of-sprint architect run over the COMPLETE deliverable
set naming the production code that CALLS each capability · end-to-end through the
product's own front door BEFORE the
sign-off ask · "called only by tests" as a checked smell · capabilities declare their
own reach) after Sprint 27 released its central deliverable inert past 1591 green tests
and a RISING coverage ratchet. Matrix rules from `strategies_orb/docs/SOLID_Refactoring_Method.md` §4;
memory: `feedback_editor_auditor_protocol` / `feedback_red_team_pass_before_presenting` /
`feedback_spec_loop_closure_audit`.
