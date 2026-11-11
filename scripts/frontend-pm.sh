# Sourced by dev.sh, start.sh and setup.sh. The frontend uses pnpm (pinned by `packageManager` in
# frontend/package.json), got through corepack when it isn't installed. Without either, it falls back to npm.
# Provides `fe` (run the package manager in frontend/) and `fe_install`.
# Keep in step with `package_manager` in backend/src/updater.rs.
export COREPACK_ENABLE_DOWNLOAD_PROMPT=0
FE_PM=npm
if command -v pnpm >/dev/null && pnpm --version >/dev/null 2>&1; then FE_PM=pnpm
elif command -v corepack >/dev/null && (cd "$(dirname "${BASH_SOURCE[0]}")/../frontend" && corepack pnpm --version >/dev/null 2>&1); then FE_PM=corepack
fi

fe() { if [ "$FE_PM" = corepack ]; then corepack pnpm "$@"; else "$FE_PM" "$@"; fi; }

fe_install() {
  # node_modules made by the other package manager is thrown away: pnpm refuses to reuse npm's without a terminal.
  if [ -d node_modules ]; then
    if [ -f node_modules/.modules.yaml ]; then [ "$FE_PM" = npm ] && rm -rf node_modules
    else [ "$FE_PM" = npm ] || rm -rf node_modules; fi
  fi
  if [ "$FE_PM" = npm ]; then npm install --no-audit --no-fund; else fe install --frozen-lockfile; fi
}
