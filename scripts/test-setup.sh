#!/usr/bin/env bash
# End-to-end test of setup.sh and start.sh on a fresh checkout, as the Windows CI runs it (in Git Bash):
# setup answered through stdin must save a projects folder Loft can open, build, and start.sh must serve Loft. On
# Windows, accepting the logon task must create one task whose Loft comes back after a restart and a crash.
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
# Stops the logon task's Loft (run-service.sh with all it started, so it can't start Loft again) and deletes the task.
remove_task() {
  [ "$WINDOWS" = 1 ] && MSYS_NO_PATHCONV=1 schtasks /Query /TN Loft >/dev/null 2>&1 || return 0
  local p; p=$(cat .loft/service.pid 2>/dev/null)
  [ -n "$p" ] && [ -e "/proc/$p/winpid" ] && MSYS_NO_PATHCONV=1 taskkill /F /T /PID "$(cat "/proc/$p/winpid")" >/dev/null 2>&1
  MSYS_NO_PATHCONV=1 schtasks /End /TN Loft >/dev/null 2>&1
  MSYS_NO_PATHCONV=1 schtasks /Delete /TN Loft /F >/dev/null 2>&1
  rm -f .loft/service.pid .loft/service.env
}
trap 'stop_loft; remove_task; rm -rf "$T"' EXIT
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

# Windows: yes to the logon task creates it and starts its Loft hidden; a second run updates it without a second task.
if [ "$WINDOWS" = 1 ] && [ "$rc" = 0 ]; then
  answers() { curl -s -m 2 -o /dev/null "http://127.0.0.1:$PORT/api/info"; }
  # Waits up to $1 seconds for Loft to answer (1) or not (0), as $2 says.
  wait_for() { for _ in $(seq 1 "$1"); do if answers; then [ "$2" = 1 ] && return 0; else [ "$2" = 0 ] && return 0; fi; sleep 1; done; return 1; }
  # Enter for no backup, then yes to the task.
  printf '\ny\n' | bash setup.sh > "$T/task.log" 2>&1
  check "setup.sh with the logon task succeeds" "$?" 0
  check "it says the task is installed" "$(grep -c '✓ installed (task "Loft"' "$T/task.log")" 1
  xml=$(MSYS_NO_PATHCONV=1 schtasks /Query /XML /TN Loft 2>/dev/null | tr -d '\0\r' | sed 's/&quot;/"/g')
  hidden=$(echo "$xml" | grep -cF "<Arguments>-w hide /usr/bin/bash \"$(cygpath -m "$ROOT")/scripts/run-service.sh\"</Arguments>")
  check "the task runs this checkout's run-service.sh, hidden" "$hidden" 1
  [ "$hidden" = 1 ] || echo "$xml" | grep -A3 '<Exec>' | sed 's/^/    | /'
  check "at logon" "$(echo "$xml" | grep -c '<LogonTrigger>')" 1
  check "its Loft serves on port $PORT" "$(grep -c 'loft is running' "$T/task.log")" 1
  [ "$(grep -c 'loft is running' "$T/task.log")" = 1 ] || { sed 's/^/    | /' "$T/task.log" | tail -n 20; cat "$LOCALAPPDATA/Loft/loft.log" 2>/dev/null | tail -n 20 | sed 's/^/    log | /'; }
  printf '\n\n' | bash setup.sh > "$T/task2.log" 2>&1
  check "a second run reinstalls it (the default)" "$(grep -c '✓ installed (task "Loft"' "$T/task2.log")" 1
  check "still one task" "$(MSYS_NO_PATHCONV=1 schtasks /Query /FO CSV /NH 2>/dev/null | tr -d '\r' | grep -c '^"\\Loft"')" 1
  if answers; then
    # A restart from the web app exits with the restart code: the task starts Loft again right away.
    curl -s -m 5 -o /dev/null -X POST -H 'Content-Type: application/json' -d '{}' "http://127.0.0.1:$PORT/api/restart"
    wait_for 20 0; check "after a restart, Loft is back" "$(wait_for 60 1 && echo 1 || echo 0)" 1
    # A crash (killed): back after run-service.sh's 10 seconds.
    for pid in $(netstat -ano | tr -d '\r' | awk -v p=":$PORT" '$2 ~ p"$" && $4 == "LISTENING" {print $5}' | sort -u); do taskkill //F //PID "$pid" >/dev/null 2>&1; done
    wait_for 20 0; check "after a crash, Loft is back" "$(wait_for 60 1 && echo 1 || echo 0)" 1
    grep '^loft: ' "$LOCALAPPDATA/Loft/loft.log" 2>/dev/null | sed 's/^/    log | /'
  fi
  remove_task
  wait_for 20 0
fi

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
