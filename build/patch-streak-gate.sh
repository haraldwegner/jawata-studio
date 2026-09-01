#!/usr/bin/env bash
#
# THE PATCH-STREAK GATE — Sprint 28d, S9a.2.
#
# A fix whose failure is DIFFERENT from the last one is not fixing the defect,
# it is moving it. Two in a row means stop fixing and run the architect.
#
# Harald, 2026-08-30, correcting how the alarm had been read for weeks:
#
#   "every time you made a change, something different failed. This is what
#    this should be about. If you make a fix the symptom is not the same but
#    something different breaks."
#
# WHY A SCRIPT AND NOT A NOTE. That alarm was already written down, twice, and
# did not fire on 2026-08-13 (eight studio releases, one design flaw) or on
# 2026-08-29 (four autocontinue fixes, four different breakages). A control that
# has failed twice on the channel it lives on does not get a third chance on
# that channel.
#
# THIS GATE CHECKS A PROXY AND SAYS SO. "The symptom changed" needs meaning, and
# a script has none. Substituting a text-shape for a meaning is exactly the
# defect that produced those four bad fixes — so the difference here is the
# DIRECTION OF ERROR, stated deliberately:
#
#   this proxy must OVER-fire. A false alarm costs one architect run.
#   A missed alarm costs a day. It has cost two.
#
# THE PROXY: release density. A patch release with ANY other release on this
# product inside the trailing window. Version numbers and tag dates only —
# nothing classified, nothing parsed, nothing to get wrong.
#
# WHY DENSITY AND NOT `major.minor` LINEAGE. The first specification keyed on
# "the previous tag on the same major.minor was also a patch". Replayed against
# the run that produced this gate, it went GREEN on v3.17.1 — the FOURTH
# autocontinue fix, the last one, the one Harald's sentence is about — because
# a minor bump (v3.17.0, an unrelated feature) landed mid-run and reset the
# lineage. It would have caught two of four in its own motivating instance.
# Density has no lineage to reset.
#
# THE OVERRIDE IS HIS, NOT THE RELEASING AGENT'S. The first design cleared a red
# streak on "the release notes carry an architect verdict line" — a line of
# markdown written by the party being gated, in a file it already writes every
# release. The gate cannot tell a real verdict from a sentence typed to clear a
# red build, so the control reduced to "when you are on a streak, write a
# sentence". That is the memory note's third wording relocated into YAML.
#
# So the escape is an environment value the agent cannot write: STREAK_OVERRIDE,
# supplied by a workflow_dispatch input or a repository variable Harald sets, or
# a re-run he triggers. The release-notes verdict line stays, DEMOTED to the
# audit trail — what the architect said — and is no longer the key, which was
# that it ran.
#
# USAGE
#   build/patch-streak-gate.sh <tag>            # the tag being released
#   build/patch-streak-gate.sh <tag> --explain  # also print the window
#
# EXIT
#   0  no streak, or overridden by him
#   1  STREAK — stop and run the architect over the run, not the last patch
#   2  misuse (bad tag, not a git repo)
#
set -uo pipefail

WINDOW_DAYS="${STREAK_WINDOW_DAYS:-7}"

die() { echo "patch-streak-gate: $*" >&2; exit 2; }

TAG="${1:-}"
[ -n "$TAG" ] || die "usage: $0 <tag> [--explain]"
EXPLAIN=false
[ "${2:-}" = "--explain" ] && EXPLAIN=true

git rev-parse --git-dir >/dev/null 2>&1 || die "not a git repository"

# --- the version must parse, or we are not judging what we think we are ---
if ! [[ "$TAG" =~ ^v?([0-9]+)\.([0-9]+)\.([0-9]+)$ ]]; then
    die "tag '$TAG' is not vMAJOR.MINOR.PATCH — refusing to guess"
fi
PATCH="${BASH_REMATCH[3]}"

# A minor or major bump is a FEATURE release and is never a streak member. This
# is load-bearing in the opposite direction from everything else here: it is the
# control that proves the gate is not simply blocking all releases.
if [ "$PATCH" -eq 0 ]; then
    echo "OK  $TAG is a feature release (patch component 0) — not a streak candidate."
    exit 0
fi

# --- when was this tag made? An unmade tag is judged as "now": the gate must
# --- work BEFORE the tag exists, which is when a release script would call it.
TAG_EPOCH=$(git log -1 --format=%ct "$TAG" 2>/dev/null)
if [ -z "$TAG_EPOCH" ]; then
    TAG_EPOCH=$(date +%s)
    WHEN="now (tag does not exist yet)"
else
    WHEN=$(date -u -d "@$TAG_EPOCH" +%Y-%m-%d\ %H:%M)
fi

CUTOFF=$(( TAG_EPOCH - WINDOW_DAYS * 86400 ))

# --- the OTHER PATCHES ON THIS SAME MINOR inside the trailing window ---
#
# IT USED TO COUNT EVERY RELEASE, and that is not the thing it is named for.
# This gate exists to catch "a run of patches where each one breaks something
# DIFFERENT from the last" — a series of repairs circling one unfixed defect.
# Counting neighbours makes a normal week of FEATURE work indistinguishable
# from that, which is the same mistake the gate is meant to detect: the
# reference measured is not the quantity meant.
#
# Measured 2026-09-01, which is why this changed. It refused v4.0.1 naming ten
# neighbours — but those ten were four feature releases on four different
# minors (3.14.0, 3.15.0, 3.16.0, 3.17.0), five patches spread across them, and
# one major. No streak at all. Meanwhile jawata-studio ran v3.17.0 through
# v3.17.6 — seven consecutive patches on ONE minor in three days — which is
# exactly the shape this gate describes and would have caught under the rule
# below.
#
# The predicate is now: patches sharing this tag's MAJOR.MINOR. A run is a run
# only if the repairs are landing on the same thing; a patch after a feature
# release is the system working, not a streak.
STREAK_SERIES="${BASH_REMATCH[1]}.${BASH_REMATCH[2]}."
NEIGHBOURS=()
while read -r t; do
    [ "$t" = "$TAG" ] && continue
    # same MAJOR.MINOR, and a PATCH of it (v1.2.3 where v1.2. matches and the
    # patch component is non-zero — v1.2.0 is the feature release the run would
    # be circling, not a member of it).
    case "$t" in
        "v${STREAK_SERIES}"*) ;;
        *) continue ;;
    esac
    [ "${t##*.}" = "0" ] && continue
    e=$(git log -1 --format=%ct "$t" 2>/dev/null) || continue
    [ -z "$e" ] && continue
    # Strictly BEFORE this tag, and inside the window. A later tag is not
    # evidence about this release — it has not happened yet.
    if [ "$e" -ge "$CUTOFF" ] && [ "$e" -lt "$TAG_EPOCH" ]; then
        NEIGHBOURS+=("$t|$(date -u -d "@$e" +%Y-%m-%d\ %H:%M)")
    fi
done < <(git tag -l 'v[0-9]*.[0-9]*.[0-9]*')

if $EXPLAIN; then
    echo "--- $TAG at $WHEN, window ${WINDOW_DAYS}d back to $(date -u -d "@$CUTOFF" +%Y-%m-%d\ %H:%M) ---"
    for n in "${NEIGHBOURS[@]:-}"; do [ -n "$n" ] && echo "    ${n%%|*}  ${n##*|}"; done
fi

if [ "${#NEIGHBOURS[@]}" -eq 0 ]; then
    echo "OK  $TAG is a patch with no other patch on v${STREAK_SERIES}x inside ${WINDOW_DAYS} days."
    exit 0
fi

# --- a streak. Only HE can clear it. ---
if [ -n "${STREAK_OVERRIDE:-}" ]; then
    echo "OVERRIDDEN  $TAG — streak of $(( ${#NEIGHBOURS[@]} + 1 )) releases cleared by STREAK_OVERRIDE='${STREAK_OVERRIDE}'."
    echo "            (This value cannot be written by the releasing agent. If it appears"
    echo "             without Harald having set it, THAT is the finding.)"
    exit 0
fi

echo "STREAK  $TAG is a patch release, and ${#NEIGHBOURS[@]} other release(s) landed on this"
echo "        product inside the last ${WINDOW_DAYS} days:"
for n in "${NEIGHBOURS[@]}"; do echo "          ${n%%|*}  ${n##*|}"; done
cat <<'WHY'

        WHAT THIS MEANS, and what it does NOT mean. It does not say this fix is
        wrong. It says the release pattern matches the one that has twice cost a
        day: a run of patches where each one breaks something DIFFERENT from the
        last, because the patches are moving one defect around rather than
        fixing it.

        THE ACTION IS NOT ANOTHER FIX. Run the architect seat as a watch-diff
        over the WHOLE run — not the last patch — and ask its question: design
        fix, or bandage?

        TO PROCEED ANYWAY: Harald sets STREAK_OVERRIDE (a workflow_dispatch
        input, a repository variable, or a re-run he triggers). Deliberately not
        anything the releasing agent can write — an escape hatch the gated party
        controls is not an escape hatch, it is a formality.
WHY
exit 1
