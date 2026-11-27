#!/usr/bin/env bash
# Runs the backend with `cargo run <args>`, and runs it again whenever it exits with Loft's restart code (after an
# update, or a restart from the web app). Any other exit ends this script with that code. Used by start.sh and dev.sh.
# Keep RESTART_CODE in step with backend/src/updater.rs.
set -uo pipefail
RESTART_CODE=75
# Tells Loft to restart by exiting with that code, rather than replacing itself (exec), which leaves this loop.
export LOFT_RESTART_BY_EXIT=1
cd "$(dirname "$0")/../backend"
while :; do
  cargo run "$@"
  code=$?
  [ "$code" -eq "$RESTART_CODE" ] || exit "$code"
  echo "loft: restarting" >&2
done
