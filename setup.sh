#!/usr/bin/env bash
# Install or check loft: requirements, first-run settings, build, and a background service (macOS: launchd, Windows: a
# logon task).
# Runs on macOS, Linux and Windows (in Git Bash). Safe to re-run; it reports what's missing and how to fix it.
set -uo pipefail
LAUNCH=$(pwd)
cd "$(dirname "$0")"
ROOT=$(pwd)
# How hints name this folder's scripts: ./setup.sh from here, a full path when run from elsewhere (the installer).
case $ROOT in "$LAUNCH") HERE=./ ;; "$HOME"/*) HERE="~${ROOT#"$HOME"}/" ;; *) HERE=$ROOT/ ;; esac
PORT=${LOFT_PORT:-7420}
SETTINGS=.loft/settings.json
LABEL=dev.loft
PLIST=~/Library/LaunchAgents/$LABEL.plist
# Windows' counterpart: a logon task that runs scripts/run-service.sh hidden.
TASK=Loft
BIN=$ROOT/backend/target/release/loft
MACOS=$([ "$(uname)" = Darwin ] && echo 1 || echo 0)
case "$(uname -s)" in MINGW*|MSYS*) WINDOWS=1 BIN=$BIN.exe ;; *) WINDOWS=0 ;; esac
# Where the Windows logon task's loft writes its output (the Mac's is ~/Library/Logs/loft.log).
[ "$WINDOWS" = 1 ] && SERVICE_LOG=$(cygpath -u "${LOCALAPPDATA:-$HOME/AppData/Local}")/Loft/loft.log

if [ -t 1 ]; then B=$'\e[1m' G=$'\e[32m' Y=$'\e[33m' R=$'\e[31m' D=$'\e[2m' N=$'\e[0m'; else B="" G="" Y="" R="" D="" N=""; fi
failed=0
section() { printf '\n%s%s%s\n' "$B" "$1" "$N"; }
ok() { printf '  %s✓%s %s\n' "$G" "$N" "$1"; }
warn() { printf '  %s!%s %s\n' "$Y" "$N" "$1"; }
fail() { printf '  %s✗%s %s\n' "$R" "$N" "$1"; failed=1; }
tip() { printf '    %s%s%s\n' "$D" "$1" "$N"; }
# How to install a package here: Homebrew on macOS, winget on Windows (its id is $2), else the distro's package manager.
pkg() {
  if [ "$WINDOWS" = 1 ]; then echo "winget install -e --id $2"
  elif [ "$MACOS" = 1 ]; then
    if command -v brew >/dev/null; then echo "brew install $1"; else echo "install Homebrew (https://brew.sh), then: brew install $1"; fi
  elif command -v apt-get >/dev/null; then echo "sudo apt-get install -y $1"
  elif command -v dnf >/dev/null; then echo "sudo dnf install -y $1"
  elif command -v pacman >/dev/null; then echo "sudo pacman -S $1"
  else echo "install $1 with your package manager"; fi
}
# Run a command behind a spinner; its output goes to $LOG, shown by the caller on failure.
LOG=$(mktemp -t loft-setup.XXXXXX)
SPIN_PID=""
trap '[ -n "$SPIN_PID" ] && kill "$SPIN_PID" 2>/dev/null; [ -t 1 ] && printf "\e[?25h"; rm -f "$LOG"' EXIT
trap 'exit 130' INT TERM
spin() {
  local label=$1; shift
  : >"$LOG"
  if [ ! -t 1 ]; then "$@" >>"$LOG" 2>&1; return; fi
  "$@" >>"$LOG" 2>&1 &
  SPIN_PID=$!
  local frames=(⠋ ⠙ ⠹ ⠸ ⠼ ⠴ ⠦ ⠧ ⠇ ⠏) i=0 start=$SECONDS t
  printf '\e[?25l'
  while kill -0 "$SPIN_PID" 2>/dev/null; do
    t=$((SECONDS - start)); [ "$t" -ge 3 ] && t=" ${D}${t}s${N}" || t=""
    printf '\r  %s%s%s %s%s\e[K' "$G" "${frames[i++ % 10]}" "$N" "$label" "$t"
    sleep 0.08
  done
  wait "$SPIN_PID"; local rc=$?
  SPIN_PID=""
  printf '\r\e[K\e[?25h'
  return $rc
}
# The last lines of a failed step's output.
show_log() { grep -v '^[[:space:]]*$' "$LOG" | tail -n "${1:-8}" | sed "s/^/    ${D}│${N} /"; }
# Answers come from stdin (so they can be piped); at EOF the default is used.
ask() { local a=""; read -r -p "  $1 " a || true; echo "${a:-$2}"; }
# A string field from settings.json (flat or nested key, first match), with JSON's \\ read back as \ (C:\\Users).
setting() { [ -f "$SETTINGS" ] && sed -n "s/.*\"$1\": *\"\([^\"]*\)\".*/\1/p" "$SETTINGS" | head -1 | sed 's/\\\\/\\/g'; }
# AXe, which lets the Simulator page tap and type. Same places the backend looks (simulator.rs).
find_axe() {
  local p
  for p in "$(command -v axe)" ~/.local/bin/axe /opt/homebrew/bin/axe /usr/local/bin/axe; do
    [ -n "$p" ] && [ -x "$p" ] && { echo "$p"; return 0; }
  done
  return 1
}
install_axe() {
  command -v brew >/dev/null || return 1
  brew tap cameroncooke/axe >/dev/null 2>&1 || return 1
  brew trust --formula cameroncooke/axe/axe >/dev/null 2>&1
  brew install cameroncooke/axe/axe >/dev/null 2>&1 && return 0
  # Homebrew refuses while the Command Line Tools lag behind Xcode. Install the formula's release by hand instead.
  local formula url sha dir=~/.local/opt/axe tmp
  formula=$(brew --repository cameroncooke/axe)/Formula/axe.rb
  url=$(sed -n 's/^ *url "\(.*\)"/\1/p' "$formula"); sha=$(sed -n 's/^ *sha256 "\(.*\)"/\1/p' "$formula")
  [ -n "$url" ] && [ -n "$sha" ] || return 1
  tmp=$(mktemp -d)
  curl -fsSL -o "$tmp/axe.tar.gz" "$url" && echo "$sha  $tmp/axe.tar.gz" | shasum -a 256 -c >/dev/null 2>&1 || { rm -rf "$tmp"; return 1; }
  rm -rf "$dir" && mkdir -p "$dir" ~/.local/bin && tar xzf "$tmp/axe.tar.gz" -C "$dir"; rm -rf "$tmp"
  local f
  for f in "$dir"/Frameworks/*.framework "$dir/axe"; do codesign --force --sign - --timestamp=none "$f" 2>/dev/null; done
  printf '#!/bin/bash\nexec "%s/axe" "$@"\n' "$dir" > ~/.local/bin/axe && chmod +x ~/.local/bin/axe
  ~/.local/bin/axe --version >/dev/null 2>&1
}
running_agents() { curl -s -m 2 "http://127.0.0.1:$PORT/api/settings" | sed -n 's/.*"running_agents": *\([0-9]*\).*/\1/p'; }
# Windows commands take /flags, which Git Bash would turn into paths.
win() { MSYS_NO_PATHCONV=1 "$@"; }
xml_escape() { sed 's/&/\&amp;/g; s/</\&lt;/g; s/>/\&gt;/g; s/"/\&quot;/g'; }
# The checkout whose loft the logon task runs (C:/…), read from its command line; empty when setup didn't make it.
task_root() {
  win schtasks /Query /XML /TN "$TASK" 2>/dev/null | tr -d '\0\r' \
    | sed -n 's|.*<Arguments>.*&quot;\(.*\)/scripts/run-service.sh&quot;.*|\1|p; s|.*<Arguments>.*"\(.*\)/scripts/run-service.sh".*|\1|p' \
    | head -1 | sed 's/&quot;/"/g; s/&lt;/</g; s/&gt;/>/g; s/&amp;/\&/g'
}
# The Windows process id of that checkout's run-service.sh while it runs (its pid file holds Git Bash's pid).
task_pid() {
  local root p; root=$(task_root); [ -n "$root" ] || return 1
  p=$(cat "$root/.loft/service.pid" 2>/dev/null) && [ -n "$p" ] || return 1
  tr '\0' ' ' < "/proc/$p/cmdline" 2>/dev/null | grep -q 'scripts/run-service\.sh' && cat "/proc/$p/winpid"
}
# Stops the task's loft: run-service.sh with everything it started, so it can't start loft again.
task_stop() {
  local p
  p=$(task_pid) && win taskkill /F /T /PID "$p" >/dev/null 2>&1
  win schtasks /End /TN "$TASK" >/dev/null 2>&1
  for _ in $(seq 1 20); do task_pid >/dev/null || break; sleep 0.5; done
  return 0
}
task_start() { win schtasks /Run /TN "$TASK" >/dev/null 2>&1; }
# Writes the task's environment (.loft/service.env: this shell's PATH, the port, the log) and (re)creates the task:
# at this user's logon, run-service.sh in Git Bash's mintty with its window hidden, as this user, with no time limit,
# also on battery. run-service.sh starts loft again after an update or a crash; Windows retries if it can't start.
task_install() {
  local me=${USERDOMAIN:+$USERDOMAIN\\}$USERNAME xml=.loft/service.xml
  mkdir -p .loft "$(dirname "$SERVICE_LOG")"
  { printf 'export PATH=%q\n' "$PATH"; printf 'export LOFT_PORT=%q\n' "$PORT"; printf 'export LOFT_SERVICE_LOG=%q\n' "$SERVICE_LOG"; } > .loft/service.env
  me=$(printf '%s' "$me" | xml_escape)
  { printf '\xff\xfe'; iconv -f UTF-8 -t UTF-16LE <<EOF; } > "$xml"
<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Runs loft ($(cygpath -m "$ROOT" | xml_escape)) at logon, hidden. Made by setup.sh: run it again to update or switch it.</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger><Enabled>true</Enabled><UserId>$me</UserId></LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author"><UserId>$me</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <RestartOnFailure><Interval>PT1M</Interval><Count>999</Count></RestartOnFailure>
    <IdleSettings><StopOnIdleEnd>false</StopOnIdleEnd><RestartOnIdle>false</RestartOnIdle></IdleSettings>
    <Priority>5</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>$(cygpath -w /usr/bin/mintty.exe | xml_escape)</Command>
      <Arguments>$(printf -- '-w hide /usr/bin/bash "%s/scripts/run-service.sh"' "$(cygpath -m "$ROOT")" | xml_escape)</Arguments>
      <WorkingDirectory>$(cygpath -w "$ROOT/backend" | xml_escape)</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
EOF
  # The running loft stops first: once re-created, the task names this checkout, and another checkout's
  # run-service.sh (which keeps the port) could no longer be found. If schtasks fails, the old task starts again.
  local was=0; task_pid >/dev/null && was=1
  task_stop
  win schtasks /Create /F /TN "$TASK" /XML "$(cygpath -w "$ROOT/$xml")" >"$LOG" 2>&1; local rc=$?
  rm -f "$xml"
  [ $rc = 0 ] || { [ $was = 1 ] && task_start; return 1; }
  task_start
}

section "Requirements"
if command -v git >/dev/null; then ok "git"
else fail "git is missing"; if [ "$MACOS" = 1 ]; then tip "Run: xcode-select --install   (Apple's command line tools, includes git)"; else tip "Run: $(pkg git Git.Git)"; fi; fi

if cargo_version=$(cargo --version 2>/dev/null); then
  ok "Rust ($(echo "$cargo_version" | cut -d' ' -f2))"
  # Loft builds with the MSVC toolchain on Windows (docs/adr/0005). No grep -q in these pipes: under pipefail, its early
  # exit can fail the pipe.
  if [ "$WINDOWS" = 1 ] && ! rustc -vV 2>/dev/null | tr -d '\r' | grep '^host: .*-msvc$' >/dev/null; then
    fail "Rust's default toolchain isn't MSVC"; tip "Run: rustup default stable-x86_64-pc-windows-msvc"
  fi
elif command -v rustup >/dev/null; then
  fail "Rust is installed but has no toolchain"; tip "Run: rustup default stable"
elif [ -x ~/.cargo/bin/cargo ]; then
  fail "Rust is installed but not on your PATH"
  if [ "$WINDOWS" = 1 ]; then tip "Open a new Git Bash, then run setup again."
  else tip "Run: . \"\$HOME/.cargo/env\"   (or open a new terminal), then run setup again."; fi
elif [ "$WINDOWS" = 1 ]; then
  fail "Rust (cargo) is missing"; tip "Run: $(pkg rustup Rustlang.Rustup)"
else
  fail "Rust (cargo) is missing"
  tip "Run: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y"
  tip "then open a new terminal so cargo is on your PATH."
fi

# Rust links with the Visual Studio Build Tools' C++ tools on Windows. vswhere comes with any Visual Studio install.
if [ "$WINDOWS" = 1 ]; then
  vc=""
  for vswhere in "$(printenv 'ProgramFiles(x86)' 2>/dev/null)" "/c/Program Files (x86)"; do
    vswhere="$vswhere/Microsoft Visual Studio/Installer/vswhere.exe"
    [ -f "$vswhere" ] || continue
    vc=$("$vswhere" -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2>/dev/null | tr -d '\r')
    break
  done
  if [ -n "$vc" ]; then ok "Visual Studio C++ build tools"
  else
    fail "the Visual Studio C++ build tools are missing (Rust needs them to link)"
    tip "Run: winget install -e --id Microsoft.VisualStudio.2022.BuildTools --override \"--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended\""
  fi
fi

node_major=$(node -v 2>/dev/null | sed 's/^v\([0-9]*\).*/\1/')
if [ -n "$node_major" ] && [ "$node_major" -ge 22 ] && command -v node >/dev/null; then ok "Node $(node -v)"
else
  if [ -n "$node_major" ]; then fail "Node 22 or newer is needed (found $(node -v))"; else fail "Node is missing (22 or newer)"; fi
  if [ "$WINDOWS" = 1 ]; then tip "Run: $(pkg node OpenJS.NodeJS.LTS)"
  elif [ -s "${NVM_DIR:-$HOME/.nvm}/nvm.sh" ]; then tip "Run: nvm install 22 && nvm alias default 22"
  elif [ "$MACOS" = 1 ] && command -v brew >/dev/null; then tip "Run: brew install node   (or use nvm: https://github.com/nvm-sh/nvm)"
  else tip "Install nvm (https://github.com/nvm-sh/nvm), open a new terminal, then: nvm install 22"; fi
fi

claude_bin=$(setting claude_bin); claude_bin=${claude_bin:-claude}
if command -v "$claude_bin" >/dev/null; then
  if "$claude_bin" auth status 2>/dev/null | grep -q '"loggedIn": *true'; then ok "Claude Code, logged in"
  elif [ -n "${SSH_CONNECTION:-}" ] && [ "$MACOS" = 1 ]; then
    # Over SSH the login keychain is out of reach, so a valid login reads as logged out.
    warn "Claude Code login can't be checked over SSH"; tip "If agents fail to start, run $claude_bin on this Mac's screen and log in."
  else fail "Claude Code isn't logged in"; tip "Run: $claude_bin   then log in (agents use this login)."; fi
else
  fail "Claude Code ($claude_bin) not found"
  if [ "$WINDOWS" = 1 ]; then tip "Run: $(pkg claude Anthropic.ClaudeCode)   then run claude once to log in."
  else tip "Run: curl -fsSL https://claude.ai/install.sh | bash   then run claude once to log in."; fi
  tip "Already installed somewhere else? Set its path under Settings → Claude command."
fi

if command -v gh >/dev/null && gh auth status >/dev/null 2>&1; then ok "GitHub CLI, logged in"
elif command -v gh >/dev/null; then warn "GitHub CLI not logged in (optional: PRs, issues, private repos)"; tip "Run: gh auth login   or add a token under Settings → GitHub token"
else warn "GitHub CLI not installed (optional: PRs, issues, private repos)"; tip "Run: $(pkg gh GitHub.cli) && gh auth login   or add a token under Settings → GitHub token"; fi

# Agents' clones go deep (agent-local-storage/<id>/<repo>/…): past Windows' 260-character limit, git and builds fail
# unless long paths are on, in Windows and in git.
if [ "$WINDOWS" = 1 ]; then
  if MSYS_NO_PATHCONV=1 reg query 'HKLM\SYSTEM\CurrentControlSet\Control\FileSystem' /v LongPathsEnabled 2>/dev/null | tr -d '\r' | grep 'REG_DWORD *0x1$' >/dev/null; then
    ok "long paths on in Windows"
  else
    warn "long paths are off in Windows: agents' clones can fail past 260 characters"
    tip "In PowerShell as administrator: New-ItemProperty -Path HKLM:\\SYSTEM\\CurrentControlSet\\Control\\FileSystem -Name LongPathsEnabled -Value 1 -PropertyType DWord -Force"
  fi
  if [ "$(git config --get --type=bool core.longpaths 2>/dev/null)" = true ]; then ok "long paths on in git"
  else warn "long paths are off in git"; tip "Run: git config --global core.longpaths true"; fi
fi
if [ "$failed" = 1 ]; then
  printf '\n  %sFix the items above, then run setup again:%s\n    %ssetup.sh\n' "$R" "$N" "$HERE"
  [ "$WINDOWS" = 1 ] && tip "Open a new Git Bash first, so it finds what winget installed."
  printf '\n'; exit 1
fi

section "Restore a backup"
. scripts/restore-backup.sh
# A fresh install is offered a backup; one with data is asked, defaulting to No (and never when there's no terminal).
restore=1; skip="start fresh"
if [ -f .loft/loft.db ]; then
  restore=0; skip="keep the current data"
  if [ -t 0 ] && [[ $(ask "This install already has its data. Restore from a backup? [y/N]" n) =~ ^[Yy] ]]; then restore=1; fi
  [ $restore = 0 ] && ok "keeping this install's data ${D}(nothing restored)${N}"
fi
if [ $restore = 1 ]; then
  dirs=(); while IFS= read -r d; do dirs+=("$d"); done < <(backup_dirs)
  stopped=0
  # Only this checkout's own service is stopped, and only once a backup is chosen; it comes back after.
  resume_service() {
    if [ $stopped = 1 ]; then if [ "$WINDOWS" = 1 ]; then task_start; else launchctl bootstrap "gui/$(id -u)" "$PLIST" 2>/dev/null; fi; fi
    stopped=0; return 0
  }
  while :; do
    list=(); while IFS= read -r l; do [ -n "$l" ] && list+=("$l"); done < <(find_backups ${dirs[@]+"${dirs[@]}"})
    if [ ${#list[@]} -eq 0 ]; then
      ok "no backups found"
      answer=$(ask "Folder with backups to restore from, or Enter to $skip:" "")
    else
      i=0; for l in "${list[@]}"; do i=$((i + 1)); size=${l##*|}; rest=${l%|*}; date=${rest##*|}; src=${rest%|*}; src=${src%/*}; printf '    %s) %s  %s  %s\n' "$i" "$date" "$size" "${src/#$HOME/~}"; done
      answer=$(ask "Restore which one? A number, another folder, or Enter to $skip:" "")
    fi
    [ -z "$answer" ] && { ok "$skip"; break; }
    if [[ $answer =~ ^[0-9]+$ ]] && [ $((10#$answer)) -ge 1 ] && [ $((10#$answer)) -le ${#list[@]} ]; then
      chosen=${list[10#$answer - 1]}; chosen=${chosen%|*|*}
    elif [[ $answer =~ ^[0-9]+$ ]]; then
      warn "no backup $answer"; [ -t 0 ] || break; continue
    else
      answer=${answer/#\~/$HOME}
      if [ -d "$answer" ]; then dirs+=("$answer"); continue; fi
      warn "Not a folder: $answer"; [ -t 0 ] || break; continue
    fi
    [ -f .loft/loft.db ] && warn "your current data moves to .loft/before-restore/ (kept, not deleted)"
    # Loft writes its database while it runs: stop this checkout's service first.
    if [ $stopped = 0 ] && [ "$WINDOWS" = 1 ] && [ "$(task_root)" -ef "$ROOT" ] && task_pid >/dev/null; then
      task_stop; stopped=1; sleep 1
    elif [ $stopped = 0 ] && [ -f "$PLIST" ] && grep -qF -e "<string>$BIN</string>" -e "<string>$BIN-signed</string>" "$PLIST" \
      && launchctl print "gui/$(id -u)/$LABEL" >/dev/null 2>&1; then
      launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null; stopped=1
      for _ in $(seq 1 20); do launchctl print "gui/$(id -u)/$LABEL" >/dev/null 2>&1 || break; sleep 0.5; done
      sleep 1
    fi
    if [ -n "$(lsof -t .loft/loft.db 2>/dev/null)" ] || curl -s -m 2 -o /dev/null "http://127.0.0.1:$PORT/api/info"; then
      fail "loft is running (port $PORT or .loft/loft.db is in use); stop it (quit the app or ${HERE}start.sh), then run ${HERE}setup.sh again"
      resume_service; exit 1
    fi
    if why=$(restore_backup "$chosen" .loft); then
      ok "restored ${chosen##*/} ${D}(any older database is in .loft/before-restore)${N}"
      tip "Agent workspaces aren't in backups: agents start with empty ones and clone their repos again."
      tip "Settings came back with it, including the projects folder and your GitHub token."
      break
    fi
    fail "$why"
    resume_service   # Loft stays up while we ask again
    [ -t 0 ] || exit 1
  done
  resume_service
fi

section "Projects folder"
if [ -f "$SETTINGS" ]; then
  dir=$(setting projects_dir)
  ok "$dir ${D}(change it under Settings)${N}"
  if [ "$WINDOWS" = 1 ] && [[ $dir == /* ]]; then
    # An older setup saved Git Bash's form (/c/Users/…), which Loft can't open.
    warn "Loft can't open a Git Bash path: change it under Settings → Projects folder to $(cygpath -m "$dir" 2>/dev/null)"
  elif [ ! -d "$dir" ]; then
    warn "that folder isn't on this computer: change it under Settings → Projects folder"
  fi
else
  while :; do
    dir=$(ask "Folder with your git repos [~/projects]:" "~/projects")
    dir=${dir/#\~/$HOME}; dir=${dir%/}
    if [ -d "$dir" ]; then break; fi
    warn "Not a folder: $dir"
    [ -t 0 ] || exit 1
  done
  dir=$(cd "$dir" && pwd)
  # Loft on Windows reads C:/Users/…, not Git Bash's /c/Users/….
  [ "$WINDOWS" = 1 ] && dir=$(cygpath -m "$dir")
  mkdir -p .loft
  printf '{\n  "projects_dir": "%s"\n}\n' "$(printf '%s' "$dir" | sed 's/\\/\\\\/g; s/"/\\"/g')" > "$SETTINGS"
  ok "$dir ${D}(saved to $SETTINGS)${N}"
fi

section "Build"
. scripts/frontend-pm.sh
# A subshell: without a terminal, spin runs it in this shell, and the cd would stick.
build_frontend() ( cd frontend && { [ -d node_modules ] || fe_install; } && fe run build )
if spin "Building the web app" build_frontend; then ok "web app"
else fail "web app build failed"; show_log; tip "To see everything: cd ${HERE}frontend && pnpm install && pnpm run build"; exit 1; fi
[ -x "$BIN" ] && label="Building loft" || label="Building loft ${D}(the first build takes a few minutes)${N}"
if spin "$label" bash -c 'cd backend && cargo build --release'; then ok "loft"
else fail "loft build failed"; show_log; tip "To see everything: cd ${HERE}backend && cargo build --release"; exit 1; fi
# Voice in the browser is transcribed on the Mac by LoftKit's helper (spec #493); without it, voice says why it's off.
if [ "$MACOS" = 1 ]; then
  if spin "Building the voice transcriber" bash -c 'cd ios/LoftKit && swift build -c release --product loft-transcribe'; then ok "voice transcriber"
  else warn "voice transcriber build failed: voice in the browser stays off ${D}(cd ${HERE}ios/LoftKit && swift build -c release --product loft-transcribe)${N}"; fi
fi

service=0
if [ "$WINDOWS" = 1 ]; then
  section "Background task"
  # One task per user, like the Mac's service: another checkout of loft may own it, or something else named Loft.
  other=""; task_dir=$(task_root)
  if [ -n "$task_dir" ] && ! [ "$task_dir" -ef "$ROOT" ]; then
    other=$task_dir; warn "the logon task runs another copy of loft ${D}(${other})${N}"
    answer=$(ask "Switch the task to this copy? [y/N]" n)
  elif [ -z "$task_dir" ] && win schtasks /Query /TN "$TASK" >/dev/null 2>&1; then
    other="a task setup didn't make"; warn "there is already a task named $TASK that setup didn't make"
    answer=$(ask "Replace it with loft's logon task? [y/N]" n)
  elif [ -n "$task_dir" ]; then
    answer=$(ask "Reinstall the logon task with this build and PATH? [Y/n]" y)
  else
    answer=$(ask "Run loft in the background, hidden, starting at logon? [y/N]" n)
  fi
  if [[ $answer =~ ^[Yy] ]]; then
    n=$(running_agents)
    if ! task_pid >/dev/null && curl -s -m 2 -o /dev/null "http://127.0.0.1:$PORT/"; then
      fail "port $PORT is already in use (is ${HERE}start.sh running?). Stop it, then run ${HERE}setup.sh again."
    elif [ -n "$n" ] && [ "$n" -gt 0 ]; then
      warn "$n agent(s) running; not restarting loft now. Run ${HERE}setup.sh again once they finish."
    elif [ ! -f /usr/bin/mintty.exe ]; then
      fail "Git Bash's mintty (/usr/bin/mintty.exe) is missing; it runs loft without a window"; tip "Reinstall Git for Windows: $(pkg git Git.Git)"
    elif task_install; then
      service=1
      ok "installed ${D}(task \"$TASK\", logs in $(cygpath -w "$SERVICE_LOG"))${N}"
      tip "It uses this shell's PATH, so npm, cargo and claude resolve the same way (re-run ${HERE}setup.sh after changing them)."
      tip "To remove it: schtasks //Delete //TN $TASK //F   then sign out and back in."
    else
      fail "schtasks couldn't create the task"; show_log
    fi
  elif [ -n "$other" ]; then
    ok "left as is${task_dir:+ ${D}(it keeps port $PORT, so run this copy with LOFT_PORT=7421 ${HERE}start.sh)${N}}"
  elif [ -n "$task_dir" ]; then
    ok "installed, left as is"
    service=1
  else
    ok "skipped ${D}(run ${HERE}start.sh to use loft in the foreground)${N}"
  fi
  if [ "$service" = 1 ]; then
    if win reg query 'HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon' /v AutoAdminLogon 2>/dev/null | tr -d '\r' | grep 'REG_SZ *1$' >/dev/null; then
      ok "automatic sign-in is on, so loft comes back after a reboot"
    else warn "the task only runs while you're signed in"; tip "For a server: turn on automatic sign-in, e.g. with Sysinternals Autologon."; fi
  fi
fi

if [ "$MACOS" = 1 ]; then
  section "Background service"
  # The service is per user, not per copy: another checkout of loft may own it.
  other=""
  [ -f "$PLIST" ] && ! grep -qF -e "<string>$BIN</string>" -e "<string>$BIN-signed</string>" "$PLIST" && other=$(sed -n 's/.*<key>ProgramArguments<\/key><array><string>\(.*\)<\/string>.*/\1/p' "$PLIST")
  if [ -n "$other" ]; then
    other=${other%-signed}
    warn "the background service runs another copy of loft ${D}(${other%/backend/target/release/loft})${N}"
    answer=$(ask "Switch the service to this copy? [y/N]" n)
  elif [ -f "$PLIST" ]; then
    answer=$(ask "Reinstall the service with this build and PATH? [Y/n]" y)
  else
    answer=$(ask "Run loft in the background, starting at login? [y/N]" n)
  fi
  if [[ $answer =~ ^[Yy] ]]; then
    n=$(running_agents)
    if ! launchctl print "gui/$(id -u)/$LABEL" >/dev/null 2>&1 && curl -s -m 2 -o /dev/null "http://127.0.0.1:$PORT/"; then
      fail "port $PORT is already in use (is ${HERE}start.sh running?). Stop it, then run ${HERE}setup.sh again."
    elif [ -n "$n" ] && [ "$n" -gt 0 ]; then
      warn "$n agent(s) running; not restarting loft now. Run ${HERE}setup.sh again once they finish."
    else
      mkdir -p ~/Library/LaunchAgents ~/Library/Logs
      # With a "loft" signing identity, the service runs the build's signed copy, so Login Items lists that and
      # its permissions survive updates. The first signing may ask on this Mac's screen to let codesign use the key.
      SERVICE_BIN=$BIN
      if security find-identity -p codesigning 2>/dev/null | grep -q '"loft"'; then
        if spin "Signing loft with the \"loft\" identity" "$BIN" sign; then SERVICE_BIN=$BIN-signed; ok "signed with the \"loft\" identity"
        else warn "couldn't sign with the \"loft\" identity; the service runs the unsigned build"; fi
      fi
      cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL</string>
  <key>ProgramArguments</key><array><string>$SERVICE_BIN</string></array>
  <key>WorkingDirectory</key><string>$ROOT/backend</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>PATH</key><string>$PATH</string>
    <key>LOFT_PORT</key><string>$PORT</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>$HOME/Library/Logs/loft.log</string>
  <key>StandardErrorPath</key><string>$HOME/Library/Logs/loft.log</string>
</dict>
</plist>
EOF
      launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null
      # bootout returns before the old job is gone; bootstrapping too soon fails with "5: Input/output error".
      for _ in $(seq 1 20); do launchctl print "gui/$(id -u)/$LABEL" >/dev/null 2>&1 || break; sleep 0.5; done
      if launchctl bootstrap "gui/$(id -u)" "$PLIST"; then
        service=1
        ok "installed ${D}($PLIST, logs in ~/Library/Logs/loft.log)${N}"
        tip "It uses this shell's PATH, so npm, cargo and claude resolve the same way (re-run ${HERE}setup.sh after changing them)."
      else
        fail "launchctl couldn't load $PLIST"
      fi
    fi
  elif [ -n "$other" ]; then
    ok "left as is ${D}(it keeps port $PORT, so run this copy with LOFT_PORT=7421 ${HERE}start.sh)${N}"
  elif [ -f "$PLIST" ]; then
    ok "installed, left as is"
    service=1
  else
    ok "skipped ${D}(run ${HERE}start.sh to use loft in the foreground)${N}"
  fi
  if [ "$service" = 1 ]; then
    if [ -n "$(defaults read /Library/Preferences/com.apple.loginwindow autoLoginUser 2>/dev/null)" ]; then ok "automatic login is on, so loft comes back after a reboot"
    else warn "the service only runs while you're logged in"; tip "For a server: System Settings → Users & Groups → Automatically log in as…"; fi
  fi

  section "macOS permissions"
  if security find-identity -p codesigning 2>/dev/null | grep -q '"loft"'; then
    ok "\"loft\" signing certificate found; loft signs itself so permissions survive updates"
    tip "The first start asks to let codesign use the key: click Always Allow."
  else
    warn "no \"loft\" signing certificate"
    tip "Without it, macOS privacy permissions granted to loft reset on every rebuild. To create it:"
    tip "Keychain Access → Certificate Assistant → Create a Certificate… → Name: loft, Certificate Type: Code Signing → Create"
  fi

  if xcrun simctl help >/dev/null 2>&1; then
    section "iOS Simulator"
    if axe=$(find_axe); then ok "AXe: the Simulator page can tap, swipe and type ${D}($axe)${N}"
    elif [[ $(ask "Install AXe (open source, github.com/cameroncooke/AXe) to control the Simulator from loft? [y/N]" n) =~ ^[Yy] ]]; then
      if install_axe && axe=$(find_axe); then ok "AXe installed ${D}($axe)${N}"
      else fail "couldn't install AXe"; tip "brew tap cameroncooke/axe && brew install axe   to see why"; fi
    else
      ok "skipped ${D}(the Simulator page is view-only without AXe)${N}"
    fi
  fi

  if command -v tailscale >/dev/null; then
    section "Remote access (Tailscale)"
    if tailscale serve status 2>/dev/null | grep -q "127.0.0.1:$PORT"; then
      ok "tailscale serve: $(tailscale serve status 2>/dev/null | grep -o 'https://[^ ]*' | head -1)"
    elif [ "$service" = 1 ] && [[ $(ask "Serve loft over HTTPS to your tailnet with tailscale serve? [y/N]" n) =~ ^[Yy] ]]; then
      if tailscale serve --bg "$PORT" >/dev/null 2>&1; then ok "tailscale serve: $(tailscale serve status 2>/dev/null | grep -o 'https://[^ ]*' | head -1)"
      else fail "tailscale serve failed"; tip "Enable HTTPS certificates first: https://login.tailscale.com/admin/dns → HTTPS Certificates"; fi
    elif [ "$service" != 1 ]; then
      ok "not served ${D}(needs the background service; or run ${HERE}start.sh, then: tailscale serve --bg $PORT)${N}"
    else
      ok "not served ${D}(to reach it from your other devices: tailscale serve --bg $PORT)${N}"
    fi
    tip "Other machines can open /code in VS Code over Remote-SSH: set Settings → SSH host to this machine's name."
  fi
fi

section "Done"
if [ "$service" = 1 ]; then
  answering() { for _ in $(seq 1 30); do curl -s -m 2 -o /dev/null "http://127.0.0.1:$PORT/api/info" && return 0; sleep 2; done; return 1; }
  if spin "Starting loft" answering; then ok "loft is running: ${B}http://localhost:$PORT${N}"
  elif [ "$WINDOWS" = 1 ]; then warn "loft isn't answering yet"; tip "Check $(cygpath -w "$SERVICE_LOG")."
  else warn "loft isn't answering yet"; tip "Check ~/Library/Logs/loft.log (a keychain prompt may be waiting on screen)."; fi
else
  ok "loft is ready ${D}($ROOT)${N}"
  tip "Start it with: ${HERE}start.sh   then open http://localhost:$PORT"
fi
tip "Notifications, auto-update and the rest are under Settings (gear icon, top right)."
printf '\n'
