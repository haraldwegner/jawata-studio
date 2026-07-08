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

**Windows** (x64 and ARM64) — download the `.msi` or `-setup.exe`; unsigned, so the first launch needs a one-time SmartScreen **More info → Run anyway**.

Per-platform detail and the bypass steps are in the [README](https://github.com/haraldwegner/goja-studio#installation).

---

## Workspaces — the core concept

A **workspace** is a named group of Java projects loaded into one GOJA process and exposed to agents as **one MCP service** (`goja-<workspace-name>`). The agent sees the combined symbol set of every project in the workspace; cross-project navigation, find-references, and refactorings work across the whole group.

- **One workspace per cohesive task.** A bundle/multi-module application (e.g. an Eclipse RCP product with 12 OSGi bundles), a monorepo, or a single project that you want isolated — each gets its own workspace.
- **Live updates.** Add or remove a project from a workspace and the running GOJA picks it up within ~1 second through a `workspace.json` file watcher. No MCP-client restart, no agent-session reload.
- **No ports.** Workspaces are identified by name. There is no port range, no per-project port allocation, no port conflicts.
- **Tool budget.** Each workspace contributes ~40 tools toward the agent's tool registration cap (Antigravity caps around 100), so roughly two active workspaces fit per Antigravity session; Cursor and Claude Code tolerate more.

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

#### Recursive search (autoscan)

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

- **Deploy to Agents** — Writes manager-owned MCP server entries (one per workspace, keyed `goja-<workspace-name>`) into the selected clients' configs, plus the rule blocks the manager maintains. Each client receives the entry shape its parser accepts: Antigravity gets `{ "serverUrl": …, "headers": … }`; Cursor / Claude Code / IntelliJ get `{ "type": "http", "url": …, "headers": … }`. Workspace add / rename / delete automatically refresh clients you have already deployed to (never-deployed clients stay untouched), and any workspace that cannot be resolved at deploy time is reported in the result instead of being silently omitted.
- **Dry run** — Same validation and diff output as Deploy, but no files are written.
- **Regenerate** — Force-rewrites the manager-managed sections, even if nothing has changed since the last write. Useful after manual edits.
- **Delete** — Removes only the manager-injected MCP servers and rule blocks from the selected clients. It does not uninstall GOJA or remove your projects.

Each of these opens a **target picker**: check Cursor / Claude / Antigravity / IntelliJ for that run only. Defaults come from each client's **Deploy** toggle under Settings → MCP Config Locations.

**Cursor (length limit):** Cursor rejects tools when `serverName + ":" + toolName` exceeds about **59–60** characters. The manager keeps the generated `goja-` ids short so the longest GOJA tool names still fit. **Antigravity** instead caps the total *number* of MCP tools registered across all servers (around 100 in current builds) — that is a separate constraint, and the main reason to keep concurrent workspaces small.

### How GOJA works — a compiler-grounded loop, not a bag of tools

GOJA is one **Java vertical over your whole workspace**, not a per-project add-on sitting beside ten per-language shims. Its ~40 tools are **parametric front doors** — a `kind`/`action` parameter folds what would otherwise be a hundred narrow tools into a handful, which keeps a multi-workspace setup under Antigravity's ~100-tool cap. But the point isn't the list; it's the **loop the tools compose into**:

- **Detect** — `find_quality_issue` / `find_modernization` surface Fowler smells, SOLID and Kerievsky violations, and modernization candidates — all compiler-resolved, so no regex false positives.
- **Goal** — `refactor_to_pattern` names the target state: a design pattern to move *toward* (state / command / template method / visitor / compose method) or *away from* (inline singleton).
- **How** — `refactoring(action=plan → apply_plan)` executes that goal as **atomic, parity-gated steps**. It compiles to zero errors after every step and runs a **purity check** — a step that smuggles in a new return/throw/branch or a relocated side-effect is *flagged, not silently applied* — and on any red it **rolls back** to the last good state. This is the machinery that makes autonomous refactoring risk-free: a fallible agent cannot leave your tree half-migrated.

Two supporting ideas ship with that loop:

- **Reuse over reinvent.** When an agent needs a near-duplicate class, letting the **compiler** derive it — `generate(kind=copy_class)` then `extract(kind=superclass)` — is cheaper and safer than the model re-authoring the code by hand. The tools exist so the agent writes *less* code, not more.
- **Health-gated honesty.** The deployed rule block (and, on Claude Code, a `PreToolUse` hook) instructs the agent: on Java work, when GOJA is unreachable, **ask — don't silently fall back to grep**. When the service is up, text search over `.java` is redirected to GOJA's compiler-accurate tools. Non-Java work is untouched.

Every mutating tool applies its change directly and returns `{ filesModified, diff, undoChangeId }` — verify with `compile_workspace`, revert with `undo_refactoring`, or pass `auto_apply: false` to stage a preview-then-commit. Detect tools carry MCP `readOnlyHint`, so a restricted client mode (e.g. Cursor Ask) can analyze without write permission.

### The tool surface (live — not enumerated here)

The authoritative, always-current list of tools and their descriptions is the running service itself. Open **Settings → Exposed Services → Test Services**: it performs a live MCP handshake and lists every tool name and description the service exposes, with a count and probe duration. Agents discover the same surface through `tools/list`, where each parametric tool's `kind`/`action` is a typed enum with per-kind descriptions. A hand-maintained copy in this help would only drift out of date after each release — so it is deliberately not kept here.

### Selected Project Status

When you click a project row, the bottom strip shows **Name**, **Project path**, **Workspace**, the **PID** of that workspace's GOJA process (if running), and the **Phase / Health** detail from the runtime. Multiple projects in the same workspace share a PID. Use the refresh icon on that strip to re-query without switching views.

---

## Memory

![Memory — Memory sources and Store & Maintenance panels, results under the actions](/help/memory.png)

*Memory sources on the left; the store with its five actions and inline results on the right.*

Your GOJA memory store — the knowledge behind the push channel that primes and steers
agents. One user-level database by default, shared by every workspace, living at
`~/.local/share/goja/`. The view has two panels:

- **Memory sources** — where **Load** finds your memory files. The usual locations are
  auto-discovered: your layered `CLAUDE.md` files, every Claude project memory folder,
  Cursor rules (`.cursor/rules`, `.cursorrules`), `AGENTS.md`, Copilot instructions.
  Add extra root folders only for anything outside those conventions. **Store mode**
  chooses between the shared user-level store (default — your knowledge is recallable
  from every workspace) and a per-workspace store. **Auto-seed on deploy** loads your
  memory into fresh residents automatically after every deploy.
- **Store & Maintenance** — the store, its size and entry count, and the actions:
  **Load** (pick up new and changed memory files — unchanged files are skipped, so
  re-loading is always safe), **Clean up** (one hygiene pass: drop aged discarded
  entries, merge duplicates, shrink the file), **Export…/Import…** (portable JSON via
  the file dialogs), and **Wipe** (delete all entries — the database itself stays; Load
  re-fills it). Results appear right below the actions, with the raw response one click
  away.

Everything else — re-checking stale Java references, curating candidate entries,
backups — happens automatically or by prompt: every action here can also be asked for
in plain words in your agent session ("load my memory files", "wipe the store"). Hover
any control for its prompt phrase.

### Memory from Cursor (and other clients)

This store is your **cross-client memory**: the same entries answer in Cursor, Claude
Code, and any MCP client. What differs is delivery — Claude Code *pushes* memory into
sessions automatically (session primer, recall before refactor tools); every other
client must be **asked**. Three phrases cover it:

- *"What do we know about `freeSlot`?"* — the agent runs a symbol/symptom **recall**;
  a match returns the closed set of known lessons for exactly that code.
- *"Record this as a lesson anchored to `pipeline.SlotManager`."* — the agent writes a
  **record** entry; it's recallable by symbol from every client afterwards.
- *"Prime yourself from the memory store."* — at the start of a session, the agent
  pulls the domain **primer** Claude Code would have received automatically.

And you don't need an agent to write memory at all: **any markdown file is memory —
and you probably already have a corpus.** Sprint docs, postmortems, ADRs, design
notes: add your `docs/` folder once under **Memory sources**, hit **Load**, and every
`.md` in it becomes agent knowledge — new and changed files on every Load, unchanged
ones skipped. The loader does the heavy lifting: each section becomes one recallable
fact, headings/bold/backticked terms become search cues, and a document whose text
names its code (`` `SlotManager.freeSlot` ``) is anchored to that symbol
automatically, no frontmatter needed — the next agent that touches `freeSlot` can be
handed the postmortem that was written about it.

One warning: if a Cursor agent says "I'll remember that" **without** calling GOJA, it
is remembering into Cursor's own chat memory — opaque, not listable, not exportable,
and invisible to every other tool. Durable memory is the one you can see in this view.

---

## Settings

![Settings — GOJA Runtime and Exposed Services](/help/settings-top.png)

*Top half of the Settings page: GOJA Runtime and Exposed Services.*

![Settings — Machine controls and MCP locations](/help/settings-bottom.png)

*Bottom half: Machine Runtime Controls (with Diagnostics workspace counts) and MCP Config Locations.*

Settings is a **two-by-two grid**: GOJA Runtime and Exposed Services on the first row, Machine Runtime Controls and MCP Config Locations on the second. The page can be taller than the window — scroll to reach **Save settings** at the bottom.

### GOJA Runtime

Controls how the global GOJA binary is sourced and updated:

- **Release source** — `haraldwegner/goja-mcp` (default) or upstream / custom. Switching saves and downloads the latest release from the new source.
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
- **Use system tray** — When enabled, closing the window keeps the manager running in the system tray. The tray menu lets you drive workspace lifecycle without opening the window:

  ![Tray menu — Open dashboard, per-workspace rows with monochrome status bullets, Start all / Reload all / Stop all services, Autostart on boot checkable, Quit](/help/tray-menu.png)

  - **Open dashboard** — raises the main manager window (its default view is the dashboard).
  - **Workspaces** — one row per workspace with a monochrome status bullet:
    - `●` running
    - `◐` starting / stopping
    - `○` stopped
    - `✗` failed
    Click a row to **toggle** that workspace: stopped/failed → start, running → stop. The bullet refreshes within ~1 s of any state change (rename in the dashboard, external `kill` of a goja process, manual start/stop in the main window).
  - **Start all services** / **Reload all services** / **Stop all services** — fan out across every loaded workspace. *Reload all* is a sequenced stop-then-start (30 s deadline) — single click for a clean restart that doesn't race the shutdown sequence.
  - **Autostart on boot** ✓ — checkable item. When set, the manager auto-launches at session login. Synced with the **Settings → System Settings → Autostart on boot** checkbox — toggle from either surface and the other reflects within ~1 s.
  - **Quit** — opens the quit prompt.

  *Why monochrome bullets?* GNOME's `gnome-shell-extension-appindicator` strips per-menu-item images at the D-Bus boundary, so colored status disks never reach the user. Monochrome unicode shapes render in the menu's own font (1× line height) and survive the appindicator pipe across every Linux desktop we ship to.

  *Linux note:* the tray relies on a StatusNotifierItem / AppIndicator host. Pop!_OS, Ubuntu 22.04+, KDE / XFCE / Cinnamon / MATE work out of the box; vanilla GNOME (Fedora Workstation, Debian GNOME) needs `gnome-shell-extension-appindicator` installed once. See the [README](https://github.com/haraldwegner/goja-studio#system-tray-on-linux) for distro-specific install commands.
- **Autostart on boot** — Start the manager automatically at session login AND restore the workspaces that were running at last shutdown. Per-OS plumbing for the manager launch: Linux writes `~/.config/autostart/*.desktop`, macOS registers a LaunchAgent, Windows touches the registry Run key. Default is opt-in (off). Mirrored in the tray menu as a checkable item — toggling from either surface updates the other. **Session restoration semantics:** if you Quit from the tray (or close-to-tray then Quit) the running workspaces stay marked Running in the manager's snapshot, and the next launch restores them ~2 s after the UI is up. If you choose **Stop and Quit**, every workspace is cleanly stopped — next launch starts none. Workspaces that were `Failed` at shutdown count as "user wanted this running" and get retried.
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
