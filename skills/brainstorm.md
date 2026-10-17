`/brainstorm <the question>`: produce OPTIONS with their trade-offs, for the person who
asked, read in this same conversation.

**This points the opposite way from `/btw`.** That one takes a thought IN, mid-flight, and
its value is costing nothing. This one produces thoughts OUT and IS the activity, so
ceremony is irrelevant here — thoroughness is the product. They are two commands for that
reason: merging them would price a capture at the cost of a brainstorm.

## What you produce

**Options, not a recommendation dressed as options.** Three to five, each with what it
means for the reader, what it costs, what it gives up, and what would have to be true for
it to be the right one. Then ONE recommendation with a single sentence of why — the
recommendation is the cheap part and the honest spread is the expensive part.

An option you would not actually defend is not an option, it is padding that makes the
list look thorough. Three real ones beat five with two inventions.

## Ground it, then generate

Generation is not the hard part and it is not where this is usually wrong. Before
producing options:

- **Ask the store.** `experience(kind=nominate, lanes=[experience, patterns],
  question=<the question in your own words>)` returns RANKED CANDIDATES plus a query_id, each with the situation it applies
  under and how it turned out. Ranking is not an answer — read each situation and decide
  which actually apply, then `experience(kind=decide, query_id=…, selected_ids=[…])`.
  **Selecting NOTHING is the right answer more often than not**, and it is a real one: it
  records that the store had nothing here, which is what the store previously could not
  say. Never select a candidate because it is the closest thing on offer.
- **Read what exists.** Where the question is about this codebase, the compiler-accurate
  tools answer it and a guess does not. An option built on a wrong belief about the code
  is worse than no option, because it reads as researched.
- **Say which facts you could not establish.** An option resting on an unverified
  assumption carries that assumption in its own line, not in a footnote.

## Who reads it, and when

**Its asker, immediately, in this conversation.** That is the whole answer, and inventing a
later reader would be the ceremony this command does not need. A brainstorm is a document
for the person who asked for it, read on arrival; it needs no consumption marker, no
backlog and no disposition rule.

Where they ask for it as a file, write one and say where. Otherwise the conversation is
the artifact.

## What it must NOT do

- **Never write to the experience store.** A generated option is not an observation, and
  recording one as experience puts fiction in the retrieval corpus — where it will rank
  against real questions forever. If a brainstorm produces something durable, it leaves by
  `/memorize` like anything else, with the cold-reader gate intact.
- **Never present a generated option as a measurement.** "This would be faster" is a
  prediction; "this was measured at 38 s against 307 s" is a fact. Separate them by
  sentence, not by tone.
- **Never collapse to one option because deciding feels like the job.** The spread IS the
  deliverable. A single path presented as the obvious one is a decision taken on the
  asker's behalf, and they asked for the options.
- **Never pad to a count.** See above.

## Report

The options, the recommendation, the facts you could not establish, and — if the store had
anything — one line on what you took from it and what it changed. If the store had nothing,
say so: an authoritative absence is worth more than a silence a reader has to interpret.
