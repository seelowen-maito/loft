//! Finding a file for `@` in the chat box by name, without walking to it folder by folder: a fuzzy search
//! over an agent's `@` base (its project's folder, else its workspace).
//!
//! In a git repo it searches what git would show: tracked files plus untracked ones `.gitignore` doesn't
//! hide, and the folders holding them. Elsewhere it walks the folder, skipping the usual noise.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::{Value, json};

/// The most entries a search returns.
pub const LIMIT: usize = 20;
/// The most paths a search looks at, so a huge folder stays fast.
const CAP: usize = 50_000;
/// Never walked into or suggested outside git.
const SKIP: [&str; 4] = [".git", "node_modules", "target", ".DS_Store"];

/// The entries under `base` whose path fuzzy-matches `query`, best first: `{ "path", "dir" }`, with
/// `path` relative to `base`.
pub async fn search(base: &Path, query: &str) -> Vec<Value> {
    let query: Vec<char> = query.trim().to_lowercase().chars().collect();
    if query.is_empty() {
        return vec![];
    }
    let entries = match git_entries(base).await {
        Some(e) => e,
        None => {
            let b = base.to_path_buf();
            tokio::task::spawn_blocking(move || walk(&b)).await.unwrap_or_default()
        }
    };
    rank(entries, &query)
        .into_iter()
        // A tracked file deleted from the disk is still in git's index.
        .filter(|(path, _)| base.join(path).symlink_metadata().is_ok())
        .take(LIMIT)
        .map(|(path, dir)| json!({ "path": path, "dir": dir }))
        .collect()
}

/// What git lists under `base`, or None when `base` isn't in a repo or the repo ignores it (an agent's
/// workspace inside the Loft checkout), which leaves it to the walk.
async fn git_entries(base: &Path) -> Option<BTreeSet<(String, bool)>> {
    crate::repos::git(base, &["rev-parse", "--is-inside-work-tree"]).await.ok()?;
    if crate::repos::git(base, &["check-ignore", "-q", "."]).await.is_ok() {
        return None;
    }
    // Run from `base`, git lists only what's under it, relative to it.
    let out = crate::repos::git(base, &["ls-files", "-z", "--cached", "--others", "--exclude-standard"]).await.ok()?;
    let mut entries = BTreeSet::new();
    for raw in out.split('\0').filter(|p| !p.is_empty()).take(CAP) {
        // An untracked nested repo comes as `sub/`.
        let (path, dir) = match raw.strip_suffix('/') {
            Some(p) => (p, true),
            None => (raw, false),
        };
        if path.split('/').any(|part| part.is_empty() || part == ".." || part == ".git") {
            continue;
        }
        let mut parent = path;
        while let Some((up, _)) = parent.rsplit_once('/') {
            entries.insert((up.to_string(), true));
            parent = up;
        }
        entries.insert((path.to_string(), dir));
    }
    Some(entries)
}

/// Everything under `base` but [`SKIP`], up to [`CAP`] entries. Symlinks aren't followed.
fn walk(base: &Path) -> BTreeSet<(String, bool)> {
    let mut entries = BTreeSet::new();
    let mut stack = vec![base.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.filter_map(|e| e.ok()) {
            if entries.len() >= CAP {
                return entries;
            }
            if SKIP.contains(&e.file_name().to_string_lossy().as_ref()) {
                continue;
            }
            let p = e.path();
            let Ok(rel) = p.strip_prefix(base) else { continue };
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
            entries.insert((rel.to_string_lossy().into_owned(), is_dir));
            if is_dir {
                stack.push(p);
            }
        }
    }
    entries
}

/// The entries `query` (lowercase) matches, best first. A match in the entry's own name beats one that
/// needs the folders above it; then the tighter match, the shorter name or path, the shallower entry.
fn rank(entries: BTreeSet<(String, bool)>, query: &[char]) -> Vec<(String, bool)> {
    let mut hits: Vec<_> = entries
        .into_iter()
        .filter_map(|(path, dir)| {
            let lower = path.to_lowercase();
            let name = lower.rsplit('/').next().unwrap_or(&lower);
            let key = match span(name, query) {
                Some(s) => (0, s, name.chars().count(), lower.len()),
                None => (1, span(&lower, query)?, lower.chars().count(), lower.len()),
            };
            Some((key, path, dir))
        })
        .collect();
    hits.sort();
    hits.into_iter().map(|(_, path, dir)| (path, dir)).collect()
}

/// The length of the shortest stretch of `hay` holding `needle` in order, or None when it doesn't.
fn span(hay: &str, needle: &[char]) -> Option<usize> {
    let hay: Vec<char> = hay.chars().collect();
    let mut best = None;
    for start in (0..hay.len()).filter(|&i| hay[i] == needle[0]) {
        let mut n = 1;
        let mut end = start;
        for (i, c) in hay.iter().enumerate().skip(start + 1) {
            if n == needle.len() {
                break;
            }
            if *c == needle[n] {
                n += 1;
                end = i;
            }
        }
        if n < needle.len() {
            // No later start can match either.
            break;
        }
        best = Some(best.map_or(end - start + 1, |b: usize| b.min(end - start + 1)));
    }
    best
}

#[cfg(test)]
mod tests {
    use super::span;

    #[test]
    fn the_span_is_the_tightest_stretch() {
        let q = |s: &str| s.chars().collect::<Vec<_>>();
        assert_eq!(span("chat.tsx", &q("chat")), Some(4));
        assert_eq!(span("c-h-a-t-chat", &q("chat")), Some(4), "the later, tighter match wins");
        assert_eq!(span("cxhxaxt", &q("chat")), Some(7));
        assert_eq!(span("tach", &q("chat")), None);
        assert_eq!(span("c", &q("c")), Some(1));
    }
}
