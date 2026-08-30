# PROPOSAL — add rule 12 to the architect seat: the second different breakage

**Sprint 28d, S9a.3b. NOT APPLIED.** `seats/architect.md` is deployed product —
it regenerates every seat command in every client — so this is a diff awaiting
Harald's word, not an edit.

## Why the seat and not just the plan

S9a.3 put this rule in the 28d plan's §3b. That binds one sprint and dies at its
boundary. **Both historical failures were outside any plan**: the 2026-08-13
Windows run (nine releases) and the 2026-08-29 autocontinue run (four) were
ordinary fixing, not sprint stages. A rule that only exists inside a plan file
would have been absent for both.

The architect seat is where it belongs because the rule's *action* is a seat run.
A rule whose remedy is "run the architect" should be readable by the architect.

## The proposed diff

Insert as **rule 12**, after rule 11 (the report structure), before the current
rule 12 ("You do not use any tools"), renumbering that to 13.

```diff
+12. THE SECOND DIFFERENT BREAKAGE IS A DESIGN ALARM, AND IT IS THE CHEAPEST ONE
+   YOU HAVE. When you are handed a run of fixes, ask one question of it that
+   needs no classification: did each fix break something DIFFERENT from the one
+   before it?
+
+   A RECURRING symptom means a fix missed. That is ordinary; it is not your
+   business. A DIFFERENT symptom after each fix means the patches are moving
+   one defect around, and each one is exposing the next face of one structure.
+   Two in a row is the alarm.
+
+   REFUSE THIS SHAPE: a third fix justified by "this one is different". That
+   sentence is true every single time — it is the symptom of the rule applying,
+   not a reason it does not. When you meet it, say so in the report and treat
+   the run, not the last patch, as your target.
+
+   Harald, 2026-08-30, correcting a note that had been read the wrong way for
+   weeks: "every time you made a change, something different failed. This is
+   what this should be about. If you make a fix the symptom is not the same but
+   something different breaks."
+
+   WHY THIS IS A SEAT RULE RATHER THAN A COUNTER SOMEWHERE. The count is
+   mechanical and is already gated at release time (a patch release with any
+   other release inside seven days is refused until he clears it). What no
+   mechanism can do is the part you are for: look at the run and say whether
+   the faces share a structure. Live cost of not doing it: jawata-studio
+   v3.7.8 through v3.7.16 in one day, then v3.16.1 through v3.17.2 across two —
+   thirteen releases, two design flaws, and in both cases the flaw was
+   reachable at fix number two.
```

## What it changes for a seat run

Nothing about the report format, the noise budget or the gates. It adds one
question the seat asks of a run it is shown, and one shape it refuses. It
composes with rule 3 (design fix or bandage) — the answer to this question is
usually what decides that verdict.

## What it does NOT do, stated so it is not read as more than it is

- It does not fire on its own. A seat runs when invoked; this rule is read once
  the architect is already looking. **The rule cannot summon the architect.**
- The mechanical half is the release check, and that half only sees releases. A
  run of four fixes inside one session with no release between them is caught by
  neither.

## Verification, if applied

- `seats/architect.md` regenerates `/refactor` in every client; a redeploy is
  required and is Harald's call.
- The rule is prose in a stance, so there is no test to go green. The honest
  check is the next run of fixes: does the architect, shown a run, name the
  change-of-symptom pattern without being told to look for it?
