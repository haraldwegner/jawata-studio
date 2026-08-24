#!/bin/sh
# Install the commit guards into one or more repositories.
#
# WHY THIS SCRIPT EXISTS: git hooks live in .git/hooks, which is NOT tracked and
# does NOT survive a clone. The commit-msg guard had been on this machine for
# months and existed nowhere else — so a fresh clone, a new machine or a CI
# checkout enforced nothing, silently. The canonical copies are here; this puts
# them where git will run them.
#
#   ./hooks/install.sh ../jawata-mcp ../jawata-studio ../jawata-enterprise
#
# Idempotent. Re-run after changing a hook.
set -e
here=$(cd "$(dirname "$0")" && pwd)
[ $# -gt 0 ] || { echo "usage: $0 <repo> [repo...]" >&2; exit 2; }
for repo in "$@"; do
    [ -d "$repo/.git" ] || { echo "skip: $repo is not a git repository" >&2; continue; }
    for hook in pre-commit commit-msg; do
        cp "$here/$hook" "$repo/.git/hooks/$hook"
        chmod +x "$repo/.git/hooks/$hook"
    done
    echo "installed pre-commit + commit-msg -> $repo"
done
