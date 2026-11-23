#!/usr/bin/env bash
# End-to-end test of setup.sh and start.sh on a fresh checkout, as the Windows CI runs it (in Git Bash):
# setup answered through stdin must save a projects folder Loft can open, build, and start.sh must serve Loft.
# `scripts/test-setup.sh` from a checkout that was never set up (no .loft/settings.json). Uses $LOFT_PORT (7420).
set -uo pipefail
cd "$(dirname "$0")/.."
ROOT=$(pwd)
PORT=${LOFT_PORT:-7420}
[ -e .loft/settings.json ] && { echo "This checkout is already set up (.loft/settings.json); run the test on a fresh one." >&2; exit 2; }
case "$(uname -s)" in MINGW*|MSYS*) WINDOWS=1 ;; *) WINDOWS=0 ;; esac
T=$(mktemp -d); START_PID=""
# Stops the Loft start.sh started: the process listening on $PORT. Loft then exits with a plain code and start.sh ends.
stop_loft() {
  [ -n "$START_PID" ] || return 0
  local pid
  if [ "$WINDOWS" = 1 ]; then
    for pid in $(netstat -ano | tr -d '\r' | awk -v p=":$PORT" '$2 ~ p"$" && $4 == "LISTENING" {print $5}' | sort -u); do taskkill //F //PID "$pid" >/dev/null 2>&1; done
  else
    for pid in $(lsof -t -iTCP:"$PORT" -sTCP:LISTEN 2>/dev/null); do kill "$pid"; done
  fi
  wait "$START_PID" 2>/dev/null; START_PID=""
}
trap 'stop_loft; rm -rf "$T"' EXIT
fails=0
check() { if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1: expected [$3], got [$2]"; fails=1; fi; }

# A machine without Claude Code (the CI) gets a stand-in that is logged in.
if ! command -v claude >/dev/null; then
  mkdir -p "$T/bin"; printf '#!/bin/sh\necho %s\n' "'{\"loggedIn\": true}'" > "$T/bin/claude"; chmod +x "$T/bin/claude"
  export PATH="$T/bin:$PATH"
fi

# The answers: Enter for no backup, then the projects folder, given as Windows writes it on Windows (C:\Users\…).
mkdir -p "$T/projects"
projects=$(cd "$T/projects" && pwd)
answer=$projects; form='/*'
[ "$WINDOWS" = 1 ] && answer=$(cygpath -w "$projects") form='[A-Za-z]:/*'
printf '\n%s\n' "$answer" | bash setup.sh > "$T/setup.log" 2>&1
rc=$?
check "setup.sh succeeds" "$rc" 0
[ "$rc" = 0 ] || sed 's/^/    | /' "$T/setup.log"
saved=$(sed -n 's/.*"projects_dir": *"\([^"]*\)".*/\1/p' .loft/settings.json 2>/dev/null)
# Windows can name one folder several ways (C:\Users\RUNNER~1\…), so it's compared as a file, not as text.
check "settings.json has the projects folder" "$([ -n "$saved" ] && [ "$saved" -ef "$projects" ] && echo same)" same
check "in the form Loft opens ($form)" "$([[ $saved == $form && $saved != *\\* ]] && echo yes || echo "$saved")" yes
again=$(bash setup.sh </dev/null 2>&1 | grep -A2 '^Projects folder$')
check "a second run reads it back" "$(echo "$again" | sed -n 2p)" "  ✓ $saved (change it under Settings)"
check "and finds the folder" "$(echo "$again" | grep -c '^  !')" 0

# start.sh builds the web app, runs the release build and serves Loft.
if [ "$rc" = 0 ]; then
  bash start.sh > "$T/start.log" 2>&1 &
  START_PID=$!
  up=0
  for _ in $(seq 1 90); do
    curl -s -m 2 -o /dev/null "http://127.0.0.1:$PORT/api/info" && { up=1; break; }
    kill -0 "$START_PID" 2>/dev/null || break
    sleep 2
  done
  check "start.sh serves Loft on port $PORT" "$up" 1
  [ "$up" = 1 ] || sed 's/^/    | /' "$T/start.log" | tail -n 30
  stop_loft
fi
exit $fails
