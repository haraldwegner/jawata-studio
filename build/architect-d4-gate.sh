#!/usr/bin/env bash
# The D4 gate — Sprint 28g, Stage 6. Every checkpoint's architect report must carry a section
# saying what was asked of the experience store and the pattern catalogue and what was selected,
# or that nothing was.
#
# WHY A GATE. Stage 6 shipped as a clause in the per-checkpoint gates plus seat text, with no code,
# and was honoured by 2 of 9 reports on disk when counted (2026-10-11). A clause nothing checks is
# a clause nobody keeps. This reads the reports and fails on one without the section.
#
# WHAT IT PROVES AND WHAT IT DOES NOT. It proves a report has a heading naming D4. It does not
# prove the consultation was run before the design, or that its selection was right; a report
# written afterwards says so in its own text (the ones added on 2026-10-11 do).
#
# Usage:  build/architect-d4-gate.sh [dir ...]      (default: the repo root, plus a sibling
#                                                   jawata-mcp checkout when one exists)
# Exit:   0 = every report carries the section · 1 = one or more do not · 2 = nothing to check
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIRS=("$@")
if [ "${#DIRS[@]}" -eq 0 ]; then
    DIRS=("$ROOT")
    [ -d "$ROOT/../jawata-mcp" ] && DIRS+=("$ROOT/../jawata-mcp")
fi
checked=0
missing=()
for d in "${DIRS[@]}"; do
    for f in "$d"/ARCHITECT-REPORT-C*.md; do
        [ -e "$f" ] || continue
        checked=$((checked + 1))
        if ! grep -qE '^#+ .*D4' "$f"; then
            missing+=("$f")
        fi
    done
done
if [ "$checked" -eq 0 ]; then
    echo "D4 gate: no ARCHITECT-REPORT-C*.md found in: ${DIRS[*]} — nothing was checked (exit 2)."
    exit 2
fi
if [ "${#missing[@]}" -gt 0 ]; then
    echo "D4 gate FAILED: ${#missing[@]} of $checked report(s) have no D4 section:"
    printf '  %s\n' "${missing[@]}"
    exit 1
fi
echo "D4 gate: PASS — $checked of $checked architect reports carry a D4 section."
