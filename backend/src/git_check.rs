//! Clean's Check: whether a clone or worktree in an agent's workspace is safe to delete, asked only when the
//! user clicks Check (`POST /api/workspaces/clean/check`). Loft never runs git in agents' repos on its own.
//!
//! The agent owns the repo and its config, and git runs as Loft, so Loft reads the config files first and runs
//! nothing when they hold any key outside a short allowlist (filter drivers, promisor remotes, fsmonitor,
//! includes…, and a ref storage other than files). Then only two read-only commands run, each with a timeout:
//! `git status --porcelain`, and `git rev-list --count <branch> --not --remotes` for each branch its refs files
//! name. With hooks and fsmonitor off, no lazy fetch, no optional locks (status doesn't refresh the index) and the
//! workspace as git's ceiling.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::state::AppState;

/// How long one git call may take.
const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Deserialize)]
pub struct CheckBody {
    agent: String,
    /// A top-level item of the agent's workspace, like `loft` or `wt1895`.
    rel: String,
}

/// What Check found.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct GitCheck {
    /// "safe" (clean and pushed), "unsafe" (unpushed commits or uncommitted changes) or "unknown" (not checked).
    pub state: &'static str,
    /// What to say, like "3 commits not pushed (feat/x)" or "Can't check: unusual git config".
    pub lines: Vec<String>,
}

fn unknown(why: &str) -> GitCheck {
    GitCheck { state: "unknown", lines: vec![format!("Can't check: {why}")] }
}

/// `POST /api/workspaces/clean/check`: checks one clone or worktree. Never ticks anything.
pub async fn check(axum::extract::State(state): axum::extract::State<AppState>, axum::Json(b): axum::Json<CheckBody>) -> axum::Json<GitCheck> {
    let projects = crate::sweep::projects(&state).remove(&b.agent).unwrap_or_default();
    axum::Json(run(&state.cfg.storage_dir, &b.agent, &b.rel, &projects).await)
}

pub async fn run(storage: &Path, agent: &str, rel: &str, projects: &[String]) -> GitCheck {
    let (ws, repo) = match place(storage, agent, rel) {
        Ok(p) => p,
        Err(why) => return unknown(why),
    };
    let common = match plain_config(&ws, &repo) {
        Ok(c) => c,
        Err(why) => return unknown(why),
    };
    let Some(status) = git(&ws, &repo, &["status", "--porcelain", "--ignore-submodules=all"]).await else { return unknown("git failed") };
    let mut lines = vec![];
    for b in branches(&common) {
        let Some(n) = git(&ws, &repo, &["rev-list", "--count", &format!("refs/heads/{b}"), "--not", "--remotes"]).await else { return unknown("git failed") };
        match n.trim().parse::<usize>() {
            Ok(0) => {}
            Ok(n) => lines.push(format!("{n} commit{} not pushed ({b})", if n == 1 { "" } else { "s" })),
            Err(_) => return unknown("git failed"),
        }
    }
    let changed = status.lines().filter(|l| !l.is_empty()).count();
    if changed > 0 {
        lines.push(format!("Uncommitted changes ({changed} file{})", if changed == 1 { "" } else { "s" }));
    }
    if !lines.is_empty() {
        return GitCheck { state: "unsafe", lines };
    }
    let mut lines = vec!["Clean, all pushed: safe to delete".to_string()];
    if crate::sweep::main_clones(&ws, projects).contains_key(rel) {
        lines.push("the agent clones it again on its next ticket".into());
    }
    GitCheck { state: "safe", lines }
}

/// The agent's workspace and the repo: a real folder at the top of it, spelled as on disk, not a symlink.
fn place(storage: &Path, agent: &str, rel: &str) -> Result<(PathBuf, PathBuf), &'static str> {
    let plain = |s: &str| !s.is_empty() && s != "." && s != ".." && !s.contains('/') && !s.contains(char::is_control);
    if !plain(agent) || agent.starts_with('.') || !plain(rel) {
        return Err("not in a workspace");
    }
    let root = storage.canonicalize().map_err(|_| "not in a workspace")?;
    let ws = root.join(agent);
    let repo = ws.join(rel);
    for p in [&ws, &repo] {
        let m = p.symlink_metadata().map_err(|_| "no longer there")?;
        if m.is_symlink() || !m.is_dir() {
            return Err("not a folder");
        }
    }
    // APFS finds a name whatever its case or Unicode form; only the name on disk is checked.
    if repo.canonicalize().ok().as_deref() != Some(repo.as_path()) {
        return Err("not the name on disk");
    }
    Ok((ws, repo))
}

/// Whether the repo's own config files (the clone's, or a worktree's and its base clone's) hold only plain keys.
/// Read as files, before any git runs. The user's global and system config are theirs, and stay out of it. The
/// folder that holds the repo's refs and config: the clone's `.git`, or a worktree's base clone's.
fn plain_config(ws: &Path, repo: &Path) -> Result<PathBuf, &'static str> {
    let dot_git = repo.join(".git");
    let m = dot_git.symlink_metadata().map_err(|_| "not a git repo")?;
    let (git_dir, common) = if m.is_dir() {
        (dot_git.clone(), dot_git)
    } else if m.is_file() {
        // A worktree: `.git` names its folder in the base clone, which names the base's own.
        let text = std::fs::read_to_string(&dot_git).map_err(|_| "not a git repo")?;
        let dir = text.trim().strip_prefix("gitdir:").ok_or("not a git repo")?.trim();
        let git_dir = repo.join(dir);
        let common = std::fs::read_to_string(git_dir.join("commondir")).map(|c| git_dir.join(c.trim())).map_err(|_| "not a git repo")?;
        (git_dir, common)
    } else {
        return Err("not a git repo");
    };
    // An agent could point them anywhere; git would then read config and objects from there.
    for d in [&git_dir, &common] {
        if !d.canonicalize().is_ok_and(|c| c.starts_with(ws)) {
            return Err("git folder outside the workspace");
        }
    }
    for f in [common.join("config"), git_dir.join("config.worktree"), common.join("config.worktree")] {
        match std::fs::read_to_string(&f) {
            Ok(text) if keys(&text).is_some_and(|ks| ks.iter().all(|k| plain_key(k))) => {}
            Ok(_) => return Err("unusual git config"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("can't read its git config"),
        }
    }
    Ok(common)
}

/// The branches the repo's refs files name (loose under `refs/heads/`, and `packed-refs`), sorted; symlinks and
/// names git wouldn't write are left out.
fn branches(common: &Path) -> Vec<String> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let (Ok(t), Some(name)) = (e.file_type(), e.file_name().to_str().map(str::to_string)) else { continue };
            let full = format!("{prefix}{name}");
            if t.is_dir() {
                walk(&e.path(), &format!("{full}/"), out);
            } else if t.is_file() {
                out.push(full);
            }
        }
    }
    let mut out = vec![];
    walk(&common.join("refs/heads"), "", &mut out);
    let packed = std::fs::read_to_string(common.join("packed-refs")).unwrap_or_default();
    out.extend(packed.lines().filter_map(|l| l.split_once(' ')?.1.strip_prefix("refs/heads/").map(str::to_string)));
    out.retain(|b| !b.is_empty() && !b.starts_with('-') && !b.contains(|c: char| c.is_control() || c.is_whitespace()) && !b.split('/').any(|p| p.is_empty() || p == "." || p == ".."));
    out.sort();
    out.dedup();
    out
}

/// The keys a git config file sets, as `section[.subsection].name`; None when a line isn't plainly one.
fn keys(text: &str) -> Option<Vec<String>> {
    let mut section = String::new();
    let mut out = vec![];
    for line in text.lines() {
        let mut l = line.trim();
        if l.is_empty() || l.starts_with('#') || l.starts_with(';') {
            continue;
        }
        // A value continued on the next line, which could read like a key.
        if l.ends_with('\\') {
            return None;
        }
        if let Some(rest) = l.strip_prefix('[') {
            let (head, after) = rest.split_once(']')?;
            section = match head.split_once(char::is_whitespace) {
                Some((s, sub)) => format!("{}.{}", s.to_ascii_lowercase(), sub.trim().strip_prefix('"')?.strip_suffix('"')?),
                None => head.to_ascii_lowercase(),
            };
            l = after.trim();
            if l.is_empty() || l.starts_with('#') || l.starts_with(';') {
                continue;
            }
        }
        let name: String = l.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
        let rest = l[name.len()..].trim_start();
        if section.is_empty() || name.is_empty() || !(rest.is_empty() || rest.starts_with('=')) {
            return None;
        }
        out.push(format!("{section}.{}", name.to_ascii_lowercase()));
    }
    Some(out)
}

/// A config key `git clone` or Loft's base clones set, which no git command turns into a command to run.
fn plain_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    let Some((section, rest)) = key.split_once('.') else { return false };
    let name = rest.rsplit_once('.').map_or(rest, |(_, n)| n);
    match section {
        // `core.hookspath` is Loft's own on base clones, and git runs here with hooks off anyway.
        "core" => !rest.contains('.') && matches!(name, "repositoryformatversion" | "filemode" | "bare" | "logallrefupdates" | "ignorecase" | "precomposeunicode" | "hookspath"),
        // `gh-resolved` is what `gh` records for the default repo.
        "remote" => rest.contains('.') && matches!(name, "url" | "fetch" | "gh-resolved"),
        "branch" => rest.contains('.') && matches!(name, "remote" | "merge"),
        "user" => !rest.contains('.') && matches!(name, "name" | "email"),
        _ => false,
    }
}

/// Runs read-only git in `repo` with hooks and fsmonitor off, no lazy fetch, no optional locks and `ws` as the
/// ceiling, killed after [`TIMEOUT`]; its output if it succeeded.
async fn git(ws: &Path, repo: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("-C")
        .arg(repo)
        .args(crate::repos::SAFE_GIT)
        .args(args)
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_CEILING_DIRECTORIES", ws)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let out = tokio::time::timeout(TIMEOUT, cmd.output()).await.ok()?.ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    fn sh(dir: &Path, args: &[&str]) {
        let out = Command::new("git").arg("-C").arg(dir).args(args).env("GIT_CONFIG_GLOBAL", "/dev/null").output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }

    /// agent-local-storage with agent `a`: `origin.git` (bare, the remote) and `loft`, a clone of it with one
    /// pushed commit on main.
    fn setup() -> (PathBuf, PathBuf) {
        let storage = std::env::temp_dir().join(format!("loft-gitcheck-{}", uuid::Uuid::new_v4()));
        let ws = storage.join("a");
        fs::create_dir_all(&ws).unwrap();
        let storage = storage.canonicalize().unwrap();
        let ws = storage.join("a");
        sh(&ws, &["init", "-q", "--bare", "-b", "main", "origin.git"]);
        sh(&ws, &["clone", "-q", "origin.git", "loft"]);
        let loft = ws.join("loft");
        sh(&loft, &["config", "user.name", "T"]);
        sh(&loft, &["config", "user.email", "t@example.com"]);
        sh(&loft, &["switch", "-q", "-c", "main"]);
        fs::write(loft.join("a.txt"), "a").unwrap();
        sh(&loft, &["add", "a.txt"]);
        sh(&loft, &["commit", "-q", "-m", "a"]);
        sh(&loft, &["push", "-q", "-u", "origin", "main"]);
        (storage, loft)
    }

    fn commit(dir: &Path, f: &str) {
        fs::write(dir.join(f), f).unwrap();
        sh(dir, &["add", f]);
        sh(dir, &["commit", "-q", "-m", f]);
    }

    #[tokio::test]
    async fn a_clean_pushed_clone_is_safe_and_a_main_clone_says_it_comes_back() {
        let (storage, _) = setup();
        let c = run(&storage, "a", "loft", &[]).await;
        assert_eq!(c, GitCheck { state: "safe", lines: vec!["Clean, all pushed: safe to delete".into()] });
        // Its project names the clone's repo (`origin`), so it is the agent's main clone.
        let c = run(&storage, "a", "loft", &["origin".into()]).await;
        assert_eq!(c.lines, ["Clean, all pushed: safe to delete", "the agent clones it again on its next ticket"]);
    }

    #[tokio::test]
    async fn unpushed_commits_are_counted_per_branch_and_uncommitted_files_too() {
        let (storage, loft) = setup();
        commit(&loft, "b.txt");
        sh(&loft, &["switch", "-q", "-c", "feat/x"]);
        commit(&loft, "c.txt");
        commit(&loft, "d.txt");
        fs::write(loft.join("a.txt"), "changed").unwrap();
        fs::write(loft.join("new.txt"), "new").unwrap();
        let c = run(&storage, "a", "loft", &[]).await;
        // feat/x has main's unpushed commit too.
        assert_eq!(c, GitCheck { state: "unsafe", lines: vec!["3 commits not pushed (feat/x)".into(), "1 commit not pushed (main)".into(), "Uncommitted changes (2 files)".into()] });
        // Packed refs count the same.
        sh(&loft, &["pack-refs", "--all"]);
        assert_eq!(run(&storage, "a", "loft", &[]).await, c);
    }

    #[tokio::test]
    async fn a_worktree_is_checked_through_its_base_clone() {
        let (storage, loft) = setup();
        sh(&loft, &["worktree", "add", "-q", "../wt", "-b", "wt-branch"]);
        let wt = storage.join("a/wt");
        assert_eq!(run(&storage, "a", "wt", &[]).await.state, "safe");
        commit(&wt, "w.txt");
        assert_eq!(run(&storage, "a", "wt", &[]).await.lines, ["1 commit not pushed (wt-branch)"]);
    }

    #[tokio::test]
    async fn a_repo_with_an_unusual_config_key_is_refused_without_running_git() {
        let (storage, loft) = setup();
        let marker = storage.join("ran");
        let cmd = format!("touch {}", marker.display());
        let extras = [
            format!("[core]\n\tfsmonitor = {cmd}\n"),
            format!("[remote \"origin\"]\n\tpromisor = true\n\tuploadpack = {cmd}\n"),
            format!("[include]\n\tpath = {}\n", storage.join("evil").display()),
            format!("[filter \"x\"]\n\tclean = {cmd}\n"),
            format!("[core]\n\tsshCommand = {cmd}\n"),
            format!("[core]\n\tpager = \\\n{cmd}\n"),
            format!("[core] fsmonitor = {cmd}\n"),
            format!("[Core]\n\tFSMonitor = {cmd}\n"),
        ];
        // A changed file, so `git status` would run a clean filter if it got that far.
        fs::write(loft.join(".gitattributes"), "* filter=x\n").unwrap();
        fs::write(loft.join("a.txt"), "changed").unwrap();
        let config = fs::read_to_string(loft.join(".git/config")).unwrap();
        for extra in extras {
            fs::write(loft.join(".git/config"), format!("{config}{extra}")).unwrap();
            assert_eq!(run(&storage, "a", "loft", &[]).await, unknown("unusual git config"), "{extra}");
        }
        // In a worktree's own config too.
        fs::write(loft.join(".git/config"), &config).unwrap();
        sh(&loft, &["worktree", "add", "-q", "../wt"]);
        fs::write(loft.join(".git/worktrees/wt/config.worktree"), format!("[core]\n\tfsmonitor = {cmd}\n")).unwrap();
        assert_eq!(run(&storage, "a", "wt", &[]).await, unknown("unusual git config"));
        assert!(!marker.exists(), "a command from the repo's config ran");
    }

    #[tokio::test]
    async fn only_a_real_folder_at_the_top_of_the_agent_s_own_workspace_is_checked() {
        let (storage, loft) = setup();
        let outside = storage.parent().unwrap().join(format!("loft-gitcheck-out-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&loft, storage.join("a/link")).unwrap();
        fs::create_dir_all(storage.join("a/plain")).unwrap();
        // A worktree whose `.git` points outside the workspace.
        fs::create_dir_all(storage.join("a/away")).unwrap();
        fs::write(storage.join("a/away/.git"), format!("gitdir: {}\n", loft.join(".git").display())).unwrap();
        fs::create_dir_all(outside.join("g")).unwrap();
        fs::write(outside.join("g/commondir"), "../g\n").unwrap();
        fs::create_dir_all(storage.join("a/far")).unwrap();
        fs::write(storage.join("a/far/.git"), format!("gitdir: {}\n", outside.join("g").display())).unwrap();
        for (agent, rel, why) in [
            ("a", "link", "not a folder"),
            ("a", "LOFT", "not the name on disk"),
            ("a", "../a/loft", "not in a workspace"),
            ("a", "..", "not in a workspace"),
            ("A", "loft", "not the name on disk"),
            (".x", "loft", "not in a workspace"),
            ("a", "plain", "not a git repo"),
            ("a", "away", "not a git repo"),
            ("a", "far", "git folder outside the workspace"),
            ("b", "loft", "no longer there"),
        ] {
            assert_eq!(run(&storage, agent, rel, &[]).await, unknown(why), "{agent}/{rel}");
        }
    }

    #[test]
    fn config_keys_are_read_as_git_writes_them() {
        let text = "[core]\n\trepositoryformatversion = 0\n\tbare = false\n; a comment\n[remote \"origin\"]\n\turl = git@github.com:a/b.git\n[branch \"feat/x.y\"]\n\tremote = origin\n";
        assert_eq!(keys(text).unwrap(), ["core.repositoryformatversion", "core.bare", "remote.origin.url", "branch.feat/x.y.remote"]);
        assert!(keys("[core]\n\tbare = false \\\n").is_none());
        assert!(keys("bare = false\n").is_none(), "a key before any section");
        assert!(keys("[core]\n\tbare! = false\n").is_none());
    }
}
