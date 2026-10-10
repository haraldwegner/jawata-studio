# ARCHITECT-REPORT-C1 — watch mode over the S1 revocation repair

Seat: architect, watch mode, read-only. Slice: studio `b9a5879` (`stop::is_typed_command` and its use in
`autonomy::note_prompt`) and the D5 gate tests that followed. Verdict: **ARCHITECT: MIXED**. The run
read the code and ran nothing; the transcript probe at the end is the editor's.

## Verdict by question

1. **Design fix or bandage — a bandage with a closing path.** `is_harness_text` still lists `command-name`
   for the stop side; the grant carves it back out through `is_typed_command`. One fact ("what counts as
   his keyboard") now has three branches, and the doc comment on `is_typed_command` itself says the list is
   "right for the stop side and wrong for the GRANT". The comment at `stop.rs:1762` says the two sides must
   never disagree about who typed; they now do. **Smallest design alternative:** one classifier on the text,
   `enum Channel { Keyboard, Command, Machine }`; the grant clears on `Keyboard | Command`; the stop side
   chooses its own policy for `Command`; no tag is listed twice.
2. **Consumers.** `is_harness_text` is called from `is_harness_line` and `note_prompt`; `is_his_keyboard`
   is the single stop-side verdict (`read_turn`, `prompt_came_from_machine`, `last_user_turn_is_machine`);
   `is_typed_command` has one production caller. **The stop side must change too, or the gap is named as
   open:** on a client that states no `turnOrigin`, a wrapped typed command is not his keyboard there, so it
   opens no human window and never raises `user_asked`; and `seats_in` matches `/cover` only at line start,
   so a wrapped `<command-name>/cover` never registers a seat.
3. **Capability with no production caller:** none. The unused capability is the missing stop-side use above.
4. **Recurring structure.** Two parallel "who typed this" paths (text and entry shape) plus the
   `machine` flag in `pipeline.rs` as a third answer used only as a log label. At least the second patch to
   this seam.

## The question the architect could not answer

Can a machine emit a `command-name`-opened prompt (a scheduled or `/loop`-fired slash command, a queued
command)? If so the carve-out lets a machine end the grant, against "the grant is read from the SHAPE of
the event, never its text". Suggested guard: honour the carve-out only when `turnOrigin == human` or the
origin is not stated.

**Editor's probe (this project's transcripts, 2026-10-11):** of the user lines that actually OPEN with
`<command-name>` or `<command-message>` — 408 — origin was `human` on 61, unstated on 345 and `isMeta: true`
on 2; none carried `turnOrigin` of `peer` or `task_notification`. So no machine origin was seen among them,
and two meta-flagged lines are unexplained. One project's transcripts is a sample, not a census.

## Disposition (S1, one round inside execution)

Not folded in code: the classifier unification and the stop-side `seats_in` / `user_asked` twin are a
behaviour change on the stop side with its own gates, outside C1's written clauses. **Carried, named open.**
