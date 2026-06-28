# goja-studio

[![License: AGPL v3](https://img.shields.io/badge/License-AGPL%20v3-blue.svg)](LICENSE)

Desktop manager for running and orchestrating GOJA MCP servers across multiple Java projects.

`goja-studio` provides a clean desktop experience for managing `goja-mcp` instances, project workspaces, runtime state, and MCP client setup. By default it pulls the runtime from a maintained fork ([haraldwegner/goja-mcp](https://github.com/haraldwegner/goja-mcp)) that ships fixes upstream has not picked up yet — see *Release source* in the Settings UI to switch sources. The manager itself remains a pure orchestrator.

## Status

**v1.0.0 (first public release)**: `goja-studio` is a fully functional desktop application on Linux (x86_64 and aarch64), macOS (Apple Silicon only — unsigned, Gatekeeper bypass required), and Windows (x64 and ARM64 — unsigned, one-time SmartScreen bypass). It supports named workspaces of multiple Java projects (each running as one shared GOJA MCP service), live `workspace.json`-driven reconciliation, automatic fork-runtime download/update, and one-click deploy of MCP entries into Cursor / Claude Desktop / Antigravity / IntelliJ-style configs. The system-tray menu drives per-workspace lifecycle without opening the window. As of v0.15.0 the manager hosts ONE resident goja JVM per workspace and writes URL endpoints into client configs — N MCP clients × M workspaces collapse to **M JVMs total** (closes the 30 GB leak from v0.14.x stdio-per-client). Intel Macs aren't in scope (Apple Silicon only since v0.14.0). v0.16.0 also adds the recursive-search autoscan import (point the Add Project form at a parent folder like ~/Projects), per-client MCP config schemas (Antigravity's serverUrl shape deploys natively), auto-refresh of deployed configs on workspace changes, and closes the port/token/JVM lifecycle leaks.

### Version timeline

- **v0.9.x** (Sprint 7) — initial Tauri shell, project registry, per-project runtime spawn.
- **v0.10.0–v0.10.6** (Sprint 9 + Sprint 10) — configurable release source (fork by default), source-resolution fix (Maven `<sourceDirectory>` / Eclipse `.classpath`), named workspaces, multi-select bulk move + drag-drop, workspace-first dashboard, `workspace.json` file-watcher for live updates.
- **v0.11.0** — Sprint 11 cutover; Help.md / README updates for fork v1.5.0's tool consolidation (66 → 55 tools) and v1.5.1's five JDT-LTK structural-refactoring tools (60 tools per service).
- **v0.11.1** — Sprint 11 closeout; refreshed help screenshots; help/README cross-links for the new "System tray on Linux" caveat.
- **v0.12.0** — Sprint 12: tray menu lifecycle controls — per-workspace toggle entries with status icons, Start all / Stop all peers, 5-second background refresh so external state changes (process death) propagate. Paired with [fork v1.6.0](https://github.com/haraldwegner/goja-mcp/releases/tag/v1.6.0) which adds `compile_workspace` and `run_tests` (62 tools per service).
- **v0.13.0** — Sprint 13: tray menu refined for the GNOME / AppIndicator reality. Per-menu-item icons get stripped on GNOME, so the colored disks shipped in v0.12.0 never reached the user — replaced with monochrome unicode bullets (`●` running, `◐` starting, `○` stopped, `✗` failed) that render reliably in the menu's own font. Cleaner menu shape: `Open dashboard` (raises the main window) → workspaces with bullets (click toggles) → Start all / Stop all → Quit. Plus a real fix: tray menu now reflects workspace renames within ~1 s instead of waiting for a runtime restart (workspace_status_summary reads names from the live config_store, not the cached runtime snapshots). 1-second poll with cache-keyed change detection so the menu doesn't flicker on steady state. Paired with [fork v1.7.0](https://github.com/haraldwegner/goja-mcp/releases/tag/v1.7.0) which ships 11 new MCP tools across Ring 2 (code generation), Ring 3 (Maven dependency management), Ring 4 (formatter / workflow polish) — **73 tools per service**.

- **v0.13.1** — packaging patch: GitHub release workflow now also builds for Linux **aarch64** on a free `ubuntu-22.04-arm` runner, so every tag publishes both `_amd64` and `_arm64` artifacts. `install.sh` now detects `uname -m` and pulls the matching AppImage automatically — the same `curl … | bash` works on x86_64 and ARM laptops/servers (verified on NVIDIA DGX Spark / GB10).
- **v0.14.0** (Sprint 14) — hardening + features + Mac packaging. Three open manager bugs FIXED: #3 single-instance enforcement (`tauri-plugin-single-instance`), #2 process-death → Failed instead of Stopped (red `✗` glyph for external kill), #1 full fix for webview-blank-on-aarch64 (env-var baked into AppImage AppRun + .deb wrapper at CI build time, covers `dpkg -i` and direct-double-click install paths). New tray + dashboard feature: **Reload all** (sequenced stop-all → poll-until-stopped → start-all, 30 s deadline). New resident-service feature: **Autostart on boot** via `tauri-plugin-autostart`, surfaced as a Settings checkbox AND a checkable tray menu item. macOS packaging in CI (`macos-14` Apple Silicon only; Intel Macs unsupported — Apple stopped shipping them in 2023). install.sh gains a Darwin branch; README documents both Option A (curl one-liner) and Option B (DMG drag-drop) install paths plus the Gatekeeper bypass for unsigned builds.
- **v0.14.1** (v0.14.x patch, hours after v0.14.0) — live-smoke fixes + one feature redesign. Three manager bugs FIXED: #4 settings ↔ tray sync (backend emits `goja://settings-changed`; Svelte store listens), #5 drop "port" from the Settings Machine Runtime Controls subtitle (ports gone since v0.10.4), #6 rename "Data Root" card → **System Settings**. Feature #7 SHIPPED: **autostart-on-boot now also restores last-running workspaces** (Quit preserves them, Stop and Quit clears them, Failed retries). Help-page screenshots re-captured for the new System Settings card + Reload all + Autostart on boot. Manager-side doc-scrub of proprietary product-name references (forward-only anonymize; fork-side scrub shipped in fork v1.8.0 same day).

- **v0.15.0** (Sprint 15, 2026-06-08) — **bug #9 PRIMARY fix end-to-end (the 30 GB JVM leak)** + **bug #8 stable jar path**, coupled with [fork v1.8.5](https://github.com/haraldwegner/goja-mcp/releases/tag/v1.8.5) (HTTP/SSE-default transport). The manager now hosts ONE resident goja JVM per workspace (per-workspace `(port, token)` allocated in range 8800-8999 + persisted in `projects.json`) and writes **URL endpoints** into deployed MCP-client configs instead of stdio commands. With 3 Claudes + Cursor open: 2 JVMs total (one per workspace) instead of 16. `autostart_on_boot=false` honored via a new `WriterMode` enum (`Remove` default strips managed entries; `Disable` opt-in writes them with `disabled: true`). Stable `~/.cache/goja-studio/tools/goja/current/goja.jar` symlink closes bug #8 (POSIX atomic rename-into-place; jar path stable across auto-downloads). Default release-repo URL bumped to `haraldwegner/goja-mcp` after the personal-account rename, with read-time migration of legacy `haraldwegner/...` values.
- **v0.17.1** (2026-06-21) — **activates Lombok comprehension** (pairs with [fork v1.10.0](https://github.com/haraldwegner/goja-mcp/releases/tag/v1.10.0), which bundles `lombok.jar`). The manager now detects Lombok in a workspace (build-file dep or `lombok.config`) and conditionally prepends `-javaagent:lombok.jar` to that resident JVM, so GOJA's analysis tools see `@Data`/`@Getter`-synthesized members. Conditional (non-Lombok workspaces unchanged) and graceful (skipped if the runtime predates v1.10.0). E2E-proven against the built product. 7 new detection tests.
- **v0.17.0** (2026-06-18) — **sharpened the GOJA-usage rule block** deployed into MCP clients. The old one-line "prefer MCP over grep" policy was too vague to change agent behaviour; now it's a specific Java→GOJA routing table (symbol→`search_symbols`, usages→`find_references`, structure→`analyze_type`, structural change→refactoring tools — shell text-search is fallback-only) plus a small-steps TDD-refactor loop (green → one refactor → recompile+test → keep or `undo_refactoring`). Existing users get it on the next Deploy. First unit tests for the rule writer.
- **v0.16.2** (2026-06-16) — Linux scroll fix (the real one). The v0.16.1 CSS attempt missed; the actual cause was the WRY/WebKitGTK webview's accelerated-compositing path initialising only partially on hybrid Intel+NVIDIA GPUs (a native GTK WebKitGTK app scrolled fine on the same stack, which isolated it — [tauri#10566](https://github.com/tauri-apps/tauri/issues/10566)). Fix: set `WEBKIT_DISABLE_COMPOSITING_MODE=1` before webview creation (Linux only, covers dev + all packages). [Bug #20](docs/bugs.md).
- **v0.16.1** (2026-06-16) — live-smoke patch for v0.16.0. **Windows:** JVM spawns windowless (`CREATE_NO_WINDOW` — no more console window that lingered and blocked the port on restart); Claude Code path fixed to `~/.claude.json`; **Claude Desktop added as a distinct deploy target** (`%APPDATA%\Claude\claude_desktop_config.json`, native-HTTP). **Linux:** faster inner + whole-page scrolling (dropped sticky `backdrop-filter` blurs that forced a full WebKitGTK recomposite per scroll frame; `contain: paint` on scroll containers). **UX/docs:** autoscan "Discover" button hides once Browse has scanned; README timeline order; Help install section now Linux/macOS/Windows. Bugs [#15–#21](docs/bugs.md).
- **v0.16.0** (Sprint 16, 2026-06-12) — **Windows joins the matrix** (x64 + ARM64, msi + nsis, unsigned with SmartScreen note) on a 5-platform CI build. **Recursive search (autoscan)**: checkbox in Add Project scans any parent folder (depth ≤ 6) for Maven/Gradle/Eclipse projects, results unfold into the candidate list for prune-and-import — no `.code-workspace` seed needed. **Bugs #10–#14 FIXED**: per-client MCP deploy schemas (Antigravity `serverUrl` deploys natively), rename migrates port/token, delete + Quit stop and release residents, one-shot orphan prune of leaked `workspaces[]` entries (with backup), deploy auto-refresh on workspace mutations + resolve-failure surfacing. Paired with [fork v1.9.0](https://github.com/haraldwegner/goja-mcp/releases/tag/v1.9.0) (refactoring auto-apply + undo, 79 tools, readOnlyHint).

See [`docs/release-notes/`](docs/release-notes/) for per-release detail.

## Docs

- [`docs/plan.md`](docs/plan.md) - project scope, boundaries, and core decisions
- [`docs/roadmap.md`](docs/roadmap.md) - sprint-level roadmap for this repository
- [`docs/architecture.md`](docs/architecture.md) - architecture direction and module boundaries
- [`docs/adr/README.md`](docs/adr/README.md) - accepted architecture decision records
- [`docs/goja-management.md`](docs/goja-management.md) - how upstream `goja-mcp` is imported, versioned, and managed
- [`docs/goja-runtime-contract.md`](docs/goja-runtime-contract.md) - exact Sprint 1 launch, transport, health, and config contract
- [`docs/sprints/sprint-1-backlog.md`](docs/sprints/sprint-1-backlog.md) - first runnable-slice backlog and acceptance criteria
- [`docs/sprints/sprint-2-backlog.md`](docs/sprints/sprint-2-backlog.md) - managed-runtime upgrade backlog, team split, and acceptance criteria
- [`docs/sprints/sprint-5-backlog.md`](docs/sprints/sprint-5-backlog.md) - paths, workspace override, and Dashboard vs Settings UX documentation
- [`docs/sprints/sprint-6-backlog.md`](docs/sprints/sprint-6-backlog.md) - radical simplification and global configuration
- [`docs/tauri-bootstrap.md`](docs/tauri-bootstrap.md) - Sprint 0 and Sprint 1 bootstrap path for the Tauri app

## Installation

### Linux

You can install or update `goja-studio` on Linux using the provided installation script. This script will download the latest `.AppImage` and set up a desktop entry for you.

Run the following command in your terminal:

```bash
curl -sSL https://raw.githubusercontent.com/haraldwegner/goja-studio/main/install.sh | bash
```

Alternatively, you can download the `.deb` or `.AppImage` files manually from the [GitHub Releases page](https://github.com/haraldwegner/goja-studio/releases).

If you launch the `.AppImage` manually, ensure it has executable permission first:

```bash
chmod +x goja-studio_<version>_amd64.AppImage   # or _aarch64 on ARM
./goja-studio_<version>_amd64.AppImage
```

### macOS

**Apple Silicon only** (M-series chips). Intel Macs aren't supported — Apple stopped shipping Intel Macs in 2023; remaining hardware is six-plus years old. Intel-Mac users can install Rosetta 2 (`softwareupdate --install-rosetta`) and run the Apple Silicon `.dmg` via translation if needed.

The macOS build is unsigned (Apple Developer signing is a separate later track), so a one-time Gatekeeper bypass is required — see *Gatekeeper bypass* below.

#### Option A — curl one-liner (terminal users)

```bash
curl -sSL https://raw.githubusercontent.com/haraldwegner/goja-studio/main/install.sh | bash
```

The Darwin branch detects `uname -s` → `Darwin`, downloads the Apple Silicon `.dmg`, mounts it, copies `goja-studio.app` into `/Applications/`, unmounts, and clears the Gatekeeper quarantine attribute so the app launches without a right-click → Open dance.

#### Option B — DMG download (GUI users)

Download `goja-studio_<version>_aarch64.dmg` from the [latest release page](https://github.com/haraldwegner/goja-studio/releases/latest), double-click to mount, drag `goja-studio.app` into `/Applications/`.

#### Gatekeeper bypass (one-time)

After install, run once in Terminal:

```bash
xattr -d com.apple.quarantine /Applications/goja-studio.app
```

Alternatively: right-click the `.app` in Finder → **Open** the first time. macOS remembers the choice for subsequent launches.

### Windows

**Windows x64 and Windows ARM64** (since v0.16.0). Each release ships two installer styles per architecture — pick either:

- `goja-studio_<version>_x64_en-US.msi` / `..._arm64_en-US.msi` — WiX MSI installer.
- `goja-studio_<version>_x64-setup.exe` / `..._arm64-setup.exe` — NSIS setup wizard.

Download from the [latest release page](https://github.com/haraldwegner/goja-studio/releases/latest) and double-click. Tauri's installer pulls in the WebView2 runtime automatically if it's missing (preinstalled on Windows 11).

#### SmartScreen bypass (one-time)

The Windows builds are unsigned (code-signing certificate is a separate later track — same situation as the macOS Gatekeeper note above), so the first launch of the installer triggers a **"Windows protected your PC"** SmartScreen dialog:

1. Click **More info**.
2. Click **Run anyway**.

This is required once per downloaded installer, not per app launch.

### System tray on Linux

`goja-studio` exposes a system-tray icon for show / start / stop / quit, with per-workspace status icons from v0.12.0 onward. The tray relies on a **StatusNotifierItem / AppIndicator** host being available in your desktop environment:

- **Pop!_OS, Ubuntu (22.04+), KDE Plasma, XFCE, Cinnamon, MATE** — works out of the box, nothing to install.
- **Vanilla GNOME (Fedora Workstation, Debian GNOME)** — install `gnome-shell-extension-appindicator` once. On Fedora: `sudo dnf install gnome-shell-extension-appindicator && gnome-extensions enable appindicatorsupport@rgcjonas.gmail.com`. On Debian: `sudo apt install gnome-shell-extension-appindicator`. Log out and back in after enabling.

If the extension isn't installed, the manager itself still runs — you just won't see the tray icon. Disable the tray entirely from Settings → *Use system tray* if you'd rather it not try.

## What It Is

`goja-studio` is a desktop application for:

- registering Java projects
- starting and stopping `goja-mcp` instances
- managing per-project runtime state and workspaces
- showing health and status for running servers
- helping MCP clients connect to the right `goja` instance
- reducing friction when using GOJA across more than one project

## What It Is Not

`goja-studio` is not:

- a fork of `goja-mcp`
- a replacement for `goja-mcp`
- an Eclipse plugin
- a Java code analysis engine by itself

Java semantic analysis and refactoring remain the responsibility of upstream `goja-mcp`.

## Why This Exists

`goja-mcp` is a strong semantic Java MCP server built on Eclipse JDT, but it is centered on a single running server/session and project-loading workflow.

This project exists to provide a higher-level desktop experience for people who want to use GOJA across multiple Java projects without manually managing processes, workspaces, configuration, and client setup.

## Goals

- Provide a modern desktop UI for GOJA operations
- Keep upstream `goja-mcp` completely unchanged
- Support multiple Java projects through managed GOJA instances
- Make project registration and runtime management simple
- Improve day-to-day usability for MCP-based Java workflows
- Work well with Cursor, VS Code, Claude Code, and other MCP-capable clients

## Non-Goals

- Reimplement Java semantic analysis
- Add custom Java refactoring logic outside GOJA
- Replace IDE-native Java tooling
- Bundle proprietary project-specific integrations into this repository

## Shipped today

- Named workspaces of multiple Java projects (one shared MCP service per workspace).
- Live workspace updates via `workspace.json` file-watcher (no MCP-client restart needed).
- Workspace-first dashboard with multi-select bulk move + drag-drop between workspaces.
- Per-workspace and global start / stop / restart, with health and status display.
- Auto-download and auto-update of the GOJA runtime from a configurable release source (fork by default).
- One-click deploy of MCP entries into Cursor / Claude Desktop / Antigravity / IntelliJ-style configs, with safe-merge or replace-managed-section semantics, optional pre-write backups, and dry-run mode.
- System-tray icon with per-workspace toggle entries (monochrome status bullets since v0.13.0 — `●` running, `◐` starting, `○` stopped, `✗` failed) and Start all / Stop all peers.
- Diagnostics, log cleanup, JDT-workspace cleanup, "start from scratch" reset.

## Planned

- Windows packaging in CI (Sprint 15 / v0.15.0 — ARM + x64, unsigned with SmartScreen-bypass docs).
- Broader QA, cross-platform testing, and edge-case hardening.
- Auto-update UX after the .AppImage download (currently the user replaces the binary by hand).
- Scan-folder mode for Add Project (Sprint 15) — pick a parent directory (e.g. `~/Projects`); reuse the existing `WalkDir` + Java-project detection for a candidate list with checkbox-prune.

## Architecture Direction

`goja-studio` is planned as a Tauri desktop application with:

- a Rust backend for process/runtime orchestration
- a desktop UI for project and server management
- integration with upstream `goja-mcp` as an external dependency
- clear separation between UI, configuration, process management, and MCP client setup

## Relationship to GOJA

This project is built around upstream [`goja-mcp`](https://github.com/pzalutski-pixel/goja-mcp).

Important design rule:

- `goja-studio` manages `goja-mcp`
- `goja-studio` does not modify `goja-mcp`

If you need semantic Java analysis, navigation, refactoring, or diagnostics, those capabilities come from GOJA itself.

## Roadmap

### Current focus
- **Sprint 15 (v0.15.0)** — close manager bug #8 (auto-downloaded fork jar breaks deployed MCP client configs because the deployed `args` reference the versioned filename); Windows installer (ARM + x64); Scan-folder mode for Add Project. See [`docs/sprints/sprint-15-backlog.md`](docs/sprints/sprint-15-backlog.md).

### Completed (manager-side)
- **Sprint 14 (v0.14.0 + v0.14.1, 2026-06-04):** Six manager bugs FIXED — #1 webview-blank-on-aarch64 (env-var baked into AppImage AppRun + .deb postinst, covers every install path); #2 process-death → `Failed` with red `✗` glyph; #3 `tauri-plugin-single-instance` (double-launch raises the existing window instead of spawning a second tray icon); #4 settings ↔ tray sync via `goja://settings-changed`; #5 drop "port" from the Settings subtitle; #6 rename "Data Root" card → **System Settings**. New features: **Reload all** (sequenced stop-all → poll-until-stopped → start-all, 30 s deadline), **Autostart on boot** with session restoration (restores workspaces that were running at last shutdown — Quit preserves, Stop and Quit clears, `Failed` retries). macOS Apple Silicon packaging in CI (unsigned; Gatekeeper bypass documented). Manager-side doc-scrub of proprietary product-name references. Paired with [fork v1.8.0](https://github.com/haraldwegner/goja-mcp/releases/tag/v1.8.0): **9 fork bugs FIXED**, three new tools (`refresh_workspace`, `find_duplicate_code`, FQN overload extending the `find_*` family), Gradle path for the Ring 3 dep tools — **75 tools per service**.
- **Sprint 13 (v0.13.0):** Tray menu refined for the GNOME / AppIndicator reality (monochrome unicode bullets, `Open dashboard` opens the main window, single-second polling with cache-keyed change detection — no flicker, name-after-rename bug fixed). Paired with fork v1.7.0 (73 tools per service: Ring 2 codegen, Ring 3 Maven dep management, Ring 4 formatter / workflow polish).
- **Sprint 12 (v0.12.0):** Tray menu lifecycle controls — per-workspace toggle entries, Start all / Stop all, 5-second background refresh.
- **Sprint 11 (v0.11.0–v0.11.1):** Cutover for fork v1.5.0–v1.5.2 (Tycho-aware Maven, workspace bundle pool for `Require-Bundle`, Gradle Tooling API, parametric tool consolidation, JDT-LTK structural refactorings).
- **Sprint 10 (v0.10.4–v0.10.6):** Named workspaces, multi-select bulk move + drag-drop, workspace-first dashboard, `workspace.json` file-watcher.
- **Sprint 9 (v0.10.0–v0.10.3):** Configurable release source, fork-default runtime, source-resolution fix shipped via fork v1.2.1.
- **Sprint 8:** Packaging and distribution (automated GitHub Releases, Linux `.deb` and `.AppImage` installers).
- **Sprint 7 (v0.9.x):** Tauri shell, system-tray scaffolding, initial deploy-to-agents flow.
- **Sprint 6:** Radical simplification and global configuration.
- **Sprint 5:** Paths, workspace overrides, and Dashboard vs Settings UX.
- **Sprint 2:** Managed-runtime upgrades and team split.
- **Sprint 1:** First runnable slice — project registry, start/stop controls, health display.

## Tech Stack

- Tauri
- Rust (backend, runtime orchestration)
- Svelte desktop frontend UI
- upstream `goja-mcp` (Eclipse JDT–driven Java analysis)

## Development

Project setup instructions now start in [`docs/tauri-bootstrap.md`](docs/tauri-bootstrap.md).

Expected local prerequisites will include:

- Rust toolchain
- Tauri prerequisites
- Java 21+
- access to `goja-mcp`

## License

**AGPL-3.0** — see [`LICENSE`](LICENSE) / [`NOTICE`](NOTICE). goja-studio is the desktop control plane for [GOJA](https://github.com/haraldwegner/goja-mcp); previously distributed as `javalens-manager` under MIT, it is AGPL-3.0 from v1.0.0. Contributions are accepted under the [Contributor License Agreement](CLA.md) — see [`CONTRIBUTING.md`](CONTRIBUTING.md).
