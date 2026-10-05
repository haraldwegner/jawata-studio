`/btw <the thought>`: write it down, put it where it belongs, and get out of the way.

**This is an INTERJECTION, and its whole value is being cheap enough to use while
thinking about something else.** You are mid-task; something occurred to the user that is
not this task. Capture it in one or two tool calls and return to what you were doing. A
capture that costs a session's attention is a capture nobody makes, so there is no
confirmation loop here, no clarifying question unless the triage genuinely cannot be
decided, and no summary of what you just did beyond one line.

## STEP 1 — triage. This is the skill's only real work

Three destinations, and which one it is follows from what the capture IS:

| the capture is | it goes to | why |
|---|---|---|
| a **defect** — something is broken, wrong, missing | an issue in its home sprint | the standing rule: the store is recall, not a backlog; file every defect as an issue in its home sprint, silently |
| durable **knowledge** — true next month, reusable by someone not here | `/memorize`'s own routing | that verb exists and carries the cold-reader gate; do not duplicate it, FORWARD to it |
| an **idea, a preference, a "we should"** | `docs/btw/<project>.md`, appended | the only one with no home today, and the gap this command exists to fill |

**If it is genuinely two things, split it and say so in your one line.** If you cannot
tell which it is, ask ONE question — but read it twice first, because most captures
announce themselves.

## STEP 2 — NEVER write to the experience store directly

Not a style rule. The store is retrieval for knowledge that is true whoever reads it, and
it refuses by gate exactly what a capture usually carries: *project state does not go in
the store at all — a sprint phase, a release announcement, "X is executing" — those are
true on the day and false a month later.* A `/btw` is most often a thought about THIS
project at THIS time. Writing those to the store is how a retrieval corpus becomes a
noticeboard.

Durable knowledge still reaches the store — through `/memorize`, which has the routing
and the gate. The difference is that `/memorize` is a RETROSPECTIVE pass with a high
durability bar, and this is an interjection whose content is frequently what that pass is
built to reject. Same format, different behaviour, and the behaviour is why they are two
verbs.

## STEP 3 — the idea lane's file, and his words survive it

`docs/btw/<project>.md`, appended, never rewritten:

```markdown
## 2026-10-05

- The slot's identity guard should probably be redundant once encapsulation lands.
  <!-- captured: 2026-10-05 -->
```

Three rules about that file:

- **APPEND ONLY.** Never edit, reorder, merge or tidy an existing entry. The file is a
  record of what was said when, and a tidied record is a record somebody has had an
  opinion about.
- **HIS WORDS, not your improvement of them.** Quote the capture as given. Where it needs
  context to be legible later, add the context as a separate sentence rather than rewriting
  the thought. A capture that has been "clarified" cannot be checked against what he meant.
- **The date is the heading, one per day.** A reader scanning for what happened in a week
  reads headings, not timestamps.

When a capture is CONSUMED by a sprint, the entry gains a marker naming it — see below.
That marker is the only edit ever made to an existing entry.

## STEP 4 — who reads it, and when

**Reader: the `/sprint` skill's editor step. Moment: the editor pass, before the raw doc
is cleaned.** That step already reads a raw working document and asks what the sprint must
cover; it reads `docs/btw/<project>.md` in the same breath and must dispose of every
UNCONSUMED entry in one of three states — in scope, deferred with a named destination, or
struck with a reason. That is the same disposition rule the auditor already enforces on
raw items, so it needs no new mechanism.

It is wired rather than optional: the capture file is an INPUT to a step that already
runs, so a sprint cannot be scoped without the captures having been looked at. A consumed
capture is marked with the sprint that took it:

```markdown
- The slot's identity guard should probably be redundant once encapsulation lands.
  <!-- captured: 2026-10-05 · consumed: sprint 8b D4 -->
```

so the next editor pass does not re-offer it. An entry with no `consumed:` marker is
unconsumed by definition, which is what makes the disposition checkable rather than
remembered.

## STEP 5 — report, in ONE line

    btw → docs/btw/jawata-studio.md (idea, appended under 2026-10-05)
    btw → issue: jawata-mcp#NN (defect)
    btw → forwarded to /memorize (durable knowledge)

Then return to whatever you were doing. If the task you interrupted had a next step, take
it in the same turn.

## What you never do

- **Never write to the experience store.** See step 2. Forward to `/memorize` instead.
- **Never rewrite an existing entry**, except to add a `consumed:` marker.
- **Never improve his wording.** Add context beside it, never over it.
- **Never ask for confirmation of a capture you could triage.** The ceremony is the one
  cost this command cannot afford.
- **Never file a capture into a sprint doc directly.** A capture is made before anyone
  knows which sprint it belongs to, and filing it is a scoping decision taken at the worst
  possible moment — the editor step makes it, with the other captures in front of it.
