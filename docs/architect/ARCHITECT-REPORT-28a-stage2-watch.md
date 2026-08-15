# Architect — watch mode, Sprint 28a checkpoint C2

Reviewed: `jawata-studio` commit `65e5bcb` ("Stage 2: Codex, Copilot CLI and VS Code join
the deploy roster"). Date 2026-08-15.

**Baseline diffed against.** There is no `ARCHITECTURE-<scope>.md` for the studio-side
client-adapter work — the sprint's design step produced
`jawata-mcp/docs/architect/ARCHITECTURE-mcp-host-boundary.md`, which governs the mcp engine
only. So the baseline here is the approved plan's Stage 2 / 2b text, the signed spec's D1,
and the architectural direction 1b established: **knowledge that varies becomes a value with
one owner.** Stating this because a watch diff against a missing artifact is a weaker check
than one against a real picture, and the reader should know which they are getting.

---

## Findings (ranked)

### 1 — INCOMPLETE DELEGATION. `client_dialect` is a half-formed `Client`, and 2b is about to build the other half somewhere else.

**This is the finding that matters, and it is time-critical: Stage 2b has not started.**

The new module owns *config-file dialect* — format, root key, URL field, entry extras — and
that part is genuinely well-placed. But a client is still described in **13 other places in
the backend**, measured, not estimated:

| # | site | file:line |
|---|---|---|
| 1 | `McpClientPaths` (a field per client) | `config.rs:118` |
| 2 | `DeployTargetFlags` (a field per client) | `config.rs:153` |
| 3 | `detect_default_mcp_client_paths` (candidates + construction) | `config.rs:197` |
| 4 | `merge_detected_mcp_paths` | `config.rs:1042` |
| 5 | `KNOWN_DEPLOY_CLIENT_IDS` | `manager_service.rs:3287` |
| 6 | `deploy_targets_for_paths` | `manager_service.rs:3341` |
| 7 | `derive_rule_path` | `manager_service.rs:3430` |
| 8 | `derive_global_rule_path` | `manager_service.rs:3457` |
| 9 | `client_still_receives_seat_commands` | `manager_service.rs:3482` |
| 10 | `derive_seat_commands_dir` | `manager_service.rs:3486` |
| 11 | `seat_artifact_paths` | `manager_service.rs:3500` |
| 12 | `utility_artifact_paths` | `manager_service.rs:3581` |
| 13 | `derive_hook_settings_path` | `manager_service.rs:4876` |

Stage 2 collapsed three literal sites into the dialect **and added one new one** (#9). Net
scatter is roughly unchanged. Adding the three clients required touching six of these by
hand, and nothing fails if one is missed — a client absent from #5 is refused at deploy with
a clear error, but a client absent from #7 or #10 silently falls through a `_` arm to a
default that may be wrong for it.

**Why now.** Stage 2b's stated deliverable is "one shared roster constant" — on the
**frontend**. Building a single roster on one side of the boundary while the other side
keeps 14 hand-maintained descriptions does not remove the bug class; it creates a second
authority that must agree with the first. **The two will drift, and the drift is exactly the
`claudeDesktop` / `claude_desktop` shape that made Claude Desktop silently undeployable
once.** Two-word clients are now three (`claude_desktop`, `copilot_cli`, and `vscode` is a
near-miss), so the surface for that bug has tripled.

**Design fix, not bandage:** one backend `Client` record — id, camelCase settings key, label,
path candidates, dialect, rule path, commands dir, supported-ness — with #5, #6, #7, #9, #10
*derived* from the list rather than written beside it, and the frontend roster generated
from or asserted against the same source. `McpClientPaths` / `DeployTargetFlags` must stay
field-per-client (they are the serde contract with existing settings files on disk, and
turning them into maps is a migration, not a refactor) — but they can be *checked* against
the roster by a test, which is the cheap 80%: a client in the roster and missing from the
struct fails the build's test run instead of shipping.

**Dispatch:** fold into Stage 2b's scope before it starts. If 2b ships a frontend-only
roster, this finding is not addressed and should be re-raised at C2b.

### 2 — The `_` fallback arms are silent defaults where a refusal would be safer.

`derive_rule_path` (#7) ends `_ => jawata-studio-rules.md`. The three new clients take that
arm, so each deploy writes an inert markdown file into `~/.codex/`, `~/.copilot/` and the VS
Code user dir that **no client reads**. Nothing is broken; the user gets litter in three
directories and the deploy reports a `rules` section it did not really deliver.

The dossier records this as a deliberate deferral to D10 / Stage 6, which is legitimate. The
architectural point is separate: a `_` arm that produces a *plausible-looking* answer is how
a missing mapping stays invisible. #5 already demonstrates the better shape — it REFUSES an
unknown id and names the known ones, and that refusal is what made the Claude Desktop bug
loud instead of silent.

**Design fix:** make the steering path an `Option`, `None` for clients with no known
steering file, and have the deploy report "no steering channel on this client" rather than
writing a file nobody reads. That is also the honest input to D3's capability matrix, which
must not claim a steering cell that was never delivered.

**Bandage alternative** (acceptable if Stage 6 is close): leave it, and make sure D3's matrix
marks steering for these three as not-delivered rather than reading the deploy's `rules`
section as evidence.

### 3 — `client_still_receives_seat_commands` is a predicate where the roster should carry a fact.

A free function whose whole body is `client != "antigravity"` is a supported-ness flag that
has not been given a home. It is correct and it is tested; it is also the fourteenth place a
client is described, and Stage 2b is about to introduce a *proper* `supported` concept with
three states (supported / available / default). When it does, this function is either
deleted or it becomes a second source of truth for the same fact.

**Design fix:** it should read `roster(client).supported`, and it should be created that way
in 2b rather than left to be reconciled later. Small, but it is the same disease as #1 and
it was introduced by this commit, which is the reason to name it rather than let it settle.

---

## Reviewed diffs — design fix or bandage?

| change | verdict |
|---|---|
| `client_dialect.rs` — dialect as a value, three literal sites collapsed | **Design fix.** Correct pattern, mirrors 1b's `HostOS.of()`. Under-scoped (finding 1), not misdirected. |
| `write_managed_toml_block` / `remove_managed_toml_block` | **Design fix.** A second format genuinely needs a second writer; dispatch is on the dialect, not on a client name. Comment preservation is a real requirement met properly rather than approximated. |
| `is_managed_mcp_key` gains the bare names | **Design fix.** The predicate now matches what the producer (`gateway_entry`) actually writes, and the test asserts against that constructor rather than a literal — so the producer and the predicate cannot drift apart again. This is the contract-both-sides check done right. |
| Antigravity: stop writing, keep removing | **Design fix.** Retaining `derive_seat_commands_dir`'s mapping so old files stay reachable is the correct call and the reasoning is recorded in the code. |
| `client_still_receives_seat_commands` | **Bandage.** See finding 3. |
| `deploy_resolves_here` + unconditional `release.yml` step | **Design fix.** It closes a gap the existing tests were structurally unable to see, and it closes it by *running on the platforms that differ* rather than by adding more Linux assertions. The `MAIN_SEPARATOR_STR` join is the detail that makes it real on Windows. |

## Below the fold

* `validate_client_config_shape` and `validate_written_toml_config` are now two validators
  asserting the same three properties in two dialects. Acceptable duplication today; if a
  third format ever arrives, the shape should be one validator driven by the dialect.
* `path_has_managed_entries` still parses JSON unconditionally, so it returns `false` for
  Codex's TOML — meaning a Codex client that HAS jawata entries is not recognised as an
  auto-refresh target. Not exercised by anything in Stage 2, but it is the same
  format-blindness the dialect exists to end, one level away.

## Skipped by record

None — no previously-declined proposal covers this scope.

## Contract-change check (rule 5)

The changed contracts are the settings file (`McpClientPaths`, `DeployTargetFlags` gain
three fields each) and the on-disk client config files.

* **Consumers, derived not recalled:** `svelte-check` over 113 files reports 0 errors, so no
  frontend consumer breaks on the added fields; the frontend's explicit object construction
  simply omits them, and serde's `#[serde(default = "default_enabled_flag")]` restores them
  as `true`. Verified by running the check, not by inspection alone.
* **Producers:** `gateway_entry` is the producer whose shape the `is_managed_mcp_key` fix
  addresses; it is now asserted against directly.
* **Must the other side change too?** Yes, and it is scheduled: the frontend must gain the
  three clients for them to be selectable in the dashboard picker. That is Stage 2b, and
  until it lands the new clients deploy through backend defaults only. Stated in the dossier
  and the commit message so it is not mistaken for done.
* **Question I cannot answer and the human can:** is every consumer of the settings file
  inside these two repositories? An external tool or a script reading `settings.json` would
  be invisible to any search I can run. If one exists, the added fields are additive and
  should be safe, but the enumeration above is incomplete by construction until that is
  answered.
