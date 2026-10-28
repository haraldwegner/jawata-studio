#!/usr/bin/env bash
# S19-10, S19-10b: "is this workspace live" (a project running or starting) is decided in ONE place, the backend's
# RuntimePhase::is_live, and reaches the screens as the `live` field of the status record. A screen reads that flag; it
# does not spell the rule out, so a new phase or a fifth caller cannot make the Delete menus, the Start/Stop button and
# the live count disagree. (The Rust side is held to the same by a test in runtime_manager.rs.)
# This fails when a screen names both phases on one line, in either order and with either quote.
# Usage:  build/live-rule-gate.sh [src-dir]   (default: src)
#         build/live-rule-gate.sh --selftest  (plants each spelling and proves the gate fails on it)
# Exit:   0 = no screen spells the rule · 1 = a screen does · 2 = nothing was checked
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SELF="$ROOT/build/live-rule-gate.sh"

if [ "${1:-}" = "--selftest" ]; then
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT
    mkdir -p "$tmp/lib/components"
    bad=0
    expect() { # name, expected exit, line to plant ("" plants nothing)
        printf '%s\n' "$3" > "$tmp/lib/components/Planted.svelte"
        "$SELF" "$tmp" >/dev/null 2>&1
        got=$?
        if [ "$got" -eq "$2" ]; then echo "ok   $1 (exit $got)"; else echo "FAIL $1: expected exit $2, got $got"; bad=1; fi
    }
    expect "a clean file passes" 0 'const phase = "stopped";'
    expect "reading the backend flag passes" 0 'const live = statuses[id]?.live === true;'
    expect "a type union of the phases passes" 0 'type Phase = "running" | "stopped" | "starting";'
    expect "the double-quoted array fails" 1 'list.some((p) => ["running", "starting"].includes(p));'
    expect "the single-quoted array fails" 1 "list.some((p) => ['running', 'starting'].includes(p));"
    expect "the reversed array fails" 1 'const live = ["starting", "running"];'
    expect "the comparison pair fails" 1 'if (phase === "running" || phase === "starting") {'
    expect "the reversed comparison pair fails" 1 'if (p === "starting" || p === "running") {'
    expect "a comparison on an optional chain fails" 1 'x = s?.phase === "running" || s?.phase === "starting";'
    "$SELF" "$tmp/no-such-dir" >/dev/null 2>&1
    got=$?
    if [ "$got" -eq 2 ]; then echo "ok   a missing source folder is not a pass (exit $got)"; else echo "FAIL a missing source folder: expected exit 2, got $got"; bad=1; fi
    [ "$bad" -eq 0 ] && echo "live-rule gate selftest: every case behaved as promised."
    exit "$bad"
fi

SRC="${1:-$ROOT/src}"
if [ ! -d "$SRC/lib" ]; then
    echo "COULD NOT CHECK: $SRC/lib does not exist."
    exit 2
fi
# Both phase names, quoted, on one line, in either order: the array and the pair of comparisons, however they are spelled.
HITS="$(grep -rnE "[\"']running[\"'].*[\"']starting[\"']|[\"']starting[\"'].*[\"']running[\"']" "$SRC" \
    --include=*.svelte --include=*.ts \
    | grep -vE '^[^:]+:[0-9]+:[[:space:]]*(export )?type [A-Za-z]+ = ' || true)"
if [ -n "$HITS" ]; then
    echo "FAILED: a screen spells out the live-phase rule. Read the status record's live field instead:"
    echo "$HITS"
    exit 1
fi
echo "ok: no screen spells out the live-phase rule."
exit 0
