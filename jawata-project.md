---
stack: Rust 2021 (CI toolchain 1.99.1), TypeScript 6, Svelte 5, Tauri 2, Node 22
build: cargo (src-tauri) and npm with Vite (the page)
source_roots: [src, src-tauri/src, src-tauri/jawata-hook/src]
green: build/run-gates.sh
---

jawata-studio is the desktop application that manages jawata's engines and agent settings.
The block above is what every jawata seat is told about this project.
