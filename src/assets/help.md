# goja-studio Help

**goja-studio** is the desktop control plane for **GOJA** — it lets you create **named workspaces** of one-or-more Java projects, runs a single shared GOJA MCP service per workspace, and **deploys** the connection details into your AI tools (Cursor, Claude Desktop, Antigravity, IntelliJ-style configs).

The point: it gives your AI agents the same IDE-grade understanding of a Java codebase that a human developer gets in Eclipse or IntelliJ — call hierarchies, type hierarchies, references, refactorings, build classpath, JDK semantics. **Java agents on steroids.**

Use **Dashboard** for day-to-day work, **Settings** for runtime paths and agent config files, and **Help** (this page) for orientation.

## Installation & Updates

**Linux** — the install script downloads the latest `.AppImage` from GitHub Releases (matching your architecture, x86_64 or aarch64), verifies its checksum, and registers a desktop entry:

```bash
curl -sSL https://raw.githubusercontent.com/haraldwegner/goja-studio/main/install.sh | bash
```

For `.deb` packages, see the [GitHub Releases page](https://github.com/haraldwegner/goja-studio/releases).

**macOS** (Apple Silicon) — download the `_aarch64.dmg`; unsigned, so clear the Gatekeeper quarantine once (`xattr -d com.apple.quarantine /Applications/goja-studio.app`).

**Windows** (x64 and ARM64, since v0.16.0) — download the `.msi` or `-setup.exe`; unsigned, so the first launch needs a one-time SmartScreen **More info → Run anyway**.

Per-platform detail and the bypass steps are in the [README](https://github.com/haraldwegner/goja-studio#installation).

---

## Workspaces — the core concept

A **workspace** is a named group of Java projects loaded into one GOJA process and exposed to agents as **one MCP service** (`jl-<workspace-name>`). The agent sees the combined symbol set of every project in the workspace; cross-project navigation, find-references, and (in fork v1.4.0+) refactorings work across the whole group.

- **One workspace per cohesive task.** A bundle/multi-module application (e.g. an Eclipse RCP product with 12 OSGi bundles), a monorepo, or a single project that you want isolated — each gets its own workspace.
- **Live updates.** Add or remove a project from a workspace and the running GOJA picks it up within ~1 second through a `workspace.json` file watcher. No MCP-client restart, no agent-session reload.
- **No ports.** Workspaces are identified by name. There is no port range, no per-project port allocation, no port conflicts.
- **Tool budget.** Each workspace contributes ~79 tools (fork v1.9.0) toward the agent's tool registration cap (Antigravity caps around 100). With the current tool surface that means ONE active workspace per Antigravity session; Cursor and Claude Code tolerate more.
- **Migration.** If you're upgrading from v0.10.3 or earlier, existing projects are auto-grouped into default workspaces named like `workspace-11100` (derived from the old `assignedPort`). Rename them through the Workspaces card or the workspace header.

---

## Dashboard

![Dashboard — top half: Workspaces card, Register Project, Managed Projects header](/help/dashboard-top.png)

![Dashboard — bottom half: Managed Projects rows + Selected Project Status strip](/help/dashboard-bottom.png)

*Workspaces card and Register Project on the left; grouped Managed Projects with the Agent deploy strip on the right; selected project status across the bottom.*

The Dashboard splits into three areas:
- **Left column** — the **Workspaces** card (pick / create / rename / delete) and the **Register Project / Import VSCode Workspace** forms below it.
- **Right column** — the **Managed Projects** view grouped by workspace, the **Agent deploy** toolbar, and the bulk-action bar that appears when you select projects.
- **Bottom strip** — full-width **Selected Project Status** for the row you most recently picked.

### Workspaces card (left)

Each row in the Workspaces card shows a workspace name, a colored **status lamp**, and the project count.

- **Status lamp colors** — slate (stopped), amber (starting / mixed), emerald (running), coral (failed). The color reflects the workspace's aggregate runtime phase, derived from its members.
- **Click** a row to make that workspace the **active** one — newly registered projects join it, and the Register Project / Import forms update their hint accordingly.
- **+ New workspace…** — inline-creates an empty workspace. It pins until either you add a project to it or you delete it.
- **Hover** a row to reveal the rename ✎ and delete ✕ icons. **Right-click** for a context menu with Rename / Delete.

### Register Project

1. **Name** — Required. Browsing for a folder fills this in from the folder's last segment (you can edit it).
2. **Project path** — The root directory of a Java/Maven/Gradle (or Eclipse PDE) project.
3. **Workspace** — Implicitly the active workspace from the left card. Pick a different one in the Workspaces card to switch.
4. **Save project** — Registers the project. The manager updates the workspace's `workspace.json` and any running GOJA picks up the new project immediately.

#### Recursive search (autoscan) — since v0.16.0

Tick the **Recursive search (autoscan)** checkbox under Project path to flip the form into discovery mode: **Browse** then scans the picked folder recursively (depth ≤ 6) for Maven/Gradle and Eclipse/PDE projects and unfolds the results right in the card — tick the ones you want, **Import selected**, and they all join the active workspace. The button reads **Discover** for hand-typed paths or rescans; the Name field is disabled (each discovered project keeps its folder name). Point it at a parent folder like `~/Projects` — no `.code-workspace` file needed.

### Import VSCode Workspace

Pick a `.code-workspace` file (**Browse**), then **Discover** to enumerate Maven/Gradle and Eclipse/PDE Java projects. Tick the rows you want and click **Import selected** — every imported project joins the currently active workspace.

### Managed Projects (grouped view)

The right pane shows one **workspace card** per workspace, with project rows nested inside. Each card has a header with the workspace name, status badge, project count, and per-workspace **Start workspace / Stop workspace / Delete workspace** actions. Click the chevron to collapse or expand the card.

Each project row inside a workspace card has:
- A **selection checkbox** on the left (see "Bulk actions" below).
- The **project name** (click to make it the *Selected project* shown in the bottom strip; click again to inline-rename).
- The **project path** below the name.
- **Refresh / Status badge / Start / Stop / Delete** on the right.
- **Right-click** for a context menu: Start project / Stop project / Rename project / Move to workspace… / Delete project.

At the very top of the pane, a metric strip shows totals: workspaces, running, stopped, projects.

### Bulk actions (multi-select)

Use the per-row checkboxes to build a **cross-workspace** selection set. Shift-click to extend a range; ctrl/cmd-click toggles a single row.

When at least one row is selected, a **bulk-action bar** appears above the workspace cards:
- **Move to workspace ▾** — move every selected project to a chosen (existing or new) workspace in one go.
- **Start selected** / **Stop selected** — fan the per-project start/stop out over the selection.
- **✕** — clear the selection.

### Drag-and-drop

Project rows are draggable. Grab any row and drop it on:
- A **workspace card header** in the right pane, or
- A **workspace row** in the left Workspaces list (handy when the destination card is collapsed or out of view).

If the row you grab is part of an active selection, the **whole selection** moves with it. Dragging an unselected row carries just that one row and leaves the selection intact. The source row dims and the drop target outlines while you drag; Esc cancels.

### Agent deploy

The **Agent deploy** strip contains **Deploy to Agents**, **Dry run**, **Regenerate**, and **Delete**. These actions do **not** start or stop GOJA — they rebuild MCP entries from your workspaces and read or write **MCP client config files** on disk (see Settings → MCP Config Locations).

- **Deploy to Agents** — Writes manager-owned MCP server entries (one per workspace, keyed `jl-<workspace-name>`) into the selected clients' configs, plus the rule blocks the manager maintains. Since v0.16.0 each client receives the entry shape its parser accepts: Antigravity gets `{ "serverUrl": …, "headers": … }`; Cursor / Claude Code / IntelliJ get `{ "type": "http", "url": …, "headers": … }`. Also since v0.16.0, workspace add / rename / delete automatically refresh clients you have already deployed to (never-deployed clients stay untouched), and any workspace that cannot be resolved at deploy time is reported in the result instead of being silently omitted.
- **Dry run** — Same validation and diff output as Deploy, but no files are written.
- **Regenerate** — Force-rewrites the manager-managed sections, even if nothing has changed since the last write. Useful after manual edits.
- **Delete** — Removes only the manager-injected MCP servers and rule blocks from the selected clients. It does not uninstall GOJA or remove your projects.

Each of these opens a **target picker**: check Cursor / Claude / Antigravity / IntelliJ for that run only. Defaults come from each client's **Deploy** toggle under Settings → MCP Config Locations.

**Cursor (length limit):** Cursor rejects tools when `serverName + ":" + toolName` exceeds about **59–60** characters. The manager keeps the generated `jl-` ids short so the longest GOJA tool names still fit. **Antigravity** instead caps the total *number* of MCP tools registered across all servers (around 100 in current builds) — that is a separate constraint, and the main reason to keep concurrent workspaces small.

### Tool surface (fork v1.9.0)

GOJA v1.9.0 registers **79 tools per workspace service** (66 in v1.4.0 → 55 after v1.5.0's parametric consolidation → 60 with v1.5.1's LTK refactorings → 62 with v1.6.0's verification tools → 73 with v1.7.0's Ring 2/3/4 expansion → 75 with v1.8.0 → 79 with v1.9.0's apply/undo primitives). Two parametric tools (`find_pattern_usages` / `find_quality_issue`) absorbed 13 narrow ones in v1.5.0 so multi-workspace setups have headroom under Antigravity's 100-tool cap.

- **`find_pattern_usages(kind, query)`** — type-anchored searches. `kind ∈ { annotation, instantiation, type_argument, cast, instanceof }`.
- **`find_quality_issue(kind, ...)`** — code-quality analyses. `kind ∈ { naming, bugs, unused, large_classes, circular_deps, reflection, throws, catches }`.

Each parametric tool's `kind` is a typed enum in the input schema with per-kind descriptions, so agents can discover what's available through `tools/list`. `find_method_references` and the position-anchored search tools stay as separate tools.

**Refactoring (since v1.5.1; auto-apply + undo since v1.9.0)** — five JDT-LTK structural refactorings: `move_class`, `move_package`, `pull_up`, `push_down`, `encapsulate_field`. They take a position (filePath / line / column, zero-based) plus refactoring-specific arguments. Since fork v1.9.0 EVERY refactoring tool applies its change directly and returns `{ filesModified, diff, undoChangeId }` — agents verify with `compile_workspace` and revert with `undo_refactoring` if needed; `auto_apply: false` stages a change for preview-then-commit. Detect tools carry MCP `readOnlyHint` annotations, so restricted client modes (e.g. Cursor Ask mode) can run analysis without write permissions. v1.5.2 closeout patch made `move_class` / `move_package` / `pull_up` / `push_down` work without a prior Eclipse session's `.metadata`; `encapsulate_field`'s happy-path is still pending an upstream JDT fix.

**Workspace verification (since v1.6.0)** — `compile_workspace` runs `IncrementalProjectBuilder` over every loaded project and aggregates `IMarker.PROBLEM` markers (compile errors, classpath errors, manifest errors) — same path Eclipse IDE's Problems view uses, catches cascading errors that per-file `get_diagnostics` misses. `run_tests` launches JUnit 4 / 5 / TestNG via JDT-LTK's launching delegate, headless, with method/class/package scope and parsed pass/fail/skip results.

**Code generation (since v1.7.0)** — six tools that bypass the small mistakes agents make hand-writing modifiers/generics/annotations. All built via `ASTRewrite` directly (no `org.eclipse.jdt.ui` dep): `generate_constructor`, `generate_getters_setters`, `generate_equals_hashcode`, `generate_tostring`, `override_methods` (query mode lists overridable signatures; generate mode emits `@Override` stubs), `generate_test_skeleton` (writes a JUnit class to the `src/test/java` mirror).

**Build & dependency management (since v1.7.0, Maven-only)** — `add_dependency` and `update_dependency` mutate `pom.xml` text-level (preserves user formatting + comments); `find_unused_dependencies` is read-only and heuristic (groupId-prefix or artifactId-substring match against source imports). Gradle/Buildship support is explicitly v1.8.x.

**Workflow polish (since v1.7.0)** — `format` (file/package/project/workspace scope, honours the project's own `.settings/org.eclipse.jdt.core.prefs`); `optimize_imports_workspace` (workspace fan-out of import optimisation, idempotent).

One v1.7.0 happy-path test ships `@Disabled` (`generate_test_skeleton` auto-detect path) due to the same Tycho-surefire fixture-build gap that has Sprint 12's `run_tests` happy-paths disabled. Production usage works against real workspaces; the test fixture's external Maven deps don't resolve onto JDT's classpath in the test runtime. v1.6.1 follow-up.

### Selected Project Status

When you click a project row, the bottom strip shows **Name**, **Project path**, **Workspace**, the **PID** of that workspace's GOJA process (if running), and the **Phase / Health** detail from the runtime. Multiple projects in the same workspace share a PID. Use the refresh icon on that strip to re-query without switching views.

---

## Settings

![Settings — GOJA Runtime and Exposed Services](/help/settings-top.png)

*Top half of the Settings page: GOJA Runtime and Exposed Services.*

![Settings — Machine controls and MCP locations](/help/settings-bottom.png)

*Bottom half: Machine Runtime Controls (with Diagnostics workspace counts) and MCP Config Locations.*

Settings is a **two-by-two grid**: GOJA Runtime and Exposed Services on the first row, Machine Runtime Controls and MCP Config Locations on the second. The page can be taller than the window — scroll to reach **Save settings** at the bottom.

### GOJA Runtime

Controls how the global GOJA binary is sourced and updated:

- **Release source** — `haraldwegner/goja-mcp` (recommended fork) or upstream / custom. Switching saves and downloads the latest release from the new source.
- **Global GOJA Source** — **Managed runtime** uses the binary the manager downloads and tracks; **Local JAR fallback** points at a specific `goja.jar` on disk.
- **Active** — Version of the managed runtime, when applicable.
- **Update policy** — *Ask before updating* vs *Always keep latest*.
- **Auto-check release source on dashboard load** — When enabled, the manager checks for newer releases when you open the Dashboard.
- **Download update** — Appears when an update is available; fetches and installs it.

### Exposed Services

**Test Services** runs a live MCP handshake against GOJA and lists the tool names and descriptions the server exposes (count and duration appear after a successful probe). Use this to confirm the runtime is reachable and that the tool surface matches expectations after a version change.

If a probe fails, fix connectivity or runtime issues before relying on **Deploy to Agents**.

### Machine Runtime Controls

- **Manager data root** — Base directory for caches, logs, and JDT workspace indexes. Each workspace's data lives under `<data_root>/workspaces/<workspace-name>/` (which is also where `workspace.json` is written).
- **Use system tray** — When enabled, closing the window keeps the manager running in the system tray. The tray menu (refined in v0.13.0; expanded in v0.14.0 / v0.14.1) lets you drive workspace lifecycle without opening the window:

  ![Tray menu — Open dashboard, per-workspace rows with monochrome status bullets, Start all / Reload all / Stop all services, Autostart on boot checkable, Quit](/help/tray-menu.png)

  - **Open dashboard** — raises the main manager window (its default view is the dashboard).
  - **Workspaces** — one row per workspace with a monochrome status bullet:
    - `●` running
    - `◐` starting / stopping
    - `○` stopped
    - `✗` failed
    Click a row to **toggle** that workspace: stopped/failed → start, running → stop. The bullet refreshes within ~1 s of any state change (rename in the dashboard, external `kill` of a goja process, manual start/stop in the main window).
  - **Start all services** / **Reload all services** *(v0.14.0)* / **Stop all services** — fan out across every loaded workspace. *Reload all* is a sequenced stop-then-start (30 s deadline) — single click for a clean restart that doesn't race the shutdown sequence.
  - **Autostart on boot** ✓ — checkable item (v0.14.0). When set, the manager auto-launches at session login. Synced with the **Settings → System Settings → Autostart on boot** checkbox in v0.14.1 — toggle from either surface and the other reflects within ~1 s.
  - **Quit** — opens the quit prompt.

  *Why monochrome bullets?* GNOME's `gnome-shell-extension-appindicator` strips per-menu-item images at the D-Bus boundary, so the colored status disks shipped in v0.12.0 never reached the user. Monochrome unicode shapes render in the menu's own font (1× line height) and survive the appindicator pipe across every Linux desktop we ship to.

  *Linux note:* the tray relies on a StatusNotifierItem / AppIndicator host. Pop!_OS, Ubuntu 22.04+, KDE / XFCE / Cinnamon / MATE work out of the box; vanilla GNOME (Fedora Workstation, Debian GNOME) needs `gnome-shell-extension-appindicator` installed once. See the [README](https://github.com/haraldwegner/goja-studio#system-tray-on-linux) for distro-specific install commands.
- **Autostart on boot** *(v0.14.0; expanded in v0.14.1)* — Start the manager automatically at session login AND restore the workspaces that were running at last shutdown. Per-OS plumbing for the manager launch: Linux writes `~/.config/autostart/*.desktop`, macOS registers a LaunchAgent, Windows touches the registry Run key. Default is opt-in (off). Mirrored in the tray menu as a checkable item — toggling from either surface updates the other (v0.14.1 added the event-driven sync). **Session restoration semantics:** if you Quit from the tray (or close-to-tray then Quit) the running workspaces stay marked Running in the manager's snapshot, and the next launch restores them ~2 s after the UI is up. If you choose **Stop and Quit**, every workspace is cleanly stopped — next launch starts none. Workspaces that were `Failed` at shutdown count as "user wanted this running" and get retried.
- **Diagnostics** — Read-only summary: paths for the projects store, settings file, state directory, and resolved data root. **Workspaces** and **Project count** mirror the Dashboard totals, useful when reporting issues.
- **Clean logs** — Removes manager runtime logs (workspaces and settings stay).
- **Clean workspaces** — Removes JDT workspace caches (forces re-index next start).
- **Start from scratch** — Runs both cleanups; stop runtimes first.

### MCP Config Locations

For each supported client (**Cursor**, **Claude**, **Antigravity**, **IntelliJ**):

- **Deploy** — When checked, the client is included in the *default* set of the deploy target picker. Override per run if you need to.
- **Current** — Effective path the manager will use (auto-detected, or your manual override).
- **Manual override path** — Use when the config file lives somewhere non-standard.

**Redetect defaults** re-runs auto-detection. **Antigravity (Google / Gemini):** the manager looks in several common locations including `~/.gemini/antigravity/mcp_config.json`. Antigravity caps total registered MCP tools (≈100), so keep concurrent workspaces small.

**Merge mode**:
- **Safe merge** — inserts or updates only the manager-owned blocks, preserving unrelated entries.
- **Replace managed section** — replaces the entire manager-delimited section. Stronger reset, still scoped to what the manager owns.

**Create backup before MCP config write** writes a timestamped backup next to each config before changes. Recommended while you're experimenting.

---

## Quick reference

| Goal | Where to go |
|------|------------------|
| Create or rename a workspace | Dashboard → Workspaces card (left) |
| Register or import projects | Dashboard left column |
| Move a project to another workspace | Right-click row → *Move to workspace…* OR drag the row onto a workspace |
| Bulk-move projects | Tick checkboxes → *Move to workspace ▾* |
| Start/stop a workspace's GOJA | Workspace header in Managed Projects |
| Push MCP entries into Cursor / Claude / etc. | Dashboard → **Agent deploy** |
| Change data root or system-tray behavior | Settings → **Machine Runtime Controls** |
| Point deploy at custom MCP config paths | Settings → **MCP Config Locations** |
| Verify GOJA exposes MCP tools | Settings → **Exposed Services** → **Test Services** |
| Find logs / settings files for a bug report | Settings → **Diagnostics** |

If something fails: check Diagnostics for paths, run **Dry run** before **Deploy**, and keep **Create backup before MCP config write** on until you trust your layout.
