# Backups

Loft keeps everything (agents, chats, issues, Deliveries, memories, runs) in `.loft/loft.db` in the Loft
folder, with its settings in `.loft/settings.json`. It backs both up on its own (`backend/src/backup.rs`).

- **When:** once a day while Loft runs, the first time no agent is running after a day without a backup
  (after 36 hours it stops waiting for a quiet moment). Settings → Backups → **Back up now** makes one at once.
- **How:** SQLite's `VACUUM INTO`, a consistent copy while Loft writes, then `PRAGMA integrity_check` on the copy.
- **Where:** `~/Library/Application Support/Loft/backups`, or the folder set in Settings → Backups (a full
  path outside the Loft folder and the projects folder, symlinks resolved, such as an external disk or an iCloud Drive folder). Each backup is a folder
  named for when it was made, `2026-11-08_031500/`, holding `loft.db` and `settings.json`. Back up now twice in the same second returns the one just made.
- **Privacy:** owner-only: folders `0700`, files `0600`. `settings.json` holds your GitHub token if you saved one.
  An iCloud Drive (or Dropbox, Google Drive…) folder sends every backup off this Mac: all your chats and
  that GitHub token are then stored by that service. Pick a local or external disk if you don't want that.
- **Retention:** the newest backup of each of the last 7 days, and of each of the last 4 weeks. Older ones
  are deleted. Anything else in the folder is left alone.
- **Failures:** Settings shows the last error. When backups keep failing and the last good one is more than
  two days old, it shows in Needs you.

## Restore

There's no in-app restore. From a Terminal on the Mac:

1. **Stop Loft** (quit the app, or stop `./start.sh`), so nothing writes to the database.
2. **Move the current database aside**, with its `-wal` and `-shm` files. A `-wal` file left next to the
   restored database would be applied to it and corrupt it.
   ```sh
   cd <Loft folder>/.loft
   mkdir -p before-restore && mv loft.db loft.db-wal loft.db-shm settings.json before-restore/ 2>/dev/null
   ```
3. **Copy the backup in:**
   ```sh
   B="$HOME/Library/Application Support/Loft/backups/2026-11-08_031500"   # the one to restore
   cp "$B/loft.db" "$B/settings.json" .
   ```
4. **Start Loft.** Agents, chats and issues are back as they were when the backup was made.

Once you've checked it, delete `before-restore/`.
