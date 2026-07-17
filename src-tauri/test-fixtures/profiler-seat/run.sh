#!/bin/sh
# The plain-java launch shape (EXECSIM mirror): a bash wrapper around a
# classpath main. The profiler seat's harness launches the SAME main under
# the dev/sim preset via debug(action=launch) instead of exec'ing this
# script — this file documents the production shape the fixture mirrors.
DIR="$(cd "$(dirname "$0")" && pwd)"
exec java -cp "$DIR/classes" com.example.ProfMain "$@"
