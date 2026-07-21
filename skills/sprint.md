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

## Phase B — the actionable plan

The signed-off spec = the baseline; the plan = the clean, in AGENT language, built from
the spec + the editor's parked mechanics. Structure per the Collaboration-spec template —
with two changes:
- **§1 is a MANAGEMENT SUMMARY the user reads** — because the spec deliberately lacks
  steps and sequence, this is where the user sees them: stages-at-a-glance, the
  **critical path** (what gates what, where the long pole is), the user's own manual
  steps and when they land, expected checkpoints. Plus the traceability matrix anchored
  to the spec's deliverables.
- **NO risk section.** Deviation or failure = STOP + the user's decision.
Auditor refuse-loop as in Phase A (baseline = the signed-off spec). **GATE 2 — plan-mode
approval, after auditor sign-off.** STOP.

## The communication audit (ENFORCED — not the agent's choice)

Every DECISION ASK, CHECKPOINT SUMMARY, and SPRINT RESULT sent to the user
must FIRST pass a fresh-context communication audit applying check 8 (the
decision test) to the message itself; on refuse, rewrite and re-audit before
sending. This is a prescribed step, executed with evidence — never satisfied
by "I kept it in mind" (Harald, 2026-07-18: "This needs to be enforced. I
don't want you to decide if you do or leave"). Mechanized hook-level
enforcement is a Sprint-26 deliverable; until it ships, THIS step is the
enforcement.

## Execution discipline (after GATE 2)

Stages sequential; STOP at every checkpoint with the summary format; every number
annotated expected-vs-actual; never advance after a failure and never deviate from the
plan without the user's decision; commits per checkpoint; push/tag/release only on the
user's explicit word; update the plan file when a user-approved change lands.
**Every release in a plan is followed by a PLANNED dogfood-in-anger + re-release
stage** (Harald 2026-07-13, Sprint 24 GATE 2: "I cannot imagine that we don't have
fixes here" — the record agrees: v2.7.1/v2.8.1/v2.9.1–.2 were all dogfood patches):
work the released features in anger on real targets; findings → fix → vX.Y.1 on the
word; a genuinely clean dogfood is recorded as "clean, no patch" WITH the probes that
prove it — the stage ends shipped either way, and the plan auditor checks such a stage
exists for every release the plan contains.
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
process review. Matrix rules from `strategies_orb/docs/SOLID_Refactoring_Method.md` §4;
memory: `feedback_editor_auditor_protocol` / `feedback_red_team_pass_before_presenting` /
`feedback_spec_loop_closure_audit`.
