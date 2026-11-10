#!/bin/sh
# Shows a Loft's agents in this machine's Herdr UI.
#   curl -fsSL https://your-loft/api/herdr/install.sh | sh -s -- https://your-loft
# Loft's Settings > Herdr UI > "Set up on another machine" gives you this line with your address.
set -eu

LOFT_URL="${1:-${LOFT_URL:-}}"
LOFT_URL="${LOFT_URL%/}"
if [ -z "${LOFT_URL}" ]; then
  echo "Usage: sh install.sh <loft-url>   (for example https://homebase.example.ts.net)" >&2
  exit 1
fi

HERDR="$(command -v herdr 2>/dev/null || true)"
[ -n "${HERDR}" ] || { [ -x "${HOME}/.local/bin/herdr" ] && HERDR="${HOME}/.local/bin/herdr"; } || true
if [ -z "${HERDR}" ]; then
  echo "Herdr isn't installed here. Install it from https://herdr.dev, then run this again." >&2
  exit 1
fi

DIR="${HOME}/.local/share/loft-herdr"
mkdir -p "${DIR}/bin" "${HOME}/.local/bin"
echo "Downloading the Loft plugin for Herdr from ${LOFT_URL}..."
curl -fsSL "${LOFT_URL}/api/herdr/files/herdr-plugin.toml" -o "${DIR}/herdr-plugin.toml"
curl -fsSL "${LOFT_URL}/api/herdr/files/loft-herdr" -o "${DIR}/bin/loft-herdr"
chmod +x "${DIR}/bin/loft-herdr"

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"
case "${OS}" in darwin) OS=macos ;; esac
case "${ARCH}" in arm64) ARCH=aarch64 ;; amd64) ARCH=x86_64 ;; esac

BIN="${DIR}/bin/loft-herdr-bin"
if curl -fsSL "${LOFT_URL}/api/herdr/bin/${OS}-${ARCH}" -o "${BIN}.tmp" 2>/dev/null; then
  mv "${BIN}.tmp" "${BIN}"
  chmod +x "${BIN}"
elif command -v cargo >/dev/null 2>&1; then
  rm -f "${BIN}.tmp"
  echo "No ready-made build for ${OS}-${ARCH}; building it with cargo (a minute or two)..."
  cargo install --quiet --git https://github.com/seelowen-maito/loft loft-herdr --root "${DIR}/cargo"
  ln -sf "${DIR}/cargo/bin/loft-herdr" "${BIN}"
else
  rm -f "${BIN}.tmp"
  echo "That Loft has no build for ${OS}-${ARCH}, and cargo isn't installed to build one. Install Rust (https://rustup.rs) and run this again." >&2
  exit 1
fi

# A plain command on PATH, so Herdr can start a chat again after it restarts.
ln -sf "${BIN}" "${HOME}/.local/bin/loft-herdr"
"${BIN}" setup "${LOFT_URL}"

# Replace an earlier link (say, to a Loft checkout) with this one.
"${HERDR}" plugin unlink loft >/dev/null 2>&1 || true
"${HERDR}" plugin link "${DIR}" >/dev/null
if "${BIN}" sync 2>/dev/null; then
  :
else
  echo "Herdr isn't running. Loft's agents appear in a Loft workspace the next time you start it."
fi
echo "Done: Loft's agents are in Herdr UI."
