# Sourced by dev.sh, start.sh and setup.sh. The frontend uses the pnpm pinned by `packageManager` in
# frontend/package.json: an installed pnpm, else corepack's, else fetched through `npm exec`.
# Never plain `npm install`: without package-lock.json it would resolve the dependencies fresh.
# Provides `fe` (run pnpm in frontend/) and `fe_install`. Keep in step with `pnpm` in backend/src/updater.rs.
export COREPACK_ENABLE_DOWNLOAD_PROMPT=0
FE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../frontend" && pwd)"
FE_PM=npm-exec
if command -v pnpm >/dev/null && pnpm --version >/dev/null 2>&1; then FE_PM=pnpm
elif command -v corepack >/dev/null && (cd "$FE_DIR" && corepack pnpm --version >/dev/null 2>&1); then FE_PM=corepack
fi

fe() {
  case $FE_PM in
    pnpm) pnpm "$@" ;;
    corepack) corepack pnpm "$@" ;;
    *) local v; v=$(sed -n 's/.*"packageManager": *"pnpm@\([0-9.]*\).*/\1/p' "$FE_DIR/package.json")
       [ -n "$v" ] || { echo "frontend/package.json has no pinned pnpm in packageManager." >&2; return 1; }
       npm exec --yes "pnpm@$v" -- "$@" ;;
  esac
}

fe_install() {
  # node_modules made by npm is thrown away: pnpm refuses to reuse it without a terminal.
  [ ! -d node_modules ] || [ -f node_modules/.modules.yaml ] || rm -rf node_modules
  fe install --frozen-lockfile --ignore-workspace
}
