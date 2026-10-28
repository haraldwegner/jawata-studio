#!/usr/bin/env bash
# S19-10: "is this workspace live" (a project running or starting) is decided in ONE place on the screens,
# src/lib/workspaceLive.ts, so a new phase or a fifth caller cannot make the Delete menus, the Start/Stop
# button and the live count disagree. The backend enforces the same rule in manager_service::deletion_refusal.
# This fails when a screen names both phases on one line again, in either order and with either quote.
# Usage:  build/live-rule-gate.sh [src-dir]   (default: src)
#         build/live-rule-gate.sh --selftest  (plants each spelling and proves the gate fails on it)
# Exit:   0 = written once · 1 = written again · 2 = nothing was checked
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SELF="$ROOT/build/live-rule-gate.sh"

if [ "${1:-}" = "--selftest" ]; then
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT
    mkdir -p "$tmp/lib/components"
    echo 'export const live = true;' > "$tmp/lib/workspaceLive.ts"
    bad=0
    expect() { # name, expected exit, line to plant ("" plants nothing)
        printf '%s\n' "$3" > "$tmp/lib/components/Planted.svelte"
        "$SELF" "$tmp" >/dev/null 2>&1
        got=$?
        if [ "$got" -eq "$2" ]; then echo "ok   $1 (exit $got)"; else echo "FAIL $1: expected exit $2, got $got"; bad=1; fi
    }
    expect "a clean file passes" 0 'const phase = "stopped";'
    expect "a type union of the phases passes" 0 'type Phase = "running" | "stopped" | "starting";'
    expect "the double-quoted array fails" 1 'list.some((p) => ["running", "starting"].includes(p));'
    expect "the single-quoted array fails" 1 "list.some((p) => ['running', 'starting'].includes(p));"
    expect "the reversed array fails" 1 'const live = ["starting", "running"];'
    expect "the comparison pair fails" 1 'if (phase === "running" || phase === "starting") {'
    expect "the reversed comparison pair fails" 1 'if (p === "starting" || p === "running") {'
    expect "a comparison on an optional chain fails" 1 'x = s?.phase === "running" || s?.phase === "starting";'
    rm -f "$tmp/lib/workspaceLive.ts"
    "$SELF" "$tmp" >/dev/null 2>&1
    got=$?
    if [ "$got" -eq 2 ]; then echo "ok   a missing module is not a pass (exit $got)"; else echo "FAIL a missing module: expected exit 2, got $got"; bad=1; fi
    [ "$bad" -eq 0 ] && echo "live-rule gate selftest: every case behaved as promised."
    exit "$bad"
fi

SRC="${1:-$ROOT/src}"
if [ ! -f "$SRC/lib/workspaceLive.ts" ]; then
    echo "COULD NOT CHECK: $SRC/lib/workspaceLive.ts does not exist."
    exit 2
fi
# Both phase names, quoted, on one line, in either order: the array and the pair of comparisons, however they are spelled.
HITS="$(grep -rnE "[\"']running[\"'].*[\"']starting[\"']|[\"']starting[\"'].*[\"']running[\"']" "$SRC" \
    --include=*.svelte --include=*.ts | grep -v '/lib/workspaceLive.ts' \
    | grep -vE '^[^:]+:[0-9]+:[[:space:]]*(export )?type [A-Za-z]+ = ' || true)"
if [ -n "$HITS" ]; then
    echo "FAILED: the live-phase rule is written outside src/lib/workspaceLive.ts:"
    echo "$HITS"
    exit 1
fi
echo "ok: the live-phase rule is written once (src/lib/workspaceLive.ts)."
exit 0
