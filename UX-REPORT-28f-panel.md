# UX seat — re-run on the 28f panel

**Seat:** `seats/ux.md`. **Subject:** `src/lib/components/MemoryView.svelte`, the Store &
Maintenance section, at the commit the measure names — jawata-studio `a1f8fee`, *"28f Stage 7:
MemoryView renders the areas answer and the accepted-row sample"* — and at HEAD, because a re-run
that reports a block which has since been removed is worse than no re-run.

**This seat PROPOSES. Nothing here is built, and no component is edited.**

---

## The census — U-ONE

| | |
|---|---|
| user-visible block markers in the whole file, at HEAD | **51** — headings, buttons, tables, column headers, hint paragraphs, placeholders, disclosures |
| hint paragraphs in the whole file | **19 at `a1f8fee`**, and the count has not fallen at HEAD |
| blocks I JUDGED | the **Store & Maintenance** section, `a1f8fee:1100`→ / HEAD:`1100`→ |
| blocks I did NOT judge, and why | everything above that heading — *Memory / Database*, *Memory sources*, the shared-scope control. They are a different deliverable's and the measure names this section. **Unjudged, not cleared** |

A census mismatch is a failure under this seat's own rule, so it is stated rather than smoothed:
I judged one section of a file whose census is 51 blocks, and the unjudged remainder is named.

---

## FINDING 1 — the package-name wall is STILL RENDERED, and I nearly reported it as gone

`{#each selectedRow.describedAreas as area (area)}` — `a1f8fee:1115`, and **HEAD:1265**. An
unbounded loop over every described package name.

**I first concluded it had been removed**, because HEAD's surrounding code now says *"Studio keeps
no list of bundles"* and *"Which packages exist is a question only a loaded project knows"*. Those
sentences are about the COMPLEMENT — which packages are *not* described — and I read them as being
about the list itself. Reading the loop settled it. Recording the near-miss because it is the
class this sprint has filed five times: a property asserted about a block rather than read out of
its source.

**What changed at HEAD is real but is not a removal:** the wall gained a count
(`{selectedRow.describedAreas.length} described`, HEAD:1279) and a truncation notice
(`describedAreasTruncated`, HEAD:1281) — and HEAD:1272 carries a comment recording that the
truncation field already existed while *"nothing here read it"*. So the figure the 28f record
missed is now present. The wall underneath it is unchanged.

**Reader and next action: NONE, and that is the finding.** Who reads a list of 54 package names,
and what do they do after reading it? There is no action: the names are not clickable, not
filterable, and not the input to anything. The count and the truncation notice *do* have a reader —
the person deciding whether the cataloguer has more to do, who acts by running
`experience(kind=describe, action=next)`, which the hint beside it already names.

**Asked for by nothing.** 28f Stage 7's own commit message states the ask: *"WHICH AREAS ARE
DESCRIBED, with the complement said in WORDS rather than left blank."* That sentence asks for an
ANSWER and for the complement in prose. It does not ask for the enumeration, and the enumeration
is what fills the panel.

**Proposed: subtract the wall, keep the count and the truncation notice.** The count answers the
question; the names answer nothing. This is U-FIVE's shape — remove, do not explain — and it is
worth saying that the explanatory hint beside the wall is how the hint paragraphs arrived.

---

## FINDING 2 — the hint paragraphs, nineteen of them, six in this section

`class="hint"` at `a1f8fee` × **19**. In the Store & Maintenance section at HEAD: lines 1108,
1163, 1213, 1239, 1277, 1293, 1318, plus two inline spans.

**Reader: mixed, and that is why they cannot be judged as one block.** Three have a real reader
and a real next action and should stay — the empty state (*"No workspaces (or residents
unreachable)"*), the starting state (*"resident starting — retrying automatically"*), and the
seeding hint (*"'Load' seeds it from your files"*). Each tells someone who is stuck what to do.

**The rest are prose about how the system works, attached to a block that was already confusing.**
A reader does not arrive at a maintenance panel to learn why enumerating packages needs a loaded
project. **Asked for by nothing:** no sentence of 28f Stage 7 asks for explanatory paragraphs; they
are the cure applied to Finding 1's wall, and they are the second addition in a chain where the
first one caused the second.

**Proposed: subtract the explanatory ones with the block they explain.** If the wall goes, four of
the six have nothing left to explain. Keep the three that are states, not lectures.

---

## FINDING 3 — the sample has its figure now, and one thing still has no reader

`{#each selectedRow.acceptedSample as row, i}` — `a1f8fee:1147`, HEAD:1311. The 28f record's
complaint was *"one ungrouped 10-of-9,859 sample"*. At HEAD the figure IS rendered
(`{selectedRow.acceptedSample.length} of …`, HEAD:1320), so that half is closed.

**Still open: the sample is ungrouped and each row is a symbol plus a summary.** Reader — the
person deciding whether the cataloguer's output is worth accepting. Next action — there is none
from here: they cannot accept, reject, or open one. **So the block informs and does not enable**,
which U-TWO calls a decoration rather than a feature. Not *asked for by nothing* — the 28f ask
names the accepted-row sample explicitly — so this is a scope question rather than an addition,
and it goes to whoever owns the view rather than being subtracted.

---

## FINDING 4 — no self-approving deviation found in this section's record

U-FOUR asks for a deviation declared inside a plan that changes what the human sees and approves
itself. **I did not find one for this panel**, and I am saying so rather than leaving the heading
out: 28f Stage 7's commit message declares what it added in prose and the sprint record then
caught the additions *afterwards*, which is a late catch rather than a self-approval. The absence
is reported as an absence.

---

## What this seat could not do, stated plainly

**The store was not consulted.** U-SIX requires `experience(kind=nominate)` before judging, and
this run is the seat's first — it is the run that proves the seat exists, and the store holds no
UX lane yet to nominate from. The next run owes that call. Recorded rather than skipped silently.

**And this run is me executing the stance, not a hosted seat.** The seat file declares `gates: []`
with a census discipline in place of a jawata gate, for the reason written there: this product's
gates answer about Java and this subject is a Svelte component. The census is the thing that can
fail, and it did constrain this report — it is why the unjudged blocks are named above instead of
being left to read as clean.
