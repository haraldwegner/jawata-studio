#!/usr/bin/env bash
# Renders every tab of the REAL frontend, against build/ux-render/mock.js, into PNGs.
# This is the ux seat's "render it, do not read class names" check. It starts its own vite on a free
# port and its own headless Chrome profile; it never touches the installed studio or your own browser.
# Usage: build/ux-render/render.sh <out-dir> [width] [height]
# Exit:  0 = every shot rendered the app (a PNG, and a DOM with its header and no error overlay) · 1 = a shot did not render · 2 = the render could not run
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="${1:?usage: render.sh <out-dir> [width] [height]}"
W="${2:-1400}"
H="${3:-1000}"
mkdir -p "$OUT"
CHROME="$(command -v google-chrome || command -v chromium || command -v chromium-browser || true)"
if [ -z "$CHROME" ]; then
    echo "COULD NOT RUN: no Chrome or Chromium on this machine."
    exit 2
fi
PORT=$((14000 + RANDOM % 1000))
( cd "$ROOT" && exec npx vite --port "$PORT" --host 127.0.0.1 --strictPort >"$OUT/vite.log" 2>&1 ) &
VITE=$!
trap 'kill $VITE 2>/dev/null' EXIT
URL="http://127.0.0.1:$PORT/build/ux-render/mock.html"
for _ in $(seq 1 60); do curl -fs "$URL" >/dev/null 2>&1 && break; sleep 0.5; done
if ! curl -fs "$URL" >/dev/null 2>&1; then
    echo "COULD NOT RUN: vite did not start (see $OUT/vite.log)."
    exit 2
fi
PROFILE="$(mktemp -d)"
fail=0
shoot() { # name, query
    "$CHROME" --headless=new --disable-gpu --no-sandbox --user-data-dir="$PROFILE" --hide-scrollbars \
        --window-size="$W,$H" --virtual-time-budget=9000 --screenshot="$OUT/$1.png" "$URL?$2" >/dev/null 2>&1
    # A PNG exists for an error page too (C14: Vite's error page rendered, the script said "rendered"): the same page's DOM must
    # hold the app's own header and no Vite error overlay, or the shot is a failure.
    "$CHROME" --headless=new --disable-gpu --no-sandbox --user-data-dir="$PROFILE" \
        --window-size="$W,$H" --virtual-time-budget=9000 --dump-dom "$URL?$2" > "$OUT/$1.html" 2>/dev/null
    if [ ! -s "$OUT/$1.png" ]; then
        echo "FAILED to render $1: no PNG"; fail=1
    elif grep -q "vite-error-overlay" "$OUT/$1.html" || ! grep -q "<h1>jawata-studio</h1>" "$OUT/$1.html"; then
        echo "FAILED to render $1: the page is not the app (error overlay, or no jawata-studio header); DOM: $OUT/$1.html"; fail=1
    else
        echo "rendered $1.png"
    fi
}
for tab in Dashboard Memory Reporting Seats Settings Help; do
    t=$(echo "$tab" | tr 'A-Z' 'a-z')
    shoot "$t" "tab=$tab"
    shoot "$t-open" "tab=$tab&open=1"
done
shoot "reporting-attention" "tab=Reporting&scenario=attention"
# What a shot SHOWS, as a browser joins it: tags and comments removed with no space put in their place,
# whitespace runs collapsed. A space lost at the edge of a template block shows up here and nowhere else
# (S19-24: "189of 11622 rows· 13 cures", "describedin", "answering(62 ms)"). The mock's own figures are fixed,
# so the exact sentences can be named.
shown() { sed -e 's/<!--[^>]*-->//g; s/<[^>]*>//g' "$OUT/$1.html" | tr -s ' \n\t' ' '; }
# The page text is captured into a variable first: `shown | grep -q` lets grep leave early on a match, shown then dies on
# SIGPIPE under pipefail, and a `reject` would report "does not show" for text that IS there (audit S19-35 c).
expect() { # shot, text it must show
    local text; text="$(shown "$1")"
    if grep -qF -- "$2" <<<"$text"; then echo "shows ($1): $2"; else echo "FAILED: $1 does not show \"$2\""; fail=1; fi
}
reject() { # shot, text it must not show
    local text; text="$(shown "$1")"
    if grep -qF -- "$2" <<<"$text"; then echo "FAILED: $1 shows \"$2\""; fail=1; else echo "does not show ($1): $2"; fi
}
# S19-36/37/38 (Memory): the store's figures are plain content, the pattern line no longer repeats the row count, the
# cataloguer sample is gone.
expect memory "Pattern library: 13 cures, all resolved"
expect memory "406 units described in 4 bundles · 54 areas summarised"
reject memory "What the cataloguer wrote"
reject memory "189 of 11622 rows"
# S19-40/41/42 (Reporting): the totals line, every shape on the page (no "5 more"), the recall figures as one sentence.
expect reporting "knowledge store answering (62 ms) · workspaces answering"
expect reporting "8 failure shapes recorded · 0 filed · 0 still to report"
expect reporting "4789 taken and never answered (the store was asked, its answer not used)"
reject reporting "5 more"
# S19-25: the fold "Workspaces not answering" names the workspace that does not answer, and only that one.
expect reporting-attention "patterns store: timed out"
reject reporting-attention "javata-dev store:"
reject reporting-attention "orb-strategy store:"
# S19-46/47 (Dashboard): the Workspaces paragraph is a mouseover now, and the Managed Projects line carries no project count.
reject dashboard "share a single MCP service"
expect dashboard "3 of 3 workspaces running"
reject dashboard "workspaces running ·"
# S19-46/51/52 (Settings): the opening sentence names no workspace count; Paths drops the two count rows.
expect settings "deploying to 6 of 6 agents"
reject settings "3 running"
reject settings-open "Project count"
# S19-53 (Help): the subtitle is gone.
reject help "Learn how to configure and deploy"
# S19-56/58 (his word, 2026-10-25): Agent deploy is its own panel, and no view says "resident" (the word is the engineers').
expect dashboard "Agent deploy"
expect reporting-attention "Workspaces not answering"
reject dashboard "esident"
reject memory "esident"
reject reporting "esident"
reject reporting-attention "esident"
reject settings-open "esident"
exit $fail
