#!/usr/bin/env bash
# loft installer:  curl -fsSL https://seelowen-maito.github.io/loft/install.sh | bash
# Downloads loft as a git checkout (so auto-update keeps working), then hands off to setup.sh.
# Safe to re-run: an existing install is updated in place.
#
#   LOFT_DIR     where loft lives (default ~/loft)
#   LOFT_REPO    git URL to install from
#   LOFT_BRANCH  branch to install (default main)
#   NO_COLOR     plain output, no animation
#
# Everything runs inside main, so a partially downloaded script does nothing.
set -uo pipefail

INSTALL_CMD="curl -fsSL https://seelowen-maito.github.io/loft/install.sh | bash"

main() {
  local dir=${LOFT_DIR:-$HOME/loft} repo=${LOFT_REPO:-https://github.com/seelowen-maito/loft.git} branch=${LOFT_BRANCH:-main}
  dir=${dir/#\~/$HOME}; dir=${dir%/}
  LOG=$(mktemp -t loft-install.XXXXXX)
  trap cleanup EXIT
  trap 'cleanup; printf "\n"; exit 130' INT TERM
  styles
  banner

  section "Your system"
  case "$(uname -s)" in
    Darwin) ok "macOS $(sw_vers -productVersion 2>/dev/null)" ;;
    Linux) ok "Linux ${D}($(uname -m))${N}" ;;
    *) fail "loft runs on macOS and Linux"; tip "On Windows, install WSL (wsl --install) and run this in an Ubuntu shell."; exit 1 ;;
  esac
  if command -v git >/dev/null; then ok "git $(git --version | cut -d' ' -f3)"
  else
    fail "git is missing"; tip "$(install_hint git xcode-select)"
    retry
  fi

  section "Download"
  # A credential prompt would hide behind the spinner; fail instead.
  export GIT_TERMINAL_PROMPT=0
  if [ -d "$dir/.git" ] && git -C "$dir" remote get-url origin 2>/dev/null | grep -q 'loft'; then
    if [ -n "$(git -C "$dir" status --porcelain --untracked-files=no 2>/dev/null)" ]; then
      warn "$(pretty "$dir") has local changes; leaving it as is"
    elif spin "Updating $(pretty "$dir")" git -C "$dir" pull --ff-only --quiet; then
      ok "updated $(pretty "$dir") ${D}($(git -C "$dir" rev-parse --short HEAD))${N}"
    else
      warn "couldn't update $(pretty "$dir"); using the copy you have"; tip "$(tail -n 1 "$LOG")"
    fi
  elif [ -e "$dir" ]; then
    fail "$(pretty "$dir") already exists and isn't a loft checkout"
    tip "Install somewhere else:  curl -fsSL https://seelowen-maito.github.io/loft/install.sh | LOFT_DIR=~/somewhere/loft bash"
    exit 1
  elif spin "Downloading loft" git clone --quiet --branch "$branch" "$repo" "$dir"; then
    ok "downloaded to $(pretty "$dir") ${D}($(git -C "$dir" rev-parse --short HEAD))${N}"
  else
    fail "couldn't download loft from $repo"
    sed 's/^/    /' "$LOG" | tail -n 4
    tip "Check your connection, then try again."
    retry
  fi

  # Hand off to setup.sh. Its questions read stdin, which is this script when piped from curl,
  # so point it at the terminal. Without one (CI), it takes the defaults.
  cleanup
  trap - EXIT INT TERM
  local input=/dev/null
  (exec </dev/tty) 2>/dev/null && input=/dev/tty
  LOFT_INSTALLER=1 exec bash "$dir/setup.sh" <"$input"
}

styles() {
  TTY=""; [ -t 1 ] && [ -z "${NO_COLOR:-}" ] && [ "${TERM:-dumb}" != dumb ] && TTY=1
  if [ -n "$TTY" ]; then
    B=$'\e[1m' D=$'\e[2m' N=$'\e[0m' G=$'\e[32m' Y=$'\e[33m' R=$'\e[31m'
    # The website's colours (site/index.html): truecolor where the terminal has it, else the nearest of 256.
    case "${COLORTERM:-}" in truecolor|24bit) TRUECOLOR=1 A=$'\e[38;2;200;242;109m' ;; *) TRUECOLOR="" A=$'\e[38;5;191m' ;; esac
  else
    B="" D="" N="" G="" Y="" R="" A=""
  fi
}

section() { printf '\n%s%s%s\n' "$B" "$1" "$N"; }
ok() { printf '  %s✓%s %s\n' "$G" "$N" "$1"; }
warn() { printf '  %s!%s %s\n' "$Y" "$N" "$1"; }
fail() { printf '  %s✗%s %s\n' "$R" "$N" "$1"; }
tip() { printf '    %s%s%s\n' "$D" "$1" "$N"; }
pretty() { case $1 in "$HOME"/*) printf '~%s' "${1#"$HOME"}" ;; *) printf '%s' "$1" ;; esac; }
retry() { printf '\n  %sFix the above, then run the installer again:%s\n    %s\n\n' "$R" "$N" "$INSTALL_CMD"; exit 1; }

# How to install a package here: Homebrew, the distro's package manager, or a fallback (e.g. xcode-select).
install_hint() {
  if [ "$(uname -s)" = Darwin ]; then
    if [ "${2:-}" = xcode-select ]; then echo "Run: xcode-select --install   (Apple's command line tools, includes git)"
    elif command -v brew >/dev/null; then echo "Run: brew install $1"
    else echo "Install Homebrew (https://brew.sh), then: brew install $1"; fi
  elif command -v apt-get >/dev/null; then echo "Run: sudo apt-get install -y $1"
  elif command -v dnf >/dev/null; then echo "Run: sudo dnf install -y $1"
  elif command -v pacman >/dev/null; then echo "Run: sudo pacman -S $1"
  elif command -v apk >/dev/null; then echo "Run: sudo apk add $1"
  else echo "Install $1 with your package manager."; fi
}

# Run a command behind a spinner. Its output goes to $LOG for the caller to show on failure.
SPIN_PID=""
spin() {
  local label=$1; shift
  : >"$LOG"
  if [ -z "$TTY" ]; then printf '  … %s\n' "$label"; "$@" >>"$LOG" 2>&1; return; fi
  "$@" >>"$LOG" 2>&1 &
  SPIN_PID=$!
  local frames=(⠋ ⠙ ⠹ ⠸ ⠼ ⠴ ⠦ ⠧ ⠇ ⠏) i=0 start=$SECONDS t
  printf '\e[?25l'
  while kill -0 "$SPIN_PID" 2>/dev/null; do
    t=$((SECONDS - start)); [ "$t" -ge 3 ] && t=" ${D}${t}s${N}" || t=""
    printf '\r  %s%s%s %s%s\e[K' "$A" "${frames[i++ % 10]}" "$N" "$label" "$t"
    sleep 0.08
  done
  wait "$SPIN_PID"; local rc=$?
  SPIN_PID=""
  printf '\r\e[K\e[?25h'
  return $rc
}

cleanup() {
  # Async commands in a script ignore Ctrl-C, so stop the one behind the spinner ourselves.
  [ -n "$SPIN_PID" ] && kill "$SPIN_PID" 2>/dev/null
  [ -n "${TTY:-}" ] && printf '\e[?25h'
  [ -n "${LOG:-}" ] && rm -f "$LOG"
}

# The website's wordmark, written out stroke by stroke like the site's hero, in its gradient; then the tagline,
# with "hang out." in the accent italic (the site sets it in Instrument Serif).
banner() {
  # Generated from the website's wordmark path (site/index.html); 19 frames of 12 rows, 6 colour bands.
  local rows=12
  local truecolor=('38;2;233;235;219' '38;2;229;236;204' '38;2;224;238;189' '38;2;219;239;174' '38;2;213;240;151' '38;2;206;241;128')
  local xterm=('38;5;188' '38;5;194' '38;5;193' '38;5;193' '38;5;192' '38;5;192')
  local frames=(
    "      |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      |      |      |      |      |      "
    "⣤⣴⠶   |      |      |      |      |      "
    "⠈     |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      |      |      |      |      |      "
    "   ⣀⣴⠟|      |      |      |      |      "
    "⣤⣴⠾⠛⠁ |      |      |      |      |      "
    "⠈     |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      |  ⣀   |      |      |      |      "
    "      | ⣼⠟   |      |      |      |      "
    "     ⢠|⣾⠋    |      |      |      |      "
    "   ⣀⣴⠟|⠁     |      |      |      |      "
    "⣤⣴⠾⠛⠁ |      |      |      |      |      "
    "⠈     |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      |   ⣀  |      |      |      |      "
    "      |   ⢻⡇ |      |      |      |      "
    "      |   ⢸⡇ |      |      |      |      "
    "      |   ⣿⠃ |      |      |      |      "
    "      |  ⣼⠏  |      |      |      |      "
    "      | ⣼⠟   |      |      |      |      "
    "     ⢠|⣾⠋    |      |      |      |      "
    "   ⣀⣴⠟|⠁     |      |      |      |      "
    "⣤⣴⠾⠛⠁ |      |      |      |      |      "
    "⠈     |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      | ⢀⣀⣀  |      |      |      |      "
    "      |⢀⣾⠋⢻⡇ |      |      |      |      "
    "      |⢸⡟ ⢸⡇ |      |      |      |      "
    "      |⣸⡇ ⣿⠃ |      |      |      |      "
    "      |⣿⡇⣼⠏  |      |      |      |      "
    "      |⣿⣼⠟   |      |      |      |      "
    "     ⢠|⣿⠋    |      |      |      |      "
    "   ⣀⣴⠟|⠁     |      |      |      |      "
    "⣤⣴⠾⠛⠁ |      |      |      |      |      "
    "⠈     |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      | ⢀⣀⣀  |      |      |      |      "
    "      |⢀⣾⠋⢻⡇ |      |      |      |      "
    "      |⢸⡟ ⢸⡇ |      |      |      |      "
    "      |⣸⡇ ⣿⠃ |      |      |      |      "
    "      |⣿⡇⣼⠏  |      |      |      |      "
    "      |⣿⣼⠟   |      |      |      |      "
    "     ⢠|⣿⠋    |⢀⣴⠟   |      |      |      "
    "   ⣀⣴⠟|⣿    ⢀|⣾⠋    |      |      |      "
    "⣤⣴⠾⠛⠁ |⢻⣇⣀⣀⣴⡿|⠃     |      |      |      "
    "⠈     | ⠙⠛⠋⠁ |      |      |      |      "
    "      |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      | ⢀⣀⣀  |      |      |      |      "
    "      |⢀⣾⠋⢻⡇ |      |      |      |      "
    "      |⢸⡟ ⢸⡇ |      |      |      |      "
    "      |⣸⡇ ⣿⠃ |      |      |      |      "
    "      |⣿⡇⣼⠏  |      |      |      |      "
    "      |⣿⣼⠟   |  ⢀⣀⣀⡀|      |      |      "
    "     ⢠|⣿⠋    |⢀⣼⠟⠋⠙⣿|⡀     |      |      "
    "   ⣀⣴⠟|⣿    ⢀|⣾⡏   ⣿|⠃     |      |      "
    "⣤⣴⠾⠛⠁ |⢻⣇⣀⣀⣴⡿|⠻⣧⣀⣠⣾⠏|      |      |      "
    "⠈     | ⠙⠛⠋⠁ | ⠈⠉⠉  |      |      |      "
    "      |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      | ⢀⣀⣀  |      |     ⣀|⣤⡀    |      "
    "      |⢀⣾⠋⢻⡇ |      |    ⢰⡟|⠉⣿    |      "
    "      |⢸⡟ ⢸⡇ |      |    ⠸⠇|⢰⡿    |      "
    "      |⣸⡇ ⣿⠃ |      |     ⢀|⣿⠃    |      "
    "      |⣿⡇⣼⠏  |      |    ⢀⣾|⠃     |      "
    "      |⣿⣼⠟   |  ⢀⣀⣀⡀|  ⢀⣴⡿⠁|      |      "
    "     ⢠|⣿⠋    |⢀⣼⠟⠋⠙⣿|⡿⠿⠟⠋  |      |      "
    "   ⣀⣴⠟|⣿    ⢀|⣾⡏   ⣿|⠃     |      |      "
    "⣤⣴⠾⠛⠁ |⢻⣇⣀⣀⣴⡿|⠻⣧⣀⣠⣾⠏|      |      |      "
    "⠈     | ⠙⠛⠋⠁ | ⠈⠉⠉  |      |      |      "
    "      |      |      |      |      |      "
    "      |      |      |      |      |      "
    "      | ⢀⣀⣀  |      |     ⣀|⣤⡀    |      "
    "      |⢀⣾⠋⢻⡇ |      |    ⢰⡟|⠉⣿    |      "
    "      |⢸⡟ ⢸⡇ |      |    ⣼⡇|⢰⡿    |      "
    "      |⣸⡇ ⣿⠃ |      |    ⣿⣇|⣿⠃    |      "
    "      |⣿⡇⣼⠏  |      |    ⣿⣿|⠃     |      "
    "      |⣿⣼⠟   |  ⢀⣀⣀⡀|  ⢀⣴⣿⠁|      |      "
    "     ⢠|⣿⠋    |⢀⣼⠟⠋⠙⣿|⡿⠿⠟⠋⣿ |      |      "
    "   ⣀⣴⠟|⣿    ⢀|⣾⡏   ⣿|⠃   ⣿ |      |      "
    "⣤⣴⠾⠛⠁ |⢻⣇⣀⣀⣴⡿|⠻⣧⣀⣠⣾⠏|    ⣿ |      |      "
    "⠈     | ⠙⠛⠋⠁ | ⠈⠉⠉  |    ⣿ |      |      "
    "      |      |      |   ⣸⡟ |      |      "
    "      |      |      |   ⠈⠁ |      |      "
    "      | ⢀⣀⣀  |      |     ⣀|⣤⡀    |      "
    "      |⢀⣾⠋⢻⡇ |      |    ⢰⡟|⠉⣿    |      "
    "      |⢸⡟ ⢸⡇ |      |    ⣼⡇|⢰⡿    |      "
    "      |⣸⡇ ⣿⠃ |      |    ⣿⣇|⣿⠃    |      "
    "      |⣿⡇⣼⠏  |      |    ⣿⣿|⠃     |      "
    "      |⣿⣼⠟   |  ⢀⣀⣀⡀|  ⢀⣴⣿⠁|      |      "
    "     ⢠|⣿⠋    |⢀⣼⠟⠋⠙⣿|⡿⠿⠟⠋⣿ |      |      "
    "   ⣀⣴⠟|⣿    ⢀|⣾⡏   ⣿|⠃   ⣿ | ⣀⣤⡆  |      "
    "⣤⣴⠾⠛⠁ |⢻⣇⣀⣀⣴⡿|⠻⣧⣀⣠⣾⠏|    ⣿⣶|⠟⠋⠁   |      "
    "⠈     | ⠙⠛⠋⠁ | ⠈⠉⠉  |  ⣰⡿⣿ |      |      "
    "      |      |      | ⢰⣟⣸⡟ |      |      "
    "      |      |      | ⠈⠻⠟⠁ |      |      "
    "      | ⢀⣀⣀  |      |     ⣀|⣤⡀    |      "
    "      |⢀⣾⠋⢻⡇ |      |    ⢰⡟|⠉⣿    |      "
    "      |⢸⡟ ⢸⡇ |      |    ⣼⡇|⢰⡿    |      "
    "      |⣸⡇ ⣿⠃ |      |    ⣿⣇|⣿⠃    |      "
    "      |⣿⡇⣼⠏  |      |    ⣿⣿|⠃    ⢠|⣿     "
    "      |⣿⣼⠟   |  ⢀⣀⣀⡀|  ⢀⣴⣿⠁|     ⣾|⠇     "
    "     ⢠|⣿⠋    |⢀⣼⠟⠋⠙⣿|⡿⠿⠟⠋⣿ |   ⢀⣼⠏|      "
    "   ⣀⣴⠟|⣿    ⢀|⣾⡏   ⣿|⠃   ⣿ | ⣀⣴⡿⠋ |      "
    "⣤⣴⠾⠛⠁ |⢻⣇⣀⣀⣴⡿|⠻⣧⣀⣠⣾⠏|    ⣿⣶|⠟⠋⠁   |      "
    "⠈     | ⠙⠛⠋⠁ | ⠈⠉⠉  |  ⣰⡿⣿ |      |      "
    "      |      |      | ⢰⣟⣸⡟ |      |      "
    "      |      |      | ⠈⠻⠟⠁ |      |      "
    "      | ⢀⣀⣀  |      |     ⣀|⣤⡀    |      "
    "      |⢀⣾⠋⢻⡇ |      |    ⢰⡟|⠉⣿    |      "
    "      |⢸⡟ ⢸⡇ |      |    ⣼⡇|⢰⡿    |      "
    "      |⣸⡇ ⣿⠃ |      |    ⣿⣇|⣿⠃    |      "
    "      |⣿⡇⣼⠏  |      |    ⣿⣿|⠃    ⢠|⣿     "
    "      |⣿⣼⠟   |  ⢀⣀⣀⡀|  ⢀⣴⣿⠁|     ⣾|⡏     "
    "     ⢠|⣿⠋    |⢀⣼⠟⠋⠙⣿|⡿⠿⠟⠋⣿ |   ⢀⣼⣿|⡇     "
    "   ⣀⣴⠟|⣿    ⢀|⣾⡏   ⣿|⠃   ⣿ | ⣀⣴⡿⠋⣿|⠇     "
    "⣤⣴⠾⠛⠁ |⢻⣇⣀⣀⣴⡿|⠻⣧⣀⣠⣾⠏|    ⣿⣶|⠟⠋⠁   |      "
    "⠈     | ⠙⠛⠋⠁ | ⠈⠉⠉  |  ⣰⡿⣿ |      |      "
    "      |      |      | ⢰⣟⣸⡟ |      |      "
    "      |      |      | ⠈⠻⠟⠁ |      |      "
    "      | ⢀⣀⣀  |      |     ⣀|⣤⡀    |      "
    "      |⢀⣾⠋⢻⡇ |      |    ⢰⡟|⠉⣿    |      "
    "      |⢸⡟ ⢸⡇ |      |    ⣼⡇|⢰⡿    |      "
    "      |⣸⡇ ⣿⠃ |      |    ⣿⣇|⣿⠃    |      "
    "      |⣿⡇⣼⠏  |      |    ⣿⣿|⠃    ⢠|⣿     "
    "      |⣿⣼⠟   |  ⢀⣀⣀⡀|  ⢀⣴⣿⠁|     ⣾|⡏     "
    "     ⢠|⣿⠋    |⢀⣼⠟⠋⠙⣿|⡿⠿⠟⠋⣿ |   ⢀⣼⣿|⡇     "
    "   ⣀⣴⠟|⣿    ⢀|⣾⡏   ⣿|⠃   ⣿ | ⣀⣴⡿⠋⣿|⠇     "
    "⣤⣴⠾⠛⠁ |⢻⣇⣀⣀⣴⡿|⠻⣧⣀⣠⣾⠏|    ⣿⣶|⠟⠋⠁  ⢿|⣇⣀    "
    "⠈     | ⠙⠛⠋⠁ | ⠈⠉⠉  |  ⣰⡿⣿ |     ⠈|⠙⠛    "
    "      |      |      | ⢰⣟⣸⡟ |      |      "
    "      |      |      | ⠈⠻⠟⠁ |      |      "
    "      | ⢀⣀⣀  |      |     ⣀|⣤⡀    |      "
    "      |⢀⣾⠋⢻⡇ |      |    ⢰⡟|⠉⣿    |      "
    "      |⢸⡟ ⢸⡇ |      |    ⣼⡇|⢰⡿    |      "
    "      |⣸⡇ ⣿⠃ |      |    ⣿⣇|⣿⠃    |      "
    "      |⣿⡇⣼⠏  |      |    ⣿⣿|⠃    ⢠|⣿     "
    "      |⣿⣼⠟   |  ⢀⣀⣀⡀|  ⢀⣴⣿⠁|     ⣾|⡏     "
    "     ⢠|⣿⠋    |⢀⣼⠟⠋⠙⣿|⡿⠿⠟⠋⣿ |   ⢀⣼⣿|⡇     "
    "   ⣀⣴⠟|⣿    ⢀|⣾⡏   ⣿|⠃   ⣿ | ⣀⣴⡿⠋⣿|⠇     "
    "⣤⣴⠾⠛⠁ |⢻⣇⣀⣀⣴⡿|⠻⣧⣀⣠⣾⠏|    ⣿⣶|⠟⠋⠁  ⢿|⣇⣀⣠⣴⡆ "
    "⠈     | ⠙⠛⠋⠁ | ⠈⠉⠉  |  ⣰⡿⣿ |     ⠈|⠙⠛⠉⠁  "
    "      |      |      | ⢰⣟⣸⡟ |      |      "
    "      |      |      | ⠈⠻⠟⠁ |      |      "
    "      | ⢀⣀⣀  |      |     ⣀|⣤⡀    |      "
    "      |⢀⣾⠋⢻⡇ |      |    ⢰⡟|⠉⣿    |      "
    "      |⢸⡟ ⢸⡇ |      |    ⣼⡇|⢰⡿    |      "
    "      |⣸⡇ ⣿⠃ |      |    ⣿⣇|⣿⠃    |      "
    "      |⣿⡇⣼⠏  |      |    ⣿⣿|⠃    ⢠|⣿     "
    "      |⣿⣼⠟   |  ⢀⣀⣀⡀|  ⢀⣴⣿⠁|     ⣾|⡏     "
    "     ⢠|⣿⠋    |⢀⣼⠟⠋⠙⣿|⡿⠿⠟⠋⣿ |   ⢀⣼⣿|⡇     "
    "   ⣀⣴⠟|⣿    ⢀|⣾⡏   ⣿|⠃   ⣿ | ⣀⣴⡿⠋⣿|⠇     "
    "⣤⣴⠾⠛⠁ |⢻⣇⣀⣀⣴⡿|⠻⣧⣀⣠⣾⠏|    ⣿⣶|⠟⠋⠁  ⢿|⣇⣀⣠⣴⡾⠃"
    "⠈     | ⠙⠛⠋⠁ | ⠈⠉⠉  |  ⣰⡿⣿ |     ⠈|⠙⠛⠉⠁  "
    "      |      |      | ⢰⣟⣸⡟ |      |      "
    "      |      |      | ⠈⠻⠟⠁ |      |      "
    "      | ⢀⣀⣀  |      |     ⣀|⣤⡀    |      "
    "      |⢀⣾⠋⢻⡇ |      |    ⢰⡟|⠉⣿    |      "
    "      |⢸⡟ ⢸⡇ |      |    ⣼⡇|⢰⡿    |      "
    "      |⣸⡇ ⣿⠃ |      |    ⣿⣇|⣿⠃    |      "
    "      |⣿⡇⣼⠏  |      |    ⣿⣿|⠃    ⢠|⣿     "
    "      |⣿⣼⠟   |  ⢀⣀⣀⡀|  ⢀⣴⣿⠁|     ⣾|⡏     "
    "     ⢠|⣿⠋    |⢀⣼⠟⠋⠙⣿|⡿⠿⠟⠋⣿ |   ⢀⣼⣿|⡇     "
    "   ⣀⣴⠟|⣿    ⢀|⣾⡏   ⣿|⠃   ⣿ | ⣀⣴⡿⠋⣿|⠇    ⣀"
    "⣤⣴⠾⠛⠁ |⢻⣇⣀⣀⣴⡿|⠻⣧⣀⣠⣾⠏|    ⣿⣶|⠟⠋⠁  ⢿|⣇⣀⣠⣴⡾⠋"
    "⠈     | ⠙⠛⠋⠁ | ⠈⠉⠉  |  ⣰⡿⣿ |     ⠈|⠙⠛⠉⠁  "
    "      |      |      | ⢰⣟⣸⡟ |      |      "
    "      |      |      | ⠈⠻⠟⠁ |      |      "
    "      | ⢀⣀⣀  |      |     ⣀|⣤⡀    |      "
    "      |⢀⣾⠋⢻⡇ |      |    ⢰⡟|⠉⣿    |      "
    "      |⢸⡟ ⢸⡇ |      |    ⣼⡇|⢰⡿    |      "
    "      |⣸⡇ ⣿⠃ |      |    ⣿⣇|⣿⠃    |      "
    "      |⣿⡇⣼⠏  |      |    ⣿⣿|⠃  ⢀⣀⣠|⣿     "
    "      |⣿⣼⠟   |  ⢀⣀⣀⡀|  ⢀⣴⣿⠁|   ⠙⠋⣿|⡏     "
    "     ⢠|⣿⠋    |⢀⣼⠟⠋⠙⣿|⡿⠿⠟⠋⣿ |   ⢀⣼⣿|⡇     "
    "   ⣀⣴⠟|⣿    ⢀|⣾⡏   ⣿|⠃   ⣿ | ⣀⣴⡿⠋⣿|⠇    ⣀"
    "⣤⣴⠾⠛⠁ |⢻⣇⣀⣀⣴⡿|⠻⣧⣀⣠⣾⠏|    ⣿⣶|⠟⠋⠁  ⢿|⣇⣀⣠⣴⡾⠋"
    "⠈     | ⠙⠛⠋⠁ | ⠈⠉⠉  |  ⣰⡿⣿ |     ⠈|⠙⠛⠉⠁  "
    "      |      |      | ⢰⣟⣸⡟ |      |      "
    "      |      |      | ⠈⠻⠟⠁ |      |      "
    "      | ⢀⣀⣀  |      |     ⣀|⣤⡀    |      "
    "      |⢀⣾⠋⢻⡇ |      |    ⢰⡟|⠉⣿    |      "
    "      |⢸⡟ ⢸⡇ |      |    ⣼⡇|⢰⡿    |      "
    "      |⣸⡇ ⣿⠃ |      |    ⣿⣇|⣿⠃    |      "
    "      |⣿⡇⣼⠏  |      |    ⣿⣿|⠃  ⢀⣀⣠|⣿⣄    "
    "      |⣿⣼⠟   |  ⢀⣀⣀⡀|  ⢀⣴⣿⠁|   ⠙⠋⣿|⡏⠉    "
    "     ⢠|⣿⠋    |⢀⣼⠟⠋⠙⣿|⡿⠿⠟⠋⣿ |   ⢀⣼⣿|⡇     "
    "   ⣀⣴⠟|⣿    ⢀|⣾⡏   ⣿|⠃   ⣿ | ⣀⣴⡿⠋⣿|⠇    ⣀"
    "⣤⣴⠾⠛⠁ |⢻⣇⣀⣀⣴⡿|⠻⣧⣀⣠⣾⠏|    ⣿⣶|⠟⠋⠁  ⢿|⣇⣀⣠⣴⡾⠋"
    "⠈     | ⠙⠛⠋⠁ | ⠈⠉⠉  |  ⣰⡿⣿ |     ⠈|⠙⠛⠉⠁  "
    "      |      |      | ⢰⣟⣸⡟ |      |      "
    "      |      |      | ⠈⠻⠟⠁ |      |      "
    "      | ⢀⣀⣀  |      |     ⣀|⣤⡀    |      "
    "      |⢀⣾⠋⢻⡇ |      |    ⢰⡟|⠉⣿    |      "
    "      |⢸⡟ ⢸⡇ |      |    ⣼⡇|⢰⡿    |      "
    "      |⣸⡇ ⣿⠃ |      |    ⣿⣇|⣿⠃    |      "
    "      |⣿⡇⣼⠏  |      |    ⣿⣿|⠃  ⢀⣀⣠|⣿⣤⣤⣤  "
    "      |⣿⣼⠟   |  ⢀⣀⣀⡀|  ⢀⣴⣿⠁|   ⠙⠋⣿|⡏⠉⠉⠁  "
    "     ⢠|⣿⠋    |⢀⣼⠟⠋⠙⣿|⡿⠿⠟⠋⣿ |   ⢀⣼⣿|⡇     "
    "   ⣀⣴⠟|⣿    ⢀|⣾⡏   ⣿|⠃   ⣿ | ⣀⣴⡿⠋⣿|⠇    ⣀"
    "⣤⣴⠾⠛⠁ |⢻⣇⣀⣀⣴⡿|⠻⣧⣀⣠⣾⠏|    ⣿⣶|⠟⠋⠁  ⢿|⣇⣀⣠⣴⡾⠋"
    "⠈     | ⠙⠛⠋⠁ | ⠈⠉⠉  |  ⣰⡿⣿ |     ⠈|⠙⠛⠉⠁  "
    "      |      |      | ⢰⣟⣸⡟ |      |      "
    "      |      |      | ⠈⠻⠟⠁ |      |      "
  )
  local n=$((${#frames[@]} / rows)) f r b parts IFS='|'
  local lead="Where your coding agents " accent="hang out."
  printf '\n'
  if [ -z "$TTY" ]; then
    for ((r = 0; r < rows; r++)); do parts=(${frames[(n - 1) * rows + r]}); printf '  %s\n' "${parts[*]}" | tr -d '|'; done
    printf '\n  %s%s\n' "$lead" "$accent"
    return
  fi
  local ink=("${xterm[@]}"); [ -n "$TRUECOLOR" ] && ink=("${truecolor[@]}")
  printf '\e[?25l'
  for ((f = 0; f < n; f++)); do
    [ "$f" -gt 0 ] && printf '\e[%dA' "$rows"
    for ((r = 0; r < rows; r++)); do
      parts=(${frames[f * rows + r]})
      printf '  '
      for ((b = 0; b < ${#parts[@]}; b++)); do printf '\e[%sm%s' "${ink[b]}" "${parts[b]}"; done
      printf '%s\e[K\n' "$N"
    done
    sleep 0.07
  done
  printf '\n  '
  for ((f = 0; f < ${#lead}; f++)); do printf '%s' "${lead:f:1}"; sleep 0.01; done
  printf '\e[3m%s' "$A"
  for ((f = 0; f < ${#accent}; f++)); do printf '%s' "${accent:f:1}"; sleep 0.03; done
  printf '%s\n\e[?25h' "$N"
}

main "$@"
