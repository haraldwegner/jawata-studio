# jawata-studio Help

**jawata-studio** runs one JAWATA service per **workspace** (a named group of Java projects) and writes the connection into your AI agents (Cursor, Claude Code, Codex, GitHub Copilot CLI, VS Code, Grok), so they get compiler-accurate understanding of your code.

Every change an agent makes through jawata comes back with an undo handle: ask it to run `refactoring(action=undo, undoChangeId=…)` to put the code back.

Hover any control for what it does. Each tab below says what it is for and what you do there.

## Dashboard

Where you set up and run workspaces.

- **Workspaces (left)** — pick the workspace new projects join; create, rename or delete one.
- **Add projects (left)** — pick a folder: the Java project in it, or every project under it, is listed and you import the ones you tick. The search knows a project by its Maven, Gradle or Eclipse file; when it finds none, **Add this folder as a project** adds the folder itself. Or import the projects of a `.code-workspace` file. It is open while you have no project and folded afterwards.
- **Managed Projects (right)** — one card per workspace. Start or stop a workspace or a single project; open a card to see its projects. Right-click a project or a workspace for rename, move and delete. Drag projects between workspaces. The status badge shows the phase; hover it for the detail and the process id.
- **Deploy to agents** — writes the jawata entries into the config files of the agents you tick. **Remove from agents** takes exactly those entries out again. Neither starts or stops anything. Which agents are offered is set in Settings → Agents.

## Memory

The knowledge store behind the recall your agents receive. One shared store by default.

- **Summary line and buttons** — **Load** reads your memory files into the store (safe to repeat), **Back up now** takes a copy, **Clean up** drops aged discarded entries, merges duplicates and shrinks the file. **More** holds Export, Import, Restore, Curation and Wipe.
- **Memory sources and store settings** — the folders **Load** reads (instruction files such as `CLAUDE.md`, rules files and `AGENTS.md` are found automatically; add your own `docs/` folders here), the store mode (one shared store or one per workspace), auto-seed on deploy, how many backup copies of the store are kept, and the folder the engine writes accepted entries to as readable markdown files.
- **What the store holds** — counts per kind. Hover a count for what that kind is.
- **What the cataloguer wrote** — a sample of the descriptions of your code, to judge their quality.
- **Curation** (under **More**) — lists the **Rules**, where you can retire one, and the **Entries**, where you can accept a candidate or promote ticked entries into a rule.

## Reporting

The page is titled **Field recording**. It shows what jawata recorded about itself on this machine: shapes only (tool, error code, counts, timing), never paths or message text. The sentence at the top says whether anything needs a look; open **Details** for the figures behind it. Type `/report` in any agent to file a bug report from your own GitHub account. The switch there stops the periodic failure reminder.

## Seats

A seat is a narrow engineering role an agent can take, typed as a command such as `/refactor` or `/debug`.

- **You type these** — each has an On switch (off removes its command from your agents) and a one-line purpose. Open a row for when it is meant to be used, which agent program and model a hire of it starts, which of your agents get its command, and **Reset to shipped**.
- **Another seat hires these** and **Inside /sprint** — no command of their own; you choose only their brain.

## Settings

- **Engine** — where the jawata engine comes from and whether it updates itself. **Test services** checks that it answers.
- **Agents** — which agents deploy writes to, and the config file of each (change it only if it is not in the usual place). Under **Advanced**: merge mode and a backup before each write.
- **This machine** — the manager's data folder, tray, autostart, and per workspace a memory limit and an opt-in debugging switch. Paths and cleanup buttons are folded.

| Agent | Config file |
|---|---|
| Cursor | `~/.cursor/mcp.json` |
| Claude Code | `~/.claude.json` |
| Codex | `~/.codex/config.toml` |
| Copilot CLI | `~/.copilot/mcp-config.json` |
| VS Code | `<config dir>/Code/User/mcp.json` |
| Grok | `~/.grok/config.toml` |

`<config dir>` is `~/.config` on Linux, `~/Library/Application Support` on macOS and `%APPDATA%` on Windows. Grok receives jawata's tools but not its guard. Antigravity cannot connect jawata and is not supported.

## Installation and updates

- **Linux** — `curl -sSL https://raw.githubusercontent.com/haraldwegner/jawata-studio/main/install.sh | bash`, or a `.deb` from the Releases page.
- **macOS** (Apple Silicon) — the `_aarch64.dmg`; clear the quarantine once with `xattr -d com.apple.quarantine /Applications/jawata-studio.app`.
- **Windows** (x64 and ARM64) — the `.msi` or `-setup.exe`; the first launch needs SmartScreen → More info → Run anyway.

The tray needs a StatusNotifierItem host on Linux; vanilla GNOME needs `gnome-shell-extension-appindicator` once. Per-platform detail is in the [README](https://github.com/haraldwegner/jawata-studio#installation).

## If something fails

Settings → This machine → Paths shows where the settings and logs are. Remove from agents and Deploy again to rewrite the entries. Keep **backup before MCP config write** on until you trust your layout.
