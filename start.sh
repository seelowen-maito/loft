#!/usr/bin/env bash
# Run loft in the foreground: build the frontend, then serve everything from the backend on :7420 (or $LOFT_PORT).
# First time? Run ./setup.sh, which checks requirements and can install it as a background service instead.
set -euo pipefail
cd "$(dirname "$0")"

for bin in cargo npm git; do
  command -v "$bin" >/dev/null || { echo "Missing '$bin'. See README.md for requirements." >&2; exit 1; }
done

(cd frontend && ([ -d node_modules ] || npm install) && npm run build)
cd backend && exec cargo run --release
