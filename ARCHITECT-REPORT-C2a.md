# ARCHITECT-REPORT-C2a — watch mode over the coverage-agent refusal

Seat: architect, watch mode, fresh-context reviewer handed the diff inline, no tools. Slice: studio
`0d45064`, `ba25208`. Verdict: **ARCHITECT: MIXED**. Folded in `e39ad76`.

## Verdict by question

1. **Design fix or bandage.** Mixed. Refusing fetch-plus-attach of a named agent is the smallest rule that
   matches the quoted requirement ("an agent downloaded into the project and run from the shell is refused
   and names `run_tests`"), but it reads a NAME, so it is a proxy for the state "coverage is collected
   outside `run_tests`", which cannot be observed when a shell command runs. Per gap:
   - Maven's jacoco plugin goal on the command line: **CLOSED** (a new arm, same signal).
   - `-javaagent:$VAR` with the variable defined in another segment: **DECLARED** (needs data flow across
     segments, which no rule in the file has).
   - A jar renamed so it no longer says "jacoco": **DECLARED** (invisible to a name rule by construction).
   The declared gaps are now in the rule's doc comment so nobody reads it as "coverage cannot be collected by
   hand".
2. **A second implementation of an existing job (D-SIX).** Partly yes, and the reviewer's prediction was
   right: the environment-assignment loop was a second reader of what `command_words` already skips, with its
   own name test. **Measured on folding it:** the shared reader decided "assignment" on the WHOLE word and
   refused any word containing a `/`, so `JAVA_TOOL_OPTIONS=-javaagent:/t/a.jar java -jar x` was read by every
   rule as a command named after the assignment. That was a live defect in the shared reader, not only in
   this rule. Fixed at the source: one `split_command` reading, asked by `command_words` and a new
   `command_assignments`, deciding by the NAME. Regression test
   `an_assignment_whose_value_is_a_path_is_still_an_assignment`.
3. **Consumers and producers.** `Verdict::Deny{reason}` reaches the agent as `agent_message`, unchanged; the
   new text names `run_tests(coverage=true, ...)` and `jawata-fallback:`. **No change needed on the consumer
   side, because** the denial is a string the existing path already carries. **Not determined:** whether any
   document lists the guard's refusals. The generated instructions in `manager_service.rs` do not list the
   suite-cadence refusal either, so none was added. The `command_words` change reaches every rule that asks
   "what does this segment run"; the full studio suite (1,060 Rust tests) is green over it.
4. **Placement.** Right: after the suite-cadence gate, before the declaration lattice, so `jawata-author:` (a
   write claim) cannot open a run refusal and `jawata-fallback:` is the single logged way through. The
   whole-command substring check for the declaration matches how the lattice already behaves.
5. **Closing design.** Extract Function (the shared assignment reader), done.

## D4 — what was asked of the experience store and the pattern catalogue, and what was selected

Asked BEFORE the verdict was folded, after the rule was built: *a shell-command guard must refuse one
specific tool invocation shape and point at the sanctioned tool, without refusing commands that merely
mention the same words, and with a declared-reason escape hatch.* Eight nominated, **one selected**:
`bfbce716` (write a guard against the state it ensures; where that cannot be observed, require the act that
establishes it). It applies: the state here is unobservable, the rule is on its correlate, and the review
question it prescribes ("what is the cheapest command that satisfies the check and still reaches the bad
state?") is exactly how the three gaps above were found. It did not change the design, because requiring an
establishing act is not available for "collected outside `run_tests`"; it made the gaps get named. The
other seven were not applicable. No catalogue pattern was selected.

## Carried, not done

- The two declared gaps above.
- The rule recognises JaCoCo only; another coverage agent is a different job.
- On the real binary the Maven-goal command was refused first by the long-run bracket rule, which runs
  before `judge`; the coverage text for that shape is proved at the unit level only.
