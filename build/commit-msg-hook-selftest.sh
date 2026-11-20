#!/usr/bin/env bash
# Proves the commit-msg hook can fail, and that it names every agent the pre-commit hook names and every agent the engine's
# GitTool refuses (28h Stage 27, the C27 audit finding 2): a commit message naming an AI agent as co-author is refused, a person is not,
# and each name in the pre-commit hook's own list is refused here too, so the two hooks cannot drift apart without this saying so.
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HOOK="$ROOT/hooks/commit-msg"
W="$(mktemp -d /tmp/jawata-commit-msg-selftest.XXXXXX)"
trap 'rm -rf "$W"' EXIT
fail=0

refuses() { # refuses <label> <message text>
  printf '%b' "$2" > "$W/msg.txt"
  if sh "$HOOK" "$W/msg.txt" > /dev/null 2>&1; then echo "SELFTEST FAIL: $1 was accepted"; fail=1; fi
}
accepts() {
  printf '%b' "$2" > "$W/msg.txt"
  if ! sh "$HOOK" "$W/msg.txt" > /dev/null 2>&1; then echo "SELFTEST FAIL: $1 was refused"; fail=1; fi
}

for who in "Claude" "Aider" "Windsurf" "Codeium" "Gemini" "Google Gemini" "Codex" "Devin" "Copilot" "ChatGPT" "GPT-4" "OpenAI" "Cursor Agent"; do
  refuses "co-author $who" "subject\n\nCo-Authored-By: $who <a@b.invalid>\n"
done
refuses "a GitHub bot as co-author" "subject\n\nCo-Authored-By: dependabot[bot] <49699333+dependabot[bot]@users.noreply.github.com>\n"
refuses "Generated with Claude" "subject\n\nGenerated with Claude Code\n"
accepts "a person as co-author" "subject\n\nCo-Authored-By: Ana Person <ana@example.invalid>\n"
accepts "a message with no trailer" "subject\n\nbody\n"

# Every name the pre-commit hook lists as an agent is refused as a co-author here too.
agents="$(sed -n "s/^AGENT='\(.*\)'$/\1/p" "$ROOT/hooks/pre-commit")"
[ -n "$agents" ] || { echo "SELFTEST FAIL: could not read the AGENT list of hooks/pre-commit"; exit 2; }
IFS='|' read -r -a names <<< "$agents"
for n in "${names[@]}"; do
  case "$n" in '\[bot\]') continue ;; esac     # an identity marker, not a name: covered by the GitHub bot case above
  refuses "the pre-commit hook's own name '$n'" "subject\n\nCo-Authored-By: $n <a@b.invalid>\n"
done
[ "$fail" = 0 ] && echo "commit-msg hook self-test: PASS"
exit "$fail"
