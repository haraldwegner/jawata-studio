# Architect review — watch mode

**Reviewed:** 2026-08-13. One day's diff on jawata-studio (v3.7.8 -> v3.7.16),
4 files, 1324 diff lines. Produced by the architect seat in a FRESH CONTEXT —
it saw the diff and the factual record, never the reasoning that produced them.

**Verdict in one line:** not eight bandages — three of the eight produced real
design fixes, and the most important (making the platform a *value* instead of
`cfg!(windows)`) is the right move. But the structure is **not immune** to the
failure that recurred; it is **currently correct**. Three live instances of the
same "fixed in one lane, not the other" shape exist *inside the fix for that
shape*, and one pair of tests thirty lines apart assert opposite things about
the same role — both green because one runs against an empty directory.

---

## P1 — INCOMPLETE DELEGATION: the runnability rule has one consumer out of six

v3.7.13 introduced the right rule: *a hook is registered only where the client
can run it*, with a home (`invocation_is_runnable`) and a test.

It is called from **exactly one production site** — the observer registration.
Derived: `grep -n 'invocation_is_runnable' src-tauri/src/manager_service.rs` =
2 definitions, 1 production call, 4 test calls.

The other five Claude Code registrations — guard, primer, recall, userprompt,
stop — are written unconditionally. Safe only while the binaries are present,
because the resolver silently falls back to the `.sh` when the binary is absent.
That case is not hypothetical: this file already carries a comment recording it
on a shipped `.deb`. On Windows that path registers five `.sh` hooks that cannot
execute — including the **Stop** gate and the **guard**.

The asymmetry is explicit in the same diff: the Cursor lane REFUSES when no
script can run; the Claude Code lane, in the same release, proceeds. **The
defect class of the day, shipped inside its own fix.**

**Fix:** one `register(role, resolved_path, platform)` that renders, applies the
rule, and writes-or-unregisters. Smallest step: move the check inside the
builders and have them return `Option<Value>`.

---

## P2 — `HostPlatform` is the right idea at the wrong depth

Genuinely the correct move — `cfg!(windows)` made the Windows branch
unrepresentable in a Linux test. But three functions TAKE the object and ask
what it is rather than asking it to do the work: `role_binary_file_name_on`,
`invocation_is_runnable_on`, `hook_command_for`. `HostPlatform::host()` is
called at **11 production sites**, each an independent decision.

**The concrete cost:** the Cursor side got a platform-parameterised seam and two
Windows-observable tests. The Claude Code side got **no `_on` variant at all** —
all six builders bake `HostPlatform::host()` internally. So **no test on any
machine can observe what a Windows Claude Code `settings.json` contains.** The
3.7.16 defect is prevented by six correct call sites, not by a structure.

**Compounding it:** derived from `.github/workflows/release.yml` — the hook
crate's tests run on all five matrix rows (line 63); the workspace suite is
gated `if: matrix.platform == 'ubuntu-22.04'` (lines 73-76). **Every one of the
eight defects was in the studio crate. The crate whose defects are all
Windows-specific is the one Windows CI does not test.**

---

## P3 — Contract drift inside the file that exists to prevent it

1. **A sibling test asserts the opposite and passes vacuously.**
   `cursor_roles_the_contract_calls_scripts_are_still_scripts` reads
   `role_generations[role]["live"]` — the Claude Code field, not the new
   `cursor` override — so for the observer it asserts the Cursor deploy must
   still write `jawata-observer.sh`. It passes only because it points at a
   directory **nothing ever creates**, so the resolver falls back to the script.
   Two tests thirty lines apart, opposite claims, both green.

2. **The section's own prose is now false.** `_why` still says "the Cursor
   deploy is all script-generation by construction"; `_scope` still says "the
   other three Cursor roles remain scripts". All four are binaries as of this
   diff — in the same commit that added a test to stop exactly this.

3. **The per-client axis has no completeness rule.** The cross-crate check reads
   only `live`; the new studio test hardcodes the observer's name. A fifth
   Cursor role would again be decided by code with no declaration.

---

## Reviewed diffs — design fix or bandage

| Change | Verdict |
|---|---|
| `HostPlatform` replacing `cfg!(windows)` | **Design fix — best change of the day** |
| Sweep moved INTO `deploy_hook_binaries` | **Design fix** |
| All payload parses routed through `parse_payload` | **Design fix** |
| `hook_command_for` split from `display_path` | **Right seam, bandage-shaped application** |
| `write_hook_config` called from the Cursor lane | **Bandage** — two independent callers remain |
| `cursor_role_is_binary_live` | **Bandage** — hardcoded per-client exception |
| `CURSOR_ROLES` table replacing three hand-kept lists | **Design fix** |
| `invocation_is_runnable` + observer unregistration | **Half a design fix** (see P1) |
| `let _ = &live_files;` | **Leftover — delete** |

**Tests: mostly discriminators, which is unusual and good.** Seven would fail
against the pre-fix code. Two are guards, correctly so. One —
`a_script_hook_is_not_registered_where_a_script_cannot_run` — is a guard on the
HELPER, not the six call sites: *"the test that makes P1 look covered when it is
not."*

> Every discriminator is only *representable* because of the fix it guards.
> That is the honest form of "no test caught it" — the suite could not have, and
> now can. But none was ever run red against the shipped defect **on the
> platform where it occurred**.

---

## Contract note (the command string a client hands to a shell)

**Derived, not recalled.** `display_path` (43 hits, each classified — six were
hook commands, the rest UI/error/config text, none executed); `"command"`
emitters; MCP registration (**URL-based — no path becomes a command there**);
every `Command::new` (paths passed as ARGUMENTS, never as shell strings);
generated script bodies (no path interpolation).

**Producers: two, not one.** `hook_command_for`, and
`managed_cursor_hook_entries_on` which builds `./hooks/{file}` itself and is NOT
routed through the seam function. Safe today (relative, separator-free); if
Cursor ever needs an absolute path, that is where the bug returns.

**Consumers in-repo — contract kept at the changed clause**, verified rather
than assumed: the managed-entry predicate normalises `\` -> `/` before matching
and matches by `contains`, so the new quotes are harmless; the hook crate's
`role_for_binary` already splits on both separators and strips `.exe`.

**Consumers outside this repository cannot be enumerated from here.** Every
installed machine carries entries written by earlier versions. Verified from
source that each upgrades cleanly — backslash entries normalise and are
REPLACED not duplicated; misnamed binaries are swept. *"That is the good news,
and it is not luck — it follows from the predicates normalising separators."*

**Does the other side have to change? No.** One-sided and backward-compatible,
verified at both recognition predicates.

**The gap this opens:** `hook-events.json` has a `seam_files` section declaring
every file crossing the hook/studio boundary. **The rendered hook command is a
seam with no row** — and it is the seam that broke for six of the eight
releases. The discipline exists in this codebase; it has not been pointed at the
boundary that keeps failing.

---

## Below the fold

- `stop::read_turn` silently `continue`s on an unparseable transcript line — a
  BOM on line 1 of a Windows-written transcript drops it from the stop gate with
  no diagnostic. Same class as the fix just shipped, adjacent lane.
- `config::load_from` also parses without stripping a BOM (lower risk).
- `also_keep` is derived in the Cursor lane, hardcoded `&[]` in the Claude one.
- The sweep's "we own the `jawata-` prefix" claim is applied to two directories
  with different ownership semantics.
- `let _ = &live_files;` — nine lines computed and discarded.
- Four Cursor script generators are now unreachable in production.
- The unwired-gate is a ratchet at 74, not a zero gate; it cannot see dead locals.
- `hook_command_for` quotes but does not escape an embedded `"`.
- No test asserts a Cursor command contains no backslash (true by construction,
  unasserted).

---

## Gates

**Ran and read:** `cargo test --workspace` exit 0 (478 passed, 0 failed, 6
ignored); `build/unwired-gate.sh` exit 0 (PASS, 74 baseline unchanged); full
read of the diff and surrounding code, `hook-events.json`, `release.yml`, both
build gates.

**Could not run — stated, not glossed:**
- **Anything on Windows.** Every Windows claim here is derived from source, not
  observed. *"That is the same epistemic position the eight releases were in,
  and it is why running the studio suite on the Windows runners is the only
  dispatch that changes it."*
- `build/seam-gate.sh` — needs a built hook binary and a published jar.
- **The jawata MCP tools do not apply** — compiler-accurate *Java* analysis;
  this diff is Rust and JSON. grep/Read/cargo used deliberately, not as a
  degraded fallback, and every derived set names the command that derived it.
