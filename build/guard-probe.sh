#!/usr/bin/env bash
# The guard probe — does the DEPLOYED hook actually refuse what it claims to?
#
# WHY THIS IS A COMMITTED SCRIPT AND NOT A SCRATCH FILE. A measurement whose
# evidence cannot be re-run is a claim. This project has been refused at a
# checkpoint for exactly that: an exhaustiveness measurement was sound, its
# probe was scratch and was never committed, and the audit correctly declined
# to accept the number. The v4.1.5 and v4.1.6 guards were verified live the
# same way, in a session scratch directory that a reboot clears — same defect,
# so the probe lives here now.
#
# It drives the INSTALLED binary at its own role name, with its own config
# beside it, because that is the thing gating a real session. The unit tests
# cover the decision functions; this covers the deployed artifact, which is a
# different claim. A guard whose rules are correct and whose binary was never
# deployed refuses nothing.
#
# Usage: guard-probe.sh [path-to-jawata-hook-guard]
#        default: ~/.claude/jawata-studio/jawata-hook-guard
# Exit 0 = every rule behaved as declared.
set -uo pipefail

GUARD="${1:-$HOME/.claude/jawata-studio/jawata-hook-guard}"
[ -x "$GUARD" ] || { echo "no deployed guard at $GUARD"; exit 2; }

pass=0
fail=0

# BOTH client dialects. Cursor prints "permission":"deny"; Claude Code prints
# "permissionDecision":"deny" inside hookSpecificOutput. Matching only one made
# every real refusal read as an allow on this probe's first run — the probe was
# wrong and the product was right, which is the failure a probe must not have.
verdict() {
  local out
  out=$(printf '%s' "$1" | "$GUARD" 2>/dev/null)
  case "$out" in
    *'"permission":"deny"'*|*'"permissionDecision":"deny"'*) echo "deny";;
    *) echo "allow";;
  esac
}

json() { printf '%s' "$1" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))'; }
bash_p() { printf '{"tool_name":"Bash","tool_input":{"command":%s}}' "$(json "$1")"; }
read_p() { printf '{"tool_name":"Read","tool_input":{"file_path":%s}}' "$(json "$1")"; }

# $1 label  $2 expected verdict  $3 payload  [$4 env assignments]
drive() {
  local label="$1" want="$2" payload="$3" envset="${4:-}" got out
  if [ -n "$envset" ]; then
    out=$(env $envset "$GUARD" <<<"$payload" 2>/dev/null)
    case "$out" in
      *'"permission":"deny"'*|*'"permissionDecision":"deny"'*) got="deny";;
      *) got="allow";;
    esac
  else
    got=$(verdict "$payload")
  fi
  if [ "$got" = "$want" ]; then
    pass=$((pass + 1)); printf 'ok    %-44s %s\n' "$label" "$got"
  else
    fail=$((fail + 1)); printf 'FAIL  %-44s got=%s want=%s\n' "$label" "$got" "$want"
  fi
}

echo "=== v4.1.6 — the four rules folded out of the dev machine ==="
drive "containment: a path outside the workspace" deny "$(read_p "$HOME/.ssh/id_rsa")"
drive "credentials: a remote address is printed"  deny "$(bash_p 'git remote -v')"
drive "an expensive gate's output is discarded"   deny \
  "$(bash_p 'cd /home/harald/CursorProjects/jawata-mcp && ./build/run-suite.sh 4 | tail -5')"
drive "a gate invoked with no absolute cd"        deny \
  "$(bash_p './build/run-suite.sh 4 > /tmp/o.txt 2>&1')"

echo
echo "=== v4.1.5 — the suite runs at a checkpoint or a release ==="
drive "a full suite, undeclared"                  deny \
  "$(bash_p 'cd /home/harald/CursorProjects/jawata-mcp && ./build/run-suite.sh 4 > /tmp/s.txt 2>&1')"
drive "a full suite, declaring its checkpoint"    allow \
  "$(bash_p 'cd /home/harald/CursorProjects/jawata-mcp && ./build/run-suite.sh 4 > /tmp/s.txt 2>&1  # checkpoint: C3')"

echo
echo "=== the shapes that must NOT fire — half of every rule's claim ==="
drive "ordinary git"                              allow "$(bash_p 'git status --short')"
drive "a config read that reaches no credential"  allow "$(bash_p 'git config --get user.name')"
drive "a system path"                             allow "$(read_p '/usr/lib/jvm/default/release')"
drive "a path inside the workspace"               allow \
  "$(read_p '/home/harald/CursorProjects/jawata-studio/package.json')"
drive "reading a captured gate's own file"        allow "$(bash_p 'tail -5 /tmp/s.txt')"

echo
echo "=== the v4.1.6 scratch-root fix: a temp dir that is not /tmp ==="
# This is the macOS shape reproduced on any platform. Before the fix the rule
# knew only the Unix literal, so it refused a program's own scratch file
# everywhere the temp dir is elsewhere — and passed on Linux by coincidence.
SIM="${TMPDIR:-/tmp}/jawata-guard-probe-sim"
mkdir -p "$SIM"
drive "a file in a scratch dir outside /tmp"      allow \
  "$(read_p "$SIM/notes.md")" "HOME=$SIM TMPDIR=$SIM"

echo
echo "=== the local shell guards: is the binary still doing their job? ==="
# These are the shapes ~/.claude/hooks/guard-workspace.sh and
# guard-gate-output.sh refuse. An "allow" here means the binary does NOT yet
# cover that script, and the script cannot be deleted.
drive "credentials: the config regexp spelling"   deny "$(bash_p 'git config --get-regexp remote')"
drive "credentials: a whole-config dump"          deny "$(bash_p 'git config --list')"
drive "containment: the tilde spelling"           deny "$(bash_p 'cat ~/.ssh/id_rsa')"
drive "containment: the HOME-variable spelling"   deny "$(bash_p 'cat $HOME/.ssh/id_rsa')"
drive "an uncaptured build tool"                  deny \
  "$(bash_p 'cd /home/harald/CursorProjects/jawata-mcp && mvn -q test')"
drive "a version probe stays exempt"              allow "$(bash_p 'mvn --version')"

echo
echo "passed=$pass failed=$fail"
[ "$fail" -eq 0 ]
