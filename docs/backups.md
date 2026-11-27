# Backups

Loft keeps everything (agents, chats, issues, Deliveries, memories, runs) in `.loft/loft.db` in the Loft
folder, with its settings in `.loft/settings.json`. It backs both up on its own (`backend/src/backup.rs`).

- **When:** once a day while Loft runs, the first time no agent is running after a day without a backup
  (after 36 hours it stops waiting for a quiet moment). Settings → Backups → **Back up now** makes one at once.
- **How:** SQLite's `VACUUM INTO`, a consistent copy while Loft writes, then `PRAGMA integrity_check` on the copy.
- **Where:** `Loft/backups` in the system's data folder (`~/Library/Application Support` on a Mac, `%APPDATA%` on
  Windows, `$XDG_DATA_HOME` or `~/.local/share` on Linux; Linux used `~/Library/Application Support` before, and
  Loft moves that folder to the new place on its first start when the new one doesn't exist yet), or the folder set in Settings → Backups (a full
  path outside the Loft folder and outside any git repository, symlinks resolved, such as an external disk or an iCloud Drive folder). Settings offers
  **iCloud Drive** in one click, on the web and the iPhone: `~/Library/Mobile Documents/com~apple~CloudDocs/Loft backups`,
  when iCloud Drive is on (`GET /api/backups`'s `icloud`). Each backup is a folder
  named for when it was made, `2026-11-08_031500/`, holding `loft.db` and `settings.json`. Back up now twice in the same second returns the one just made.
- **Privacy:** owner-only: folders `0700`, files `0600`; on Windows, an ACL that gives only your user access. `settings.json` holds your GitHub token if you saved one.
  An iCloud Drive (or Dropbox, Google Drive…) folder sends every backup off this Mac: all your chats and
  that GitHub token are then stored by that service. Pick a local or external disk if you don't want that.
- **Retention:** the newest backup of each of the last 7 days, and of each of the last 4 weeks. Older ones
  are deleted. Anything else in the folder is left alone.
- **Failures:** Settings shows the last error. When backups keep failing and the last good one is more than
  two days old, it shows in Needs you.

## Move Loft or restore on a new install

Setup restores a backup for you. Run `./setup.sh` (or the one-line installer). On an install with no
`.loft/loft.db` yet (a new Mac, or a new Loft folder) it offers one straight away. On an install that already has
data it asks "Restore from a backup? [y/N]" first; No (the default) keeps everything as it is. With no terminal it
never restores over an existing database. It lists the backups it finds in the default folder, in
iCloud Drive's "Loft backups" and, on Linux, in the old default folder if Loft hasn't moved it yet, newest first with their date, size and the folder they are in (so an iCloud or typed folder is easy to tell from a local one; symlinked files and folders dated in the future are skipped), and asks which to restore. You can also type
another folder (an external disk; Tab completes the path, and a bare name is looked for in your home folder, 3 levels deep, among the folders holding backups), or press Enter to start a fresh Loft.

Choosing one stops Loft if it runs as this folder's background service (and starts it again afterwards, also if the
restore is refused), says so when your current data is about to move, and moves any database aside to
`.loft/before-restore/<date_time>/`, a new folder each time (with its `-wal` and `-shm` files, so an old `-wal` is never applied to the restored database), copies in the
backup's `loft.db` and `settings.json`, and checks the copy with `PRAGMA integrity_check` before it replaces anything.
A damaged backup is refused and leaves the install as it was. Then setup carries on and starts Loft.

- If Loft is running some other way (the app, `start.sh`, another copy's service), setup asks you to stop it and
  changes nothing.
- Agent workspaces aren't in backups: after a restore, agents start with empty workspaces and clone their repos again.
- Settings come back with it, including the projects folder and your GitHub token. If the projects folder isn't on
  this Mac, change it under Settings.
- Once you've checked it, delete `before-restore/`.

`scripts/test-restore.sh` tests finding backups, a good restore over an existing database, a second restore, a
corrupt backup, a failed move and a missing `sqlite3`.

### By hand

If setup can't do it, from a Terminal on the Mac:

1. **Stop Loft** (quit the app, or stop `./start.sh`), so nothing writes to the database.
2. **Move the current database aside**, with its `-wal` and `-shm` files. A `-wal` file left next to the
   restored database would be applied to it and corrupt it.
   ```sh
   cd <Loft folder>/.loft
   mkdir -p before-restore && mv loft.db loft.db-wal loft.db-shm settings.json before-restore/ 2>/dev/null
   ```
3. **Copy the backup in:**
   ```sh
   B="$HOME/Library/Application Support/Loft/backups/2026-11-08_031500"   # the one to restore (the default folder on a Mac)
   cp "$B/loft.db" "$B/settings.json" .
   ```
4. **Start Loft.** Agents, chats and issues are back as they were when the backup was made.

Once you've checked it, delete `before-restore/`.
