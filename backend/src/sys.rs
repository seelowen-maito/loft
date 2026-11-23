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

/// The program `bin` names, as [`find_tool`] finds it: on Windows `claude` is `claude.exe` or `claude.cmd`. When
/// nothing is found, `bin` itself.
pub fn program(bin: &str) -> PathBuf {
    find_tool(bin).unwrap_or_else(|| PathBuf::from(bin))
}

/// Where the tool `bin` is: on PATH (with the extensions Windows runs, there), else in the folders it is usually
/// installed in, since a background service or a logon task can have a short PATH. A path is that file, with an
/// extension added on Windows. None when it's nowhere.
pub fn find_tool(bin: &str) -> Option<PathBuf> {
    find_tools(bin).into_iter().next()
}

/// Every copy of the tool `bin` [`find_tool`] would find, in the order it looks: for a tool whose first copy may not
/// answer (a CLI on PATH that can't reach the app installed elsewhere).
pub fn find_tools(bin: &str) -> Vec<PathBuf> {
    let exts = if cfg!(windows) { run_extensions(std::env::var("PATHEXT").ok()) } else { vec![] };
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    locate_all(bin, std::env::var_os("PATH"), &exts, &install_paths(bin, std::env::consts::OS, &env))
}

/// Sets `cmd` to start without a console window on Windows, where a console program started by Loft (which may
/// have no console of its own) would otherwise flash one up. Elsewhere it does nothing.
pub fn no_window(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(cmd, 0x0800_0000); // CREATE_NO_WINDOW
    cmd
}

/// The extensions Windows runs, lowercased, from `PATHEXT` (`.COM;.EXE;.BAT;.CMD` when unset).
fn run_extensions(pathext: Option<String>) -> Vec<String> {
    let pathext = pathext.filter(|e| !e.trim().is_empty()).unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into());
    pathext.split(';').map(|e| e.trim().to_ascii_lowercase()).filter(|e| e.starts_with('.')).collect()
}

/// The file `bin` runs, given `PATH`, the extensions to try (Windows', or none elsewhere) and where else it may be
/// installed (`fallback`, paths to the program without its extension). A name with one of `exts` is itself; otherwise
/// each extension is tried in order, in each absolute PATH folder for a bare name, then at each fallback. A path
/// (absolute, or with a separator) is only ever that file.
#[cfg(test)]
fn locate(bin: &str, path: Option<std::ffi::OsString>, exts: &[String], fallback: &[PathBuf]) -> Option<PathBuf> {
    locate_all(bin, path, exts, fallback).into_iter().next()
}

/// Every file [`locate`] could pick, in its order, each once.
fn locate_all(bin: &str, path: Option<std::ffi::OsString>, exts: &[String], fallback: &[PathBuf]) -> Vec<PathBuf> {
    if bin.is_empty() {
        return vec![];
    }
    let windows = !exts.is_empty();
    let has_ext = Path::new(bin).extension().is_some_and(|e| exts.contains(&format!(".{}", e.to_string_lossy().to_ascii_lowercase())));
    let with_exts = |p: &Path| -> Vec<PathBuf> {
        if !windows || has_ext {
            return vec![p.to_path_buf()];
        }
        exts.iter().map(|e| PathBuf::from(format!("{}{e}", p.display()))).collect()
    };
    if Path::new(bin).is_absolute() || bin.contains('/') || (windows && bin.contains('\\')) {
        return with_exts(Path::new(bin)).into_iter().filter(|p| p.is_absolute() && p.is_file()).take(1).collect();
    }
    let dirs = path.map(|p| std::env::split_paths(&p).filter(|d| d.is_absolute()).collect::<Vec<_>>()).unwrap_or_default();
    let mut found: Vec<PathBuf> = vec![];
    for p in dirs.into_iter().map(|d| d.join(bin)).chain(fallback.iter().cloned()).flat_map(|p| with_exts(&p)) {
        if p.is_file() && !found.contains(&p) {
            found.push(p);
        }
    }
    found
}

/// Where the tool `bin` is usually installed on `os` (`std::env::consts::OS`), as paths to the program without its
/// extension, given the environment (`env`): Homebrew's folders on a Mac and Linux, and on Windows winget's, Scoop's
/// and Chocolatey's shared folders and each tool's own installer folder.
fn install_paths(bin: &str, os: &str, env: &dyn Fn(&str) -> Option<PathBuf>) -> Vec<PathBuf> {
    let at = |var: &str, rest: &str| env(var).map(|d| rest.split('/').fold(d, |d, part| d.join(part)));
    let mut dirs: Vec<Option<PathBuf>> = vec![];
    if os == "windows" {
        dirs.extend([at("LOCALAPPDATA", "Microsoft/WinGet/Links"), at("USERPROFILE", "scoop/shims"), at("ProgramData", "chocolatey/bin")]);
        dirs.extend(match bin {
            // The native installer, then npm's.
            "claude" => vec![at("USERPROFILE", ".local/bin"), at("APPDATA", "npm")],
            // VS Code installed for the user, then for everyone.
            "code" => vec![at("LOCALAPPDATA", "Programs/Microsoft VS Code/bin"), at("ProgramFiles", "Microsoft VS Code/bin")],
            "tailscale" => vec![at("ProgramFiles", "Tailscale")],
            "gh" => vec![at("ProgramFiles", "GitHub CLI")],
            _ => vec![],
        });
    } else {
        dirs.extend([Some("/opt/homebrew/bin".into()), Some("/usr/local/bin".into())]);
        if bin == "claude" {
            dirs.push(at("HOME", ".local/bin"));
        }
    }
    let mut paths: Vec<PathBuf> = dirs.into_iter().flatten().map(|d| d.join(bin)).collect();
    // The Mac app bundles its own CLI.
    if os == "macos" && bin == "tailscale" {
        paths.push("/Applications/Tailscale.app/Contents/MacOS/Tailscale".into());
    }
    paths
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

    #[cfg(windows)]
    #[test]
    fn paths_for_the_web_and_prompts_use_forward_slashes() {
        assert_eq!(forward_slashes(Path::new(r"work\meuez")), "work/meuez");
        assert_eq!(forward_slashes(&Path::new("work").join("client").join("app.rs")), "work/client/app.rs");
        assert_eq!(forward_slashes(Path::new(r"C:\Users\me")), "C:/Users/me");
    }

    #[test]
    fn a_bare_name_finds_a_cmd_or_exe_on_path() {
        let dir = crate::testdir::for_this_test("sys-program");
        let (npm, native) = (dir.join("npm"), dir.join("native"));
        std::fs::create_dir_all(&npm).unwrap();
        std::fs::create_dir_all(&native).unwrap();
        std::fs::write(npm.join("claude.cmd"), "").unwrap();
        let path = |dirs: &[&Path]| Some(std::env::join_paths(dirs).unwrap());
        let win = |bin: &str, path, pathext: Option<&str>| locate(bin, path, &run_extensions(pathext.map(String::from)), &[]);
        assert_eq!(win("claude", path(&[&npm]), None), Some(npm.join("claude.cmd")));
        // PATH order first, then PATHEXT's.
        std::fs::write(native.join("claude.exe"), "").unwrap();
        assert_eq!(win("claude", path(&[&native, &npm]), None), Some(native.join("claude.exe")));
        assert_eq!(win("claude", path(&[&npm, &native]), Some(".EXE;.CMD")), Some(npm.join("claude.cmd")));
        // A name with an extension, or a path, is that file.
        assert_eq!(win("claude.cmd", path(&[&native, &npm]), None), Some(npm.join("claude.cmd")));
        let full = native.join("claude").to_string_lossy().into_owned();
        assert_eq!(win(&full, None, None), Some(native.join("claude.exe")));
        // Relative PATH folders and relative paths are skipped, and nothing found is None.
        assert_eq!(win("claude", Some("npm".into()), None), None);
        assert_eq!(win("./claude", path(&[&native]), None), None);
        assert_eq!(win("codex", path(&[&native, &npm]), None), None);
        assert_eq!(win("", path(&[&native]), None), None);
        // Without extensions (Unix) a name is only itself.
        assert_eq!(locate("claude", path(&[&native, &npm]), &[], &[]), None);
        std::fs::write(npm.join("claude"), "").unwrap();
        assert_eq!(locate("claude", path(&[&native, &npm]), &[], &[]), Some(npm.join("claude")));
    }

    #[test]
    fn a_tool_off_path_is_found_where_it_is_usually_installed() {
        let dir = crate::testdir::for_this_test("sys-find-tool");
        let (on_path, installed) = (dir.join("bin"), dir.join("Program Files").join("Tailscale"));
        std::fs::create_dir_all(&on_path).unwrap();
        std::fs::create_dir_all(&installed).unwrap();
        std::fs::write(installed.join("tailscale.exe"), "").unwrap();
        let exts = run_extensions(None);
        let path = Some(std::env::join_paths([&on_path]).unwrap());
        let fallback = [installed.join("tailscale")];
        assert_eq!(locate("tailscale", path.clone(), &exts, &fallback), Some(installed.join("tailscale.exe")));
        // PATH comes first.
        std::fs::write(on_path.join("tailscale.exe"), "").unwrap();
        assert_eq!(locate("tailscale", path.clone(), &exts, &fallback), Some(on_path.join("tailscale.exe")));
        // Every copy, PATH's first, each once.
        let twice = Some(std::env::join_paths([&on_path, &on_path]).unwrap());
        assert_eq!(locate_all("tailscale", twice, &exts, &fallback), [on_path.join("tailscale.exe"), installed.join("tailscale.exe")]);
        // A path given is never looked for elsewhere.
        let elsewhere = dir.join("nowhere").join("tailscale").to_string_lossy().into_owned();
        assert_eq!(locate(&elsewhere, path, &exts, &fallback), None);
    }

    #[test]
    fn each_tool_has_its_usual_install_folders() {
        let env = |k: &str| {
            let v = match k {
                "LOCALAPPDATA" => "C:/Users/me/AppData/Local",
                "APPDATA" => "C:/Users/me/AppData/Roaming",
                "USERPROFILE" => "C:/Users/me",
                "ProgramFiles" => "C:/Program Files",
                "ProgramData" => "C:/ProgramData",
                "HOME" => "/Users/me",
                _ => return None,
            };
            Some(PathBuf::from(v))
        };
        let paths = |bin: &str, os: &str| install_paths(bin, os, &env).iter().map(|p| p.to_string_lossy().replace('\\', "/")).collect::<Vec<_>>();
        let shared = |bin: &str| ["C:/Users/me/AppData/Local/Microsoft/WinGet/Links", "C:/Users/me/scoop/shims", "C:/ProgramData/chocolatey/bin"].map(|d| format!("{d}/{bin}"));
        let has = |all: &[String], want: &str| assert!(all.iter().any(|p| p == want), "{want} not in {all:?}");
        for bin in ["claude", "code", "tailscale", "ffmpeg", "ffprobe", "gh"] {
            let all = paths(bin, "windows");
            for p in shared(bin) {
                has(&all, &p);
            }
        }
        has(&paths("claude", "windows"), "C:/Users/me/.local/bin/claude");
        has(&paths("claude", "windows"), "C:/Users/me/AppData/Roaming/npm/claude");
        has(&paths("code", "windows"), "C:/Users/me/AppData/Local/Programs/Microsoft VS Code/bin/code");
        has(&paths("code", "windows"), "C:/Program Files/Microsoft VS Code/bin/code");
        has(&paths("tailscale", "windows"), "C:/Program Files/Tailscale/tailscale");
        has(&paths("gh", "windows"), "C:/Program Files/GitHub CLI/gh");
        // Unset folders are left out.
        assert_eq!(install_paths("gh", "windows", &|_| None), Vec::<PathBuf>::new());
        // Homebrew elsewhere, and the Mac app's own Tailscale CLI.
        assert_eq!(paths("ffmpeg", "linux"), ["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg"]);
        has(&paths("claude", "macos"), "/Users/me/.local/bin/claude");
        has(&paths("tailscale", "macos"), "/Applications/Tailscale.app/Contents/MacOS/Tailscale");
        assert!(!paths("tailscale", "linux").iter().any(|p| p.contains("Applications")));
    }
}
