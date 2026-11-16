//! Where the agents' workspaces live: outside any repository (docs/adr/0005-workspaces-outside-the-checkout.md),
//! so a command in a workspace whose clone is missing finds no checkout above it. They used to live in Loft's own
//! checkout; [`migrate`] moves them out once, on start.

use std::path::{Path, PathBuf};

use crate::db::Db;

/// The storage folder's name, kept where it moved: the checkout guard knows a workspace by it (kill_guard.rs).
pub const NAME: &str = "agent-local-storage";

/// The storage folder of the Loft checkout at `root`: `<data home>/loft/<root>/agent-local-storage`, with `root` named
/// like Claude Code names folders (`-Users-me-dev-loft`). One per checkout, so a copy of Loft an agent runs from its
/// own clone keeps workspaces of its own, as it did inside that clone.
pub fn dir_for(root: &Path, data_home: &Path) -> PathBuf {
    let real = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let name: String = real.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    data_home.join("loft").join(name).join(NAME)
}

/// `$XDG_DATA_HOME`, else `~/.local/share`, on macOS too: `~/Library/Application Support` has a space, which every
/// path in an agent's commands would then have to quote.
pub fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".local/share"))
}

/// Moves the workspaces from `old` (`<checkout>/agent-local-storage`) to `new` when `old` is there and `new` isn't,
/// and returns the folder to use. Run on start, before any run. After the move it points what stored `old`'s path
/// at `new`: every text in the database (chats, memories, tasks, ...), the clones' and worktrees' git links, and
/// Claude Code's transcripts (in its `projects` folder), so sessions resume; each workspace also gets a copy of the
/// checkout's Claude Code memory, which it shared while inside it. A failed move keeps `old` and says so.
pub fn migrate(old: &Path, new: &Path, db: &Db, projects: &Path) -> PathBuf {
    if !old.is_dir() {
        return new.to_path_buf();
    }
    if new.exists() {
        tracing::warn!("agents' workspaces: {} and {} both exist; using {}, the old one is left as it is", old.display(), new.display(), new.display());
        return new.to_path_buf();
    }
    let old = old.canonicalize().unwrap_or_else(|_| old.to_path_buf());
    let moved = match new.parent() {
        Some(parent) => std::fs::create_dir_all(parent).and_then(|_| std::fs::rename(&old, new)),
        None => Err(std::io::Error::other("no parent folder")),
    };
    if let Err(e) = moved {
        tracing::error!("agents' workspaces: couldn't move {} to {}: {e}. Loft keeps using {}", old.display(), new.display(), old.display());
        return old;
    }
    tracing::info!("agents' workspaces: moved {} to {}", old.display(), new.display());
    let (from, to) = (old.to_string_lossy(), new.to_string_lossy());
    match db.replace_everywhere(&from, &to) {
        Ok(n) => tracing::info!("agents' workspaces: {n} stored texts now name {to}"),
        Err(e) => tracing::error!("agents' workspaces: couldn't update the stored paths: {e}"),
    }
    let memory = crate::sessions::transcripts_dir_in(projects, old.parent().unwrap_or(&old)).join("memory");
    for entry in std::fs::read_dir(new).into_iter().flatten().flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') || !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let ws = entry.path();
        repoint_git(&ws, &from, &to, 3);
        let (was, now) = (crate::sessions::transcripts_dir_in(projects, &old.join(&name)), crate::sessions::transcripts_dir_in(projects, &ws));
        if was.is_dir() && !now.exists() {
            match std::fs::rename(&was, &now) {
                Ok(()) => tracing::info!("agents' workspaces: moved transcripts {} to {}", was.display(), now.display()),
                Err(e) => tracing::warn!("agents' workspaces: couldn't move transcripts {}: {e}", was.display()),
            }
        }
        if memory.is_dir() && !now.join("memory").exists() {
            copy_files(&memory, &now.join("memory"));
        }
    }
    new.to_path_buf()
}

/// Points the git links under `dir` (to `depth` folders down) from `from` to `to`: a worktree's or submodule's
/// `.git` file, a clone's `.git/config` and its worktrees' `gitdir`. Symlinks aren't followed.
fn repoint_git(dir: &Path, from: &str, to: &str, depth: usize) {
    let rewrite = |file: &Path| {
        if let Ok(text) = std::fs::read_to_string(file)
            && text.contains(from)
            && let Err(e) = std::fs::write(file, text.replace(from, to))
        {
            tracing::warn!("agents' workspaces: couldn't update {}: {e}", file.display());
        }
    };
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let Ok(ty) = entry.file_type() else { continue };
        let path = entry.path();
        if entry.file_name() == ".git" {
            if ty.is_file() {
                rewrite(&path);
            } else if ty.is_dir() {
                rewrite(&path.join("config"));
                for w in std::fs::read_dir(path.join("worktrees")).into_iter().flatten().flatten() {
                    rewrite(&w.path().join("gitdir"));
                }
            }
        } else if ty.is_dir() && depth > 0 {
            repoint_git(&path, from, to, depth - 1);
        }
    }
}

/// Copies the files directly in `from` into a new folder `to`.
fn copy_files(from: &Path, to: &Path) {
    if let Err(e) = std::fs::create_dir_all(to) {
        tracing::warn!("agents' workspaces: couldn't make {}: {e}", to.display());
        return;
    }
    for f in std::fs::read_dir(from).into_iter().flatten().flatten() {
        if f.file_type().is_ok_and(|t| t.is_file()) {
            let _ = std::fs::copy(f.path(), to.join(f.file_name()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// git as an agent's command would run it, without the ceiling Loft (or this test's own run) sets.
    fn git(dir: &Path, args: &[&str]) -> std::process::Output {
        Command::new("git").args(args).current_dir(dir).env_remove("GIT_CEILING_DIRECTORIES").output().unwrap()
    }

    fn ok(out: std::process::Output) -> String {
        assert!(out.status.success(), "{out:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A temporary home with the user's loft checkout at `dev/loft`, a git repository.
    fn home() -> (crate::testdir::TempDir, PathBuf, PathBuf) {
        let home = crate::testdir::TempDir::new("home");
        let home_path = home.canonicalize().unwrap();
        let root = home_path.join("dev/loft");
        std::fs::create_dir_all(&root).unwrap();
        ok(git(&root, &["init", "-q"]));
        (home, home_path, root)
    }

    #[test]
    fn a_fresh_install_keeps_workspaces_outside_the_checkout_where_a_missing_clone_finds_no_repository() {
        let (_home, home, root) = home();
        let storage = dir_for(&root, &home.join(".local/share"));
        assert_eq!(storage.file_name().unwrap(), NAME, "the guard knows a workspace by this name");
        assert!(!storage.starts_with(&root) && !storage.starts_with(home.join("dev")), "{}", storage.display());
        let ws = storage.join("agent-1");
        std::fs::create_dir_all(&ws).unwrap();
        let out = git(&ws, &["status"]);
        assert!(!out.status.success() && String::from_utf8_lossy(&out.stderr).contains("not a git repository"), "{out:?}");
        // Another checkout, such as one an agent runs Loft from, has workspaces of its own.
        assert_ne!(dir_for(&ws, &home.join(".local/share")), storage);
    }

    #[test]
    fn the_checkout_guard_leaves_commands_in_a_workspace_with_a_missing_clone_alone() {
        let (_home, home, root) = home();
        let missing = dir_for(&root, &home.join(".local/share")).join("agent-1/loft");
        std::fs::create_dir_all(missing.parent().unwrap()).unwrap();
        for command in ["cargo build", "pnpm install", "git checkout main"] {
            let changes = crate::kill_guard::checkout_changes(command, &missing, &[]);
            assert!(!changes.is_empty(), "{command}: the guard still sees the change");
            assert!(changes.iter().all(|c| crate::kill_guard::in_agent_storage(&c.folder) && !c.folder.starts_with(&root)), "{command}: {changes:?}");
        }
        // Where the workspaces used to be, git there (with no ceiling, as through `sh -c`) changed the user's checkout.
        let old = root.join(NAME).join("agent-1/loft");
        std::fs::create_dir_all(old.parent().unwrap()).unwrap();
        assert_eq!(crate::kill_guard::checkout_changes("git checkout main", &old, &[])[0].folder, root);
    }

    #[test]
    fn an_existing_install_moves_its_workspaces_once_and_agents_keep_their_clones_transcripts_and_memory() {
        let (_home, home, root) = home();
        let projects = home.join(".claude/projects");
        let old = root.join(NAME);
        let ws = old.join("agent-1");
        // A clone with a worktree, notes, and Loft's own files.
        let origin = home.join("origin");
        std::fs::create_dir_all(&origin).unwrap();
        ok(git(&origin, &["init", "-q", "-b", "main"]));
        ok(git(&origin, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "one"]));
        std::fs::create_dir_all(&ws).unwrap();
        ok(git(&ws, &["clone", "-q", origin.to_str().unwrap(), "loft"]));
        ok(git(&ws.join("loft"), &["worktree", "add", "-q", ws.join("wt1").to_str().unwrap(), "-b", "t1"]));
        std::fs::create_dir_all(ws.join("notes")).unwrap();
        std::fs::write(ws.join("notes/plan.md"), "plan").unwrap();
        std::fs::create_dir_all(old.join(".agent-files")).unwrap();
        // Claude Code's transcripts of the workspace, and the checkout's memory the agent shared.
        let was = crate::sessions::transcripts_dir_in(&projects, &ws);
        std::fs::create_dir_all(&was).unwrap();
        std::fs::write(was.join("s1.jsonl"), "{}").unwrap();
        let memory = crate::sessions::transcripts_dir_in(&projects, &root).join("memory");
        std::fs::create_dir_all(&memory).unwrap();
        std::fs::write(memory.join("MEMORY.md"), "- a fact").unwrap();
        // Stored paths: a memory and a chat message.
        let dir = crate::testdir::TempDir::new("db");
        let db = Db::open(&dir.join("loft.db")).unwrap();
        let clone = format!("{}/loft", ws.display());
        db.conn().execute("INSERT INTO memories (agent_id, content, created_at) VALUES ('agent-1', ?1, 'now')", [format!("my clone is {clone}")]).unwrap();
        db.conn().execute("INSERT INTO messages (agent_id, role, kind, content, created_at) VALUES ('agent-1', 'agent', 'text', ?1, 'now')", [format!("cd {clone}")]).unwrap();

        let new = dir_for(&root, &home.join(".local/share"));
        assert_eq!(migrate(&old, &new, &db, &projects), new);
        assert!(!old.exists());
        let ws = new.join("agent-1");
        assert_eq!(std::fs::read_to_string(ws.join("notes/plan.md")).unwrap(), "plan");
        assert!(new.join(".agent-files").is_dir());
        // The clone and its worktree still work, and know each other at the new path.
        ok(git(&ws.join("loft"), &["status"]));
        assert_eq!(ok(git(&ws.join("wt1"), &["branch", "--show-current"])), "t1");
        let list = ok(git(&ws.join("loft"), &["worktree", "list", "--porcelain"]));
        assert!(list.contains(&format!("worktree {}", ws.join("wt1").display())) && !list.contains("prunable"), "{list}");
        // Stored paths name the new place.
        let now = format!("{}/loft", ws.display());
        let memory_row: String = db.conn().query_row("SELECT content FROM memories", [], |r| r.get(0)).unwrap();
        let message: String = db.conn().query_row("SELECT content FROM messages", [], |r| r.get(0)).unwrap();
        assert_eq!((memory_row.as_str(), message.as_str()), (format!("my clone is {now}").as_str(), format!("cd {now}").as_str()));
        // Sessions resume from the new folder, with the memory the agent had.
        let now = crate::sessions::transcripts_dir_in(&projects, &ws);
        assert!(now.join("s1.jsonl").exists() && !was.exists());
        assert_eq!(std::fs::read_to_string(now.join("memory/MEMORY.md")).unwrap(), "- a fact");

        // Once: the next start uses the new folder and changes nothing.
        assert_eq!(migrate(&old, &new, &db, &projects), new);
        assert!(!old.exists() && ws.join("loft").is_dir());
    }

    #[test]
    fn a_failed_move_keeps_loft_working_from_the_old_folder() {
        let (_home, home, root) = home();
        let old = root.join(NAME);
        std::fs::create_dir_all(old.join("agent-1/notes")).unwrap();
        // The data folder can't be made: a file is in its way.
        std::fs::write(home.join("blocked"), "").unwrap();
        let new = dir_for(&root, &home.join("blocked"));
        let dir = crate::testdir::TempDir::new("db");
        let db = Db::open(&dir.join("loft.db")).unwrap();
        db.conn().execute("INSERT INTO memories (agent_id, content, created_at) VALUES ('agent-1', ?1, 'now')", [old.display().to_string()]).unwrap();
        assert_eq!(migrate(&old, &new, &db, &home.join(".claude/projects")), old);
        assert!(old.join("agent-1/notes").is_dir() && !new.exists());
        let row: String = db.conn().query_row("SELECT content FROM memories", [], |r| r.get(0)).unwrap();
        assert_eq!(row, old.display().to_string());
    }
}
