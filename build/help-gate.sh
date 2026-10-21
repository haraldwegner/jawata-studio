#!/usr/bin/env bash
# jawata-gate: help
# THE HELP PAGE NAMES EVERY TAB THE APP HAS.
#
# WHY. The in-app Help said nothing about the Reporting and Seats tabs and described the Memory
# view as having five actions when it had nine (found by his walkthrough, 2026-10-21). A page that
# copies the app's surface is wrong from the first change nobody mirrored, and nothing announces
# it. The tabs are the one part of that surface a script can read: every button of the `nav-tabs`
# in App.svelte is a tab, and each one except Help itself must be a `## <name>` heading in help.md.
#
# Usage: build/help-gate.sh [App.svelte] [help.md]      (the two paths are for the self-test)
# Exit:  0 = every tab has a section · 1 = a tab has none · 2 = the gate could not read its inputs
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="${1:-$ROOT/src/App.svelte}"
HELP="${2:-$ROOT/src/assets/help.md}"

for f in "$APP" "$HELP"; do
    if [ ! -r "$f" ]; then
        echo "COULD NOT RUN: cannot read $f — nothing was checked."
        exit 2
    fi
done

# The tab labels: the text line after each `class="tab ..."` button's `type="button">`.
tabs=$(awk '
    /class="tab / { intab = 1; next }
    intab && /type="button"/ { seen = 1; next }
    intab && seen && /^[[:space:]]*>[[:space:]]*$/ { next }
    intab && seen {
        gsub(/^[[:space:]]+|[[:space:]]+$/, "");
        if ($0 != "" && $0 != ">" && $0 != "</button>") { print; intab = 0; seen = 0 }
    }
' "$APP")

if [ -z "$tabs" ]; then
    echo "COULD NOT RUN: found no tab in $APP — the gate's reading of the nav is out of date."
    exit 2
fi

missing=0
count=0
while IFS= read -r tab; do
    [ "$tab" = "Help" ] && continue
    count=$((count + 1))
    if ! grep -qiE "^##+ +${tab}( |$)" "$HELP"; then
        echo "MISSING: the app has a '$tab' tab and help.md has no '## $tab' section."
        missing=$((missing + 1))
    fi
done <<< "$tabs"

if [ "$missing" -ne 0 ]; then
    echo "help gate FAILED: $missing of $count tabs have no section."
    exit 1
fi
echo "help gate: all $count tabs have a section."
exit 0
