//! Folder suggestions for the Projects folder field in Settings: as the user types, the folders inside the parent of
//! what they typed, and the folders found by name in their home, each with how many git repos it holds.
//!
//! Only the user's: it's served on `/api` alone, never as an agent tool, and a browser's cross-site read is refused
//! (ADR 0004). The search skips what setup.sh's search skips (scripts/find-folders.sh).

use std::path::{Path, PathBuf};

use axum::extract::Query;
use axum::http::HeaderMap;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

/// At most this many suggestions.
pub const LIMIT: usize = 12;
/// How deep below home the search by name looks, like setup.sh's.
const DEPTH: usize = 3;

#[derive(Deserialize)]
pub struct SuggestQuery {
    #[serde(default)]
    q: String,
}

/// `GET /api/folders/suggest?q=<typed text>`: `[{ path, label, repos }]`, `label` being `path` with `~` for home.
pub async fn suggest_route(headers: HeaderMap, Query(q): Query<SuggestQuery>) -> crate::api::ApiResult<Value> {
    crate::guard::same_site_read(&headers)?;
    let Some(home) = dirs::home_dir() else { return Ok(Json(json!([]))) };
    let found = tokio::task::spawn_blocking(move || suggest(&q.q, &home)).await.unwrap_or_default();
    Ok(Json(found.into_iter().map(|s| json!({ "path": s.path, "label": s.label, "repos": s.repos })).collect()))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub path: PathBuf,
    pub label: String,
    pub repos: usize,
}

/// Folders for `typed`: first the ones in its parent folder whose name starts with its last part (`~` is `home`; a
/// relative path is taken from `home`), then the ones under `home` named like that last part that hold a git repo.
pub fn suggest(typed: &str, home: &Path) -> Vec<Suggestion> {
    let typed = typed.trim();
    let expanded = match typed.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("{}{rest}", home.display()),
        _ => typed.to_string(),
    };
    let (parent, part) = match expanded.rfind('/') {
        Some(i) => (Some(home.join(&expanded[..=i])), &expanded[i + 1..]),
        None => (None, expanded.as_str()),
    };
    let mut paths: Vec<PathBuf> = Vec::new();
    if let Some(parent) = parent {
        paths.extend(children(&parent).into_iter().filter(|p| starts_with(p, part)));
    }
    if !part.is_empty() {
        let mut named = Vec::new();
        by_name(home, part, DEPTH, &mut named);
        named.sort();
        paths.extend(named.into_iter().filter(|p| repos(p) > 0));
    }
    let mut out: Vec<Suggestion> = Vec::new();
    for path in paths {
        if out.len() == LIMIT {
            break;
        }
        if out.iter().any(|s| s.path == path) {
            continue;
        }
        let label = match path.strip_prefix(home) {
            Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => path.display().to_string(),
        };
        out.push(Suggestion { repos: repos(&path), label, path });
    }
    out
}

/// What setup.sh's search doesn't look in: hidden folders, Library, node_modules, caches, build output and the like.
fn skipped(name: &str) -> bool {
    name.starts_with('.')
        || matches!(name, "Library" | "Applications" | "node_modules" | "target" | "vendor" | "venv" | "dist" | "build")
        || name.to_ascii_lowercase().contains("cache")
}

/// The folders in `dir` that aren't skipped, sorted. Symlinks to folders count, but a search never follows them.
fn children(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| !skipped(&e.file_name().to_string_lossy()))
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    out.sort();
    out
}

fn starts_with(path: &Path, part: &str) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    name.starts_with(&part.to_lowercase())
}

/// Folders under `dir`, at most `depth` levels down, whose name starts with `part` (any case).
fn by_name(dir: &Path, part: &str, depth: usize, out: &mut Vec<PathBuf>) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        if !e.file_type().is_ok_and(|t| t.is_dir()) || skipped(&e.file_name().to_string_lossy()) {
            continue;
        }
        let path = e.path();
        if starts_with(&path, part) {
            out.push(path.clone());
        }
        by_name(&path, part, depth - 1, out);
    }
}

/// How many git repos `dir` holds: its subfolders with a `.git` folder or file, like setup.sh's `has_repos`.
pub fn repos(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries.flatten().filter(|e| e.path().join(".git").exists()).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TempDir;

    fn mkdirs(root: &Path, dirs: &[&str]) {
        for d in dirs {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
    }

    fn rows(found: &[Suggestion]) -> Vec<(&str, usize)> {
        found.iter().map(|s| (s.label.as_str(), s.repos)).collect()
    }

    fn home() -> TempDir {
        let home = TempDir::new("folders");
        mkdirs(&home, &[
            "projects/site/.git",
            "projects/api/.git",
            "projects/notes",
            "Documents/projects/app/.git",
            "Documents/Projects-old/thing/.git",
            "Documents/proj-empty/stuff",
            "Library/projects/x/.git",
            "node_modules/projects/y/.git",
            ".config/projects/z/.git",
            "Caches/projects/w/.git",
            "work/deep/er/projects/v/.git",
        ]);
        // A worktree or submodule's .git is a file.
        std::fs::create_dir_all(home.join("projects/linked")).unwrap();
        std::fs::write(home.join("projects/linked/.git"), "gitdir: elsewhere").unwrap();
        home
    }

    #[test]
    fn a_path_prefix_lists_the_parents_folders_that_start_with_it_and_their_repo_counts() {
        let home = home();
        let typed = format!("{}/pro", home.display());
        assert_eq!(rows(&suggest(&typed, &home)), [("~/projects", 3), ("~/Documents/Projects-old", 1), ("~/Documents/projects", 1)]);
        // `~` is home, and a trailing slash lists every folder in it.
        assert_eq!(rows(&suggest("~/projects/", &home)), [("~/projects/api", 0), ("~/projects/linked", 0), ("~/projects/notes", 0), ("~/projects/site", 0)]);
        // The parent's folders come first, with or without repos, then the ones found by name.
        assert_eq!(rows(&suggest("~/Documents/p", &home))[..4], [("~/Documents/Projects-old", 1), ("~/Documents/proj-empty", 0), ("~/Documents/projects", 1), ("~/projects", 3)]);
        assert_eq!(suggest("~/projects", &home)[0].path, home.join("projects"));
        assert_eq!(suggest("~/projects", &home)[0].label, "~/projects");
    }

    #[test]
    fn a_name_finds_the_folders_with_repos_named_like_it_in_home_up_to_three_levels() {
        let home = home();
        // Any case, by its start; folders with no repo are left out; four levels down is too deep.
        assert_eq!(rows(&suggest("projects", &home)), [("~/Documents/Projects-old", 1), ("~/Documents/projects", 1), ("~/projects", 3)]);
        assert_eq!(rows(&suggest("PROJ", &home)).len(), 3);
        assert!(suggest("nothing-like-it", &home).is_empty());
        assert!(suggest("", &home).is_empty());
    }

    #[test]
    fn excluded_folders_never_show() {
        let home = home();
        let all: Vec<String> = ["projects", "~/", "x", "y", "z", "w"].iter().flat_map(|q| suggest(q, &home)).map(|s| s.label).collect();
        for bad in ["Library", "node_modules", ".config", "Caches", ".git"] {
            assert!(!all.iter().any(|l| l.split('/').any(|p| p == bad)), "{bad} in {all:?}");
        }
        assert_eq!(rows(&suggest("~/", &home)), [("~/Documents", 0), ("~/projects", 3), ("~/work", 0)]);
        assert!(suggest("~/projects/site/.g", &home).is_empty(), "no .git folder either");
    }

    #[test]
    fn at_most_limit_suggestions() {
        let home = TempDir::new("folders-many");
        for i in 0..LIMIT + 5 {
            mkdirs(&home, &[&format!("p{i:02}/r/.git")]);
        }
        assert_eq!(suggest("~/p", &home).len(), LIMIT);
    }

    #[tokio::test]
    async fn only_the_user_gets_it_not_agents_or_other_sites() {
        let st = crate::tasks::tests::state();
        let app = axum::Router::new().nest("/api", crate::api_routes()).with_state(st.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let url = format!("{base}/api/folders/suggest?q=~/");
        let ok = reqwest::Client::new().get(&url).header("sec-fetch-site", "same-origin").send().await.unwrap();
        assert_eq!(ok.status(), 200);
        assert!(ok.json::<Value>().await.unwrap().is_array());
        let cross = reqwest::Client::new().get(&url).header("sec-fetch-site", "cross-site").send().await.unwrap();
        assert_eq!(cross.status(), 403, "a page on another site can't read the user's folders");
        // Agents work through their MCP tools, and none of them suggests folders.
        let tools = crate::mcp::tools(&st);
        let names: Vec<&str> = tools.as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
        assert!(!names.is_empty() && !names.iter().any(|n| n.contains("folder")), "no agent tool reads the user's folders: {names:?}");
    }
}
