#!/usr/bin/env bash
# The cross-product seam gate — Sprint 28 "E2E means E2E", closed for real.
#
# Outcome audit F10: the seam test in the hook crate drives the real binary
# against a MOCK store serving a committed capture, so a producer-side move —
# the store's answer changing shape — leaves the fixture untouched and the
# test green. That is precisely the shape of test the sprint's own rule
# forbids: a changed contract needs one test exercising producer and consumer
# TOGETHER, and it must fail when either side moves.
#
# This script is that test. A REAL prompt, the REAL hook binary under its
# deployed role name, the REAL published jawata store over HTTP, and the
# answer must arrive as injectable context. If the store's envelope moves,
# the hook goes silent and this fails. If the hook's cue or parse path
# breaks, same. Nothing here is canned.
#
# Usage: seam-gate.sh <hook-binary> <jawata.jar>
# Exit 0 = the seam is proven. Anything else = the release must not ship.
set -euo pipefail

HOOK_BINARY="$(readlink -f "$1")"
JAR="$(readlink -f "$2")"
[ -x "$HOOK_BINARY" ] || { echo "seam-gate: $HOOK_BINARY is not executable"; exit 1; }
[ -f "$JAR" ] || { echo "seam-gate: $JAR not found"; exit 1; }

WORK="$(mktemp -d /tmp/jawata-seam-gate.XXXXXX)"
PORT=8977
TOKEN="seam-gate-$$"
RESIDENT_PID=""
cleanup() {
    [ -n "$RESIDENT_PID" ] && kill "$RESIDENT_PID" 2>/dev/null || true
    rm -rf "$WORK"
}
trap cleanup EXIT

# The PRODUCER: the real store, from the published artifact. Embedder off —
# keyword recall is the degrade contract the e2e gate proves separately, and
# it keeps this gate fast and model-free.
java -Djawata.embed.disabled=true \
     -jar "$JAR" -data "$WORK/ws" -port "$PORT" -token "$TOKEN" \
     > "$WORK/resident.log" 2>&1 &
RESIDENT_PID=$!

for i in $(seq 1 120); do
    grep -q "READY" "$WORK/resident.log" 2>/dev/null && break
    kill -0 "$RESIDENT_PID" 2>/dev/null || {
        echo "seam-gate: the resident died before READY:"; tail -20 "$WORK/resident.log"; exit 1; }
    sleep 1
done
grep -q "READY" "$WORK/resident.log" || {
    echo "seam-gate: the resident never reached READY in 120s"; tail -20 "$WORK/resident.log"; exit 1; }

call() {
    curl -s --max-time 10 -X POST \
         -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
         -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"experience\",\"arguments\":$1}}" \
         "http://127.0.0.1:$PORT/mcp"
}

# Seed one entry the prompt below can only find through the real store.
SEED='{"kind":"record","type":"lesson","operation":"seam-gate","language":"java",
  "summary":"The gearbox synchronizer needs double declutching before every downshift on this transmission."}'
R="$(call "$SEED")"
# The tool result arrives JSON-escaped inside the MCP envelope: \"stored\":true.
case "$R" in
    *'stored\":true'*|*'"stored":true'*) ;;
    *) echo "seam-gate: seeding the store failed: $(printf '%s' "$R" | head -c 300)"; exit 1 ;;
esac

# The CONSUMER: the real hook binary, deployed exactly as the studio deploys
# it — role name via argv[0], config on disk beside it.
mkdir -p "$WORK/hooks"
cp "$HOOK_BINARY" "$WORK/hooks/jawata-hook-userprompt"
chmod 0755 "$WORK/hooks/jawata-hook-userprompt"
printf '{"client":"claude-code","token":"%s","url":"http://127.0.0.1:%s/mcp"}' \
    "$TOKEN" "$PORT" > "$WORK/hooks/hook_config.json"

OUT="$(printf '{"prompt":"why does the gearbox synchronizer need double declutching on a downshift"}' \
    | "$WORK/hooks/jawata-hook-userprompt")"

case "$OUT" in
    *additionalContext*synchronizer*|*additionalContext*declutching*)
        echo "seam-gate OK: prompt -> hook -> real store -> context ($(printf '%s' "$OUT" | wc -c) bytes)" ;;
    *)
        echo "seam-gate FAIL: the answer did not cross the seam as context."
        echo "hook stdout: $(printf '%s' "$OUT" | head -c 400)"
        echo "resident log tail:"; tail -10 "$WORK/resident.log"
        exit 1 ;;
esac
