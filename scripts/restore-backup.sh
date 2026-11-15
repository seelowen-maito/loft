# Restoring a Loft backup (docs/backups.md). Sourced by setup.sh; scripts/test-restore.sh tests it.

# Where backups are looked for: the default folder, then iCloud Drive's "Loft backups". Only folders that exist.
backup_dirs() {
  local d
  for d in "$HOME/Library/Application Support/Loft/backups" "$HOME/Library/Mobile Documents/com~apple~CloudDocs/Loft backups"; do
    [ -d "$d" ] && echo "$d"
  done
}

# The backups in the given folders, newest first, one per line: `path|2026-11-08 03:15|12M`.
# A backup is a folder named for when it was made (2026-11-08_031500) that holds a loft.db.
find_backups() {
  local d b n
  for d in "$@"; do
    for b in "$d"/[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]_[0-9][0-9][0-9][0-9][0-9][0-9]; do
      [ -f "$b/loft.db" ] || continue
      n=${b##*/}
      printf '%s|%s|%s|%s\n' "$n" "${n:0:10} ${n:11:2}:${n:13:2}" "$(du -h "$b/loft.db" | cut -f1)" "$b"
    done
  done | sort -r | while IFS='|' read -r _ date size b; do printf '%s|%s|%s\n' "$b" "$date" "$size"; done
}

# Restore backup folder $1 into the Loft data folder $2 (.loft). The copy is checked before anything is touched, so a
# bad backup leaves the install as it was. Any database is moved aside with its -wal and -shm (a stale -wal would be
# applied to the restored database). Prints why on failure.
restore_backup() {
  local b=$1 data=$2 tmp="$2/restore.tmp"
  [ -f "$b/loft.db" ] || { echo "no loft.db in $b"; return 1; }
  mkdir -p "$data" && rm -rf "$tmp" && mkdir -m 700 "$tmp" || { echo "can't write to $data"; return 1; }
  cp "$b/loft.db" "$tmp/" && { [ ! -f "$b/settings.json" ] || cp "$b/settings.json" "$tmp/"; } || { rm -rf "$tmp"; echo "couldn't copy the backup"; return 1; }
  if [ "$(sqlite3 "$tmp/loft.db" 'PRAGMA integrity_check' 2>&1)" != ok ]; then
    rm -rf "$tmp"; echo "the backup's database is damaged (integrity check failed); nothing was changed"; return 1
  fi
  local aside="$data/before-restore" f
  for f in loft.db loft.db-wal loft.db-shm settings.json; do
    [ -e "$data/$f" ] || continue
    mkdir -p "$aside" && mv "$data/$f" "$aside/" || { rm -rf "$tmp"; echo "couldn't move $f aside"; return 1; }
  done
  mv "$tmp"/* "$data/" && rmdir "$tmp"
}
