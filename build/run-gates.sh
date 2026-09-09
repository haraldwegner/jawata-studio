#!/usr/bin/env bash
# The LOCAL gate run for jawata-studio — 2026-09-09, Harald's ruling:
# "There shouldn't be a gate in the ci but on this machine! Having the ci in the
# gate and firing on this as the last step is nonsense."
#
# WHY THIS FILE EXISTS. jawata-studio had no local suite runner at all: the local
# flow was `cargo test --workspace` and nothing else, while three committed gates
# lived in build/ and ran only in the release job — or, in one case, nowhere.
# Measured that day:
#
#     seam-gate       CI refs=1   local refs=0
#     unwired-gate    CI refs=1   local refs=0
#     guard-probe     CI refs=0   local refs=0     <- runs NEVER
#
# What that cost, on the day it was measured: the v4.2.0 release failed on the
# hollow-wiring gate because `parse_netstat_pid` is reached only by test code on a
# non-Windows build. That was true for a day, in commits whose issues were already
# closed on the tracker, and nothing on this machine could say so — the first thing
# to look was the release job, on the fifth platform, after four had already built
# and uploaded. jawata-mcp failed the identical gate in the same hour.
#
# A gate whose only run is the last step before publishing cannot do the job it
# exists for. Its subject is code BUILT AND NOT WIRED, which you want to hear about
# while you still remember writing it.
#
# Usage:  build/run-gates.sh
# Exit:   0 = everything green · 1 = a gate failed · 2 = a gate COULD NOT RUN
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

echo "=== suite: the Rust workspace ==="
( cd "$ROOT/src-tauri" && cargo test --workspace )
SUITE=$?
if [ "$SUITE" -ne 0 ]; then
    echo "FAILED: the workspace suite. The gates below are NOT run — they would be"
    echo "reporting on a tree whose tests do not pass."
    exit 1
fi

echo
echo "=== gate: hollow wiring (items reached only by test code) ==="
# EXIT 2 IS NOT A PASS. The gate answers 2 when it could not run at all — no cargo,
# no baseline, a compile error, or a comparison that collapsed. Reading that as green
# is how a gate becomes decoration, so it is failed distinctly and says why.
"$ROOT/build/unwired-gate.sh"
UNWIRED=$?
if [ "$UNWIRED" -eq 2 ]; then
    echo "FAILED: the hollow-wiring gate could NOT RUN (exit 2) — nothing was checked."
    exit 2
elif [ "$UNWIRED" -ne 0 ]; then
    exit "$UNWIRED"
fi

# NOT RUN HERE, AND EACH FOR ITS OWN STATED REASON rather than by omission:
#
#   build/seam-gate.sh   — drives the REAL hook binary against the REAL published
#     store over HTTP. It needs a live resident, so running it unconditionally here
#     would fail the local suite on a machine that simply has nothing deployed. A
#     gate that cries wolf gets ignored, which is worse than a gate that is absent.
#     It stays in the release job and is worth running by hand before a release.
#
#   build/guard-probe.sh — drives the INSTALLED binary at its deployed role name, so
#     it is a probe of a DEPLOYMENT rather than of this tree. It is referenced by
#     nothing at all today, CI included; that is worth a decision rather than a
#     silent wiring, and it is recorded here so the next reader meets it.
echo
echo "ALL LOCAL GATES GREEN."
exit 0
