//! Git dashboard: scans the projects directory (recursively) and collects local git stats plus
//! GitHub PRs/issues for repositories with a GitHub remote.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::{Local, TimeZone};
use serde::Serialize;
use serde_json::Value;
use tokio::process::Command;

use crate::state::AppState;

const ACTIVITY_DAYS: i64 = 182;
const GIT_TTL: Duration = Duration::from_secs(30);
const GITHUB_TTL: Duration = Duration::from_secs(300);

#[derive(Default)]
pub struct RepoCache {
    git: Mutex<Option<(Instant, Vec<RepoSummary>)>>,
    github: Mutex<HashMap<String, (Instant, GithubInfo)>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepoSummary {
    pub name: String,
    pub path: String,
    pub branch: String,
    pub remote: Option<String>,
    pub github: Option<String>,
    pub ahead: i64,
    pub behind: i64,
    pub staged: i64,
    pub modified: i64,
    pub untracked: i64,
    pub stashes: i64,
    pub branches: i64,
    pub total_commits: i64,
    pub last_commit: Option<Commit>,
    /// Commits per day for the last ACTIVITY_DAYS days, oldest first.
    pub daily: Vec<i64>,
    pub languages: Vec<(String, i64)>,
    pub contributors: Vec<(String, i64)>,
    /// The project's websites, from `sites.json` at the repo root (see [`read_sites`]).
    pub sites: Vec<Site>,
    pub error: Option<String>,
}

/// A website that belongs to a project: its live site, an admin page, docs, a staging copy.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Site {
    pub name: String,
    pub url: String,
    pub description: Option<String>,
    /// a heading to group it under ("public", "admin")
    pub group: Option<String>,
    /// only reachable from this machine or the tailnet
    pub private: bool,
}

/// most sites shown for one project
const MAX_SITES: usize = 50;

/// The sites listed in `<repo>/sites.json`: `{ "sites": [{ "name", "url", "description"?, "group"?, "private"? }] }`
/// (or just the array). Entries without a name or an http(s) URL are skipped; no file is no sites.
fn read_sites(dir: &Path) -> Vec<Site> {
    let Ok(text) = std::fs::read_to_string(dir.join("sites.json")) else { return vec![] };
    let Ok(v) = serde_json::from_str::<Value>(&text) else { return vec![] };
    let list = v.get("sites").unwrap_or(&v).as_array().cloned().unwrap_or_default();
    let text = |e: &Value, k: &str| e.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(String::from);
    list.iter()
        .filter_map(|e| {
            let url = text(e, "url").filter(|u| u.starts_with("https://") || u.starts_with("http://"))?;
            Some(Site {
                name: text(e, "name")?,
                url,
                description: text(e, "description"),
                group: text(e, "group"),
                private: e.get("private").and_then(Value::as_bool).unwrap_or(false),
            })
        })
        .take(MAX_SITES)
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct Commit {
    pub hash: String,
    pub subject: String,
    pub author: String,
    pub timestamp: i64,
    pub insertions: Option<i64>,
    pub deletions: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct GithubInfo {
    pub slug: String,
    pub stars: i64,
    pub forks: i64,
    pub watchers: i64,
    pub default_branch: String,
    pub description: Option<String>,
    pub pulls: Vec<GithubItem>,
    pub issues: Vec<GithubItem>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GithubItem {
    pub number: i64,
    pub title: String,
    pub author: String,
    pub url: String,
    pub draft: bool,
    pub labels: Vec<String>,
    pub comments: i64,
    pub created_at: String,
    pub updated_at: String,
}

pub(crate) async fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .await
        .context("spawning git")?;
    if !out.status.success() {
        anyhow::bail!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A project name as Loft stores it: a name or tag, or a folder path relative to the projects folder
/// (`work/isa`, as the Repos scan names nested repos). Never absolute, never `..`, `.` or empty parts,
/// so it can't name a folder outside the projects folder.
pub fn project_name(raw: &str) -> Result<String, String> {
    let p = raw.trim();
    if p.is_empty() {
        return Ok(String::new());
    }
    let bad = p.chars().count() > 100
        || p.starts_with(['/', '\\', '~'])
        || p.contains('\\')
        || p.chars().any(char::is_control)
        || p.split('/').any(|c| c.is_empty() || c == "." || c == "..")
        || Path::new(p).is_absolute();
    if bad {
        return Err("project must be a name, or a folder inside the projects folder (no `..`, no absolute path)".into());
    }
    Ok(p.to_string())
}

/// The folder of `project` inside the projects folder, resolved (symlinks too); None when it isn't one.
pub fn project_dir(state: &AppState, project: &str) -> Option<std::path::PathBuf> {
    let name = project_name(project).ok().filter(|n| !n.is_empty())?;
    let root = state.settings.boot().projects_dir.canonicalize().ok()?;
    let dir = root.join(&name).canonicalize().ok()?;
    dir.starts_with(&root).then_some(dir)
}

/// `owner/repo` of a project from its folder's `origin`; the error says what's missing, without a full stop.
pub async fn github_repo(state: &AppState, project: &str) -> Result<String, String> {
    let project = project.trim();
    project_name(project)?;
    let Some(dir) = project_dir(state, project) else {
        return Err(format!("The project folder {} isn't a git repo", state.settings.boot().projects_dir.join(project).display()));
    };
    if !dir.join(".git").exists() {
        return Err(format!("The project folder {} isn't a git repo", dir.display()));
    }
    let remote = git(&dir, &["remote", "get-url", "origin"]).await.map_err(|_| format!("The project {project} has no `origin` remote"))?;
    github_slug(&remote).ok_or_else(|| format!("The project {project}'s `origin` ({}) isn't on GitHub", strip_userinfo(remote.trim())))
}

/// A remote URL without the `user:token@` part, for messages.
fn strip_userinfo(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else { return url.to_string() };
    let host_end = rest.find('/').unwrap_or(rest.len());
    match rest[..host_end].rfind('@') {
        Some(at) => format!("{scheme}://{}", &rest[at + 1..]),
        None => url.to_string(),
    }
}

/// Parse `owner/repo` from any GitHub remote URL form.
pub fn github_slug(remote: &str) -> Option<String> {
    let r = remote.trim().trim_end_matches('/').trim_end_matches(".git");
    let rest = r
        .strip_prefix("git@github.com:")
        .or_else(|| r.strip_prefix("ssh://git@github.com/"))
        .or_else(|| r.strip_prefix("https://github.com/"))
        .or_else(|| r.strip_prefix("http://github.com/"))
        .or_else(|| r.split_once("@github.com/").map(|(_, s)| s))?;
    let mut parts = rest.split('/');
    let (owner, repo) = (parts.next()?, parts.next()?);
    Some(format!("{owner}/{repo}"))
}

fn language_for(path: &str) -> Option<&'static str> {
    let ext = Path::new(path).extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "rs" => "Rust",
        "ts" | "tsx" | "mts" | "cts" => "TypeScript",
        "js" | "jsx" | "mjs" | "cjs" => "JavaScript",
        "py" => "Python",
        "go" => "Go",
        "rb" => "Ruby",
        "java" => "Java",
        "kt" | "kts" => "Kotlin",
        "swift" => "Swift",
        "c" | "h" => "C",
        "cc" | "cpp" | "hpp" | "cxx" => "C++",
        "cs" => "C#",
        "php" => "PHP",
        "lua" => "Lua",
        "zig" => "Zig",
        "ex" | "exs" => "Elixir",
        "dart" => "Dart",
        "scala" => "Scala",
        "sh" | "bash" | "zsh" => "Shell",
        "html" | "htm" => "HTML",
        "css" | "scss" | "sass" | "less" => "CSS",
        "vue" => "Vue",
        "svelte" => "Svelte",
        "astro" => "Astro",
        "md" | "mdx" => "Markdown",
        "sql" => "SQL",
        "gd" => "GDScript",
        "glsl" | "wgsl" | "hlsl" => "Shaders",
        _ => return None,
    })
}

async fn summarize(name: String, dir: PathBuf) -> RepoSummary {
    let mut s = RepoSummary {
        name,
        path: dir.to_string_lossy().into_owned(),
        branch: String::new(),
        remote: None,
        github: None,
        ahead: 0,
        behind: 0,
        staged: 0,
        modified: 0,
        untracked: 0,
        stashes: 0,
        branches: 0,
        total_commits: 0,
        last_commit: None,
        daily: vec![0; ACTIVITY_DAYS as usize],
        languages: vec![],
        contributors: vec![],
        sites: vec![],
        error: None,
    };
    s.sites = read_sites(&dir);
    if let Err(e) = fill(&dir, &mut s).await {
        s.error = Some(e.to_string());
    }
    s
}

async fn fill(dir: &Path, s: &mut RepoSummary) -> Result<()> {
    let since = format!("--since={ACTIVITY_DAYS} days ago");
    let log_args = ["log", "--all", since.as_str(), "--format=%ct"];
    let (status, remote, count, last, log, stash, branches, files, shortlog) = tokio::join!(
        git(dir, &["status", "--porcelain=v2", "--branch"]),
        git(dir, &["remote", "get-url", "origin"]),
        git(dir, &["rev-list", "--count", "HEAD"]),
        git(dir, &["log", "-1", "--shortstat", "--format=%H%x1f%s%x1f%an%x1f%ct"]),
        git(dir, &log_args),
        git(dir, &["stash", "list"]),
        git(dir, &["branch", "--format=%(refname:short)"]),
        git(dir, &["ls-files"]),
        git(dir, &["shortlog", "-sn", "--no-merges", "HEAD"]),
    );

    let status = status?;
    for line in status.lines() {
        if let Some(b) = line.strip_prefix("# branch.head ") {
            s.branch = b.to_string();
        } else if let Some(ab) = line.strip_prefix("# branch.ab ") {
            let mut it = ab.split_whitespace();
            s.ahead = it.next().and_then(|v| v.trim_start_matches('+').parse().ok()).unwrap_or(0);
            s.behind = it.next().and_then(|v| v.trim_start_matches('-').parse().ok()).unwrap_or(0);
        } else if line.starts_with("? ") {
            s.untracked += 1;
        } else if line.starts_with("1 ") || line.starts_with("2 ") || line.starts_with("u ") {
            let xy = line.split_whitespace().nth(1).unwrap_or("..").as_bytes();
            if xy.first().is_some_and(|c| *c != b'.') {
                s.staged += 1;
            }
            if xy.get(1).is_some_and(|c| *c != b'.') {
                s.modified += 1;
            }
        }
    }

    if let Ok(r) = remote {
        let r = r.trim().to_string();
        s.github = github_slug(&r);
        s.remote = Some(r);
    }
    s.total_commits = count.ok().and_then(|c| c.trim().parse().ok()).unwrap_or(0);
    if let Ok(last) = last {
        let mut lines = last.lines().filter(|l| !l.trim().is_empty());
        if let Some(head) = lines.next() {
            let f: Vec<&str> = head.split('\u{1f}').collect();
            if f.len() == 4 {
                let stat = lines.next().unwrap_or("");
                let num_before = |word: &str| {
                    stat.split(',')
                        .find(|p| p.contains(word))
                        .and_then(|p| p.split_whitespace().next()?.parse().ok())
                };
                s.last_commit = Some(Commit {
                    hash: f[0].into(),
                    subject: f[1].into(),
                    author: f[2].into(),
                    timestamp: f[3].parse().unwrap_or(0),
                    insertions: num_before("insertion"),
                    deletions: num_before("deletion"),
                });
            }
        }
    }

    if let Ok(log) = log {
        let today = Local::now().date_naive();
        for ts in log.lines().filter_map(|l| l.trim().parse::<i64>().ok()) {
            if let Some(dt) = Local.timestamp_opt(ts, 0).single() {
                let age = (today - dt.date_naive()).num_days();
                if (0..ACTIVITY_DAYS).contains(&age) {
                    s.daily[(ACTIVITY_DAYS - 1 - age) as usize] += 1;
                }
            }
        }
    }
    s.stashes = stash.map(|o| o.lines().count() as i64).unwrap_or(0);
    s.branches = branches.map(|o| o.lines().count() as i64).unwrap_or(0);

    if let Ok(files) = files {
        let mut langs: BTreeMap<&str, i64> = BTreeMap::new();
        for f in files.lines() {
            if let Some(l) = language_for(f) {
                *langs.entry(l).or_default() += 1;
            }
        }
        let mut v: Vec<(String, i64)> = langs.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        v.truncate(6);
        s.languages = v;
    }
    if let Ok(sl) = shortlog {
        s.contributors = sl
            .lines()
            .filter_map(|l| {
                let (n, name) = l.trim().split_once('\t')?;
                Some((name.to_string(), n.trim().parse().ok()?))
            })
            .take(5)
            .collect();
    }
    Ok(())
}

/// How many folders deep under `root` to look for repos (`work/client/app` is 3).
const MAX_DEPTH: usize = 4;

/// Every repo under `root`, named by its path relative to it (`work/meuez`).
/// A repo is a folder with a `.git` directory; we don't look inside one, so
/// worktrees and submodules (whose `.git` is a file) aren't listed separately.
fn discover(root: &Path) -> Vec<(String, PathBuf)> {
    fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<(String, PathBuf)>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for p in rd.filter_map(|e| e.ok()).map(|e| e.path()) {
            let hidden = p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'));
            if hidden || !p.is_dir() {
                continue;
            }
            if p.join(".git").is_dir() {
                let name = p.strip_prefix(root).unwrap().to_string_lossy().into_owned();
                out.push((name, p));
            } else if depth < MAX_DEPTH {
                walk(root, &p, depth + 1, out);
            }
        }
    }
    let mut out = vec![];
    walk(root, root, 1, &mut out);
    out.sort();
    out
}

pub async fn list(state: &AppState, force: bool) -> Vec<RepoSummary> {
    if !force {
        if let Some((at, v)) = &*state.repos.git.lock().unwrap() {
            if at.elapsed() < GIT_TTL {
                return v.clone();
            }
        }
    }
    let dirs = discover(&state.settings.boot().projects_dir);
    let handles: Vec<_> = dirs.into_iter().map(|(n, d)| tokio::spawn(summarize(n, d))).collect();
    let mut out = vec![];
    for h in handles {
        if let Ok(s) = h.await {
            out.push(s);
        }
    }
    *state.repos.git.lock().unwrap() = Some((Instant::now(), out.clone()));
    out
}

pub async fn recent_commits(state: &AppState, name: &str, limit: usize) -> Result<Vec<Commit>> {
    let dir = state.settings.boot().projects_dir.join(name);
    anyhow::ensure!(
        name.split('/').all(|c| !c.is_empty() && c != "." && c != "..") && dir.join(".git").is_dir(),
        "unknown repo {name}"
    );
    let out = git(
        &dir,
        &["log", &format!("-{limit}"), "--shortstat", "--format=@@%H%x1f%s%x1f%an%x1f%ct"],
    )
    .await?;
    let mut commits = vec![];
    for chunk in out.split("@@").filter(|c| !c.trim().is_empty()) {
        let mut lines = chunk.lines().filter(|l| !l.trim().is_empty());
        let Some(head) = lines.next() else { continue };
        let f: Vec<&str> = head.split('\u{1f}').collect();
        if f.len() != 4 {
            continue;
        }
        let stat = lines.next().unwrap_or("");
        let num = |word: &str| {
            stat.split(',')
                .find(|p| p.contains(word))
                .and_then(|p| p.split_whitespace().next()?.parse().ok())
        };
        commits.push(Commit {
            hash: f[0].into(),
            subject: f[1].into(),
            author: f[2].into(),
            timestamp: f[3].parse().unwrap_or(0),
            insertions: num("insertion"),
            deletions: num("deletion"),
        });
    }
    Ok(commits)
}

// ---- GitHub ------------------------------------------------------------

async fn gh_get(state: &AppState, path: &str) -> Result<Value> {
    crate::github::get(state, path).await
}

fn to_item(v: &Value) -> GithubItem {
    GithubItem {
        number: v["number"].as_i64().unwrap_or(0),
        title: v["title"].as_str().unwrap_or("").into(),
        author: v["user"]["login"].as_str().unwrap_or("").into(),
        url: v["html_url"].as_str().unwrap_or("").into(),
        draft: v["draft"].as_bool().unwrap_or(false),
        labels: v["labels"]
            .as_array()
            .map(|a| a.iter().filter_map(|l| l["name"].as_str().map(Into::into)).collect())
            .unwrap_or_default(),
        comments: v["comments"].as_i64().unwrap_or(0),
        created_at: v["created_at"].as_str().unwrap_or("").into(),
        updated_at: v["updated_at"].as_str().unwrap_or("").into(),
    }
}

async fn fetch_github(state: &AppState, slug: &str) -> GithubInfo {
    let mut info = GithubInfo { slug: slug.into(), ..Default::default() };
    let (p_repo, p_pulls, p_issues) = (
        format!("/repos/{slug}"),
        format!("/repos/{slug}/pulls?state=open&per_page=30"),
        format!("/repos/{slug}/issues?state=open&per_page=30"),
    );
    let (repo, pulls, issues) = tokio::join!(gh_get(state, &p_repo), gh_get(state, &p_pulls), gh_get(state, &p_issues));
    match repo {
        Ok(r) => {
            info.stars = r["stargazers_count"].as_i64().unwrap_or(0);
            info.forks = r["forks_count"].as_i64().unwrap_or(0);
            info.watchers = r["subscribers_count"].as_i64().unwrap_or(0);
            info.default_branch = r["default_branch"].as_str().unwrap_or("").into();
            info.description = r["description"].as_str().map(Into::into);
        }
        Err(e) => info.error = Some(e.to_string()),
    }
    if let Ok(Value::Array(p)) = pulls {
        info.pulls = p.iter().map(to_item).collect();
    }
    if let Ok(Value::Array(i)) = issues {
        // The issues endpoint also returns PRs; drop those.
        info.issues = i.iter().filter(|v| v.get("pull_request").is_none()).map(to_item).collect();
    }
    info
}

pub async fn github(state: &AppState, force: bool) -> HashMap<String, GithubInfo> {
    let repos = list(state, false).await;
    let mut out = HashMap::new();
    let mut todo = vec![];
    {
        let cache = state.repos.github.lock().unwrap();
        for r in &repos {
            let Some(slug) = &r.github else { continue };
            match cache.get(slug) {
                Some((at, info)) if !force && at.elapsed() < GITHUB_TTL => {
                    out.insert(r.name.clone(), info.clone());
                }
                _ => todo.push((r.name.clone(), slug.clone())),
            }
        }
    }
    let fetched = futures::future::join_all(todo.into_iter().map(|(name, slug)| {
        let state = state.clone();
        async move {
            let info = fetch_github(&state, &slug).await;
            (name, slug, info)
        }
    }))
    .await;
    let mut cache = state.repos.github.lock().unwrap();
    for (name, slug, info) in fetched {
        // Don't cache failures for long; they're usually rate limits.
        if info.error.is_none() {
            cache.insert(slug, (Instant::now(), info.clone()));
        }
        out.insert(name, info);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Site, discover, github_slug, read_sites};

    #[test]
    fn discovers_nested_repos() {
        let root = std::env::temp_dir().join(format!("loft-discover-{}", std::process::id()));
        for d in ["top/.git", "top/inner/.git", "work/meuez/.git", "work/notes", ".hidden/x/.git", "a/b/c/d/deep/.git"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::create_dir_all(root.join("work/meuez-worktrees/feat")).unwrap();
        std::fs::write(root.join("work/meuez-worktrees/feat/.git"), "gitdir: ../../meuez/.git").unwrap();
        let names: Vec<String> = discover(&root).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["top", "work/meuez"]);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn parses_remotes() {
        assert_eq!(github_slug("git@github.com:a/b.git").as_deref(), Some("a/b"));
        assert_eq!(github_slug("https://github.com/a/b").as_deref(), Some("a/b"));
        assert_eq!(github_slug("https://x:tok@github.com/a/b.git").as_deref(), Some("a/b"));
        assert_eq!(github_slug("https://gitlab.com/a/b"), None);
    }

    #[test]
    fn reads_sites() {
        let dir = std::env::temp_dir().join(format!("loft-sites-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read_sites(&dir).is_empty(), "no file, no sites");
        std::fs::write(
            dir.join("sites.json"),
            r#"{ "sites": [
                { "name": "Website", "url": "https://example.com", "group": "public" },
                { "name": "Admin", "url": "http://box:5173/admin", "description": "accounts", "private": true },
                { "name": "No url" },
                { "name": "Bad", "url": "javascript:alert(1)" }
            ] }"#,
        )
        .unwrap();
        let sites = read_sites(&dir);
        assert_eq!(sites.len(), 2);
        assert_eq!(sites[1], Site {
            name: "Admin".into(),
            url: "http://box:5173/admin".into(),
            description: Some("accounts".into()),
            group: None,
            private: true,
        });
        std::fs::write(dir.join("sites.json"), "not json").unwrap();
        assert!(read_sites(&dir).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_project_name_never_leaves_the_projects_folder() {
        use super::project_name;
        for ok in ["loft", "work/isa", "Laufey & Gustavo", " api ", ""] {
            assert!(project_name(ok).is_ok(), "{ok}");
        }
        for bad in ["../x", "..", "a/../b", "/Users/me/secret", "~/repo", "a//b", "./a", "a/", "C:\\x", "a\\..\\b"] {
            assert!(project_name(bad).is_err(), "{bad}");
        }
        assert_eq!(super::strip_userinfo("https://user:ghp_secret@github.com/o/r.git"), "https://github.com/o/r.git");
        assert_eq!(super::strip_userinfo("https://gitlab.com/o/r.git"), "https://gitlab.com/o/r.git");
        assert_eq!(super::strip_userinfo("git@gitlab.com:o/r.git"), "git@gitlab.com:o/r.git");
    }

    #[tokio::test]
    async fn a_project_outside_the_projects_folder_reaches_no_repo_and_no_github() {
        use serde_json::json;
        let st = crate::issues::tests::state();
        let fake = crate::github::tests::own_fake(&st);
        // A repo outside the projects folder, with a GitHub origin, reachable by `..`, an absolute path or a symlink.
        let outside = st.settings.boot().projects_dir.parent().unwrap().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let git = |args: &[&str]| assert!(std::process::Command::new("git").arg("-C").arg(&outside).args(args).output().unwrap().status.success());
        git(&["init", "-q"]);
        git(&["remote", "add", "origin", "https://github.com/victim/private"]);
        std::os::unix::fs::symlink(&outside, st.settings.boot().projects_dir.join("link")).unwrap();
        for p in ["../outside", outside.to_str().unwrap(), "link"] {
            assert!(super::github_repo(&st, p).await.is_err(), "{p}");
        }

        // Through the API: refused for an issue and an agent.
        assert!(crate::tasks::create_task(axum::extract::State(st.clone()), axum::Json(json!({ "title": "x", "project": "../outside" }))).await.is_err());
        let (t, _) = crate::tasks::create(&st, &json!({ "title": "x" }), "user").unwrap();
        assert!(crate::tasks::update(axum::extract::State(st.clone()), axum::extract::Path(t.id.clone()), axum::Json(json!({ "project": outside.to_str().unwrap() }))).await.is_err());
        let a = crate::tasks::tests::agent(&st, "Dev");
        let mut patched = a.clone();
        assert!(crate::api::tests::apply_project(&mut patched, "../outside").is_err());

        // A project stored before this check (or through the symlink) still can't publish there.
        let mut old = st.db.get_task(&t.id).unwrap().unwrap();
        old.project = "link".into();
        st.db.save_task(&old).unwrap();
        assert!(crate::mirror::publish(&st, &t.id, "user").await.is_err());
        assert!(fake.seen("/").is_empty(), "no GitHub call");
    }

}
