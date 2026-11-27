#!/usr/bin/env bash
# Tests for scripts/find-folders.sh: setup.sh's search for a folder by name. `scripts/test-find-folders.sh`
set -uo pipefail
cd "$(dirname "$0")"
. ./restore-backup.sh
. ./find-folders.sh
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
fails=0
check() { if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1: expected [$3], got [$2]"; fails=1; fi; }
repo() { mkdir -p "$1/.git"; }
found() { find_folders "$1" "${2:-has_repos}" "$T/home" | sed "s#$T/home/##" | tr '\n' ' '; }

# One folder with repos.
repo "$T/home/code/app"
check "one result" "$(found code)" "code "
check "any case" "$(found CODE)" "code "

# Several, three levels down at most; a repo whose .git is a file (a worktree) counts.
repo "$T/home/projects/a"; mkdir -p "$T/home/work/projects/b"; echo "gitdir: x" > "$T/home/work/projects/b/.git"
repo "$T/home/a/b/projects/c"; repo "$T/home/a/b/c/projects/d"
check "several, sorted, 3 levels down" "$(found projects)" "a/b/projects projects work/projects "

# None: no such name, or the name without repos in it.
mkdir -p "$T/home/empty/x" "$T/home/notes/projects"; echo hi > "$T/home/notes/projects/readme"
check "no such folder" "$(found nothing)" ""
check "a folder without repos isn't listed" "$(found empty)" ""
check "an empty name finds nothing" "$(found "")" ""

# Exclusions: hidden folders, Library, node_modules, caches and build output aren't searched.
for d in Library .config node_modules Caches .cache target; do repo "$T/home/$d/projects/z"; done
repo "$T/home/app/node_modules/projects/z"
check "excluded folders aren't searched" "$(found projects)" "a/b/projects projects work/projects "

# The backups question looks for folders holding Loft backups instead.
mkdir -p "$T/home/disk/loft/2026-11-08_031500" "$T/home/old/loft/notes"; sqlite3 "$T/home/disk/loft/2026-11-08_031500/loft.db" "CREATE TABLE t(v);"
check "backups: only folders with a backup" "$(found loft has_backups)" "disk/loft "
check "a missing root finds nothing" "$(find_folders projects has_repos "$T/none")" ""
exit $fails
