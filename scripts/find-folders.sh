# Finding a folder by name for setup.sh's folder questions (projects, backups). scripts/test-setup.sh tests it.

# Folders under $3 (default: home), at most 3 levels down, named $1 (any case) and passing test $2 (a function given
# the folder), sorted, one per line. Hidden folders, Library, node_modules, caches, build output and the like aren't
# searched.
find_folders() {
  local name=$1 test=$2 root=${3:-$HOME} d
  [ -n "$name" ] && [ -d "$root" ] || return 0
  find "$root" -mindepth 1 -maxdepth 3 \
    \( -name '.*' -o -name Library -o -name Applications -o -name node_modules -o -iname '*cache*' \
       -o -name target -o -name vendor -o -name venv -o -name dist -o -name build \) -prune \
    -o -type d -iname "$name" -print 2>/dev/null | sort | while IFS= read -r d; do "$test" "$d" && echo "$d"; done
}

# A folder holding at least one git repo (a subfolder with a .git folder or file).
has_repos() {
  local g
  for g in "$1"/*/.git; do [ -e "$g" ] && return 0; done
  return 1
}

# A folder holding at least one Loft backup (find_backups, from restore-backup.sh).
has_backups() { [ -n "$(find_backups "$1")" ]; }
