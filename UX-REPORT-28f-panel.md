# UX seat — re-run on the 28f panel (second version)

**Seat:** `seats/ux.md`. **Subject:** `src/lib/components/MemoryView.svelte`, the Store & Maintenance
section, **lines 1098–1593 at HEAD**, against the spec that asked for it:
`jawata-enterprise/docs/sprints/jawata-mcp/sprint-28f-database.md` **:262–306**.

**This seat PROPOSES. Nothing here is built, and no component is edited.**

> **This is the second version, and it replaces a first that a fresh-context audit REFUSED (C7).**
> The first claimed to have judged "the Store & Maintenance section" while giving rows for three
> blocks of roughly fifty; quoted an implementer's *commit message* as the sentence that asked for
> something; named a reader as "someone who is stuck", which U-TWO forbids by name; and called its
> census "51" by a tally that was never stated and counted the whole file rather than the section.
> Each is corrected below, and the corrections are listed rather than made silently because the
> seat's own rule is that a mismatch is a failure, not a caveat.

---

## The census — U-ONE

**The method, stated so it can be repeated.** Inside lines 1098–1593, count one block per opening
marker of: `h1`–`h6`, `p`, `button`, `label`, `select`, `input`, `textarea`, `th`, `table`,
`details`, `summary`, `li` (a loop's template once), `pre`. A one-line script reproduces it.

| | |
|---|---|
| container markers in the section, by that method | **54** |
| span-level strips the method does not count, each named in the group that holds it | the lane strip, the namespace strip, the cures strip, the described-units strip, the area wall |
| hint paragraphs (`class="hint"`) in the section | **18** (20 in the whole file) |
| `title=` tooltips in the section — user-visible prose the method does NOT count | **18**; the longest is 326 characters and five more exceed 160 |
| blocks judged below | **all 54, in 14 groups**; the tooltips and the span strips are judged with their group |

**The first version's "51" is retracted.** It was a differently-defined whole-file tally and is not
comparable to the 54 here. A census figure that cannot be reproduced is not a census.

---

## What asked for what — and the limit of this column

**Located and quoted (the 28f database spec, `sprint-28f-database.md`):**

- **:267–268** — *"An area nobody has described answers 'not described yet' — never silence — and
  coverage is reported per bundle."*
- **:274–275** — *"You see a sample per bundle; the review sweep later prunes jobs nobody ever chooses."*
- **:302–303** — *"…a sample per bundle is shown to you."*

**NOT located:** the Stage 5 lane split (G4), the D27 catalogue (G5), the versions list (G10), the
curation blocks (G11–G13) and the result panel (G14) enter through other sprints' specs, which this
run did not read. Their only provenance here is **an in-code comment citing a ruling** (for example
`:1173`, *"His ruling: 'They are the pattern templates for the architect…'"*). That is a claim
about a source, not the source, and the column below says **"in-code citation only"** rather than
"asked for by nothing" wherever that is all there is. **"Asked for by nothing" is therefore a
finding about the 28f Stage 7 blocks only; for the rest it is *not found in the spec searched*.**

---

## The 54 blocks, in 14 groups

| Group | Lines | Markers | Reader, in a situation | Next action | Asked for by |
|---|---|---|---|---|---|
| **G1** store selector sentence | 1098–1106 | 2 | someone with more than one store deciding which to act on | choose one | in-code citation only |
| **G2** empty state | 1107–1110 | 1 hint | someone whose workspace is not running, looking at an empty table | start the workspace | in-code citation only — **keep: a state** |
| **G3** store table, radio | 1111–1150 | 7 | the same person picking the store the buttons below act on | select a row | in-code citation only |
| **G4** lane strip + hint | 1151–1168 | 1 hint | someone judging what kinds of knowledge the store holds | **none** — it informs | in-code citation only; the **hint at :1163 is explanatory** |
| **G5** catalogue strip + hint | 1169–1225 | 1 hint | the architect seat's owner, checking the pattern library's share of the store | **none** | **his ruling at :1173 asks for the BLOCK**; the explanatory hint at :1213 is not that ruling |
| **G6** described-units strip + lost-writes notice + hint | 1226–1253 | 1 hint | someone deciding whether the cataloguer has more to do | run `describe next` (the hint names it) | the **strip: :268 "coverage is reported per bundle"**. The **hint at :1239 — asked for by nothing**. The lost-writes notice stays: it corrects a number the reader would otherwise trust |
| **G7** areas: the wall, the count, the truncation notice, the complement, the empty state | 1254–1301 | 2 hints | see Finding 1 | see Finding 1 | see Finding 1 |
| **G8** the sample + its caption | 1302–1325 | 2 | see Finding 3 | see Finding 3 | **:274–275 and :302–303 ask for the sample**; the caption's second half (*"at most two rows share a class…"*) explains the implementation and was **asked for by nothing** |
| **G9** nine actions + two state hints | 1326–1407 | 9 buttons, 2 hints | a store owner who has decided what to do to the store | each button is its own action; **Wipe** confirms first | in-code citation only. The **tooltips are paragraph-sized prose** (Load's is 326 characters) and are hints by another route |
| **G10** versions | 1408–1446 | 5 | someone recovering after a destructive action | Restore a version | in-code citation only (*"Sprint 28f D2"*). Hint :1419 is a state, keep; **:1440 explains a naming scheme** |
| **G11** rules | 1447–1495 | 5 | a curator retiring a standing rule | Retire | in-code citation only. :1456 is a state with an instruction, keep; **:1491 is explanatory** |
| **G12** entries | 1496–1535 | 7 | a curator deciding which unvouched entries to accept | Promote one; tick sources | in-code citation only. :1500 is a state, keep; **:1531 states a deliberate product rule (no accept-all) and is explanatory** |
| **G13** promote-to-rule | 1536–1568 | 4 | a curator who has ticked entries and is writing the rule | Promote to rule | in-code citation only (a quoted `promote_rule(ids…)`). **:1562 is explanatory** |
| **G14** result panel | 1569–1593 | 7 | whoever just ran an action | read the outcome; expand the raw response | in-code citation only (*"Harald, 2026-07-06"*). :1582 and :1590 are states, keep |

**The 18 hints, sorted by what they are** (the first version said "six" and listed seven — wrong):
**8 states, keep** (1108, 1336, 1405, 1419, 1456, 1500, 1582, 1590) · **2 ask for the complement in
words, requested by spec :267–268, keep** (1277, 1293) · **8 explanatory** (1163, 1213, 1239, 1318,
1440, 1491, 1531, 1562). Of the eight, **two are 28f Stage 7's and asked for by nothing** (1239,
1318); **six enter through other sprints** and are *not found in the spec searched*.

---

## FINDING 1 — the package-name wall is STILL RENDERED (G7)

`{#each selectedRow.describedAreas as area (area)}` — **HEAD:1265**. An unbounded loop over every
described package name. *(I first concluded it had been removed: HEAD's surrounding sentences are
about the complement — which packages are NOT described — and I read them as being about the
list. Reading the loop settled it. Recorded because it is the near-miss this sprint has filed five
times: a property asserted about a block rather than read from its source.)*

**Reader and next action: none, and that is the finding.** The names are not clickable, not
filterable, and the input to nothing. The **count** (`{n} described`, :1279) and the **truncation
notice** (:1281) do have a reader and an action — the person deciding whether the cataloguer has
more to do, who runs `describe next`.

**Asked for by nothing — now sourced properly.** Spec **:267–268** asks that an undescribed area
*answer* "not described yet", and that *coverage be reported per bundle*. The first version quoted
the implementer's commit message (*"WHICH AREAS ARE DESCRIBED, with the complement said in WORDS"*)
as the ask; that is the implementer's reading, and it asks for an answer plus words, **not for the
enumeration.** No sentence of the spec asks for the list of names. **That finding stands, on the
spec's words and not the commit's.**

**What the audit was right about, and it narrows my earlier claim.** The commit message asks for the
complement in words, and the hint at :1285 *is* that — and spec :267–268 asks for it ("never
silence"). So **the complement hints are requested and are not subtracted**. The first version
lumped them in with the explanatory ones.

**Proposed: subtract the wall; keep the count, the truncation notice and the complement.**

---

## FINDING 2 — the hints: eight explanatory, two of them 28f Stage 7's (G4–G13)

See the sort above. **Proposed: subtract the eight, starting with the two that are 28f Stage 7's
(:1239, :1318), which are asked for by nothing.** The other six are not this measure's to remove:
they belong to other sprints, and **one of them (:1213) sits inside a block his own ruling put in
(G5)** — a blanket "remove explanatory hints" would hit it, so that one goes to him, not to a rule.

The first version's claim that *"if the wall goes, four of the six have nothing left to explain"* is
**retracted**: by reading each, about one hint (:1277) is about the wall; :1239, :1163 and :1213
explain other blocks.

**The tooltips are the same defect through a different attribute.** Eighteen `title=` strings,
the longest 326 characters, are user-visible prose the census method does not count. They are named
here as unreached for judgement beyond this sentence: **a subtraction proposal that removed the
hint paragraphs and left the tooltips would move the prose, not remove it.**

---

## FINDING 3 — the sample (G8)

Spec **:274–275** and **:302–303** ask for it, so this is a scope question and not an addition. At HEAD the
figure *is* rendered (`{n} of {total}`, :1319–1320), which closes the half the 28f record named.

**Still open:** the sample is ungrouped, one symbol plus a summary per row. Reader — someone
deciding whether the cataloguer's output is worth accepting. Next action — **none from here**: they
cannot accept, reject, or open a row. So the block *informs and does not enable*, which U-TWO calls
a decoration rather than a feature. **Goes to the view's owner as a question; not subtracted.**

---

## FINDING 4 — a self-approving deviation: NOT FOUND, which is not the same as ABSENT

U-FOUR asks for a deviation declared inside a plan that changes what the human sees. **I searched
the 28f spec (:262–306) and one commit message.** I did **not** read the 28f *plan* or its
checkpoint records, which is where a *"DECLARED DEVIATION"* would be. So the first version's
"none found … reported as an absence" overstated: **nothing was found in what was read, and the
place such a thing lives was not read.** Carried as an open item, not as a clean result.

---

## What this seat could not do, stated plainly

1. **The store was consulted this time.** `experience(kind=nominate)` returned 8 candidates; none
   applies to *deciding what a view should contain*. The two that touch the studio — the app-wide
   stylesheet fact, and the lesson that a list of what a product exposes outside the code is a copy
   — were each checked against the Seats tab this stage added (Help lists no tabs; `FieldView`
   carries its own scoped style block, so `SeatsView`'s matches practice). **Selected: nothing.**
   The first version skipped this call on the claim that the store holds no UX lane, without making
   it; that was an inference presented as a result.
2. **This is still me executing the stance, not a hosted seat.** `seats/ux.md` is `fires: on a diff
   that changes a view`, and **nothing fires it**: it has no typed command, appears in no phrase
   table, and no hook watches a view diff. It is invoked when a person or I remember to — the shape
   this project has named *wired, not just built* and refuses. Filed as C7-1.
3. **Nothing yet shows the seat works on code its author did not write**, beyond this re-run of a
   panel an earlier session built. A seat file asserting it finds additions is not a demonstration.
4. **Not reached:** the other sprints' specs (G4, G5, G10–G14 provenance), the 28f plan and
   checkpoint records (Finding 4), and the tooltips beyond this paragraph.
