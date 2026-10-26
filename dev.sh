#!/usr/bin/env bash
# Run the Rust backend (:7420) and the Vite dev server (:7421) together.
set -euo pipefail
cd "$(dirname "$0")"
[ -d frontend/node_modules ] || (cd frontend && npm install)
trap 'kill 0' EXIT
(cd backend && cargo run) &
(cd frontend && npm run dev) &
wait
