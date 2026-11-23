//! Where the agents' workspaces live: outside any repository (docs/adr/0005-workspaces-outside-the-checkout.md),
//! so a command in a workspace whose clone is missing finds no checkout above it. They used to live in Loft's own
//! checkout; [`migrate`] moves them out on start, and finishes a move a crash interrupted.

use std::path::{Path, PathBuf};

use crate::db::Db;

/// The storage folder's name, kept where it moved: the checkout guard knows a workspace by it (kill_guard.rs).
pub const NAME: &str = "agent-local-storage";
/// In Loft's data folder (`.loft`): the storage folder chosen on the first start, used on every later one.
const CHOSEN: &str = "storage-dir";
/// In Loft's data folder, while a move isn't finished: the old folder's path.
const MARKER: &str = "moving-workspaces";
/// In Loft's data folder: the database as it was before the move rewrote its paths.
const DB_COPY: &str = "loft.db.before-0005";
/// Files bigger than this in a workspace aren't notes to rewrite.
const TEXT_MAX: u64 = 1_000_000;

/// The storage folder of the Loft checkout at `root`: `<data home>/loft/<root's name>-<hash of its path>/agent-local-storage`
/// (`loft-1a2b3c4d`). One per checkout, so a copy of Loft an agent runs from its own clone keeps workspaces of its own,
/// as it did inside that clone. Short, so Claude Code's transcript folders for the workspaces stay under its
/// 200-character name limit.
pub fn dir_for(root: &Path, data_home: &Path) -> PathBuf {
    use sha2::{Digest, Sha256};
    let real = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let base: String = real.file_name().unwrap_or_default().to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let hash: String = Sha256::digest(real.as_os_str().as_encoded_bytes()).iter().take(4).map(|b| format!("{b:02x}")).collect();
    data_home.join("loft").join(format!("{base}-{hash}")).join(NAME)
}

/// A path as a file's contents: its bytes on Unix, UTF-8 on Windows.
fn path_bytes(p: &Path) -> Vec<u8> {
    #[cfg(unix)]
    return std::os::unix::ffi::OsStrExt::as_bytes(p.as_os_str()).to_vec();
    #[cfg(not(unix))]
    p.to_string_lossy().into_owned().into_bytes()
}

/// The path [`path_bytes`] wrote.
fn path_from_bytes(b: &[u8]) -> PathBuf {
    #[cfg(unix)]
    return PathBuf::from(<std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(b));
    #[cfg(not(unix))]
    PathBuf::from(String::from_utf8_lossy(b).into_owned())
}

/// `$XDG_DATA_HOME`, else `~/.local/share`, on macOS too: `~/Library/Application Support` has a space, which every
/// path in an agent's commands would then have to quote.
pub fn data_home() -> anyhow::Result<PathBuf> {
    let xdg = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute());
    let home = || dirs::home_dir().filter(|h| h.is_absolute()).map(|h| h.join(".local/share"));
    xdg.or_else(home).ok_or_else(|| anyhow::anyhow!("no home folder to keep the agents' workspaces in: set HOME"))
}

/// The storage folder Loft uses: the one recorded in its data folder `data_dir`, else [`dir_for`] `data_home()`,
/// recorded now, so a later start with another `$XDG_DATA_HOME` (launchd's, a shell's), or none, keeps the same one.
pub fn chosen(root: &Path, data_dir: &Path, data_home: impl FnOnce() -> anyhow::Result<PathBuf>) -> anyhow::Result<PathBuf> {
    let file = data_dir.join(CHOSEN);
    if let Ok(text) = std::fs::read_to_string(&file) {
        let dir = PathBuf::from(text.trim());
        if dir.is_absolute() {
            return Ok(dir);
        }
        tracing::warn!("agents' workspaces: {} doesn't hold a full path; choosing again", file.display());
    }
    let dir = dir_for(root, &data_home()?);
    std::fs::write(&file, path_bytes(&dir))?;
    tracing::info!("agents' workspaces: chose {}", dir.display());
    Ok(dir)
}

/// Moves the workspaces from `old` (`<checkout>/agent-local-storage`) to `new`, and returns the folder to use. Run
/// on start, before any run.
///
/// A move starts when `old` is there and `new` isn't (or is empty): the database is copied to `loft.db.before-0005`
/// and a marker naming `old` is written in Loft's data folder `data_dir`, then `old` is renamed. While the marker is
/// there, every start redoes the steps after the rename, which are safe to repeat: every text in the database
/// (chats, memories, tasks, ...), the git links of clones, worktrees and submodules, notes and other text files in
/// the workspaces outside clones, and Claude Code's transcripts (in its `projects` folder) of the workspaces and
/// the folders in them, so sessions resume; each workspace also gets a copy of the checkout's Claude Code memory,
/// which it shared while inside it. The marker goes once all of them worked.
///
/// `new` must be a safe place first ([`safe_destination`]). When it isn't, or already holds files with no move under
/// way, or the rename fails, nothing moves and `old` stays in use; the second value says why, for Needs you. A fresh
/// install (no `old`) with an unsafe `new` doesn't start. A symlinked `old` isn't moved either.
///
/// Once a move is done (its database copy is there, no marker) and `new` holds the workspaces, an `old` that
/// reappeared is never used again: Docker recreates a missing bind-mount source of a container made before the
/// move as an empty folder. One of empty folders (and Loft's `.agent-files`) is removed; one with files is left for
/// the user, and Needs you names it ([`reappeared`]).
pub fn migrate(old: &Path, new: &Path, root: &Path, projects_dir: &Path, data_dir: &Path, db: &Db, projects: &Path) -> anyhow::Result<(PathBuf, Option<String>)> {
    let marker = data_dir.join(MARKER);
    let kept = |old: &Path, why: String| {
        tracing::error!("agents' workspaces: not moved from {} to {}: {why}. Loft keeps using {}", old.display(), new.display(), old.display());
        let _ = std::fs::remove_file(&marker);
        // No move happened: a copy left now would make a later start take the move for done.
        let _ = std::fs::remove_file(data_dir.join(DB_COPY));
        Ok((old.to_path_buf(), Some(why)))
    };
    let old = match std::fs::read(&marker) {
        Ok(path) => {
            let old = path_from_bytes(&path);
            tracing::warn!("agents' workspaces: finishing the move from {} to {} an earlier start began", old.display(), new.display());
            old
        }
        Err(_) => {
            let done = data_dir.join(DB_COPY).exists() && std::fs::read_dir(new).is_ok_and(|mut d| d.next().is_some());
            if !old.is_dir() || done {
                // A fresh install, or the move is done.
                safe_destination(new, root, projects_dir).map_err(|why| anyhow::anyhow!("agents' workspaces can't be in {}: {why}", new.display()))?;
                if old.is_dir() {
                    if !old.is_symlink() && only_folders(old, true) {
                        match remove_folders(old, true) {
                            Ok(()) => tracing::warn!("agents' workspaces: removed {}, which reappeared empty after the move", old.display()),
                            Err(e) => tracing::error!("agents' workspaces: couldn't remove {}, which reappeared after the move: {e}", old.display()),
                        }
                    } else {
                        tracing::error!("agents' workspaces: {} reappeared after the move and holds files; Loft uses {} and leaves it", old.display(), new.display());
                    }
                }
                return Ok((new.to_path_buf(), None));
            }
            if old.is_symlink() {
                return kept(old, format!("{} is a symlink", old.display()));
            }
            if let Err(why) = safe_destination(new, root, projects_dir) {
                return kept(old, why);
            }
            // An empty `new` (an earlier start made it) is no reason not to move.
            if new.exists() && std::fs::remove_dir(new).is_err() {
                let why = format!("{} already holds files. Look at what is there and delete it; the next start moves the workspaces", new.display());
                return kept(old, why);
            }
            let copy = data_dir.join(DB_COPY);
            // A copy left by an earlier attempt whose rename failed is older than the database now.
            let _ = std::fs::remove_file(&copy);
            let started = db.copy_to(&copy).and_then(|_| Ok(std::fs::write(&marker, path_bytes(old))?));
            if let Err(e) = started {
                return kept(old, format!("couldn't copy the database or write {}: {e}", marker.display()));
            }
            old.to_path_buf()
        }
    };
    if old.is_dir() {
        // Checked again: this may be a later start, finishing a move a crash stopped before the rename.
        let moved = match safe_destination(new, root, projects_dir) {
            Err(why) => Err(why),
            Ok(()) if new.exists() => Err(format!("{} already holds files", new.display())),
            Ok(()) => std::fs::rename(&old, new).map_err(|e| format!("couldn't rename: {e}")),
        };
        if let Err(why) = moved {
            return kept(&old, why);
        }
        tracing::info!("agents' workspaces: moved {} to {}", old.display(), new.display());
    } else if let Err(why) = safe_destination(new, root, projects_dir) {
        // A later start finishing a move: the workspaces are already in `new`, which changed since. The marker stays.
        anyhow::bail!("agents' workspaces: can't finish the move to {}: {why}. Loft doesn't start until it is a safe folder again", new.display());
    }

    let (from, to) = (old.to_string_lossy().into_owned(), new.to_string_lossy().into_owned());
    let mut ok = true;
    match db.replace_everywhere(&from, &to) {
        Ok(n) => tracing::info!("agents' workspaces: {n} stored texts now name {to}"),
        Err(e) => {
            tracing::error!("agents' workspaces: couldn't update the stored paths: {e}");
            ok = false;
        }
    }
    ok &= move_transcripts(projects, &old, new);
    let memory = crate::sessions::transcripts_dir_in(projects, old.parent().unwrap_or(&old)).join("memory");
    for entry in std::fs::read_dir(new).into_iter().flatten().flatten() {
        if entry.file_name().as_encoded_bytes().starts_with(b".") || !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let ws = entry.path();
        ok &= repoint_git(&ws, &from, &to, 3);
        ok &= rewrite_files(&ws, &from, &to, 4);
        let own = crate::sessions::transcripts_dir_in(projects, &ws).join("memory");
        if memory.is_dir() && !own.exists() {
            ok &= copy_files(&memory, &own);
        }
        ok &= rewrite_files(&own, &from, &to, 0);
    }
    if ok && std::fs::remove_file(&marker).is_ok() {
        tracing::info!("agents' workspaces: the move to {} is done", new.display());
    } else if !ok {
        tracing::error!("agents' workspaces: the move to {} isn't finished; Loft tries the rest again on its next start", new.display());
    }
    Ok((new.to_path_buf(), None))
}

/// Whether `dir` holds nothing but folders (and, at the `top`, Loft's `.agent-files` with anything in it). A symlink
/// counts as a file.
fn only_folders(dir: &Path, top: bool) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    entries.into_iter().all(|e| {
        let Ok(e) = e else { return false };
        let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
        is_dir && ((top && e.file_name() == ".agent-files") || only_folders(&e.path(), false))
    })
}

/// Removes `dir`, bottom-up, folder by folder: a file that appeared since [`only_folders`] looked (a container
/// writing into its bind mount) makes it fail, never goes. Only the `top`'s `.agent-files`, Loft's own, goes whole.
fn remove_folders(dir: &Path, top: bool) -> std::io::Result<()> {
    for e in std::fs::read_dir(dir)? {
        let e = e?;
        if !e.file_type()?.is_dir() {
            continue;
        }
        if top && e.file_name() == ".agent-files" {
            std::fs::remove_dir_all(e.path())?;
        } else {
            remove_folders(&e.path(), false)?;
        }
    }
    std::fs::remove_dir(dir)
}

/// The old storage folder in the checkout `root`, when it is there though Loft uses `storage` (one with files that
/// reappeared after the move), for Needs you.
pub fn reappeared(root: &Path, storage: &Path) -> Option<PathBuf> {
    let old = root.join(NAME);
    (old != storage && old.symlink_metadata().is_ok()).then_some(old)
}

/// Why `new` (`<data home>/loft/<checkout>/agent-local-storage`) isn't a safe place for the workspaces, if it isn't:
/// it, its parent or `<data home>/loft` is a symlink (an agent could have planted one), or its real path is in Loft's
/// checkout `root` or in the projects folder `projects_dir` (unless that is the home folder or above it, which holds
/// the data folder by design). A git work tree around it only gets a warning. Makes `<data home>/loft` and the folder for this checkout,
/// owner-only.
fn safe_destination(new: &Path, root: &Path, projects_dir: &Path) -> Result<(), String> {
    let key = new.parent().ok_or("it has no parent folder")?;
    let loft = key.parent().ok_or("it has no parent folder")?;
    for p in [loft, key, new] {
        if p.symlink_metadata().is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(format!("{} is a symlink", p.display()));
        }
    }
    let made = std::fs::create_dir_all(loft.parent().unwrap_or(loft)).and_then(|_| crate::owner_only::create_dir(key, true));
    made.map_err(|e| format!("couldn't make {}: {e}", key.display()))?;
    for p in [loft, key] {
        crate::owner_only::dir(p).map_err(|e| format!("couldn't make {} owner-only: {e}", p.display()))?;
    }
    // Links anywhere above count: the real path decides.
    let real = key.canonicalize().map_err(|e| format!("couldn't resolve {}: {e}", key.display()))?;
    if real.starts_with(root.canonicalize().unwrap_or_else(|_| root.to_path_buf())) {
        return Err(format!("{} is inside Loft's checkout", real.display()));
    }
    let real_of = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let projects_dir = real_of(projects_dir);
    let home = dirs::home_dir().map(|h| real_of(&h));
    if real.starts_with(&projects_dir) && !home.is_some_and(|h| h.starts_with(&projects_dir)) {
        return Err(format!("{} is inside the projects folder", real.display()));
    }
    // A home folder kept in git (dotfiles) is fine: GIT_CEILING_DIRECTORIES stops agents' git below it.
    if let Some(repo) = real.ancestors().find(|a| a.join(".git").exists()) {
        tracing::warn!("agents' workspaces: {} is inside the git work tree at {}; agents' git stops at the workspaces (GIT_CEILING_DIRECTORIES)", real.display(), repo.display());
    }
    Ok(())
}

/// Whether the real path `folder` is in the agents' workspaces: inside `storage`, the real path of the storage folder
/// Loft uses, and nowhere else, whatever its folders are named (the old `<checkout>/agent-local-storage` included).
/// Nothing is when `storage` isn't a full path.
pub fn holds_in(folder: &Path, storage: &Path) -> bool {
    storage.is_absolute() && folder.starts_with(storage)
}

/// Replaces `from` with `to` in `file` when it holds it. False when it couldn't.
fn rewrite(file: &Path, from: &str, to: &str) -> bool {
    // Never through a symlink: it may lead anywhere.
    if !file.symlink_metadata().is_ok_and(|m| m.file_type().is_file()) {
        return true;
    }
    let Ok(text) = std::fs::read_to_string(file) else { return true };
    if !text.contains(from) {
        return true;
    }
    std::fs::write(file, text.replace(from, to)).map_err(|e| tracing::warn!("agents' workspaces: couldn't update {}: {e}", file.display())).is_ok()
}

/// Points the git links under `dir` (to `depth` folders down) from `from` to `to`: a worktree's or submodule's
/// `.git` file, a clone's `.git/config`, its worktrees' `gitdir` and its submodules' configs (`core.worktree`).
/// Symlinks aren't followed.
fn repoint_git(dir: &Path, from: &str, to: &str, depth: usize) -> bool {
    let mut ok = true;
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let Ok(ty) = entry.file_type() else { continue };
        let path = entry.path();
        if entry.file_name() == ".git" {
            if ty.is_file() {
                ok &= rewrite(&path, from, to);
            } else if ty.is_dir() {
                ok &= rewrite(&path.join("config"), from, to);
                for w in std::fs::read_dir(path.join("worktrees")).into_iter().flatten().flatten() {
                    ok &= rewrite(&w.path().join("gitdir"), from, to);
                }
                ok &= submodule_configs(&path.join("modules"), from, to);
            }
        } else if ty.is_dir() && depth > 0 {
            ok &= repoint_git(&path, from, to, depth - 1);
        }
    }
    ok
}

/// Every `config` in the folders under a `.git/modules` folder: each submodule's, at any depth (a submodule's
/// name may hold `/`, and submodules nest).
fn submodule_configs(dir: &Path, from: &str, to: &str) -> bool {
    let mut ok = true;
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            ok &= rewrite(&entry.path().join("config"), from, to) & submodule_configs(&entry.path(), from, to);
        }
    }
    ok
}

/// Rewrites the small text files under `dir`, to `depth` folders down, outside git clones (their files are the
/// project's), hidden folders, build output and `scratch/` (Loft clears it). Symlinks aren't followed.
fn rewrite_files(dir: &Path, from: &str, to: &str, depth: usize) -> bool {
    let mut ok = true;
    if dir.join(".git").exists() {
        return ok;
    }
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let Ok(ty) = entry.file_type() else { continue };
        let name = entry.file_name();
        if name.as_encoded_bytes().starts_with(b".") {
            continue;
        }
        if ty.is_file() && entry.metadata().is_ok_and(|m| m.len() <= TEXT_MAX) {
            ok &= rewrite(&entry.path(), from, to);
        } else if ty.is_dir() && depth > 0 && !["node_modules", "target", "scratch"].contains(&&*name.to_string_lossy()) {
            ok &= rewrite_files(&entry.path(), from, to, depth - 1);
        }
    }
    ok
}

/// Renames Claude Code's transcript folders of the workspaces in `new` and every folder in them, made while they
/// were in `old`, to the same under `new`. A sibling of `old` (`agent-local-storage-old`) isn't one.
fn move_transcripts(projects: &Path, old: &Path, new: &Path) -> bool {
    let name = |p: &Path| crate::sessions::transcripts_dir_in(projects, p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (from, to) = (name(old), name(new));
    let encode = |s: &str| -> String { s.chars().flat_map(|c| if c.is_ascii_alphanumeric() { vec![c] } else { vec!['-'; c.len_utf16()] }).collect() };
    let workspaces: Vec<String> = std::fs::read_dir(new)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) && !e.file_name().as_encoded_bytes().starts_with(b"."))
        .map(|e| format!("-{}", encode(&e.file_name().to_string_lossy())))
        .collect();
    let mut ok = true;
    for entry in std::fs::read_dir(projects).into_iter().flatten().flatten() {
        let was = entry.file_name().to_string_lossy().into_owned();
        let in_workspace = |r: &&str| workspaces.iter().any(|w| r.strip_prefix(w.as_str()).is_some_and(|after| after.is_empty() || after.starts_with('-')));
        let Some(rest) = was.strip_prefix(&from).filter(in_workspace) else { continue };
        let now = format!("{to}{rest}");
        // Claude Code cuts longer names and adds a hash, which can't be made from the name alone.
        if now.len() > 200 {
            tracing::warn!("agents' workspaces: transcripts {was} stay: their new name would be too long");
            continue;
        }
        let now = projects.join(now);
        if now.exists() {
            tracing::warn!("agents' workspaces: transcripts {was} stay: {} is there already", now.display());
            continue;
        }
        match std::fs::rename(entry.path(), &now) {
            Ok(()) => tracing::info!("agents' workspaces: moved transcripts {was} to {}", now.display()),
            Err(e) => {
                tracing::error!("agents' workspaces: couldn't move transcripts {was}: {e}");
                ok = false;
            }
        }
    }
    ok
}

/// Copies the files directly in `from` into a new folder `to`.
fn copy_files(from: &Path, to: &Path) -> bool {
    let mut ok = true;
    if let Err(e) = std::fs::create_dir_all(to) {
        tracing::warn!("agents' workspaces: couldn't make {}: {e}", to.display());
        return false;
    }
    for f in std::fs::read_dir(from).into_iter().flatten().flatten() {
        if f.file_type().is_ok_and(|t| t.is_file())
            && let Err(e) = std::fs::copy(f.path(), to.join(f.file_name()))
        {
            tracing::warn!("agents' workspaces: couldn't copy {}: {e}", f.path().display());
            ok = false;
        }
    }
    ok
}

#[cfg(test)]
// Some of these use symlinks or file modes, which are Unix only.
#[cfg_attr(not(unix), allow(dead_code))]
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
        std::fs::create_dir_all(root.join(".loft")).unwrap();
        ok(git(&root, &["init", "-q"]));
        (home, home_path, root)
    }

    /// An install from before the move, in a temporary home: `agent-1`'s workspace in the checkout with a clone,
    /// a worktree, a submodule config, notes, Claude Code transcripts (of the workspace and of its clone) and the
    /// checkout's memory, and a database naming the clone in a memory and a message.
    struct Old {
        _home: crate::testdir::TempDir,
        home: PathBuf,
        root: PathBuf,
        db: Db,
    }

    impl Old {
        fn make() -> Old {
            let (_home, home, root) = home();
            let ws = root.join(NAME).join("agent-1");
            let origin = home.join("origin");
            std::fs::create_dir_all(&origin).unwrap();
            ok(git(&origin, &["init", "-q", "-b", "main"]));
            ok(git(&origin, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "one"]));
            std::fs::create_dir_all(&ws).unwrap();
            ok(git(&ws, &["clone", "-q", origin.to_str().unwrap(), "loft"]));
            ok(git(&ws.join("loft"), &["worktree", "add", "-q", ws.join("wt1").to_str().unwrap(), "-b", "t1"]));
            let sub = ws.join("loft/.git/modules/sub");
            std::fs::create_dir_all(&sub).unwrap();
            std::fs::write(sub.join("config"), format!("[core]\n\tworktree = {}/loft/sub\n", ws.display())).unwrap();
            std::fs::create_dir_all(ws.join("notes")).unwrap();
            std::fs::write(ws.join("notes/plan.md"), format!("clone: {}/loft", ws.display())).unwrap();
            std::fs::create_dir_all(root.join(NAME).join(".agent-files")).unwrap();
            let projects = home.join(".claude/projects");
            for cwd in [ws.clone(), ws.join("loft")] {
                let t = crate::sessions::transcripts_dir_in(&projects, &cwd);
                std::fs::create_dir_all(&t).unwrap();
                std::fs::write(t.join("s1.jsonl"), "{}").unwrap();
            }
            // A sibling of the storage folder, not a workspace.
            std::fs::create_dir_all(crate::sessions::transcripts_dir_in(&projects, &root.join(format!("{NAME}-old")))).unwrap();
            let memory = crate::sessions::transcripts_dir_in(&projects, &root).join("memory");
            std::fs::create_dir_all(&memory).unwrap();
            std::fs::write(memory.join("MEMORY.md"), format!("- clone at {}/loft", ws.display())).unwrap();
            let db = Db::open(&root.join(".loft/loft.db")).unwrap();
            let clone = format!("{}/loft", ws.display());
            db.conn().execute("INSERT INTO memories (agent_id, content, created_at) VALUES ('agent-1', ?1, 'now')", [format!("my clone is {clone}")]).unwrap();
            db.conn().execute("INSERT INTO messages (agent_id, role, kind, content, created_at) VALUES ('agent-1', 'agent', 'text', ?1, 'now')", [format!("cd {clone}")]).unwrap();
            Old { _home, home, root, db }
        }

        fn old_dir(&self) -> PathBuf {
            self.root.join(NAME)
        }

        fn new_dir(&self) -> PathBuf {
            dir_for(&self.root, &self.home.join(".local/share"))
        }

        fn data(&self) -> PathBuf {
            self.root.join(".loft")
        }

        fn projects(&self) -> PathBuf {
            self.home.join(".claude/projects")
        }

        fn migrate(&self) -> PathBuf {
            self.migrate_why().0
        }

        /// The folder to use, and why the workspaces didn't move, if they didn't.
        fn migrate_why(&self) -> (PathBuf, Option<String>) {
            migrate(&self.old_dir(), &self.new_dir(), &self.root, &self.home.join("dev"), &self.data(), &self.db, &self.projects()).unwrap()
        }

        fn rows(&self) -> (String, String) {
            let memory = self.db.conn().query_row("SELECT content FROM memories", [], |r| r.get(0)).unwrap();
            let message = self.db.conn().query_row("SELECT content FROM messages", [], |r| r.get(0)).unwrap();
            (memory, message)
        }

        /// Everything an agent had works at the new place.
        #[cfg(unix)]
        fn assert_moved(&self) {
            let ws = self.new_dir().join("agent-1");
            assert!(!self.old_dir().exists() && self.new_dir().join(".agent-files").is_dir());
            let clone = format!("{}/loft", ws.display());
            assert_eq!(std::fs::read_to_string(ws.join("notes/plan.md")).unwrap(), format!("clone: {clone}"));
            ok(git(&ws.join("loft"), &["status"]));
            assert_eq!(ok(git(&ws.join("wt1"), &["branch", "--show-current"])), "t1");
            let list = ok(git(&ws.join("loft"), &["worktree", "list", "--porcelain"]));
            assert!(list.contains(&format!("worktree {}", ws.join("wt1").display())) && !list.contains("prunable"), "{list}");
            assert!(std::fs::read_to_string(ws.join("loft/.git/modules/sub/config")).unwrap().contains(&format!("{clone}/sub")));
            assert_eq!(self.rows(), (format!("my clone is {clone}"), format!("cd {clone}")));
            for cwd in [ws.clone(), ws.join("loft")] {
                assert!(crate::sessions::transcripts_dir_in(&self.projects(), &cwd).join("s1.jsonl").exists(), "{}", cwd.display());
            }
            let memory = crate::sessions::transcripts_dir_in(&self.projects(), &ws).join("memory/MEMORY.md");
            assert_eq!(std::fs::read_to_string(memory).unwrap(), format!("- clone at {clone}"));
            assert!(!self.data().join(MARKER).exists(), "the move is done");
            assert!(crate::sessions::transcripts_dir_in(&self.projects(), &self.root.join(format!("{NAME}-old"))).is_dir(), "not a workspace's");
            use std::os::unix::fs::PermissionsExt;
            for dir in [self.new_dir().parent().unwrap(), self.new_dir().parent().unwrap().parent().unwrap()] {
                assert_eq!(std::fs::metadata(dir).unwrap().permissions().mode() & 0o777, 0o700, "{}", dir.display());
            }
            let before = Db::open(&self.data().join(DB_COPY)).unwrap();
            let kept: String = before.conn().query_row("SELECT content FROM memories", [], |r| r.get(0)).unwrap();
            assert!(kept.contains(&self.old_dir().display().to_string()), "the copy is from before: {kept}");
        }
    }

    #[test]
    fn a_fresh_install_keeps_workspaces_outside_the_checkout_where_a_missing_clone_finds_no_repository() {
        let (_home, home, root) = home();
        let storage = chosen(&root, &root.join(".loft"), || Ok(home.join(".local/share"))).unwrap();
        assert_eq!(storage, dir_for(&root, &home.join(".local/share")));
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
    fn the_chosen_folder_stays_when_the_data_home_changes() {
        let (_home, home, root) = home();
        let no_home = || Err(anyhow::anyhow!("no home"));
        assert!(chosen(&root, &root.join(".loft"), no_home).is_err(), "nothing to choose from");
        let first = chosen(&root, &root.join(".loft"), || Ok(home.join(".local/share"))).unwrap();
        assert_eq!(chosen(&root, &root.join(".loft"), || Ok(home.join("elsewhere"))).unwrap(), first);
        // A start with no home folder still finds the recorded one.
        assert_eq!(chosen(&root, &root.join(".loft"), no_home).unwrap(), first);
    }

    #[test]
    fn the_checkout_guard_leaves_commands_in_a_workspace_with_a_missing_clone_alone() {
        let (_home, home, root) = home();
        let missing = dir_for(&root, &home.join(".local/share")).join("agent-1/loft");
        std::fs::create_dir_all(missing.parent().unwrap()).unwrap();
        for command in ["cargo build", "pnpm install", "git checkout main"] {
            let changes = crate::kill_guard::checkout_changes(command, &missing, &[]);
            assert!(!changes.is_empty(), "{command}: the guard still sees the change");
            let storage = missing.parent().unwrap().parent().unwrap();
            assert!(changes.iter().all(|c| crate::kill_guard::in_agent_storage(&c.folder, storage) && !c.folder.starts_with(&root)), "{command}: {changes:?}");
        }
        // Where the workspaces used to be, git there (with no ceiling, as through `sh -c`) changed the user's checkout.
        let old = root.join(NAME).join("agent-1/loft");
        std::fs::create_dir_all(old.parent().unwrap()).unwrap();
        assert_eq!(crate::kill_guard::checkout_changes("git checkout main", &old, &[])[0].folder, root);
    }

    #[cfg(unix)]
    #[test]
    fn an_existing_install_moves_its_workspaces_once_and_agents_keep_their_clones_notes_transcripts_and_memory() {
        let o = Old::make();
        assert_eq!(o.migrate(), o.new_dir());
        o.assert_moved();
        // Once: the next start uses the new folder and changes nothing.
        assert_eq!(o.migrate(), o.new_dir());
        o.assert_moved();
    }

    #[cfg(unix)]
    #[test]
    fn a_move_a_crash_interrupted_is_finished_on_the_next_start() {
        // Killed after the marker, before the rename.
        let o = Old::make();
        std::fs::write(o.data().join(MARKER), path_bytes(&o.old_dir())).unwrap();
        o.db.copy_to(&o.data().join(DB_COPY)).unwrap();
        assert_eq!(o.migrate(), o.new_dir());
        o.assert_moved();

        // Killed right after the rename: nothing else was updated.
        let o = Old::make();
        o.db.copy_to(&o.data().join(DB_COPY)).unwrap();
        std::fs::write(o.data().join(MARKER), path_bytes(&o.old_dir())).unwrap();
        safe_destination(&o.new_dir(), &o.root, &o.home.join("dev")).unwrap();
        std::fs::rename(o.old_dir(), o.new_dir()).unwrap();
        assert_eq!(o.migrate(), o.new_dir());
        o.assert_moved();
    }

    #[cfg(unix)]
    #[test]
    fn a_move_to_finish_checks_the_destination_again() {
        // Killed right after the rename; the destination is then swapped for a link into the checkout.
        let o = Old::make();
        o.db.copy_to(&o.data().join(DB_COPY)).unwrap();
        std::fs::write(o.data().join(MARKER), path_bytes(&o.old_dir())).unwrap();
        safe_destination(&o.new_dir(), &o.root, &o.home.join("dev")).unwrap();
        std::fs::rename(o.old_dir(), o.new_dir()).unwrap();
        let moved = o.home.join("moved");
        std::fs::rename(o.new_dir(), &moved).unwrap();
        std::fs::create_dir_all(o.root.join("inside")).unwrap();
        std::os::unix::fs::symlink(o.root.join("inside"), o.new_dir()).unwrap();
        let rows = o.rows();
        let refused = migrate(&o.old_dir(), &o.new_dir(), &o.root, &o.home.join("dev"), &o.data(), &o.db, &o.projects());
        assert!(refused.is_err_and(|e| e.to_string().contains("symlink")));
        assert_eq!(o.rows(), rows, "nothing points at it");
        assert!(o.data().join(MARKER).exists(), "the next start tries again");
        // Once the folder is back, the move finishes.
        std::fs::remove_file(o.new_dir()).unwrap();
        std::fs::rename(&moved, o.new_dir()).unwrap();
        assert_eq!(o.migrate(), o.new_dir());
        o.assert_moved();
    }

    #[cfg(unix)]
    #[test]
    fn a_step_that_fails_is_tried_again_on_the_next_start() {
        let o = Old::make();
        // The database can't be written during the first start.
        o.db.conn().execute_batch("CREATE TRIGGER no BEFORE UPDATE ON memories BEGIN SELECT RAISE(ABORT, 'read-only'); END;").unwrap();
        assert_eq!(o.migrate(), o.new_dir());
        assert!(o.data().join(MARKER).exists(), "not done");
        assert!(o.rows().0.contains(&o.old_dir().display().to_string()), "one transaction: nothing changed");
        o.db.conn().execute_batch("DROP TRIGGER no;").unwrap();
        assert_eq!(o.migrate(), o.new_dir());
        o.assert_moved();
    }

    #[cfg(unix)]
    #[test]
    fn both_folders_full_moves_nothing_and_an_empty_new_one_is_no_obstacle() {
        let o = Old::make();
        std::fs::create_dir_all(o.new_dir().join("agent-2")).unwrap();
        let rows = o.rows();
        let (dir, why) = o.migrate_why();
        assert_eq!(dir, o.old_dir(), "the folders the database names stay in use");
        assert!(why.is_some_and(|w| w.contains("already holds files")), "Needs you says why");
        assert!(o.old_dir().join("agent-1/notes/plan.md").exists() && !o.new_dir().join("agent-1").exists());
        assert_eq!(o.rows(), rows, "the database still names what is still there");
        assert!(!o.data().join(MARKER).exists() && !o.data().join(DB_COPY).exists());

        let o = Old::make();
        std::fs::create_dir_all(o.new_dir()).unwrap();
        assert_eq!(o.migrate(), o.new_dir());
        o.assert_moved();
    }

    #[cfg(unix)]
    #[test]
    fn an_old_folder_that_reappears_after_the_move_is_never_used_again() {
        // Docker recreated the old folder's bind-mount sources, empty, after the move.
        let o = Old::make();
        assert_eq!(o.migrate(), o.new_dir());
        std::fs::create_dir_all(o.old_dir().join("agent-2/supabase/volumes/db")).unwrap();
        std::fs::create_dir_all(o.old_dir().join(".agent-files")).unwrap();
        std::fs::write(o.old_dir().join(".agent-files/env.sh"), "").unwrap();
        assert_eq!(o.migrate_why(), (o.new_dir(), None));
        assert!(!o.old_dir().exists(), "the empty folders are removed");
        assert_eq!(reappeared(&o.root, &o.new_dir()), None);
        o.assert_moved();

        // One with a file in it is left for the user, and Needs you names it.
        std::fs::create_dir_all(o.old_dir().join("agent-2/db")).unwrap();
        std::fs::write(o.old_dir().join("agent-2/db/data"), "x").unwrap();
        assert_eq!(o.migrate_why(), (o.new_dir(), None));
        assert!(o.old_dir().join("agent-2/db/data").exists(), "it deleted a file");
        assert_eq!(reappeared(&o.root, &o.new_dir()), Some(o.old_dir()));
        assert!(o.new_dir().join("agent-1/notes/plan.md").exists());

        // A symlink is a file: left too.
        std::fs::remove_dir_all(o.old_dir()).unwrap();
        std::fs::create_dir_all(o.old_dir().join("agent-2")).unwrap();
        std::os::unix::fs::symlink(&o.home, o.old_dir().join("agent-2/link")).unwrap();
        assert_eq!(o.migrate(), o.new_dir());
        assert!(o.old_dir().join("agent-2/link").symlink_metadata().is_ok());
        assert!(o.home.join("dev").exists(), "it followed the link");
        assert_eq!(reappeared(&o.root, &o.old_dir()), None, "not while Loft uses it");

        // An old folder that is a symlink, even to empty folders, is left and shown.
        std::fs::remove_dir_all(o.old_dir()).unwrap();
        let target = o.home.join("target/agent-2");
        std::fs::create_dir_all(&target).unwrap();
        std::os::unix::fs::symlink(target.parent().unwrap(), o.old_dir()).unwrap();
        assert_eq!(o.migrate(), o.new_dir());
        assert!(o.old_dir().is_symlink() && target.is_dir());
        assert_eq!(reappeared(&o.root, &o.new_dir()), Some(o.old_dir()));
    }

    #[test]
    fn removing_the_empty_folders_never_deletes_a_file_that_appeared_since() {
        let (_home, home, _root) = home();
        let old = home.join(NAME);
        std::fs::create_dir_all(old.join("agent-2/db")).unwrap();
        std::fs::create_dir_all(old.join(".agent-files/x")).unwrap();
        std::fs::write(old.join(".agent-files/x/env.sh"), "").unwrap();
        assert!(only_folders(&old, true));
        // A container writes into its bind mount after the check.
        std::fs::write(old.join("agent-2/db/data"), "x").unwrap();
        assert!(remove_folders(&old, true).is_err());
        assert!(old.join("agent-2/db/data").exists());
        std::fs::remove_file(old.join("agent-2/db/data")).unwrap();
        remove_folders(&old, true).unwrap();
        assert!(!old.exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_planted_destination_moves_nothing() {
        // The destination is a link to a folder an agent made.
        let o = Old::make();
        let elsewhere = o.home.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::create_dir_all(o.new_dir().parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&elsewhere, o.new_dir()).unwrap();
        let rows = o.rows();
        assert_eq!(o.migrate(), o.old_dir());
        assert!(o.old_dir().join("agent-1/notes/plan.md").exists() && std::fs::read_dir(&elsewhere).unwrap().next().is_none());
        assert_eq!(o.rows(), rows);
        assert!(!o.data().join(MARKER).exists());

        // `<data home>/loft` leads back into the checkout.
        let o = Old::make();
        let loft = o.new_dir().parent().unwrap().parent().unwrap().to_path_buf();
        std::fs::create_dir_all(loft.parent().unwrap()).unwrap();
        std::fs::create_dir_all(o.root.join("inside")).unwrap();
        std::os::unix::fs::symlink(o.root.join("inside"), &loft).unwrap();
        assert_eq!(o.migrate(), o.old_dir());
        assert!(o.old_dir().join("agent-1").is_dir() && std::fs::read_dir(o.root.join("inside")).unwrap().next().is_none());

        // A link above it, or a git repository around it, counts by the real path.
        let o = Old::make();
        let share = o.new_dir().parent().unwrap().parent().unwrap().parent().unwrap().to_path_buf();
        std::fs::create_dir_all(share.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&o.root, &share).unwrap();
        assert_eq!(o.migrate(), o.old_dir());
        // A home folder kept in git (dotfiles) is no reason to stay: the ceiling covers agents' git.
        let o = Old::make();
        ok(git(&o.home, &["init", "-q"]));
        assert_eq!(o.migrate_why(), (o.new_dir(), None));
        assert!(o.new_dir().join("agent-1").is_dir());

        // A data folder in the projects folder, where agents make repos.
        let o = Old::make();
        let inside = dir_for(&o.root, &o.home.join("dev/share"));
        let (dir, why) = migrate(&o.old_dir(), &inside, &o.root, &o.home.join("dev"), &o.data(), &o.db, &o.projects()).unwrap();
        assert_eq!(dir, o.old_dir());
        assert!(why.is_some_and(|w| w.contains("projects folder")));

        // A fresh install doesn't start on a planted destination.
        let (_home, home, root) = home();
        let new = dir_for(&root, &home.join(".local/share"));
        std::fs::create_dir_all(new.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&root, &new).unwrap();
        let db = Db::open(&root.join(".loft/loft.db")).unwrap();
        let refused = migrate(&root.join(NAME), &new, &root, &home.join("dev"), &root.join(".loft"), &db, &home.join(".claude/projects"));
        assert!(refused.is_err_and(|e| e.to_string().contains("symlink")));
    }

    #[test]
    fn only_the_configured_storage_folder_holds_workspaces() {
        let storage = Path::new("/data/loft/loft-1a2b3c4d/agent-local-storage");
        assert!(holds_in(&storage.join("agent-1/loft"), storage));
        assert!(holds_in(storage, storage));
        // The old folder, or any other of that name, isn't trusted.
        assert!(!holds_in(Path::new("/Users/me/dev/loft/agent-local-storage/agent-1"), storage));
        assert!(!holds_in(Path::new("/tmp/agent-local-storage/x"), storage));
        // Nothing is trusted when Loft named no folder.
        assert!(!holds_in(&storage.join("agent-1"), Path::new("")));
    }

    #[test]
    fn a_failed_move_keeps_loft_working_from_the_old_folder() {
        let o = Old::make();
        // The data folder can't be made: a file is in its way.
        std::fs::write(o.home.join("blocked"), "").unwrap();
        let new = dir_for(&o.root, &o.home.join("blocked"));
        let rows = o.rows();
        assert_eq!(migrate(&o.old_dir(), &new, &o.root, &o.home.join("dev"), &o.data(), &o.db, &o.projects()).unwrap().0, o.old_dir());
        assert!(o.old_dir().join("agent-1/notes").is_dir() && !new.exists());
        assert_eq!(o.rows(), rows);
        assert!(!o.data().join(MARKER).exists(), "the next start tries again from the start");

        // It does, once the way is clear, with a copy of the database as it is then.
        o.db.conn().execute("UPDATE memories SET content = content || ' (later)'", []).unwrap();
        std::fs::remove_file(o.home.join("blocked")).unwrap();
        std::fs::create_dir_all(o.home.join("blocked")).unwrap();
        assert_eq!(migrate(&o.old_dir(), &new, &o.root, &o.home.join("dev"), &o.data(), &o.db, &o.projects()).unwrap().0, new);
        let before = Db::open(&o.data().join(DB_COPY)).unwrap();
        let kept: String = before.conn().query_row("SELECT content FROM memories", [], |r| r.get(0)).unwrap();
        assert!(kept.ends_with(" (later)") && kept.contains(&o.old_dir().display().to_string()), "{kept}");
    }

    #[cfg(unix)]
    #[test]
    fn a_rename_that_failed_leaves_no_sign_of_a_finished_move() {
        use std::os::unix::fs::PermissionsExt;
        let o = Old::make();
        // The checkout can't be written, so the rename fails after the database was copied.
        std::fs::set_permissions(&o.root, std::fs::Permissions::from_mode(0o500)).unwrap();
        let (dir, why) = o.migrate_why();
        std::fs::set_permissions(&o.root, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(dir, o.old_dir());
        assert!(why.is_some_and(|w| w.contains("couldn't rename")));
        assert!(!o.data().join(DB_COPY).exists() && !o.data().join(MARKER).exists());

        // Something then lands in the new folder: the real workspaces are still the old ones.
        std::fs::create_dir_all(o.new_dir().join("agent-2")).unwrap();
        std::fs::write(o.new_dir().join("agent-2/x"), "").unwrap();
        let (dir, why) = o.migrate_why();
        assert_eq!(dir, o.old_dir());
        assert!(why.is_some_and(|w| w.contains("already holds files")));
        assert!(o.old_dir().join("agent-1/notes/plan.md").exists());
    }
}
