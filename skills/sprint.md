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
  audit baseline. **On the user's sign-off of the clean spec, the raw is
  DELETED** — the clean supersedes it.
  **Immutability is ONE-DIRECTIONAL, and a requirement arriving mid-run goes HERE**
  (Harald, 2026-08-11). The editor may never remove, reword or soften a raw item — that
  is the property the whole audit rests on. The USER may always add: a requirement that
  comes out of discussion after `/sprint` has started is appended to the raw as a dated
  entry in their own words, and the next audit round re-baselines against the raw as it
  now stands. **It does not go straight into the clean.** Sprint 28a D12 is the case that
  found this: a requirement from discussion went directly into the clean, so the auditor
  had no row to compare it against and could only check it for internal soundness —
  which is exactly what a well-built wrong thing has. Appending costs nothing (the raw is
  deleted at sign-off anyway) and it keeps every deliverable inside the one check that
  matters.
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

## The auditor's checks

**1 to 12 are BLOCKING — they end in SIGN-OFF or REFUSE and the auditor decides them.
13 is the exception: it ESCALATES.** Whether a simpler solution is good enough is the
user's judgement, so check 13's product is a named decision for them, never a verdict.
(Check 11 applies to the plan only.)

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

   AND THE REASON, because it decides which way to default: the lazy path is
   free FOR THE AGENT and expensive for the human. A full test suite costs the
   agent nothing — it sleeps through the wall-clock — while every minute lands
   on the person waiting. So a diligence mechanism offered as an opt-in is not
   merely un-shipped, it is structurally guaranteed never to fire. Proven: an
   impacted-test selection measured at 26 of 354 classes, 38 s against 307 s,
   shipped as a flag and chosen by nobody. Default it on at the choke, or do
   not build it.
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

12. **IS THERE A REQUIREMENT AT ALL?** (Harald, 2026-08-11: *"You tend to
   overcomplicate. An auditor should check if there is a requirement at all."*)
   Checks 1–11 all assume the requirement exists and ask whether the clean
   preserved it. **This one asks whether it exists.** Every deliverable names the
   requirement it serves and quotes it from the raw. Then the blocking question:
   **is this the smallest thing that satisfies those words?** Machinery the quoted
   requirement does not demand is a REFUSE, and so is a deliverable whose only
   provenance is "it enables" or "it follows from" — an inference is not a
   requirement.

   **This is a text comparison, not a judgement** (Harald, 2026-08-11: *"if the
   raw contains the requirements an auditor can check if you overshoot"*). Because
   the requirement is quoted, the auditor does not have to form an opinion about
   whether the editor over-built — it reads the sentence, reads the deliverable,
   and names what the deliverable does that the sentence never asked for. That
   list IS the finding. It is the same discipline as the traceability matrix,
   pointed the other way: the matrix catches what the clean LOST, this catches
   what it ADDED.

13. **IS THERE AN EASIER WAY?** (Harald, 2026-08-11: *"the auditor should ask
   nasty questions, like are all the tests and the way of testing necessary or do
   you achieve the goals with them. Or is there an easier architecture and the
   current solution is over the top."*) Check 12 asks whether each piece traces to
   a requirement. **This one accepts the requirements and attacks the SOLUTION.**
   Ask, hard, and in the user's terms:

   - Is there a simpler way to reach the same sprint goals altogether?
   - Are all these tests necessary, and is this WAY of testing necessary — or do
     the goals hold with fewer, cheaper, or different ones?
   - Is the architecture over the top for what it buys? Would a smaller structure
     do the same job?
   - Which deliverable costs the most and buys the least?

   **THIS CHECK ESCALATES, IT DOES NOT REFUSE.** Whether a simpler solution is
   good enough is a judgement only the user can make, so the product here is a
   NAMED DECISION for them — the simpler alternative stated concretely, what it
   gives up, what it saves — never a verdict the auditor reaches alone. That is
   the one difference from every other check: 1 through 12 are the auditor's to
   decide; 13 is the user's, and the auditor's job is to ensure it REACHES them
   instead of being settled quietly by whoever wrote the spec.

14. **NO ACTIVITY REWRITTEN AS A PROPERTY.** For every verb in the RAW that names
   WORK — design, investigate, decide, measure, benchmark, prototype — confirm the
   clean version still names work, attached to something schedulable. **The failure
   shape to refuse:** the raw says *a design is needed*; the clean says *the thing
   exists and behaves correctly*. Nothing was deleted, the traceability row is
   present, the measure is correct — and no stage will ever schedule the design,
   because a property is something the delivery HAS, not something anyone does.

   This is not caught by check 3 or check 6: the requirement is present, so it is
   neither absent nor undisposed. It has changed grammatical category, and that is
   invisible to every check that asks whether it is still there.

   Proven live in Sprint 28c: the requirement said a design change was necessary,
   the clean spec described the deliverable as loaded with the behaviour as a
   measure clause, and roughly twenty gates passed with the work absent. Harald
   found it; no gate did.

   Two things must be distinguished or this check invents work. A verb legitimately
   disappears when the question got SETTLED during the discussion — *investigate
   which parser* became *uses this parser*. It illegitimately disappears when it
   dissolved into a property nobody will build. The test is whether an answer
   exists: if you cannot point at one, the verb was lost.

   And the deeper fix this check only approximates: **nothing in the chain compares
   the clean document against the raw directly.** Each audit compares a later
   artifact to an earlier one, and each passes correctly. Where that comparison can
   be added, add it; this check is the cheap proxy for it.


   Two guards. **Do not manufacture an alternative to look diligent** — if the
   solution is already the simple one, say so in a sentence and move on; a
   fabricated option spends the user's attention for nothing. And **cheaper is not
   automatically better**: state what the simpler path gives up, especially where
   the expensive path exists because something already failed.

   **The failure this catches is not scope-dropping, it is scope-inventing**, and
   declaring the invention does not cure it. Sprint 28a D12 is the case: the user
   said *"do it at the beginning, because I do not want to do the test runs
   twice."* I built impact selection over the reference graph, which then required
   a network call before every shell command, which then required a whole risk
   decision (R12) with three options and a recommendation. Two audit rounds
   analysed that risk seriously. **It existed only because of machinery no
   requirement asked for** — his sentence needed a table written before the sweep,
   and R12 vanished with the mechanism that invented it.

   **A deliverable with no raw row is itself the finding.** Do not invent a
   special anchoring rule for it — a requirement from discussion belongs in the
   raw (see the one-directional immutability rule above), so its absence there
   means the editor skipped a step. REFUSE, and the fix is to append the user's
   words to the raw and re-derive, not to argue the deliverable's merits.
15. **A QUANTITY MUST REACH A DELIVERABLE THAT PRODUCES IT.** (Harald,
   2026-09-01, after Sprint 28d.) When the raw states a scope quantity — "build
   the missing 45", "15 → 60", "all six" — extract it VERBATIM and carry the
   NUMBER into its traceability row. Then, for every deliverable, state the SIZE
   OF ITS OUTPUT and compare the two arithmetically.

   **REFUSE THIS SHAPE:** the raw says *build the missing N*; a deliverable says
   *survey / rank / select which ones are needed*, and its measure is about the
   survey being well-evidenced; the measure is then met, honestly and exactly,
   over a population far smaller than N. Nothing is absent, nothing is
   unmeasured, no clause was softened — the requirement was replaced by a
   selection step whose output size nobody stated.

   Three rules make it decidable rather than a judgement:

   - **A selecting deliverable owes a FLOOR.** How many of the candidates ship?
     Unstated is not "to be determined", it is UNBOUNDED, and unbounded is the
     finding. A ranking is a priority order, never a commitment.
   - **A selection step's INPUT sets its output.** If that input is drawn from
     another part of the sprint, say so plainly: that part is now the
     scope-setter for this one, and the quantity has been handed to it silently.
   - **A quantity in the TITLE or the task sentence must be traced to a
     deliverable that produces it**, even where no selection step exists. A
     headline number with no owning deliverable is a REFUSE on its own.

   A measure being correct is not a defence. Ask what POPULATION it is measured
   over, and whether that is the population the quantity named.

   **Live cost, and it is why this check exists.** Sprint 28d was named *Fowler*
   and its spec said, in its own summary, *"jawata performs about 15 of Fowler's
   ~60 refactorings… This sprint builds the missing atomic operations
   ourselves."* Its first deliverable then scoped the survey to *"the reachable
   catalogue entries' 'after' shapes"* — the OTHER half of the sprint. The survey
   returned 8 candidates; 4 shipped; coverage moved from ~15 to ~18 of 60. Every
   checkpoint passed, and the survey's measure was met exactly as written,
   because it was evidenced over a population the requirement never named. The
   number was on the page, three paragraphs above the deliverable that shrank it.


## THE ROUND CAP — three, four at the outside (Harald, 2026-08-11, BINDING)

*"We need to cut this behaviour by the count of audits. We should not have more than 3 or
4 rounds typically. Every small detail now which is a bit wording here and then — and we
have not even started planning nor implementation which might turn out wrong anyhow. We
need a basis for the output we produce to compare, but this is again overkill."*

**The spec is a BASIS FOR COMPARISON, not a finished artifact.** Its job is to be good
enough that the implementation can be judged against it. Polish beyond that is waste, and
it is waste spent before the plan exists — on text the implementation may invalidate.

**HARD CAP: 3 rounds. A 4th only if round 3 found something genuinely blocking.** At the
cap, remaining findings are ACCEPTED AS-IS or written into the spec as named open items.
They are never another round. Sprint 28a ran to TEN and the last three found almost nothing
but text the editor had itself generated.

**Only these are BLOCKING findings** — everything else is a note folded silently or not at
all:
1. A claim about the code or the product that is FALSE.
2. A requirement dropped, narrowed or softened against the raw.
3. A deliverable with no measure, or a measure that cannot fail.
4. A deferral with no named home.

**NOT blocking, ever:** wording · tone · a stale cross-reference · a count restated · trail
phrasing · a heading order · the same fact expressed differently in two places. If an
auditor returns these as blocking, the editor fixes what is cheap and IGNORES the rest.

**TWO EDITOR HABITS GENERATE THE FINDINGS.** Sprint 28a proved both:
- **A long audit trail is auditable text.** Narrating each round's findings in prose adds
  ~30 lines per round, and the next auditor audits THAT. The trail is verdict + what
  changed, a few lines each. It is not a confession log, and it is not written for the
  auditor — the user reads it.
- **Never state the same fact in two places.** Every duplicate is a future finding, because
  a fix lands in one copy. One home per fact; everywhere else points at it.

**The cap applies to the plan's audit too.**

## The traceability matrix — anchored to the RAW

| RAW requirement (verbatim) | **Its quantity, if it states one** | Where the CLEAN satisfies it + its measure | **That deliverable's output size** | kept / deferred-with-home / **DROPPED → refuse** |

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
   **THE ARTIFACT LOOP DOES NOT WAIT FOR THE USER — only GATE 1 does** (Harald,
   2026-08-11: *"And what have you been waiting for? That's your loop."*). This
   is the editor↔auditor loop over a DOCUMENT: it has no checkpoints and no flag,
   it simply runs to sign-off. A revision is not a terminal state — the instant
   the clean changes, including a change the user just dictated, the next round
   launches in the same turn, before any summary is written. Reporting a revision
   and waiting is the turn-boundary failure wearing process clothes: it looks like
   deference and it is the sprint asleep. On 2026-08-11 the editor revised one
   deliverable four times across four turns, summarising after each, and
   re-audited only when the user asked whether it had been audited at all.
   **While a round runs, edits to the audited document STOP** — an auditor reading
   a moving file reports on a version that no longer exists. Everything else
   continues.
   **Do not confuse this with autocontinue**, which belongs to plan EXECUTION and
   is defined at Phase B. The instant the clean changes — including a change the user
   themself just dictated — **the next audit round is launched in the same turn**,
   before any summary is written. Reporting a revision and waiting is the
   turn-boundary failure wearing process clothes: it looks like deference and it
   is the sprint asleep. On 2026-08-11 the editor revised the same deliverable
   four times across four turns, summarising after each, and re-audited only when
   the user asked whether it had been audited at all.
   **While a round runs, work that would edit the audited document STOPS** — an
   auditor reading a moving file reports on a version that no longer exists.
   Everything else continues.
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

**And count RELEASES per defect SHAPE, not per defect.** Eight releases shipped in
one day, each fixing a defect the previous one missed or introduced — a missing file
extension, residue in one lane, config never written for another, an unrunnable
script, a byte-order mark — and all eight were the SAME flaw: platform knowledge
scattered across call sites with no owning boundary. The second release on one shape
is the alarm. There should never be a third, and there certainly should not be an
eighth.

**Why the suite said nothing:** the failing branch was compiled out on the machine
the tests ran on, so it was unrepresentable in the suite and every green count was
about the other platform. A gate that structurally cannot execute the code under
repair is not evidence, and its greenness is the most misleading kind.
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

**Every upward message answers three questions before it is sent.** They are
Harald's, ruled 2026-09-03, and they replaced the communicator agent that used
to read every draft:

1. **Can he OPEN every fact in it?** A repo file, a command he can run, a URL.
   Never a path under `~/.claude`, never a store id, never a count only you can
   see. If a fact fails this, either give him the thing that carries it or cut
   the fact.
2. **Has this conversation DEFINED every term?** Not "is it a common word" —
   has it been introduced here, in words, with what it means. An abbreviation
   is defined at first use or it does not appear.
3. **Is the implementation detail BELOW the point?** The answer, the decision
   or the state comes first. Mechanics follow it, and only what he needs to
   judge the answer.

Plus the content rules that carry the meaning: bottom line first (green/red,
moving/blocked-on-what, in the first two lines); the decision test — what is
broken, or explicitly that NOTHING is; what yes changes; what no costs; one
recommendation; no loop interior (a same-session found-and-fixed is never
reported as an open problem); a gate reported as what it PROVES, never as how
it ran.

## Why this is a ruling and not an agent

The `communicator` subagent read every self-initiated message from 2026-08-07
until v4.0.0. What it actually caught, measured over a full session, was three
things — a reference he could not open, a count with no object, an undefined
term — and every one is a checklist item rather than a judgement. What it cost
was its whole readback rendered to him beside the message it was reviewing.
Harald: *"The communicator is annoying. I see the same output twice. It is not
far away from what is originally said. Can we instead add a ruling."*

**And the reason a ruling is enough HERE, when it was not enough for stopping.**
A rule the agent applies to itself holds only where the agent has no motive to
break it. There is no reward in writing an unclear message — so the three
questions above bind. There IS a reward in ending a turn, which is why the stop
gate does not ask the agent to judge its own stop: it puts that to the
`autocontinue` seat, a fresh context reading the transcript itself. Judge where
there is a motive; rule where there is not.

The mechanical residual stays in the hook, because both halves are decidable
from the text with nothing to consult: a message over the length budget is
blocked with the three questions attached, and an undefined abbreviation is
named back.

**The general form, and the reason it keeps mattering:** a fix is GENERAL only
in a BINDING layer. The experience store carries knowledge and is a recall
NOMINEE. Skill text binds the agent that reads it. Only a hook enforces. Every
rule broken during Sprint 28's night — the turn boundary, the review pass, the
architect at every checkpoint — lived in exactly one of the first two layers.

## Execution discipline (after GATE 2)

Stages sequential; STOP at every checkpoint with the summary format; every number
annotated expected-vs-actual; never advance after a failure and never deviate from the
plan without the user's decision; commits per checkpoint; push/tag/release only on the
user's explicit word; update the plan file when a user-approved change lands.

**AUTOCONTINUE IS A FLAG, AND IT BELONGS TO PLAN EXECUTION ONLY** (Harald,
2026-08-11). Scope and effect, exactly:

- It applies to **executing an approved plan** — nothing else. It is not a mode
  the agent is ever in by default, and it is never inferred.
- The user sets it, at their discretion, and only they can.
- **Default (no flag): the agent posts the checkpoint summary and WAITS for
  `continue`.**
- **With the flag: the checkpoints are UNTOUCHED** — every one still happens,
  still posts its full summary, still runs its gate, still applies its exit
  criteria. The single difference is that the agent **does not wait for the word
  before moving on**.
- **A checkpoint that FAILS its exit criteria stops regardless of the flag** —
  it reports the failure with diagnostics and waits. Autocontinue removes the
  wait, never the gate.

**THE AUDITOR'S SIGN-OFF CONSUMES THE CLOSE DECISION** (Harald, 2026-08-29,
after a "DECISION: close?" ask slept a granted session six hours: *"Don't we
have the auditor who is checking the gate? If he says go then there shouldn't
have been any reason to stop."*). The checkpoint's own fresh-context audit IS
the close authority; asking the user afterwards re-litigates a decided question
in front of a judge with less information than the auditor had. THE WRONG SHAPE,
REFUSE IT IN YOUR OWN DRAFT: a checkpoint summary dressed as a decision ask —
"DECISION: close CN?", "say close and I continue" — especially one whose own
text admits nothing blocks. Under the flag: gates green + audit sign-off →
close, post the summary (abnormalities and the user's accumulated open items as
a LEDGER, never a blocker), and continue. The only checkpoint asks that survive:
a blocking residue the agent cannot repair, and outward-facing acts (release,
push, public filings), which no auditor's sign-off ever covers.

**A REPAIRABLE REFUSE IS REPAIRED, NEVER ASKED ABOUT** (Harald, same day,
verbatim: *"A refuse from auditor which can be repaired by the agent has to be
repaired. I don't want to be asked after each refuse."*). Inside the round cap
(3, a 4th only if round 3 found something genuinely blocking — the same cap as
Phase A, and the AUDIT-FIX LOOP tripwire in the stop gate backs it
mechanically), the refuse→repair→re-audit loop is WORK and contains no user
question. At the cap, two different outcomes and only one of them asks:
non-blocking residue is accepted as-is and recorded as named open items — the
close proceeds; BLOCKING residue still standing is the genuine stop, reported
with diagnostics. An ask per refuse and an unbounded repair loop are both
failures; the cap is what makes "no asks inside the loop" safe.

**AND THAT LOOP CANNOT SEE ITS OWN REPETITION — THE EDITOR MUST HAND IT OVER**
(2026-09-04, Sprint 28d C6). One check was rewritten FOUR times across four
rounds. Every version answered exactly the case the previous round had named,
and every version left the same hole one step to the side. Five rounds ran over
that sequence and not one said *this is the fourth*; the person who named the
defect class did it from a single word in a status report, before seeing any
code. Both instruments were working as designed: an audit inspects the changes
since the last gate, so four versions across four rounds never appear together
in any one of them, and the auditor is started fresh ON PURPOSE, which is the
same property that stops it knowing a previous round already repaired this
place.

So the duty is the EDITOR's, and its trigger is mechanical rather than a
judgement — knowing you are in a recurrence is precisely what nobody in the
loop has. Before handing a repair back for re-audit: **has this place been
repaired before in this effort?** `git log -L <range>:<file>` over the lines
the repair touches, kept to this effort's commits. Two minutes, and it needs no
prior suspicion.

When the answer is yes, TWO things follow. Give the auditor that place's
history alongside the change — each previous version and what each one was
answering, as FACTS and never your reasoning, since the reasoning is what the
freshness exists to exclude. And dispatch the architect over the RUN rather
than writing repair number three: it is the actuator, and its rules 12 and 14
are this same alarm read from the seat's side. A finding whose SHAPE matches
the last round's is the alarm — "every round found something real" is not
evidence the review is working.

**Then mind the turn boundary** (Harald 2026-08-07, Sprint 28: the sprint halted
silently at a checkpoint summary and resumed only because an unrelated command
woke the session). The agent runs only while a turn is active, and a turn starts
only from user input or a background job's completion. So under the flag: a
checkpoint summary is MID-TURN text, never a turn's end; and a turn may end only
(a) genuinely blocked on the user's decision or (b) with a background job RUNNING
whose completion re-invokes the agent. Writing "work continues" while nothing is
armed to wake the session is describing machinery that does not exist.

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
2a. **A CHANGED CONTRACT NEEDS ONE TEST EXERCISING PRODUCER AND CONSUMER TOGETHER,
   wherever they live — and that test must FAIL when either side moves.** (Sprint 28,
   "E2E means E2E"; written here because a rule left as prose in a closed sprint is
   the instrument blindness that sprint existed to end.) Two per-product tests are
   not an end-to-end test: the hook outage lived exactly in the seam both sides'
   suites stopped short of. Falsifiability is the clause that matters — a
   consumer-side test against a committed capture of the producer's answer stays
   green when the producer moves, and a self-check that emits a canned string
   before calling the other side cannot fail at all; neither satisfies this rule.
   For a cross-repo seam that means one gate drives the REAL artifact of each side
   together, however inconvenient the plumbing.
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
