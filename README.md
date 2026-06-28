# GOJA Studio

[![GitHub release](https://img.shields.io/github/v/release/haraldwegner/goja-studio)](https://github.com/haraldwegner/goja-studio/releases)
[![License: AGPL v3](https://img.shields.io/badge/License-AGPL%20v3-blue.svg)](LICENSE)

**The desktop control plane for [GOJA](https://github.com/haraldwegner/goja-mcp)** — compiler-accurate
Java intelligence for AI agents. goja-studio downloads the GOJA engine, manages named workspaces of
Java projects, runs them efficiently, and wires GOJA into your AI agent's MCP config in one click.

A Tauri desktop app for Linux, macOS, and Windows. You point it at your Java projects; it does the
rest.

```bash
# Linux — one line installs the app and a desktop entry
curl -sSL https://raw.githubusercontent.com/haraldwegner/goja-studio/main/install.sh | bash
```

---

## Install

| Platform | How |
|---|---|
| **Linux** (x86_64 / aarch64) | `curl -sSL https://raw.githubusercontent.com/haraldwegner/goja-studio/main/install.sh \| bash` — downloads the matching `.AppImage` and registers a desktop entry. Or grab the `.AppImage` / `.deb` from [releases](https://github.com/haraldwegner/goja-studio/releases/latest). |
| **macOS** (Apple Silicon) | Download the `.dmg` from [releases](https://github.com/haraldwegner/goja-studio/releases/latest) and drag to Applications. Unsigned: right-click → **Open** once to clear Gatekeeper. |
| **Windows** (x64 / ARM64) | Download the `.msi` or `_x64-setup.exe` from [releases](https://github.com/haraldwegner/goja-studio/releases/latest). Unsigned: on the SmartScreen prompt choose **More info → Run anyway**. |

**Prerequisite:** **Java 21+** on the `PATH` — the GOJA engine it runs is a JVM process.

The installer pulls the right artifact for your CPU automatically. To update, re-run the one-liner
(Linux) or install the newer package; goja-studio also self-updates the GOJA engine from its
[releases](https://github.com/haraldwegner/goja-mcp/releases).

---

## What it does

- **Workspace-first.** Group Java projects into named workspaces. Each workspace loads into one
  GOJA process and is exposed as a single MCP service your agent can call. Add a project by browsing
  to it, or point autoscan at a parent folder (e.g. `~/Projects`) to import a whole tree.
- **One JVM per workspace, not per client.** goja-studio runs a single resident GOJA engine per
  workspace and writes a URL endpoint into each client config. Three agents and a workspace are
  **one** JVM, not one per agent — memory stays flat no matter how many clients connect.
- **One-click MCP deploy.** Writes the correct server entry into Cursor, Claude Desktop,
  Antigravity, and IntelliJ-style configs — each in that client's own schema — and re-syncs them
  when a workspace changes.
- **Auto-managed engine.** Polls for new GOJA releases and downloads the matching runtime; the jar
  path stays stable across updates.
- **Lives in the tray.** A system-tray menu drives per-workspace start/stop without opening the
  window. Optional autostart-on-boot restores the workspaces you had running.

---

## How it works

goja-studio is a pure orchestrator — a Tauri (Rust + web UI) app. It never analyses Java itself;
that is entirely the GOJA engine's job. The manager:

1. Resolves and downloads the platform-matched **[goja-mcp](https://github.com/haraldwegner/goja-mcp)**
   engine.
2. For each workspace, writes `<data-dir>/workspace.json` (the project list the engine watches) and
   launches one resident engine on an allocated `(port, token)`.
3. Deploys the resulting MCP endpoint into your chosen clients' configs and keeps them in sync.

State lives under your platform's standard config / state / cache directories for `goja-studio`.

### System tray on Linux

The tray icon uses AppIndicator. On GNOME, install the
[AppIndicator extension](https://extensions.gnome.org/extension/615/appindicator-support/) so the
icon and its menu appear; KDE and most other desktops show it out of the box. Left-click opens the
menu (status glyphs: ● running · ◐ starting · ○ stopped · ✗ failed).

---

## Building from source

```bash
git clone https://github.com/haraldwegner/goja-studio.git
cd goja-studio
npm install
npm run tauri build          # produces installers under src-tauri/target/release/bundle
```

Requires the [Tauri prerequisites](https://tauri.app/start/prerequisites/) (Rust + your platform's
webview/build deps) and Node.js. `npm run tauri dev` runs the app against a dev build.

---

## License

**[AGPL-3.0](LICENSE)** — see also [`NOTICE`](NOTICE). goja-studio is the desktop control plane for
[GOJA](https://github.com/haraldwegner/goja-mcp). Contributions are accepted under the
[Contributor License Agreement](CLA.md); see [`CONTRIBUTING.md`](CONTRIBUTING.md).
