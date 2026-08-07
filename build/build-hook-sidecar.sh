#!/usr/bin/env bash
# Sprint 28 Stage 7 (D-SHIM c) — build the hook binary as a Tauri SIDECAR.
#
# `tauri.conf.json` declares `externalBin: ["binaries/jawata-hook"]`. Tauri does
# NOT copy that path literally: it appends the Rust target triple and copies
# `binaries/jawata-hook-<triple>[.exe]` into the bundle beside the app
# executable. Get the suffix wrong and the build fails with "binary not found"
# — loudly, which is the good case; get the SOURCE wrong and you ship a stale
# binary silently, which is why this script always rebuilds rather than reusing
# whatever is in target/.
#
# Usage:  build/build-hook-sidecar.sh [target-triple]
#         (default: the host triple, which is what a local `tauri build` wants)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/src-tauri"
TRIPLE="${1:-$(rustc -vV | awk '/^host:/ {print $2}')}"
[ -n "$TRIPLE" ] || { echo "sidecar: could not determine the target triple"; exit 2; }

EXT=""
case "$TRIPLE" in *windows*) EXT=".exe" ;; esac

echo "sidecar: building jawata-hook for $TRIPLE"
cd "$CRATE"
if [ "$TRIPLE" = "$(rustc -vV | awk '/^host:/ {print $2}')" ]; then
    cargo build --release -p jawata-hook
    BUILT="$CRATE/target/release/jawata-hook$EXT"
else
    rustup target add "$TRIPLE" >/dev/null 2>&1 || true
    cargo build --release -p jawata-hook --target "$TRIPLE"
    BUILT="$CRATE/target/$TRIPLE/release/jawata-hook$EXT"
fi

[ -f "$BUILT" ] || { echo "sidecar: cargo reported success but $BUILT is absent"; exit 2; }

DEST_DIR="$CRATE/binaries"
DEST="$DEST_DIR/jawata-hook-$TRIPLE$EXT"
mkdir -p "$DEST_DIR"
# Unlink before copying: the same ETXTBSY hazard the deploy has, for the same
# reason — a previous sidecar may still be executing from a dev install.
rm -f "$DEST"
cp "$BUILT" "$DEST"
chmod +x "$DEST" 2>/dev/null || true

echo "sidecar: $DEST"
ls -l "$DEST"
