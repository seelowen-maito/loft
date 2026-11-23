#!/usr/bin/env bash
# End-to-end test of setup.sh and start.sh on a fresh checkout, as the Windows CI runs it (in Git Bash):
# setup answered through stdin must save a projects folder Loft can open, build, and start.sh must serve Loft. On
# Windows, saying no leaves no logon task, and accepting it must create one task whose Loft comes back after a restart
# and a crash; a second checkout that takes the task over must stop the first one's Loft and serve its own.
# `scripts/test-setup.sh` from a checkout that was never set up (no .loft/settings.json). Uses $LOFT_PORT (7420).
set -uo pipefail
cd "$(dirname "$0")/.."
ROOT=$(pwd)
PORT=${LOFT_PORT:-7420}
[ -e .loft/settings.json ] && { echo "This checkout is already set up (.loft/settings.json); run the test on a fresh one." >&2; exit 2; }
case "$(uname -s)" in MINGW*|MSYS*) WINDOWS=1 ;; *) WINDOWS=0 ;; esac
T=$(mktemp -d); START_PID=""; B=""
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
# Stops the logon task's Loft (run-service.sh with all it started, so it can't start Loft again) in this checkout and
# the second one, and deletes the task.
remove_task() {
  [ "$WINDOWS" = 1 ] && MSYS_NO_PATHCONV=1 schtasks /Query /TN Loft >/dev/null 2>&1 || return 0
  local r p
  for r in "$ROOT" ${B:+"$B"}; do
    p=$(cat "$r/.loft/service.pid" 2>/dev/null)
    [ -n "$p" ] && [ -e "/proc/$p/winpid" ] && MSYS_NO_PATHCONV=1 taskkill /F /T /PID "$(cat "/proc/$p/winpid")" >/dev/null 2>&1
    rm -f "$r/.loft/service.pid" "$r/.loft/service.env"
  done
  MSYS_NO_PATHCONV=1 schtasks /End /TN Loft >/dev/null 2>&1
  MSYS_NO_PATHCONV=1 schtasks /Delete /TN Loft /F >/dev/null 2>&1
}
remove_checkout() { [ -n "$B" ] && git worktree remove --force "$B" >/dev/null 2>&1; B=""; }
trap 'stop_loft; remove_task; remove_checkout; rm -rf "$T"' EXIT
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
if [ "$WINDOWS" = 1 ] && [ "$rc" = 0 ]; then
  # No answer means no to the logon task: Loft runs only through start.sh.
  check "no to the logon task skips it" "$(grep -A1 '^Background task$' "$T/setup.log" | grep -c '✓ skipped')" 1
  check "and leaves no task" "$(MSYS_NO_PATHCONV=1 schtasks /Query /TN Loft >/dev/null 2>&1 && echo found || echo "not found")" "not found"
fi

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

    # A second checkout (a worktree, with this one's release build so it doesn't start from scratch) takes the task
    # over: this checkout's Loft stops and the other one's serves the port.
    B=$T/loft-b
    if git worktree add -q --detach "$B" HEAD && mkdir -p "$B/backend/target" && cp -r backend/target/release "$B/backend/target/"; then
      old=$(cat .loft/service.pid 2>/dev/null)
      printf '\n%s\ny\n' "$answer" | (cd "$B" && bash setup.sh) > "$T/switch.log" 2>&1
      check "setup.sh in a second checkout succeeds" "$?" 0
      check "it sees the task runs this one" "$(grep -c 'the logon task runs another copy of loft' "$T/switch.log")" 1
      check "and switches it" "$(grep -c '✓ installed (task "Loft"' "$T/switch.log")" 1
      xml=$(MSYS_NO_PATHCONV=1 schtasks /Query /XML /TN Loft 2>/dev/null | tr -d '\0\r' | sed 's/&quot;/"/g')
      check "the task runs the second checkout's run-service.sh" "$(echo "$xml" | grep -cF "\"$(cygpath -m "$B")/scripts/run-service.sh\"")" 1
      check "this checkout's run-service.sh stopped" "$([ -n "$old" ] && [ ! -e "/proc/$old" ] && echo stopped || echo running)" stopped
      wait_for 60 1
      pid=$(netstat -ano | tr -d '\r' | awk -v p=":$PORT" '$2 ~ p"$" && $4 == "LISTENING" {print $5}' | sort -u | head -1)
      exe=$([ -n "$pid" ] && powershell -NoProfile -Command "(Get-Process -Id $pid).Path" 2>/dev/null | tr -d '\r')
      check "the second checkout's Loft serves port $PORT" "$([ -n "$exe" ] && [ "$(cygpath -u "$exe")" -ef "$B/backend/target/release/loft.exe" ] && echo second || echo "${exe:-nothing}")" second
      [ "$(grep -c '✓ installed (task "Loft"' "$T/switch.log")" = 1 ] || sed 's/^/    | /' "$T/switch.log" | tail -n 30
    else
      check "a second checkout" "couldn't make it" made
    fi
  fi
  remove_task
  remove_checkout
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
