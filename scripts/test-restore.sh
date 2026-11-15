#!/usr/bin/env bash
# Tests for scripts/restore-backup.sh: finding backups, a good restore, a corrupt backup. `scripts/test-restore.sh`
set -uo pipefail
cd "$(dirname "$0")"
. ./restore-backup.sh
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
fails=0
check() { if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1: expected [$3], got [$2]"; fails=1; fi; }
mk() { mkdir -p "$1"; sqlite3 "$1/loft.db" "CREATE TABLE t(v); INSERT INTO t VALUES ('$2');"; echo "{\"projects_dir\": \"$2\"}" > "$1/settings.json"; }

mk "$T/b/2026-11-07_031500" old; mk "$T/b/2026-11-08_031500" new; mkdir -p "$T/b/notes" "$T/b/2026-11-09_000000"
mk "$T/icloud/2026-11-06_010000" icloud
check "finds backups newest first, skipping folders without a loft.db" "$(find_backups "$T/b" "$T/icloud" | cut -d'|' -f1,2 | sed "s#$T/##")" $'b/2026-11-08_031500|2026-11-08 03:15\nb/2026-11-07_031500|2026-11-07 03:15\nicloud/2026-11-06_010000|2026-11-06 01:00'
check "no folders, no backups" "$(find_backups "$T/none")" ""

# A good restore moves the old database aside with its -wal, and brings in the backup's database and settings.
D=$T/loft/.loft; mkdir -p "$D"; echo stale > "$D/loft.db"; echo wal > "$D/loft.db-wal"
check "restore succeeds" "$(restore_backup "$T/b/2026-11-08_031500" "$D"; echo rc=$?)" "rc=0"
check "the backup's data is in" "$(sqlite3 "$D/loft.db" 'SELECT v FROM t')" new
check "its settings are in" "$(cat "$D/settings.json")" '{"projects_dir": "new"}'
check "no old -wal next to it" "$(ls "$D" | tr '\n' ' ')" "before-restore loft.db settings.json "
check "the old files are kept aside" "$(ls "$D"/before-restore/* | tr '\n' ' ')" "loft.db loft.db-wal "

# A second restore keeps the first one's files: each goes to its own folder.
sleep 1; echo second > "$D/loft.db"
restore_backup "$T/b/2026-11-07_031500" "$D" >/dev/null
check "two restores, two folders" "$(ls "$D/before-restore" | wc -l | tr -d ' ')" 2
check "the first one's files are still there" "$(cat "$D"/before-restore/*/loft.db | grep -c stale)" 1

# A corrupt backup is refused and leaves the install as it was.
mkdir -p "$T/b/2026-11-10_000000"; head -c 4096 /dev/urandom > "$T/b/2026-11-10_000000/loft.db"
D2=$T/loft2/.loft; mkdir -p "$D2"; echo keep > "$D2/loft.db"
out=$(restore_backup "$T/b/2026-11-10_000000" "$D2"); rc=$?
check "corrupt backup refused" "$rc" 1
check "with a clear message" "${out:0:30}" "the backup's database is damag"
check "the existing database is untouched" "$(cat "$D2/loft.db")" keep
check "nothing half-restored is left" "$(ls "$D2" | tr '\n' ' ')" "loft.db "

# A failed move aside puts everything back: a read-only before-restore can't take the files.
D3=$T/loft3/.loft; mkdir -p "$D3/before-restore"; echo keep3 > "$D3/loft.db"; echo cfg > "$D3/settings.json"; chmod 500 "$D3/before-restore"
out=$(restore_backup "$T/b/2026-11-08_031500" "$D3"); rc=$?
chmod 700 "$D3/before-restore"
check "a failed move aside is refused" "$rc" 1
check "and says so" "${out:0:24}" "couldn't move loft.db as"
check "the database is still there" "$(cat "$D3/loft.db")" keep3
check "no half-restore left" "$(ls "$D3" | tr '\n' ' ')" "before-restore loft.db settings.json "

# Without sqlite3 it says so (not "damaged") and changes nothing.
D4=$T/loft4/.loft; mkdir -p "$D4"; echo keep4 > "$D4/loft.db"
out=$(PATH=/nonexistent /bin/bash -c '. ./restore-backup.sh; restore_backup "$1" "$2"' _ "$T/b/2026-11-08_031500" "$D4"); rc=$?
check "no sqlite3 is refused" "$rc" 1
check "and says that" "${out:0:20}" "sqlite3 isn't instal"
check "the database is untouched" "$(cat "$D4/loft.db")" keep4

# A fresh folder (no .loft yet) works too.
check "restore into a new folder" "$(restore_backup "$T/icloud/2026-11-06_010000" "$T/new/.loft"; sqlite3 "$T/new/.loft/loft.db" 'SELECT v FROM t')" icloud
exit $fails
