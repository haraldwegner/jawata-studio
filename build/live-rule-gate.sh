#!/usr/bin/env bash
# S19-10: "is this workspace live" (a project running or starting) is decided in ONE place on the screens,
# src/lib/workspaceLive.ts, so a new phase or a fifth caller cannot make the Delete menus, the Start/Stop
# button and the live count disagree. The backend enforces the same rule in manager_service::deletion_refusal.
# This fails when a screen spells the pair of phases out again.
# Usage:  build/live-rule-gate.sh [src-dir]   (default: src)
# Exit:   0 = written once · 1 = written again · 2 = nothing was checked
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="${1:-$ROOT/src}"
if [ ! -f "$SRC/lib/workspaceLive.ts" ]; then
    echo "COULD NOT CHECK: $SRC/lib/workspaceLive.ts does not exist."
    exit 2
fi
# The two spellings the four copies used: the array of both phases, and the pair of comparisons.
HITS="$(grep -rnE '"running", *"starting"|=== *"running" *\|\| *[a-z]+ *=== *"starting"' "$SRC" \
    --include=*.svelte --include=*.ts | grep -v '/lib/workspaceLive.ts' || true)"
if [ -n "$HITS" ]; then
    echo "FAILED: the live-phase rule is written outside src/lib/workspaceLive.ts:"
    echo "$HITS"
    exit 1
fi
echo "ok: the live-phase rule is written once (src/lib/workspaceLive.ts)."
exit 0
