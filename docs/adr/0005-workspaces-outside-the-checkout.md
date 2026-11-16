# Agents' workspaces live outside any repository

Status: accepted (#577).

## Context
Agents' workspaces lived in `<loft checkout>/agent-local-storage`, inside the user's own Loft git checkout. When an agent's command missed its own clone (not cloned yet, removed by clean-up, or run from the workspace root), the nearest repository above was the user's checkout. The checkout guard (kill_guard.rs) rightly asked the user about "a change to your checkout" for commands that couldn't touch it. `GIT_CEILING_DIRECTORIES` stops plain git there, but not git through `sh -c`, `cargo`, or anything else. Loosening the guard to tell those cases apart (#570, PR #571) kept opening bypasses.

## Decision
Workspaces live outside any repository and outside the projects folder: `$XDG_DATA_HOME/loft/<checkout>/agent-local-storage`, by default `~/.local/share/loft/...`, on macOS too.
- **The folder keeps its name**, `agent-local-storage`: the guard knows a workspace by it.
- **No space in the path**: `~/Library/Application Support` would make every path in an agent's commands need quoting.
- **One per checkout** (`<checkout>` is its path with every non-alphanumeric character as `-`, like Claude Code's project folders), so a copy of Loft an agent runs from its clone for QA keeps workspaces of its own instead of sweeping the real ones.
- **A one-time move on start** (`storage::migrate`), before any run: when `<checkout>/agent-local-storage` exists and the new folder doesn't, it is renamed there. Then every text column in the database that named the old path names the new one (chats, memories, tasks, ...), the clones' and worktrees' git links are repointed, Claude Code's transcripts move so sessions resume, and each workspace gets a copy of the checkout's Claude Code memory, which it shared while inside the checkout. A failed rename (another volume, no permission) logs an error and Loft keeps using the old folder; there is no copy fallback.
- **The guard stays as strict as before**: nothing in kill_guard.rs changes. `GIT_CEILING_DIRECTORIES` stays, for a home folder kept in git.

## Consequences
- A missing clone in a workspace finds no repository above it, so the guard has nothing to ask about.
- Workspaces are no longer deleted with the checkout. A Loft run with `LOFT_ROOT` (demos, tests) gets its own folder under the data folder, left behind when that root goes.
- Backups must not sit in the workspaces folder; Loft's checkout stays refused as a git repository.
