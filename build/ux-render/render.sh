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
exit $fail
