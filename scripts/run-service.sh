#!/usr/bin/env bash
# Runs loft for the Windows logon task that setup.sh installs (the launchd service's counterpart): the release build,
# started again right away when it exits with Loft's restart code (after an update, or a restart from the web app),
# and 10 seconds after any other exit (a crash), as KeepAlive does on the Mac. Everything goes to $LOFT_SERVICE_LOG.
# Keep RESTART_CODE in step with backend/src/updater.rs.
set -uo pipefail
RESTART_CODE=75
cd "$(dirname "$0")/.."
# The task starts with Windows' own environment: this brings the PATH and port setup.sh ran with.
. .loft/service.env
# setup.sh stops this process, and everything it started, to reinstall the task or restore a backup.
echo $$ > .loft/service.pid
mkdir -p "$(dirname "$LOFT_SERVICE_LOG")"
cd backend
while :; do
  echo "loft: starting ($(date '+%Y-%m-%d %H:%M:%S'))"
  ./target/release/loft.exe
  code=$?
  if [ "$code" -eq "$RESTART_CODE" ]; then echo "loft: restarting"
  else echo "loft: exited with code $code; starting again in 10s"; sleep 10; fi
done >> "$LOFT_SERVICE_LOG" 2>&1
