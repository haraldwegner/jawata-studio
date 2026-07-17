# ARCHITECTURE-conductor — the seat-command generator + conductor delivery (Sprint 25a)

> Design-mode artifact (architect seat convention, Sprint 25 D10). Version 1,
> 2026-07-18 — produced between spec sign-off and plan promotion (the /sprint
> design-mode step this sprint itself ships as D3). Watch mode diffs checkpoint
> changes against THIS picture.

## Target architecture

```
seats/*.md  ──(single source; 7 seats; SHIPPED IN THE BINARY via include_str!,
   │          materialized-if-absent into <config>/seats/ — config wins, so a
   │          user-edited seat regenerates every channel on redeploy)
   ▼
parse_seat_definition (runner.rs — REUSED, never a second parser)
   ▼
src-tauri/src/conductor.rs (NEW module — pure functions, no I/O in renderers)
   ├─ COMMAND_MAP: javadoc-writer→/javadocs · test-writer→/cover ·
   │               architect→/refactor · debugger→/debug · profiler→/profile
   │               (spec-editor/spec-auditor: named in the section, live in /sprint)
   ├─ render_claude_skill(seat)      → SKILL.md text (frontmatter + stance +
   │                                    Lane-1 loop contract)
   ├─ render_cursor_command(seat)    → command .md text
   ├─ render_antigravity_workflow(seat) → workflow .md text  [R1: verify format]
   ├─ render_claudeai_skill_zip(seats) → zip bytes (ZipWriter — first write use)
   ├─ render_phrase_table(seats)     → markdown table (phrase → seat)
   └─ render_conductor_section(seats, client) → rule-block lines
                       (catalog · when-to-involve-architect · discipline ·
                        per-client tail: "seat commands installed" | phrase table)
   ▼
manager_service.rs (EXISTING lifecycle — the only I/O layer)
   ├─ deploy_to_client: writes  ~/.claude/skills/<n>/SKILL.md ·
   │    .cursor/commands/<n>.md · antigravity workflows dir ·
   │    <config>/exports/jawata-seats-skill.zip (claude.ai; uploaded once)
   ├─ build_rule_block(+client): conductor section appended — the ONE deliberate
   │    invariant change: body-identical → identical-except-conductor-section
   └─ Delete removers + Regenerate force_rewrite for every new artifact kind
```

## Where new code lands

`conductor.rs` (renderers + tests) and surgical edits in `manager_service.rs`
(deploy/remove wiring, rule-block section, tests). One paragraph edit in
`~/.claude/skills/sprint/SKILL.md` (the design-mode step).

## What must not be touched

The runner loop/gates/journal (runner.rs beyond reusing the parser), seat file
SEMANTICS (frontmatter keys unchanged; stance text is the seats' own), the five
existing rule-block sections, jawata-mcp (toolCount 45), the guard-hook
machinery.

## Dependency direction

conductor.rs depends on runner.rs (parser) and std only; manager_service.rs
depends on conductor.rs; nothing depends on conductor.rs's I/O because it has
none.

## The Lane-1 loop contract embedded in every command

The stance handoff: detect (the seat's detector calls) → do → verify (the
seat's NAMED gate calls — real jawata tools) → PROPOSE (present diff/files,
never auto-apply) → record the outcome (`experience kind=record`). Model/tier
frontmatter is runner-only and does NOT leak into commands — the front-door
agent runs as itself.
