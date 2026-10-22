# jawata-studio Help

**jawata-studio** runs one JAWATA service per **workspace** (a named group of Java projects) and writes the connection into your AI agents (Cursor, Claude Code, Codex, GitHub Copilot CLI, VS Code, Grok), so they get compiler-accurate understanding of your code.

Every change an agent makes through jawata comes back with an undo handle: ask it to run `refactoring(action=undo, undoChangeId=…)` to put the code back.

Hover any control for what it does.

## Build a workspace

This is the job the studio is for. Five steps, all on the Dashboard:

1. **Create the workspace.** In **Workspaces** (left), click **+ New workspace…**, type a name, press Enter. Click a workspace to make it the one new projects join; it is highlighted.
2. **Add its projects.** Open **Add projects** (left).
   - **From a folder:** type or **Browse** to a Java project's folder, or a folder that holds several. The search starts at once (Enter also starts it) and lists every Maven, Gradle or Eclipse project under it, the folder itself included. Untick what you do not want, then **Import selected**.
   - **A plain source folder** with no Maven, Gradle or Eclipse file is not found by the search. Then **Add this folder as a project** adds it as it is.
   - **From a `.code-workspace` file:** **Browse** to the file, **Discover**, then **Import selected**.
3. **Check what landed.** In **Managed Projects** (right), open the workspace's card: each project shows its name and its folder. Drag a project onto another workspace on the left to move it. Right-click a project to start, stop, rename, move or delete it; right-click a workspace to start, stop, rename or delete it.
4. **Start it.** **Start workspace** on its card, or **Start all**. The badge turns **RUNNING** once the service has loaded every project; hover it for the detail.
5. **Connect your agents.** **Deploy to agents** writes the jawata entries into the agents' config files. Restart or reload an agent to pick them up. **Remove from agents** takes exactly those entries out again.

**Deleting.** Every delete asks first and names what it removes; your source files on disk are never deleted. A running or starting workspace cannot be deleted, and neither can one of its projects: stop the workspace first. **Delete all workspaces** is at the bottom of the Workspaces panel.

## Dashboard

Everything above. **Reload all** stops every service and starts it again.

## Memory

The knowledge store behind what your agents recall. **Load** reads your memory files into it, **Back up now** takes a copy, **Clean up** drops aged discarded entries and merges duplicates; **More** holds Reload status, Export, Import, Restore, Curation and Wipe. **Status** says what the last action did. **Memory sources and store settings** sets which folders Load reads and how many backups are kept.

## Reporting

What jawata recorded about itself on this machine: shapes only (tool, error code, counts), never paths or text. The sentence on top says whether anything needs a look; **Details** lists the failures behind it. Type `/report` in any agent to file one from your own GitHub account.

## Seats

A seat is a narrow role an agent takes when you type its command, such as `/refactor` or `/debug`. The switch beside each takes its command out of your agents. Open a row for when it is meant to be used, which agent program and model a hire of it starts, which of your agents get its command, and **Reset to shipped**.

## Settings

**Engine**: where the jawata engine comes from and whether it updates itself; **Test services** checks that it answers. **Agents**: which agents Deploy writes to, and each one's config file (under **Advanced**: merge mode and a backup before each write):

| Agent | Config file |
|---|---|
| Cursor | `~/.cursor/mcp.json` |
| Claude Code | `~/.claude.json` |
| Codex | `~/.codex/config.toml` |
| Copilot CLI | `~/.copilot/mcp-config.json` |
| VS Code | `<config dir>/Code/User/mcp.json` |
| Grok | `~/.grok/config.toml` |

`<config dir>` is `~/.config` on Linux, `~/Library/Application Support` on macOS and `%APPDATA%` on Windows. Grok receives jawata's tools but not its guard. Antigravity cannot connect jawata and is not supported. **This machine**: the data folder, tray, autostart, per-workspace memory limits, and cleanup.

## Installation and updates

- **Linux**: `curl -sSL https://raw.githubusercontent.com/haraldwegner/jawata-studio/main/install.sh | bash`, or a `.deb` from the Releases page.
- **macOS** (Apple Silicon): the `_aarch64.dmg`; clear the quarantine once with `xattr -d com.apple.quarantine /Applications/jawata-studio.app`.
- **Windows** (x64 and ARM64): the `.msi` or `-setup.exe`; the first launch needs SmartScreen → More info → Run anyway.

The tray needs a StatusNotifierItem host on Linux; vanilla GNOME needs `gnome-shell-extension-appindicator` once. The tray menu shows the studio and engine versions above Quit. Per-platform detail is in the [README](https://github.com/haraldwegner/jawata-studio#installation).

## If something fails

Settings → This machine shows where the settings and logs are. **Remove from agents** and **Deploy to agents** again rewrite the entries. Keep **backup before MCP config write** on until you trust your layout.
