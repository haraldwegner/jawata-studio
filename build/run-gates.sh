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

# THE HOOK SIDECAR FIRST, as the release job does (REL44-1, caught in the act at the 4.4.1 gate).
# tauri.conf.json declares externalBin ["binaries/jawata-hook"], and Tauri's build script copies
# binaries/jawata-hook-<triple> into target/debug/ as `jawata-hook` — the very path the hook's
# integration tests run (CARGO_BIN_EXE_jawata-hook). On this machine that file was a build from
# 7 August: after any change that re-runs the build script (a version bump in tauri.conf.json),
# the tests ran the August hook — 4,969,944 bytes against a current ~60 MB — and the one rule it
# lacked failed. Building the sidecar from the current source first makes that copy current.
echo "=== sidecar: the hook, built from this tree ==="
if ! "$ROOT/build/build-hook-sidecar.sh"; then
    echo "COULD NOT RUN: the hook sidecar did not build, and the suite would test whatever"
    echo "stale copy src-tauri/binaries holds."
    exit 2
fi

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

echo
echo "=== gate: the frontend type pass ==="
# THE ONLY AUTOMATED READING THE FRONTEND GETS. This repository has no frontend
# test framework, so every .svelte file is uncovered BY CONSTRUCTION — the Rust
# suite above cannot see one line of it. `svelte-check` is therefore not a nicety
# here; it is the whole of the frontend's verification, and until Sprint 28f it
# was wired into no gate at all. Found at C1, while shipping a MemoryView change
# that nothing else in this tree could have read.
#
# EXIT 2 IS NOT A PASS, for the same reason the gate above says so: a missing
# node_modules or an npm that is not there must fail distinctly rather than look
# green. `npm run check` exits 1 on a type error and non-zero when it cannot run,
# so the two are separated by asking whether the tool is present first.
if [ ! -d "$ROOT/node_modules" ]; then
    echo "FAILED: the frontend type pass could NOT RUN — no node_modules in $ROOT."
    echo "Run 'npm install' first. Nothing about the frontend was checked."
    exit 2
fi
( cd "$ROOT" && npm run --silent check )
TYPES=$?
if [ "$TYPES" -ne 0 ]; then
    echo "FAILED: the frontend type pass. A .svelte or .ts error is invisible to every"
    echo "other gate here, because nothing else in this repository reads that code."
    exit 1
fi

echo
echo "=== gate: the Help page names every tab ==="
# 2026-10-21, his walkthrough: Help never mentioned Reporting or Seats. Exit 2 is "nothing was checked".
"$ROOT/build/help-gate.sh"
HELP=$?
if [ "$HELP" -ne 0 ]; then
    echo "FAILED: the help gate (exit $HELP). A tab the app has must have a section in src/assets/help.md."
    exit "$HELP"
fi

echo
echo "=== gate: no screen spells out the live-workspace rule ==="
# S19-10. Exit 2 is "nothing was checked".
"$ROOT/build/live-rule-gate.sh" --selftest && "$ROOT/build/live-rule-gate.sh"
LIVE=$?
if [ "$LIVE" -ne 0 ]; then
    echo "FAILED: the live-rule gate (exit $LIVE). Read workspace liveness from the status record's live field."
    exit "$LIVE"
fi

echo
echo "=== gate: every architect report carries its D4 section ==="
# Sprint 28g Stage 6. Exit 2 is "nothing was checked", which is not a pass.
"$ROOT/build/architect-d4-gate.sh"
D4=$?
if [ "$D4" -ne 0 ]; then
    echo "FAILED: the D4 gate (exit $D4). A checkpoint report must say what it asked the"
    echo "experience store and the pattern catalogue and what it selected, or that it selected none."
    exit "$D4"
fi

echo
echo "=== gate: the engine-surface check can fail ==="
# The studio's copy of the engine's tool surface (src-tauri/engine-surface.json) is compared with the
# published engine in the release job. This proves the comparison fails when it should: on a flipped
# flag, a dropped tool, a dropped retired name, an engine that does not publish the flag.
"$ROOT/build/engine-surface-selftest.sh"
SURFACE=$?
if [ "$SURFACE" -ne 0 ]; then
    echo "FAILED: the engine-surface self-test (exit $SURFACE). The release job's comparison would not be trusted."
    exit "$SURFACE"
fi

# NOT RUN HERE, AND EACH FOR ITS OWN STATED REASON rather than by omission:
#
#   build/seam-gate.sh   — drives the REAL hook binary against the REAL published
#     store over HTTP. It needs a live resident, so running it unconditionally here
#     would fail the local suite on a machine that simply has nothing deployed. A
#     gate that cries wolf gets ignored, which is worse than a gate that is absent.
#     It stays in the release job and is worth running by hand before a release.
#
#   build/engine-surface.sh check — asks the PUBLISHED engine what it lists and compares it with the
#     committed copy; it needs the engine's release and the network, so it runs in the release job and by
#     hand (build/engine-surface.sh check <jar> <record>) before a release.
#
#   build/guard-probe.sh — drives the INSTALLED binary at its deployed role name, so
#     it is a probe of a DEPLOYMENT rather than of this tree. It is referenced by
#     nothing at all today, CI included; that is worth a decision rather than a
#     silent wiring, and it is recorded here so the next reader meets it.
echo
echo "ALL LOCAL GATES GREEN."
exit 0
