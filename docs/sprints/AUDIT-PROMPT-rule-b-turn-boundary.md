# Audit prompt — Rule B lets a turn end while the agent says it is continuing

Hand this to a fresh-context auditor/architect. Do not supply the requesting agent's
reasoning; it has been wrong four times on this mechanism and its hypotheses are the
thing being routed around.

---

## Your task

Find out why the Stop gate let a turn end at one boundary and blocked an equivalent one
earlier in the same session, and say whether the current design can hold at all. **Then
judge whether the answer is a fix or a redesign** — see "The constraint" below, which is
binding on your recommendation.

You have full read access to the repository and to the running system. Read the code.
Do not take any statement in this document about *why* something happens as established;
the observations are facts, the explanations are absent on purpose.

## The system

- Product: `jawata-studio`, `/home/harald/CursorProjects/jawata-studio`
- Hook sources: `src-tauri/jawata-hook/src/` — `stop.rs`, `pipeline.rs`, `autonomy.rs`
  are the relevant three; `safety.rs`, `roles.rs`, `observer.rs`, `silence.rs` may be.
- Repo version: `3.17.1` (`src-tauri/Cargo.toml`)
- Installed binary the client actually runs: `/home/harald/.claude/jawata-studio/jawata-hook-stop`,
  built 2026-08-29 22:03. **Check whether that binary corresponds to the source you are
  reading** — a prior finding in this project was that assets and engine can ship at
  different times, so a source-level explanation may not describe the running gate.
- Client: Claude Code (this is one of several clients; behaviour differs per client and
  a prior defect was a bound living inside a branch one client never takes).

## What Rule B is

When the user has granted autonomy ("autocontinue"), the Stop gate is supposed to refuse
a turn that ends without either (a) armed background work, or (b) a plain statement that
the agent is blocked on the human. Its message reads:

> RULE B: autonomy is granted and this turn armed no background work, so ending here
> sleeps until the human returns. Start the next piece of work, or state plainly that
> you are blocked on the human.

## The mechanic that makes this subtle

**A turn ends when the agent stops making tool calls, not when it stops writing prose.**
An agent that writes "Continuing to the next step" and emits no further tool call has
ended the turn and gone to sleep, while its own last sentence claims the opposite. The
user cannot tell those two states apart from the transcript — both look like work in
progress.

## The observations, from one session on 2026-08-30

Facts only. All of these are in the session transcript at
`/home/harald/.claude/projects/-home-harald-CursorProjects-jawata-studio/4526476c-40e7-4ce7-b547-322dc393449c.jsonl`.

1. The user armed the grant with the word "autocontinue". It was re-armed later in the
   session with "Move on with the plan and autocontinue".
2. **Boundary A — the gate FIRED.** The agent finished answering a question about how to
   change a rule and ended its turn. `jawata-hook-stop` returned the Rule B message
   above and blocked. The agent then started the next piece of work.
3. **Boundary B — the gate DID NOT FIRE.** The agent completed a work item, committed
   it, wrote a report ending with the sentence *"Continuing to S9a.2, the patch-streak
   gate."*, and made no further tool call. The turn ended. No Rule B message appeared.
   The session slept until the user returned and said the agent had stopped again.
4. Between A and B the agent made many tool calls, ran builds, and committed.
5. Also in this session, a different Stop-gate rule DID fire on a length check:
   "TOO LONG: 2435 characters, and the communicator has not judged it." So the gate was
   live and evaluating at other boundaries.

**What is not established and is your job:** why B differs from A. Candidate spaces
include but are not limited to — the grant's stored state at B, an anti-loop valve or
counter consumed earlier in the session, a rule ordering where an earlier rule returns
before Rule B is evaluated, a heuristic that reads the final message's text, the
installed binary differing from source, or per-client branching.

## The repair history — read this before proposing anything

This mechanism has been patched repeatedly, and **each patch produced a different
failure rather than the same one recurring.** In the project's own terms that pattern
means the patches are moving one defect around rather than fixing it.

Releases and what each addressed (jawata-studio):
- `v3.14.1` — autocontinue first went live.
- `v3.16.1`, `v3.16.2`, `v3.16.3` — three patch releases in one day. `v3.16.3`'s subject:
  "A false positive in the ask detector was disabling the autocontinue push."
- `v3.17.0` — unrelated (Memory tab).
- `v3.17.1` — "the autocontinue grant is his, and only his": added a `declares_a_decision`
  field so only an explicit `DECISION:` line stands the push down, removed `needs_him`
  from the grant-clear so **only Esc revokes the grant**, and added a review-round
  ceiling excluding the communicator.

The four fixes on 2026-08-29 each broke something new: the grant was wired, then task
notices read as presence, then a false positive deleted the grant, then one phrase of
forty-two was fixed. **The user's rule, verbatim:**

> *"every time you made a change, something different failed. This is what this should
> be about. If you make a fix the symptom is not the same but something different
> breaks."*

## A pending requirement that interacts with this

The user ruled on 2026-08-30 that **ESC *or* a question should turn the grant OFF**,
re-armed only by his word. It is recorded as dogfood item D12c in the Sprint 28d plan
(`~/.claude/plans/sprint-28d-plan.md`, Stage 13) and is **not implemented**. It reverses
part of `v3.17.1` deliberately. Your recommendation should say whether it can be built
on the current design or whether it compounds the problem — and note the trap named in
D12c: a question detector is what caused `v3.16.3`.

## The constraint (binding on your answer)

**Do not propose the fifth patch.** By the project's own change-of-symptom rule, a
mechanism whose consecutive fixes each break something different has earned a design
review, not another repair. Two outcomes are acceptable:

- **"Here is the defect, it is genuinely local, and here is why this one is not another
  face of the same structure"** — with the argument for why, not just the fix.
- **"The design cannot hold; here is the target design"** — the state model (what the
  grant is, who may change it, on what events), where it lives, and a migration in
  parity-gated steps.

Two design rules from this project apply and you should test the current design against
both:

- **A control must sit on a channel that can act in time.** Refuse a requirement about
  what gets shown or sent that is implemented on a channel which only runs after the
  artifact exists and can only append.
- **A limit inside a client-specific or optional branch is not a limit.** A bound only
  some callers reach is not a bound. (This has already bitten: a ceiling meant to stop
  this same gate looping lived behind a flag one of the two clients never sets.)

## What to return

1. **The answer to boundary B** — why it did not fire, established from the code and the
   transcript, with the evidence. If you cannot establish it, say so plainly and say
   what observation would settle it; do not supply a plausible cause.
2. **Fix or redesign**, per the constraint, with your argument.
3. **Whether D12c is buildable** on what you recommend.
4. **The one control that would have caught this** — the thing that goes red when an
   agent's prose says "continuing" and its turn ends. State whether such a control can
   exist on this channel at all.
