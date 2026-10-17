#!/usr/bin/env bash
# Proves build/engine-surface.py can fail: it passes on a faithful copy and fails, with the exit code it
# promises, on each way the studio's copy of the engine's tool surface can be wrong. A check that has only
# ever been seen to pass has not been seen to check anything.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PY="$ROOT/build/engine-surface.py"
W="$(mktemp -d /tmp/jawata-surface-selftest.XXXXXX)"
trap 'rm -rf "$W"' EXIT
fail=0

cat > "$W/wire.json" <<'JSON'
{"jsonrpc":"2.0","id":1,"result":{"tools":[
 {"name":"rename_symbol","_meta":{"jawata/changesCode":true}},
 {"name":"compile_workspace","_meta":{"jawata/changesCode":false}},
 {"name":"hierarchy","_meta":{"jawata/changesCode":true}}]}}
JSON
printf '# record\nrename_symbol\tlive\tv1\tv2\ncompile_workspace\tlive\tv1\tv2\nhierarchy\tlive\tv3\tv3\nmove_method\tretired\tv1\tv2\nundo_refactoring\tretired\tv1\tv2\n' > "$W/record.txt"

python3 -I "$PY" generate --tools-list "$W/wire.json" --record "$W/record.txt" --version 9.9.9 > "$W/vendored.json" \
  || { echo "SELFTEST FAIL: generate did not run"; exit 2; }

expect() { # expect <exit> <label> <command...>
  want="$1"; label="$2"; shift 2
  "$@" > "$W/out.txt" 2>&1; got=$?
  if [ "$got" -ne "$want" ]; then echo "SELFTEST FAIL: $label — expected exit $want, got $got"; cat "$W/out.txt"; fail=1
  else echo "ok   $label (exit $got)"; fi
}

expect 0 "a faithful copy passes" python3 -I "$PY" check --tools-list "$W/wire.json" --record "$W/record.txt" --vendored "$W/vendored.json"

sed 's/"hierarchy", "changesCode": true/"hierarchy", "changesCode": false/' "$W/vendored.json" > "$W/t1.json"
expect 1 "a flipped changesCode flag fails" python3 -I "$PY" check --tools-list "$W/wire.json" --record "$W/record.txt" --vendored "$W/t1.json"

grep -v '"move_method"' "$W/vendored.json" | sed 's/"undo_refactoring",/"undo_refactoring"/' > "$W/t2.json"
expect 1 "a dropped retired name fails" python3 -I "$PY" check --tools-list "$W/wire.json" --record "$W/record.txt" --vendored "$W/t2.json"

grep -v '"compile_workspace"' "$W/vendored.json" > "$W/t3.json"
expect 1 "a dropped live tool fails" python3 -I "$PY" check --tools-list "$W/wire.json" --record "$W/record.txt" --vendored "$W/t3.json"

sed 's/9\.9\.9/1.0.0/' "$W/vendored.json" > "$W/t4.json"
expect 0 "a different engineVersion alone does not fail" python3 -I "$PY" check --tools-list "$W/wire.json" --record "$W/record.txt" --vendored "$W/t4.json"

sed 's/,"_meta":{"jawata\/changesCode":true}//' "$W/wire.json" > "$W/wire-old.json"
expect 2 "an engine that does not publish the flag is refused, not guessed at" python3 -I "$PY" check --tools-list "$W/wire-old.json" --record "$W/record.txt" --vendored "$W/vendored.json"

printf 'rename_symbol\tlive\tv1\tv2\nmove_method\tretired\tv1\tv2\n' > "$W/record-short.txt"
expect 1 "an engine that disagrees with its own record fails" python3 -I "$PY" check --tools-list "$W/wire.json" --record "$W/record-short.txt" --vendored "$W/vendored.json"

expect 2 "a missing vendored file is not a pass" python3 -I "$PY" check --tools-list "$W/wire.json" --record "$W/record.txt" --vendored "$W/absent.json"

[ "$fail" -eq 0 ] && echo "engine-surface selftest: every case behaved as promised." || exit 1
