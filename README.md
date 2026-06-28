# GOJA Studio (goja-studio)

[![License: AGPL v3](https://img.shields.io/badge/License-AGPL%20v3-blue.svg)](LICENSE)

**The desktop control plane for [GOJA](https://github.com/haraldwegner/goja-mcp)** — a Tauri
app that manages named workspaces of Java projects, runs the GOJA MCP runtime, and deploys MCP
server entries into Cursor / Claude / Antigravity / IntelliJ-style configs with one click.

goja-studio is a pure orchestrator: it downloads and updates the GOJA runtime, writes
`workspace.json` for live add/remove reconciliation, hosts **one resident JVM per workspace**
(URL endpoints into client configs — N clients × M workspaces collapse to M JVMs), and drives
per-workspace lifecycle from the system tray without opening the window. By default it pulls
the runtime from [haraldwegner/goja-mcp](https://github.com/haraldwegner/goja-mcp); switch
sources via *Release source* in Settings.

## Platforms

Linux (x86_64, aarch64), macOS (Apple Silicon), Windows (x64, ARM64). macOS/Windows builds are
unsigned (one-time Gatekeeper / SmartScreen bypass).

## Build

```bash
cd src-tauri && cargo build && cargo test   # Rust backend
npm install && npm run build                # frontend
npm run tauri dev                           # full desktop app (dev)
```

## Upgrading from javalens-manager

goja-studio is the renamed successor to `javalens-manager`. On first launch it **migrates**
your existing config/state/cache directories (`~/.config|.local/state|.cache/javalens-manager`
→ `…/goja-studio`) so your workspaces and settings carry over, and it continues to recognise
previously-deployed `jl-…` / `javalens-…` MCP entries for clean reconciliation.

## Licence

**AGPL-3.0** (see [`LICENSE`](LICENSE) / [`NOTICE`](NOTICE)). Contributions are accepted under
the [Contributor License Agreement](CLA.md) — see [`CONTRIBUTING.md`](CONTRIBUTING.md).
