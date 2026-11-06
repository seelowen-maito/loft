//! Finding a file for `@` in the chat box by name, without walking to it folder by folder: a fuzzy search
//! over an agent's `@` base (its project's folder, else its workspace).
//!
//! In a git repo it searches what git would show: tracked files plus untracked ones `.gitignore` doesn't
//! hide, and the folders holding them. Elsewhere it walks the folder, skipping the usual noise.
//! A listing is kept for a few seconds, so typing a name only re-ranks it.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// The most entries a search returns.
pub const LIMIT: usize = 20;
/// The most paths a search looks at, so a huge folder stays fast.
const CAP: usize = 50_000;
/// Never walked into or suggested outside git.
const SKIP: [&str; 4] = [".git", "node_modules", "target", ".DS_Store"];
/// How long a listing is reused before the folder is listed again.
const TTL: Duration = Duration::from_secs(5);

/// One entry of a listing, with its lowercase path ready for ranking.
struct Listed {
    path: String,
    dir: bool,
    lower: String,
    /// Where the entry's own name starts in `lower`.
    name_at: usize,
}

type Listing = Arc<Vec<Listed>>;

/// The latest listing of each `@` base, reused for [`TTL`].
#[derive(Default)]
pub struct Listings(Mutex<HashMap<PathBuf, (Instant, Listing)>>);

impl Listings {
    async fn get(&self, base: &Path) -> Listing {
        {
            let mut map = self.0.lock().unwrap();
            map.retain(|_, (at, _)| at.elapsed() < TTL);
            if let Some((_, l)) = map.get(base) {
                return l.clone();
            }
        }
        let entries = match git_entries(base).await {
            Some(e) => e,
            None => {
                let b = base.to_path_buf();
                tokio::task::spawn_blocking(move || walk(&b)).await.unwrap_or_default()
            }
        };
        let listed = Arc::new(
            entries
                .into_iter()
                .map(|(path, dir)| {
                    let lower = path.to_lowercase();
                    let name_at = lower.rfind('/').map_or(0, |i| i + 1);
                    Listed { path, dir, lower, name_at }
                })
                .collect::<Vec<_>>(),
        );
        self.0.lock().unwrap().insert(base.to_path_buf(), (Instant::now(), listed.clone()));
        listed
    }
}

/// The entries under `base` whose path fuzzy-matches `query`, best first: `{ "path", "dir" }`, with
/// `path` relative to `base`.
pub async fn search(listings: &Listings, base: &Path, query: &str) -> Vec<Value> {
    let query: Vec<char> = query.trim().to_lowercase().chars().collect();
    if query.is_empty() {
        return vec![];
    }
    let listed = listings.get(base).await;
    rank(&listed, &query)
        .into_iter()
        // A tracked file deleted from the disk is still in git's index.
        .filter(|e| base.join(&e.path).symlink_metadata().is_ok())
        .take(LIMIT)
        .map(|e| json!({ "path": e.path, "dir": e.dir }))
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
fn rank<'a>(entries: &'a [Listed], query: &[char]) -> Vec<&'a Listed> {
    let mut hits: Vec<_> = entries
        .iter()
        .filter_map(|e| {
            let name = &e.lower[e.name_at..];
            let key = match span(name, query) {
                Some(s) => (0, s, name.chars().count(), e.lower.len()),
                None => (1, span(&e.lower, query)?, e.lower.chars().count(), e.lower.len()),
            };
            Some((key, &e.path, e.dir, e))
        })
        .collect();
    hits.sort_unstable_by(|a, b| (a.0, a.1, a.2).cmp(&(b.0, b.1, b.2)));
    hits.into_iter().map(|(.., e)| e).collect()
}

/// The length, in chars, of the shortest stretch of `hay` holding `needle` in order, or None when it doesn't.
fn span(hay: &str, needle: &[char]) -> Option<usize> {
    let mut best = None;
    for (start, first) in hay.char_indices().filter(|&(_, c)| c == needle[0]) {
        let mut n = 1;
        let mut len = 1;
        for (seen, c) in hay[start + first.len_utf8()..].chars().enumerate() {
            if n == needle.len() {
                break;
            }
            if c == needle[n] {
                n += 1;
                len = seen + 2;
            }
        }
        if n < needle.len() {
            // No later start can match either.
            break;
        }
        best = Some(best.map_or(len, |b: usize| b.min(len)));
    }
    best
}

#[cfg(test)]
mod tests {
    use super::{Listings, TTL, search, span};
    use std::time::Instant;

    #[tokio::test]
    async fn a_listing_is_reused_until_it_goes_stale() {
        let base = std::env::temp_dir().join(format!("loft-find-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(base.join("src")).unwrap();
        std::fs::write(base.join("src/chat.rs"), "").unwrap();
        let listings = Listings::default();
        let paths = |v: Vec<serde_json::Value>| v.iter().map(|e| e["path"].as_str().unwrap().to_string()).collect::<Vec<_>>();

        assert_eq!(paths(search(&listings, &base, "chat").await), ["src/chat.rs"]);
        std::fs::write(base.join("src/chatter.rs"), "").unwrap();
        assert_eq!(paths(search(&listings, &base, "chat").await), ["src/chat.rs"], "the folder isn't listed again");
        std::fs::remove_file(base.join("src/chat.rs")).unwrap();
        assert!(search(&listings, &base, "chat").await.is_empty(), "a file gone from the disk isn't offered");

        for (at, _) in listings.0.lock().unwrap().values_mut() {
            *at = Instant::now() - TTL;
        }
        assert_eq!(paths(search(&listings, &base, "chat").await), ["src/chatter.rs"], "a stale listing is redone");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn the_span_is_the_tightest_stretch() {
        let q = |s: &str| s.chars().collect::<Vec<_>>();
        assert_eq!(span("chat.tsx", &q("chat")), Some(4));
        assert_eq!(span("c-h-a-t-chat", &q("chat")), Some(4), "the later, tighter match wins");
        assert_eq!(span("cxhxaxt", &q("chat")), Some(7));
        assert_eq!(span("tach", &q("chat")), None);
        assert_eq!(span("c", &q("c")), Some(1));
        assert_eq!(span("ça-va", &q("av")), Some(3), "counted in chars, not bytes");
    }
}
