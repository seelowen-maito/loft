//! The few system calls Loft makes that differ between Unix and Windows.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Replaces this process with `cmd`; returns only on failure. Windows has no `exec`, so there it always fails.
pub fn exec(cmd: &mut Command) -> std::io::Error {
    #[cfg(unix)]
    return std::os::unix::process::CommandExt::exec(cmd);
    #[cfg(not(unix))]
    {
        let _ = cmd;
        std::io::Error::new(std::io::ErrorKind::Unsupported, "Windows can't replace a running process")
    }
}

/// This computer's name, lowercased: `gethostname` on Unix, `COMPUTERNAME` on Windows. Empty when unknown.
pub fn hostname() -> String {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0;
        let n = buf.iter().position(|&b| b == 0).unwrap_or(0);
        if ok { String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase() } else { String::new() }
    }
    #[cfg(not(unix))]
    std::env::var("COMPUTERNAME").unwrap_or_default().to_ascii_lowercase()
}

/// The user this process runs as. Windows has no numeric user id: there it is 0, for every process.
pub fn uid() -> u32 {
    #[cfg(unix)]
    return unsafe { libc::getuid() };
    #[cfg(not(unix))]
    0
}

/// When the file's metadata last changed (ctime), which an edit can't set back. Windows has none: there it is when
/// the file was created, which a copy (keeping the old mtime) also sets to the day it arrived.
pub fn ctime(m: &std::fs::Metadata) -> std::time::SystemTime {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::time::SystemTime::UNIX_EPOCH + std::time::Duration::new(m.ctime().max(0) as u64, m.ctime_nsec().clamp(0, 999_999_999) as u32)
    }
    #[cfg(not(unix))]
    m.created().unwrap_or(std::time::SystemTime::UNIX_EPOCH)
}

/// [`std::fs::canonicalize`] without the `\\?\` prefix Windows puts on it: the path as the user, Claude Code (which
/// names a session's folder after it, for `/resume`) and the agents' prompts know it.
pub fn canonical(p: impl AsRef<Path>) -> std::io::Result<PathBuf> {
    let real = std::fs::canonicalize(p)?;
    #[cfg(windows)]
    if let Some(plain) = real.to_str().and_then(without_verbatim) {
        return Ok(PathBuf::from(plain));
    }
    Ok(real)
}

/// [`canonical`] as a method, like `Path::canonicalize`.
pub trait Canonical {
    fn canonical(&self) -> std::io::Result<PathBuf>;
}

impl Canonical for Path {
    fn canonical(&self) -> std::io::Result<PathBuf> {
        canonical(self)
    }
}

/// `\\?\C:\x` as `C:\x`, and `\\?\UNC\host\share\x` as `\\host\share\x`, when the plain form names the same file. None
/// for any other path, and (like the `dunce` crate) for one that plain would read differently: 260 characters or more,
/// a part that ends in a dot or a space, a reserved name (`CON`, `nul.txt`, `COM1`) or a character Windows paths can't
/// hold.
#[cfg_attr(not(windows), allow(dead_code))]
fn without_verbatim(s: &str) -> Option<String> {
    let rest = s.strip_prefix(r"\\?\")?;
    let (plain, parts) = if let Some(unc) = rest.strip_prefix(r"UNC\") {
        (format!(r"\\{unc}"), unc)
    } else {
        let b = rest.as_bytes();
        if !(b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':') {
            return None;
        }
        (rest.to_string(), rest.get(2..).unwrap_or_default())
    };
    let reserved = |part: &str| {
        let stem = part.split('.').next().unwrap_or_default().trim_end().to_ascii_uppercase();
        matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
            || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && stem.as_bytes()[3].is_ascii_digit())
    };
    let plain_ok = plain.encode_utf16().count() < 260
        && parts.split('\\').filter(|p| !p.is_empty()).all(|p| !p.ends_with(['.', ' ']) && !reserved(p) && !p.contains(|c: char| c < ' ' || "<>:\"/|?*".contains(c)));
    plain_ok.then_some(plain)
}

/// `p` as Git Bash writes it, for the scripts and commands Loft gives agents: `C:\Users\me` is `/c/Users/me` on
/// Windows. Elsewhere `p` itself.
pub fn bash_path(p: &Path) -> String {
    let s = p.to_string_lossy();
    if cfg!(windows) { git_bash_form(&s) } else { s.into_owned() }
}

fn git_bash_form(s: &str) -> String {
    let s = s.replace('\\', "/");
    let b = s.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b.len() == 2 || b[2] == b'/') {
        return format!("/{}{}", (b[0] as char).to_ascii_lowercase(), &s[2..]);
    }
    s
}

/// `p` with forward slashes on Windows (`C:/Users/me`), which both Git Bash and Claude Code's file tools read: the
/// paths an agent's prompt shows. Elsewhere `p` itself.
pub fn forward_slashes(p: &Path) -> String {
    let s = p.to_string_lossy();
    if cfg!(windows) { s.replace('\\', "/") } else { s.into_owned() }
}

/// The program `bin` names. On Windows a bare name is looked up on PATH with the extensions Windows runs (`claude`
/// is `claude.exe` or `claude.cmd`), and a path without an extension tries them too. Elsewhere, or when nothing is
/// found, `bin` itself.
pub fn program(bin: &str) -> PathBuf {
    #[cfg(windows)]
    if let Some(p) = with_extensions(bin, std::env::var_os("PATH"), std::env::var("PATHEXT").ok()) {
        return p;
    }
    PathBuf::from(bin)
}

/// The file `bin` runs on Windows, given `PATH` and `PATHEXT` (`.COM;.EXE;.BAT;.CMD` when unset): one with an extension
/// is itself, otherwise the extensions are tried in order, in each absolute PATH folder for a bare name.
#[cfg_attr(not(windows), allow(dead_code))]
fn with_extensions(bin: &str, path: Option<std::ffi::OsString>, pathext: Option<String>) -> Option<PathBuf> {
    let pathext = pathext.filter(|e| !e.trim().is_empty()).unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into());
    let exts: Vec<String> = pathext.split(';').map(|e| e.trim().to_ascii_lowercase()).filter(|e| e.starts_with('.')).collect();
    let has_ext = Path::new(bin).extension().is_some_and(|e| exts.contains(&format!(".{}", e.to_string_lossy().to_ascii_lowercase())));
    let names: Vec<String> = if has_ext { vec![bin.to_string()] } else { exts.iter().map(|e| format!("{bin}{e}")).collect() };
    if bin.is_empty() {
        return None;
    }
    if bin.contains(['/', '\\']) {
        return names.into_iter().map(PathBuf::from).find(|p| p.is_file());
    }
    let dirs: Vec<PathBuf> = path.map(|p| std::env::split_paths(&p).filter(|d| d.is_absolute()).collect()).unwrap_or_default();
    dirs.iter().find_map(|d| names.iter().map(|n| d.join(n)).find(|p| p.is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_paths_lose_the_verbatim_prefix() {
        assert_eq!(without_verbatim(r"\\?\C:\Users\me\loft").as_deref(), Some(r"C:\Users\me\loft"));
        assert_eq!(without_verbatim(r"\\?\UNC\nas\share\repo").as_deref(), Some(r"\\nas\share\repo"));
        assert_eq!(without_verbatim(r"C:\Users\me"), None);
        assert_eq!(without_verbatim(r"\\?\Volume{1234}\x"), None);
        // Kept where the plain path would name another file, or none.
        for odd in [r"\\?\C:\repo\trailing.", r"\\?\C:\repo\space \x", r"\\?\C:\repo\CON", r"\\?\C:\repo\nul.txt", r"\\?\C:\repo\com1", r"\\?\C:\a?b", r"\\?\C:\a/b"] {
            assert_eq!(without_verbatim(odd), None, "{odd}");
        }
        assert_eq!(without_verbatim(&format!(r"\\?\C:\{}", "a".repeat(300))), None);
        // Names that only look like those are plain.
        assert_eq!(without_verbatim(r"\\?\C:\console\com10\.git\a.b").as_deref(), Some(r"C:\console\com10\.git\a.b"));
        let dir = crate::testdir::for_this_test("sys-canonical");
        let real = canonical(dir.join(".")).unwrap();
        assert!(real.is_absolute() && real.is_dir() && !real.to_string_lossy().starts_with(r"\\?\"), "{}", real.display());
    }

    #[test]
    fn git_bash_writes_drive_paths_under_a_letter_folder() {
        assert_eq!(git_bash_form(r"C:\Users\me\loft.exe"), "/c/Users/me/loft.exe");
        assert_eq!(git_bash_form("D:/work"), "/d/work");
        assert_eq!(git_bash_form("C:"), "/c");
        assert_eq!(git_bash_form(r"\\nas\share"), "//nas/share");
        assert_eq!(git_bash_form("/usr/bin"), "/usr/bin");
        assert_eq!(git_bash_form("Cx:/no"), "Cx:/no");
    }

    #[test]
    fn a_bare_name_finds_a_cmd_or_exe_on_path() {
        let dir = crate::testdir::for_this_test("sys-program");
        let (npm, native) = (dir.join("npm"), dir.join("native"));
        std::fs::create_dir_all(&npm).unwrap();
        std::fs::create_dir_all(&native).unwrap();
        std::fs::write(npm.join("claude.cmd"), "").unwrap();
        let path = |dirs: &[&Path]| Some(std::env::join_paths(dirs).unwrap());
        assert_eq!(with_extensions("claude", path(&[&npm]), None), Some(npm.join("claude.cmd")));
        // PATH order first, then PATHEXT's.
        std::fs::write(native.join("claude.exe"), "").unwrap();
        assert_eq!(with_extensions("claude", path(&[&native, &npm]), None), Some(native.join("claude.exe")));
        assert_eq!(with_extensions("claude", path(&[&npm, &native]), Some(".EXE;.CMD".into())), Some(npm.join("claude.cmd")));
        // A name with an extension, or a path, is that file.
        assert_eq!(with_extensions("claude.cmd", path(&[&native, &npm]), None), Some(npm.join("claude.cmd")));
        let full = native.join("claude").to_string_lossy().into_owned();
        assert_eq!(with_extensions(&full, None, None), Some(native.join("claude.exe")));
        // Relative PATH folders are skipped, and nothing found is None.
        assert_eq!(with_extensions("claude", Some("npm".into()), None), None);
        assert_eq!(with_extensions("codex", path(&[&native, &npm]), None), None);
        assert_eq!(with_extensions("", path(&[&native]), None), None);
    }
}
