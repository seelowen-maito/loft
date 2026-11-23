//! sccache shares compiled Rust dependencies between agents, so a second agent building the same crates reuses
//! what the first built. Loft uses it only when it's installed (`brew install sccache`); it never installs it.
//! One cache lives in Loft's data folder, capped at [`LIMIT`], and only agent runs get `RUSTC_WRAPPER`: the
//! user's shell is left alone.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// `SCCACHE_CACHE_SIZE` and the cap Settings shows.
pub const LIMIT: &str = "10G";
pub const LIMIT_BYTES: u64 = 10 * 1024 * 1024 * 1024;

/// Where Loft may be started with a PATH that lacks Homebrew's folders (a launch agent).
const BREW: [&str; 2] = ["/opt/homebrew/bin", "/usr/local/bin"];

/// The cache folder, in Loft's data folder `data_dir`.
pub fn cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("sccache")
}

/// The installed `sccache`, found on `path` (a PATH value) or in Homebrew's folders.
fn find(path: Option<OsString>) -> Option<PathBuf> {
    let path = path.unwrap_or_default();
    std::env::split_paths(&path).chain(BREW.iter().map(PathBuf::from)).map(|d| d.join("sccache")).find(|p| p.is_file())
}

pub fn installed() -> Option<PathBuf> {
    find(std::env::var_os("PATH"))
}

/// Points a command at the shared cache when sccache is installed; otherwise leaves it as it is.
pub fn apply(cmd: &mut tokio::process::Command, data_dir: &Path) {
    apply_with(cmd, installed(), data_dir)
}

fn apply_with(cmd: &mut tokio::process::Command, bin: Option<PathBuf>, data_dir: &Path) {
    if let Some(bin) = bin {
        cmd.env("RUSTC_WRAPPER", bin).env("SCCACHE_DIR", cache_dir(data_dir)).env("SCCACHE_CACHE_SIZE", LIMIT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envs(cmd: &tokio::process::Command) -> Vec<(String, String)> {
        cmd.as_std().get_envs().filter_map(|(k, v)| Some((k.to_string_lossy().into(), v?.to_string_lossy().into()))).collect()
    }

    #[test]
    fn an_installed_sccache_is_the_wrapper_with_one_capped_cache_in_the_data_folder() {
        let mut cmd = tokio::process::Command::new("true");
        apply_with(&mut cmd, Some("/bin/sccache".into()), Path::new("/data"));
        let e = envs(&cmd);
        assert!(e.contains(&("RUSTC_WRAPPER".into(), "/bin/sccache".into())), "{e:?}");
        assert!(e.contains(&("SCCACHE_DIR".into(), "/data/sccache".into())), "{e:?}");
        assert!(e.contains(&("SCCACHE_CACHE_SIZE".into(), "10G".into())), "{e:?}");
    }

    #[test]
    fn without_sccache_the_command_is_untouched() {
        let mut cmd = tokio::process::Command::new("true");
        apply_with(&mut cmd, None, Path::new("/data"));
        assert!(envs(&cmd).is_empty());
    }

    #[test]
    fn it_is_found_on_the_path() {
        let d = std::env::temp_dir().join(format!("sccache-find-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert_ne!(find(Some(d.clone().into())), Some(d.join("sccache")));
        std::fs::write(d.join("sccache"), "").unwrap();
        assert_eq!(find(Some(d.clone().into())), Some(d.join("sccache")));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
