#!/usr/bin/env bash
# Sprint 28 (D-UNWIRED) — the release gate for hollow wiring, Rust side.
#
# The Rust equivalent of "every caller is a test": an item the compiler calls
# dead when cfg(test) is OFF but not when it is ON is used exclusively by test
# code. Dead in BOTH is the ordinary unused check and is NOT this.
#
# Two properties make the comparison real, and both were got wrong first:
#   * `cargo build` vs `cargo test` is NOT the comparison. `cargo test` builds
#     the plain lib target too, so its warning set contains the plain lib's
#     warnings and the two invocations are identical by construction. The
#     targets must be compiled separately (cargo rustc --lib, with and without
#     --profile test).
#   * `--force-warn dead_code` overrides in-source `#[allow(dead_code)]`. A
#     blanket allow on a module is exactly where hollow code hides — src/lib.rs
#     carried one over 72 test-only items for three sprints.
#
# Usage:  build/unwired-gate.sh [--update-baseline]
# Exit:   0 = no new findings · 1 = new findings · 2 = DID NOT RUN
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CRATE="$ROOT/src-tauri"
BASELINE="$ROOT/build/unwired-baseline.txt"
UPDATE=0
[ "${1:-}" = "--update-baseline" ] && UPDATE=1

WORK="$(mktemp -d -t jawata-unwired-rs-XXXXXX)"
trap 'rm -rf "$WORK"' EXIT

command -v cargo >/dev/null || { echo "gate: RESULT=no-cargo — cannot run. exit 2"; exit 2; }
cd "$CRATE" || { echo "gate: RESULT=no-crate at $CRATE. exit 2"; exit 2; }

# Touch the crate root so both compiles emit their lints (a cached unit emits
# none, and an empty warning set would read as "clean").
deadcode() {   # deadcode <outfile> <package> [extra cargo args...]
    local out="$1"; shift
    local pkg="$1"; shift
    # Touch the crate root so the unit actually recompiles — a cached unit
    # emits no lints, and an empty warning set would read as "clean".
    case "$pkg" in
        jawata-studio) touch src/lib.rs ;;
        jawata-hook)   touch jawata-hook/src/lib.rs ;;
    esac
    # Both crates have a lib target now (the hook gained one so integration
    # tests can drive its modules), so --lib is right for both.
    local target="--lib"
    # shellcheck disable=SC2086
    cargo rustc -p "$pkg" $target --message-format=short "$@" -- --force-warn dead_code \
        > "$WORK/raw.txt" 2>&1
    grep -E ': warning: ' "$WORK/raw.txt" \
        | sed 's/:[0-9]*:[0-9]*: warning: /|/' \
        | LC_ALL=C sort -u > "$out"
    if grep -qE '^error' "$WORK/raw.txt"; then
        echo "gate: RESULT=compile-error during the audit compile:"
        grep -E '^error' "$WORK/raw.txt" | head -5
        return 2
    fi
    # PER-ARM PROOF THAT THIS ARM RAN (C5 audit F8). The aggregate guards below
    # work on the MERGED set, so an arm that silently stopped compiling would be
    # invisible behind the other crate's warnings — the shape this script argues
    # against in its own header. An empty warning set cannot be the signal: a
    # clean crate legitimately produces none (the hook crate does). What proves
    # the arm ran is cargo saying it compiled or finished.
    if ! grep -qE "Compiling $pkg |Finished|Fresh $pkg " "$WORK/raw.txt"; then
        echo "gate: RESULT=arm-did-not-run — cargo reported neither Compiling nor Finished"
        echo "gate: for $pkg. This arm measured nothing, and the merged totals would hide it."
        head -5 "$WORK/raw.txt"
        return 2
    fi
    return 0
}

# BOTH crates. The hook crate is where Sprint 28's new code lives, and a gate
# blind to the code its own sprint adds is the shape it exists to catch.
deadcode "$WORK/studio-plain.txt" jawata-studio || exit 2
deadcode "$WORK/studio-cfgtest.txt" jawata-studio --profile test || exit 2
deadcode "$WORK/hook-plain.txt" jawata-hook || exit 2
deadcode "$WORK/hook-cfgtest.txt" jawata-hook --profile test || exit 2
cat "$WORK/studio-plain.txt" "$WORK/hook-plain.txt" | LC_ALL=C sort -u > "$WORK/plain.txt"
cat "$WORK/studio-cfgtest.txt" "$WORK/hook-cfgtest.txt" | LC_ALL=C sort -u > "$WORK/cfgtest.txt"

PLAIN=$(wc -l < "$WORK/plain.txt")
CFGTEST=$(wc -l < "$WORK/cfgtest.txt")
echo "gate: dead-code items — plain lib=$PLAIN, cfg(test) lib=$CFGTEST"

# A compile that produced nothing is not a clean crate; it is a scan that did
# not look. The crate has a known non-empty dead set, so zero means the
# --force-warn path stopped working.
if [ "$PLAIN" -eq 0 ]; then
    echo "gate: RESULT=examined-nothing — the plain-lib compile emitted no dead_code lints"
    echo "gate: at all. Either --force-warn stopped overriding #[allow], or nothing"
    echo "gate: recompiled. 'No hollow items' would be a claim about code never linted."
    exit 2
fi

LC_ALL=C comm -23 "$WORK/plain.txt" "$WORK/cfgtest.txt" > "$WORK/current.txt"
HOLLOW=$(wc -l < "$WORK/current.txt")
echo "gate: hollow (dead without cfg(test), alive with it) = $HOLLOW"

# THE COMPARISON MUST STILL BE A COMPARISON. If --profile test ever stops
# applying cfg(test) — a toolchain change, a renamed flag — the two sets become
# identical, the difference is empty, and this gate would print "74 items
# FIXED" and exit 0. That is the vacuous comparison this arm was built to
# escape, passing itself. The crate has a known non-empty hollow set, so an
# empty one means the method broke, not that the code got clean; clearing it
# legitimately is a --update-baseline away and says so.
if [ "$HOLLOW" -eq 0 ] && [ -s "$BASELINE" ]; then
    echo "gate: RESULT=comparison-collapsed — the two warning sets are identical, so the"
    echo "gate: cfg(test) build is no longer distinguishable from the plain one and this"
    echo "gate: gate is measuring nothing. It is NOT evidence that $(wc -l < "$BASELINE") baseline"
    echo "gate: item(s) were fixed. If they really were, re-run with --update-baseline."
    exit 2
fi
if [ "$CFGTEST" -ge "$PLAIN" ]; then
    echo "gate: RESULT=comparison-collapsed — the cfg(test) build reported as many dead items"
    echo "gate: ($CFGTEST) as the plain build ($PLAIN). Test code is supposed to USE things;"
    echo "gate: if it uses nothing, the test target did not compile as one."
    exit 2
fi

if [ "$UPDATE" = "1" ]; then
    cp "$WORK/current.txt" "$BASELINE"
    echo "gate: baseline UPDATED — $(wc -l < "$BASELINE") item(s). Commit it with the change that justifies it."
    exit 0
fi

[ -f "$BASELINE" ] || { echo "gate: RESULT=no-baseline at $BASELINE"; exit 2; }
LC_ALL=C sort "$BASELINE" > "$WORK/baseline.sorted"
NEW=$(LC_ALL=C comm -13 "$WORK/baseline.sorted" "$WORK/current.txt")
FIXED=$(LC_ALL=C comm -23 "$WORK/baseline.sorted" "$WORK/current.txt")

if [ -n "$FIXED" ]; then
    echo "gate: $(printf '%s\n' "$FIXED" | wc -l) baseline item(s) FIXED — re-baseline to keep the ratchet tight:"
    printf '%s\n' "$FIXED" | sed 's/^/  - /'
fi

if [ -n "$NEW" ]; then
    echo "gate: FAIL — $(printf '%s\n' "$NEW" | wc -l) NEW item(s) used only by test code:"
    printf '%s\n' "$NEW" | sed 's/^/  + /'
    echo "gate: wire it from production, delete it, or run --update-baseline with the reason."
    exit 1
fi

echo "gate: PASS — no new test-only items ($(wc -l < "$BASELINE") in baseline, unchanged)."
exit 0
