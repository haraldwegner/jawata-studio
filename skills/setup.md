Bare `/setup`: establish what THIS project is, confirm it with the human, and write
`jawata-project.md` at the project's root. `/setup <path>`: the same, for the project at
that path.

**The four facts, and they are the whole file:** the stack, the build system, the source
roots, and what counts as green. Every seat's brief opens with them quoted, so a wrong
one is wrong in every run afterwards, and an absent file makes every seat say out loud
that it was told nothing.

## Why a FILE at the project's root, and not studio config

The declaration lives with the code: committed, reviewed, and true on any machine that
checks the repository out. The studio's own `projects.json` holds this machine's
bookkeeping about a workspace — resident port, token, heap ceiling — and is true only
here. They are not alternatives and neither can stand in for the other.

## STEP 1 — read, do not infer

Establish each fact from the project itself, and keep what you read:

- **stack** — the language and its version, as the project states it. A Maven
  `maven.compiler.release`, a `rust-toolchain.toml`, a `go.mod` line, an
  `engines` field. The version the project DECLARES, never the one installed on this
  machine; those differ exactly when it matters.
- **build** — what a seat must drive to build anything here. Name the tool and, where the
  repository has more than one, say which is authoritative.
- **source roots** — where the source actually is. Plural because a workspace of bundles
  has several, and a seat scoped to the wrong root finds nothing and reports that there
  is nothing. Prefer the build model's own answer over the directory names: in a
  compiler-aware workspace, ask the project model rather than guessing from paths.
- **green** — the command that says the tree is green. One command a seat can RUN and read
  the answer of.

## STEP 2 — PROVE the green command, by running it

This is the one fact you must not take on trust, and the reason is downstream: every
seat's verify gate leans on it, so a `green:` that does not run turns every seat's
"green" into a sentence nobody checked.

Run it. Report what it did — the exit status and the summary line it printed. If it fails
on a tree the human believes is green, that is a finding about the project and it goes in
the report, not quietly into the file.

**Capture its output to a file rather than piping it away.** A long gate re-run to recover
a line you dropped costs the human real time.

## STEP 3 — SHOW the four facts with their evidence, and ask

Present them as four lines, each with where you read it. Then ask the human to confirm or
correct, in one message.

**Never write a fact you could not establish.** The file has no partial form — the parser
refuses a declaration missing any of the four and names the field — and that refusal is
deliberate: a seat handed three facts guesses the fourth, which is precisely what the
declaration exists to stop. So a fact you cannot read from the project is a QUESTION for
the human, not a blank and not a plausible default.

## STEP 4 — write it

```
---
stack: Java 21
build: maven
source_roots: [org.jawata.mcp/src, org.jawata.core/src]
green: mvn -q verify
---

Anything a human wants to add about the project goes below the frontmatter. Nothing
reads it, so it is a place for the things a reader needs and a seat does not.
```

The frontmatter keys are exact and an unknown key is an ERROR when the file is read — a
typo'd field must not silently become an absent one. Then re-read the file you wrote and
confirm it parses, because a declaration that does not parse is reported to every seat as
MALFORMED, which is its own state and not the same as having none.

## STEP 5 — report

Four lines, each fact with its evidence; what the green command did when you ran it; the
path written; and anything the human corrected. If you stopped to ask and the answer has
not arrived, say which fact is open and that nothing was written — a file with a guessed
field in it is worse than no file, because the absent file announces itself in every
brief and the guess does not.

## What you never do

- **Never guess a fact to complete the file.** See step 3. This is the whole discipline.
- **Never write a declaration for a project you were not pointed at.** The path is the
  human's to choose.
- **Never edit the body below the frontmatter.** It is the human's prose about their own
  project.
- **Never report green without having run the command.** A verdict from recall is wrong
  every time, and this one is load-bearing for every seat that follows.
