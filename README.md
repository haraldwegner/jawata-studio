# JAWATA Studio

[![GitHub release](https://img.shields.io/github/v/release/haraldwegner/jawata-studio)](https://github.com/haraldwegner/jawata-studio/releases)
[![License: AGPL v3](https://img.shields.io/badge/License-AGPL%20v3-blue.svg)](LICENSE)

**The desktop control plane for [JAWATA](https://github.com/haraldwegner/jawata-mcp)** — surgical,
risk-free Java refactoring for autonomous agents. jawata-studio downloads the JAWATA engine, manages
named workspaces of Java projects, runs them efficiently, and wires JAWATA into your AI agent's MCP
config in one click.

A Tauri desktop app for Linux, macOS, and Windows. You point it at your Java projects; it does the
rest.

```bash
# Linux — one line installs the app and a desktop entry
curl -sSL https://raw.githubusercontent.com/haraldwegner/jawata-studio/main/install.sh | bash
```

---

## Install

| Platform | How |
|---|---|
| **Linux** (x86_64 / aarch64) | `curl -sSL https://raw.githubusercontent.com/haraldwegner/jawata-studio/main/install.sh \| bash` — downloads the matching `.AppImage` and registers a desktop entry. Or grab the `.AppImage` / `.deb` from [releases](https://github.com/haraldwegner/jawata-studio/releases/latest). |
| **macOS** (Apple Silicon) | Download the `.dmg` from [releases](https://github.com/haraldwegner/jawata-studio/releases/latest) and drag to Applications. Unsigned: right-click → **Open** once to clear Gatekeeper. |
| **Windows** (x64 / ARM64) | Download the `.msi` or `_x64-setup.exe` from [releases](https://github.com/haraldwegner/jawata-studio/releases/latest). Unsigned: on the SmartScreen prompt choose **More info → Run anyway**. |

**Prerequisite:** **Java 21+** on the `PATH` — the JAWATA engine it runs is a JVM process.

The installer pulls the right artifact for your CPU automatically. To update, re-run the one-liner
(Linux) or install the newer package; jawata-studio also self-updates the JAWATA engine from its
[releases](https://github.com/haraldwegner/jawata-mcp/releases).

---

## What it does

- **Workspace-first.** Group Java projects into named workspaces. Each workspace loads into one
  JAWATA process and is exposed as a single MCP service your agent can call. Add a project by browsing
  to it, or point autoscan at a parent folder (e.g. `~/Projects`) to import a whole tree.
- **One JVM per workspace, not per client.** jawata-studio runs a single resident JAWATA engine per
  workspace and writes a URL endpoint into each client config. Three agents and a workspace are
  **one** JVM, not one per agent — memory stays flat no matter how many clients connect.
- **One-click MCP deploy.** Writes the correct server entry into Cursor, Claude Desktop,
  Antigravity, and IntelliJ-style configs — each in that client's own schema — and re-syncs them
  when a workspace changes.
- **Makes the agent actually use JAWATA.** Deploy writes a trigger→tool guide (a "prefer JAWATA over
  grep" rule with an intent→tool table) into each client and — on Claude Code — a `PreToolUse` hook
  that **enforces try-first**: a `grep` over `.java`, or a hand-edit of an existing `.java` file, is
  blocked with a redirect to the right JAWATA tool — unless you already looked it up via JAWATA this
  session, or you declare `jawata-fallback: <why>` (logged, versioned). Advertising the tools wasn't
  enough; the hook makes *not* using them the inconvenient path. Health-gated (engine down → **ask,
  don't silently degrade**); non-Java work is left untouched.
- **Learns from what happens next.** A companion `PostToolUse` observer (never blocks) records
  declared fallbacks, ungrounded `.java` reads, and compile/test outcomes into a versioned log — the
  signal for sharpening the guidance over time, and JAWATA's own feature backlog.
- **Auto-managed engine.** Polls for new JAWATA releases and downloads the matching runtime; the jar
  path stays stable across updates.
- **Lives in the tray.** A system-tray menu drives per-workspace start/stop without opening the
  window. Optional autostart-on-boot restores the workspaces you had running.
- **One memory, every client.** The JAWATA knowledge store is your agent's durable, cross-client
  memory — markdown files in your git plus a symbol-anchored store, written and recalled from
  Cursor, Claude Code, or any MCP client. In Claude Code it's pushed into sessions automatically
  (primer + recall hooks); in Cursor, ask: *"what do we know about `freeSlot`?"* (recall),
  *"record this as a lesson anchored to `pipeline.SlotManager`"* (record), *"prime yourself from
  the memory store"* (session primer). Or skip the agent entirely: any markdown is memory — and
  your `docs/` folder is probably already a corpus. Add it under Memory sources and Load turns
  sprint docs, postmortems and ADRs into agent knowledge: one fact per section, search cues from
  the headings, auto-anchored to the code the text names — the next agent touching `freeSlot` gets
  handed the postmortem written about it. If an agent says "I'll remember that" without calling
  JAWATA, it's remembering
  into an opaque chat memory no other tool can see — durable memory is the one in the studio's
  Memory view: listable, promotable, exportable, yours.

---

## How it works

jawata-studio is a pure orchestrator — a Tauri (Rust + web UI) app. It never analyses Java itself;
that is entirely the JAWATA engine's job. The manager:

1. Resolves and downloads the platform-matched **[jawata-mcp](https://github.com/haraldwegner/jawata-mcp)**
   engine.
2. For each workspace, writes `<data-dir>/workspace.json` (the project list the engine watches) and
   launches one resident engine on an allocated `(port, token)`.
3. Deploys the resulting MCP endpoint into your chosen clients' configs and keeps them in sync.

State lives under your platform's standard config / state / cache directories for `jawata-studio`.

### System tray on Linux

The tray icon uses AppIndicator. On GNOME, install the
[AppIndicator extension](https://extensions.gnome.org/extension/615/appindicator-support/) so the
icon and its menu appear; KDE and most other desktops show it out of the box. Left-click opens the
menu (status glyphs: ● running · ◐ starting · ○ stopped · ✗ failed).

---

## Building from source

```bash
git clone https://github.com/haraldwegner/jawata-studio.git
cd jawata-studio
npm install
npm run tauri build          # produces installers under src-tauri/target/release/bundle
```

Requires the [Tauri prerequisites](https://tauri.app/start/prerequisites/) (Rust + your platform's
webview/build deps) and Node.js. `npm run tauri dev` runs the app against a dev build.

---

## License

**[AGPL-3.0](LICENSE)** — see also [`NOTICE`](NOTICE). jawata-studio is the desktop control plane for
[JAWATA](https://github.com/haraldwegner/jawata-mcp). Contributions are accepted under the
[Contributor License Agreement](CLA.md); see [`CONTRIBUTING.md`](CONTRIBUTING.md).
