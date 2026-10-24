# Dogfood of studio 4.4.10 and engine 4.4.15 — failures and fixes

Written 2026-10-24 on his word ("You make a list of failures and bug fixes necessary due to dog food. Then I make another walkthrough. You fold into the plan then"). Each row says what was seen and what proves it. Nothing here is folded into the plan yet.

## Already done in this round (committed, not pushed)

| # | What | Proof |
|---|---|---|
| D1 | The tray's second line reads `jawata studio 4.4.10 / engine 4.4.15`, and the health sits in the Workspaces line (`Workspaces — 3 of 3 residents healthy`). When a workspace is amber the line carries the whole sentence naming it and why, because a tray without tooltips has no other place for it. With no workspace the health keeps a line of its own. | Four tray tests; three failed before the change. His look at the tray decides. |

## Failures seen, with the fix proposed

| # | Seen | Evidence | Proposed fix | Home |
|---|---|---|---|---|
| F1 | The red "1" on `simple-gradle` and `gradle-latin1-script` in Cursor's file tree comes back after every restart. | The editor's own problem list: "Supplied javaHome is not a valid folder", naming a path under `redhat.java-1.57.0`, an extension version that is no longer installed. Still present after a full Cursor restart and after three settings in `jawata-dev.code-workspace` (samples excluded from import, Gradle runtime set to JDK 21, Gradle import off). | The stale path is probably in the Java extension's saved state, which only "Java: Clean Java Language Server Workspace → Restart and delete" removes. Not studio code. It matters because the engine's release check fails when the editor writes Gradle caches into the samples (it happened once, 2026-10-23). | His action, or his approval to read `~/.config/Cursor` |
| F2 | In this session the jawata hook programs and the three jawata tool servers are gone. | `~/.claude/jawata-studio/` holds no `jawata-hook-guard` or `jawata-hook-stop` (only `backup-v3.16.0/*.bak`); `~/.claude/settings.json` names no jawata hook; the session was told the three servers were removed from its configuration; the latest prompt carried no hook context. **Cause not established.** | First ask: did he press "Remove from agents"? If not, Deploy after the 4.4.10 update lost them: reproduce with the Dashboard's deploy status and the config files' modification times. | Studio patch if a defect |
| F3 | The stop hook answered "THE autocontinue SEAT RAN AND RETURNED NO VERDICT" about nine times today although the seat's verdict was in its report. | The hook's reader accepts an indented verdict (`verdict_in` trims leading spaces; a test pins it), so indentation is not the cause. It binds the verdict to the judge's tool call, and a judge run in the background reports back as a separate message. **Cause not established.** | Cheapest first step: take the transcript lines of one failing case and run them through the hook's own test harness (`read_turn`) to see which line it ignores. | Studio patch (`src-tauri/jawata-hook/src/stop.rs`) |
| F4 | The "ANSWER FIRST OR WORK FIRST" guard refused three plain read-only checks (release status reads and a log read) in turns that cross-session messages had opened, not a message of his. | The refusal text says "his own message opened it". **Cause not established**; it may be the hook counting a peer's message as his. | Same method as F3: replay the turn through the hook's tests. | Studio patch |
| F5 | The Seats view's scroll fix is a candidate: cards inside the scrolling view draw no shadow. | Unmeasured; WebKitGTK has no frame counter here. | His scroll in the installed 4.4.10 judges it. If still slow, the bar's blur effect (`backdrop-filter`) is next. | Him, then C11-11 |
| F6 | Patterns recheck: all three services run engine 4.4.15 (checked from the process list), so the recheck can run. | Expected 0 compile errors on `patterns` (it was 5 on 4.4.13, all in `page-object`, caused by the folder holding its own sources next to its modules'). | Run `compile_workspace` and `run_tests` on `page-object` in a session that has the patterns tools (see F2). | Me, after F2 |
| F7 | The machine's 128 file-watcher slots are full; Cursor and its helpers hold about 67 (measured by the 28k session). Any program that opens a watcher can fail. | Six engine tests failed on it on 2026-10-23. | Engine side fixed by the 28k session (workspace watcher polls when refused), unreleased. The limit is a system setting: `sudo sysctl fs.inotify.max_user_instances=1024`. | His setting; engine in the next engine release |
| F8 | The studio has no check on a push to main; only the tag's release job runs. The engine's CI is what caught the 4.4.14 failures on macOS and Windows. | `.github/workflows/` holds `release.yml` (tags) and `cla.yml` only. | Add a test job (cargo test and svelte-check on Linux, macOS and Windows) on push to main. It changes the release process, so it needs his word. | His word |

## Open on the engine side (not a studio fix)

- **S19-15:** on Windows a path given in short-name form (`C:\Users\RUNNER~1\…`) is not matched to the project loaded under its long name. The tests no longer exercise it. Planned for the OS-coverage sprint.
- **S19-13:** the rule "take the store's lock before using its connection" lives in about 60 call sites; one forgot it (S19-12, fixed). Planned for Sprint 28h.
- **Unreleased engine work:** the 28k session has committed engine work on `jawata-mcp` main (including the watcher fix) that no release carries yet.

## Mistakes of mine, already corrected

- I called engine 4.4.14 released after watching only the publish job; the CI run on the same commit failed on macOS and Windows. 4.4.15 fixed it and I now wait for every workflow on the pushed commit before tagging.
- My first release check for 4.4.15 never started: its wait loop matched its own command line. Fixed; recorded as a lesson.

## For his walkthrough of 4.4.10

1. **Tray:** Open dashboard, then the versions line, then Workspaces with the health in their line.
2. **Seats:** scroll the whole list; is it fast now (F5)?
3. **Add projects:** pick an IntelliJ project folder or its `.idea/modules.xml`. The new screen was checked from its source only; the render harness the 28k session restored (`build/ux-render`, commit 20519ae) can produce a picture.
4. **Help:** the tray sentence and the IntelliJ line are new.
5. **Deleting** (4.4.9): a running or starting workspace cannot be deleted, nor one of its projects; every delete asks first.
