# Agents' workspaces live outside any repository

Status: accepted (#577).

## Context
Agents' workspaces lived in `<loft checkout>/agent-local-storage`, inside the user's own Loft git checkout. When an agent's command missed its own clone (not cloned yet, removed by clean-up, or run from the workspace root), the nearest repository above was the user's checkout. The checkout guard (kill_guard.rs) rightly asked the user about "a change to your checkout" for commands that couldn't touch it. `GIT_CEILING_DIRECTORIES` stops plain git there, but not git through `sh -c`, `cargo`, or anything else. Loosening the guard to tell those cases apart (#570, PR #571) kept opening bypasses.

## Decision
Workspaces live outside any repository and outside the projects folder: `$XDG_DATA_HOME/loft/<checkout>/agent-local-storage`, by default `~/.local/share/loft/...`, on macOS too.
- **The folder keeps its name**, `agent-local-storage`: the guard knows a workspace by it.
- **No space in the path**: `~/Library/Application Support` would make every path in an agent's commands need quoting.
- **One per checkout**: `<checkout>` is the checkout's folder name and 8 hex characters of a hash of its path (`loft-1a2b3c4d`), so a copy of Loft an agent runs from its clone for QA keeps workspaces of its own instead of sweeping the real ones. It is short, so Claude Code's transcript folders for the workspaces stay under its 200-character name limit.
- **Chosen once**: the first start records the folder in `.loft/storage-dir`, and later starts use it, so a different `$XDG_DATA_HOME` (launchd's, a shell's) can't point Loft at an empty second folder. Without a home folder (and `$XDG_DATA_HOME`) the first start fails; later ones read the recorded folder.
- **Loft's checkout is `cfg.root`**, never the workspaces' parent: herdr's plugin, the iOS transcribe helper and the backup check use it.
- **A move on start** (`storage::migrate`), before any run, when `<checkout>/agent-local-storage` is there and the new folder isn't (or is empty):
  1. the database is copied to `.loft/loft.db.before-0005` (replacing one an earlier failed attempt left), and a marker, `.loft/moving-workspaces`, records the old path;
  2. the old folder is renamed to the new one. If that fails (another volume, no permission), the marker goes, an error is logged and Loft keeps using the old folder. There is no copy fallback;
  3. while the marker is there, every start runs these steps, each safe to repeat: every text value in the database that names the old path names the new one (one transaction: chats, memories, tasks, ...); git links are repointed (a worktree's or submodule's `.git` file, `.git/config`, worktrees' `gitdir`, submodules' configs with `core.worktree`); small text files in the workspaces outside clones (`notes/` and the like) are rewritten; Claude Code's transcript folders of the workspaces and every folder in them move, so sessions resume; each workspace gets a copy of the checkout's Claude Code memory (which it shared while inside the checkout), with its paths rewritten. The marker goes once all of them worked, so a crash or a failed step is finished on the next start.
- **Only a safe destination**: agents can write in the data folder unasked, so before a move (and on every start without one) `<data home>/loft`, the checkout's folder in it and the destination must not be symlinks, and the destination's real path must not be in Loft's checkout or any git work tree. `<data home>/loft` and the checkout's folder are made owner-only (0700). An unsafe destination, or one that already holds files with no move under way, moves nothing: Loft logs an error and keeps the old folder. A fresh install with an unsafe destination doesn't start, and neither does a home folder kept in git.
- **The guard trusts only the storage folder Loft uses**: `in_agent_storage` (kill_guard.rs) checks the real path against it (set on start; in the hook, read from the run's `GIT_CEILING_DIRECTORIES`), so a folder that is merely named `agent-local-storage`, such as the old one, isn't trusted. Otherwise the guard's rules are unchanged. `GIT_CEILING_DIRECTORIES` stays, for git itself.

## Not migrated
Agents see the old path in these; most rebuild or go on their own:
- Files inside clones and worktrees (the project's files), build output (`target/`, `.cargo-build/`, `node_modules/` shebangs and links, Python virtualenvs): rebuild or reinstall them.
- `scratch/` (Loft clears it in a week) and files over 1 MB.
- Text inside Claude Code transcripts (each message's `cwd`), and transcript folders whose new name would pass 200 characters (Claude Code cuts and hashes those).
- Processes still running from before the restart (an idle agent's dev server): they keep working, since a rename on one volume keeps their open files, but their environment and printed paths are old, and a `cd <old path>` fails.
- The checkout's own Claude Code memory is copied, not moved: the user's sessions in the checkout keep theirs.
- `.loft/loft.db.before-0005` is kept; delete it once the move looks right.

## Consequences
- A missing clone in a workspace finds no repository above it, so the guard has nothing to ask about.
- Workspaces are no longer deleted with the checkout. A Loft run with `LOFT_ROOT` (demos, tests) gets its own folder under the data folder, left behind when that root goes.
- Backups must not sit in Loft's checkout or the workspaces folder.
