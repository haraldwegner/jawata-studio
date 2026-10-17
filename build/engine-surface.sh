#!/usr/bin/env bash
# Ask a real engine what it publishes, then refresh or check the studio's copy of it.
#
#   engine-surface.sh refresh <jawata.jar> <published-tool-names.txt> <engine-version>
#       rewrites src-tauri/engine-surface.json from the engine at <jar>.
#   engine-surface.sh check   <jawata.jar> <published-tool-names.txt>
#       exit 0 when the committed file holds what that engine publishes; 1 when it does not.
#
# The engine is the PUBLISHED jar, started the way seam-gate.sh starts it, and asked for tools/list over
# HTTP: the list is what a client is told, not what the engine's source says it ships. The record
# (build/published-tool-names.txt in jawata-mcp, at the engine's own tag) supplies the retired names, which
# tools/list cannot, because a retired tool is by definition not listed.
#
# A failure to start, to answer, or to read is exit 2 and never a pass.
set -euo pipefail

MODE="${1:-}"
JAR="$(readlink -f "${2:-/nonexistent}")" || true
RECORD="${3:-}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VENDORED="$ROOT/src-tauri/engine-surface.json"

[ "$MODE" = "refresh" ] || [ "$MODE" = "check" ] || { echo "usage: engine-surface.sh refresh|check <jar> <record> [version]"; exit 2; }
[ -f "$JAR" ] || { echo "engine-surface: $JAR not found"; exit 2; }
[ -f "$RECORD" ] || { echo "engine-surface: record $RECORD not found"; exit 2; }

WORK="$(mktemp -d /tmp/jawata-engine-surface.XXXXXX)"
PORT="${SURFACE_PORT:-8979}"
TOKEN="surface-$$"
RESIDENT_PID=""
cleanup() {
    [ -n "$RESIDENT_PID" ] && kill "$RESIDENT_PID" 2>/dev/null || true
    rm -rf "$WORK"
}
trap cleanup EXIT

java -Djawata.embed.disabled=true \
     -jar "$JAR" -data "$WORK/ws" -port "$PORT" -token "$TOKEN" \
     > "$WORK/resident.log" 2>&1 &
RESIDENT_PID=$!
for i in $(seq 1 120); do
    grep -q "READY" "$WORK/resident.log" 2>/dev/null && break
    kill -0 "$RESIDENT_PID" 2>/dev/null || { echo "engine-surface: the resident died before READY:"; tail -20 "$WORK/resident.log"; exit 2; }
    sleep 1
done
grep -q "READY" "$WORK/resident.log" || { echo "engine-surface: the resident never reached READY in 120s"; exit 2; }

curl -sf --max-time 30 -X POST \
     -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
     -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' \
     "http://127.0.0.1:$PORT/mcp" > "$WORK/tools-list.json" \
     || { echo "engine-surface: tools/list did not answer"; exit 2; }

if [ "$MODE" = "refresh" ]; then
    VERSION="${4:?refresh needs the engine version as the fourth argument}"
    python3 -I "$ROOT/build/engine-surface.py" generate \
        --tools-list "$WORK/tools-list.json" --record "$RECORD" --version "$VERSION" > "$WORK/surface.json"
    cp "$WORK/surface.json" "$VENDORED"
    echo "engine-surface: wrote $VENDORED (engine $VERSION)"
else
    python3 -I "$ROOT/build/engine-surface.py" check \
        --tools-list "$WORK/tools-list.json" --record "$RECORD" --vendored "$VENDORED"
fi
