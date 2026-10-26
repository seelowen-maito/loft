#!/usr/bin/env bash
# Install or check loft: requirements, first-run settings, build, and (macOS) a background service.
# Safe to re-run; it reports what's missing and how to fix it.
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
BIN=$ROOT/backend/target/release/loft
MACOS=$([ "$(uname)" = Darwin ] && echo 1 || echo 0)

if [ -t 1 ]; then B=$'\e[1m' G=$'\e[32m' Y=$'\e[33m' R=$'\e[31m' D=$'\e[2m' N=$'\e[0m'; else B="" G="" Y="" R="" D="" N=""; fi
failed=0
section() { printf '\n%s%s%s\n' "$B" "$1" "$N"; }
ok() { printf '  %s✓%s %s\n' "$G" "$N" "$1"; }
warn() { printf '  %s!%s %s\n' "$Y" "$N" "$1"; }
fail() { printf '  %s✗%s %s\n' "$R" "$N" "$1"; failed=1; }
tip() { printf '    %s%s%s\n' "$D" "$1" "$N"; }
# How to install a package here: Homebrew on macOS, else the distro's package manager.
pkg() {
  if [ "$MACOS" = 1 ]; then
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
# A string field from settings.json (flat or nested key, first match).
setting() { [ -f "$SETTINGS" ] && sed -n "s/.*\"$1\": *\"\([^\"]*\)\".*/\1/p" "$SETTINGS" | head -1; }
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

section "Requirements"
if command -v git >/dev/null; then ok "git"
else fail "git is missing"; if [ "$MACOS" = 1 ]; then tip "Run: xcode-select --install   (Apple's command line tools, includes git)"; else tip "Run: $(pkg git)"; fi; fi

if cargo_version=$(cargo --version 2>/dev/null); then ok "Rust ($(echo "$cargo_version" | cut -d' ' -f2))"
elif command -v rustup >/dev/null; then
  fail "Rust is installed but has no toolchain"; tip "Run: rustup default stable"
elif [ -x ~/.cargo/bin/cargo ]; then
  fail "Rust is installed but not on your PATH"
  tip "Run: . \"\$HOME/.cargo/env\"   (or open a new terminal), then run setup again."
else
  fail "Rust (cargo) is missing"
  tip "Run: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y"
  tip "then open a new terminal so cargo is on your PATH."
fi

node_major=$(node -v 2>/dev/null | sed 's/^v\([0-9]*\).*/\1/')
if [ -n "$node_major" ] && [ "$node_major" -ge 22 ] && command -v npm >/dev/null; then ok "Node $(node -v)"
else
  if [ -n "$node_major" ]; then fail "Node 22 or newer is needed (found $(node -v))"; else fail "Node is missing (22 or newer)"; fi
  if [ -s "${NVM_DIR:-$HOME/.nvm}/nvm.sh" ]; then tip "Run: nvm install 22 && nvm alias default 22"
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
  tip "Run: curl -fsSL https://claude.ai/install.sh | bash   then run claude once to log in."
  tip "Already installed somewhere else? Set its path under Settings → Claude command."
fi

if command -v gh >/dev/null && gh auth status >/dev/null 2>&1; then ok "GitHub CLI, logged in"
elif command -v gh >/dev/null; then warn "GitHub CLI not logged in (optional: PRs, issues, private repos)"; tip "Run: gh auth login   or add a token under Settings → GitHub token"
else warn "GitHub CLI not installed (optional: PRs, issues, private repos)"; tip "Run: $(pkg gh) && gh auth login   or add a token under Settings → GitHub token"; fi
if [ "$failed" = 1 ]; then printf '\n  %sFix the items above, then run setup again:%s\n    %ssetup.sh\n\n' "$R" "$N" "$HERE"; exit 1; fi

section "Projects folder"
if [ -f "$SETTINGS" ]; then
  ok "$(setting projects_dir) ${D}(change it under Settings)${N}"
else
  while :; do
    dir=$(ask "Folder with your git repos [~/projects]:" "~/projects")
    dir=${dir/#\~/$HOME}; dir=${dir%/}
    if [ -d "$dir" ]; then break; fi
    warn "Not a folder: $dir"
    [ -t 0 ] || exit 1
  done
  dir=$(cd "$dir" && pwd)
  mkdir -p .loft
  printf '{\n  "projects_dir": "%s"\n}\n' "$(printf '%s' "$dir" | sed 's/\\/\\\\/g; s/"/\\"/g')" > "$SETTINGS"
  ok "$dir ${D}(saved to $SETTINGS)${N}"
fi

section "Build"
build_frontend() { cd frontend && { [ -d node_modules ] || npm install --no-audit --no-fund; } && npm run build; }
if spin "Building the web app" build_frontend; then ok "web app"
else fail "web app build failed"; show_log; tip "To see everything: cd ${HERE}frontend && npm install && npm run build"; exit 1; fi
[ -x "$BIN" ] && label="Building loft" || label="Building loft ${D}(the first build takes a few minutes)${N}"
if spin "$label" bash -c 'cd backend && cargo build --release'; then ok "loft"
else fail "loft build failed"; show_log; tip "To see everything: cd ${HERE}backend && cargo build --release"; exit 1; fi

service=0
if [ "$MACOS" = 1 ]; then
  section "Background service"
  # The service is per user, not per copy: another checkout of loft may own it.
  other=""
  [ -f "$PLIST" ] && ! grep -qF "<string>$BIN</string>" "$PLIST" && other=$(sed -n 's/.*<key>ProgramArguments<\/key><array><string>\(.*\)<\/string>.*/\1/p' "$PLIST")
  if [ -n "$other" ]; then
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
      cat > "$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL</string>
  <key>ProgramArguments</key><array><string>$BIN</string></array>
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
  else warn "loft isn't answering yet"; tip "Check ~/Library/Logs/loft.log (a keychain prompt may be waiting on screen)."; fi
else
  ok "loft is ready ${D}($ROOT)${N}"
  tip "Start it with: ${HERE}start.sh   then open http://localhost:$PORT"
fi
tip "Notifications, auto-update and the rest are under Settings (gear icon, top right)."
printf '\n'
