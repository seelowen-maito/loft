//! macOS keys privacy permissions (e.g. Full Disk Access) to a binary's code signature.
//! Cargo builds are ad-hoc signed, so every rebuild would lose them. When the keychain has a code-signing
//! identity named "loft", Loft runs as a copy of its build signed with it, so permissions survive updates.
//!
//! A copy, not the build itself: an update builds over the build while Loft keeps running until its agents are
//! idle, and macOS can't check a running binary whose file changed ("the code on disk does not match what is
//! running"), so meanwhile it would ask again for everything Loft was allowed.
//!
//! The background service starts the signed copy itself (setup.sh points it there with `loft sign`), so macOS
//! lists the signed copy under Login Items. Started that way after an update, the copy hands over to the newer
//! build, which signs a fresh copy and runs it.

use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const IDENTITY: &str = "loft";
const IDENTIFIER: &str = "dev.loft";
/// The signed copy is the build's name with this, beside it.
const SIGNED: &str = "-signed";

/// SHA-1 of the "loft" signing certificate, if the keychain has one.
fn identity_hash() -> Option<String> {
    let out = Command::new("security").args(["find-identity", "-p", "codesigning"]).output().ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find(|l| l.contains(&format!("\"{IDENTITY}\"")))
        .and_then(|l| l.split_whitespace().nth(1))
        .map(String::from)
}

fn signed_with(path: &std::path::Path, hash: &str) -> bool {
    let req = format!("-R=identifier \"{IDENTIFIER}\" and certificate leaf = H\"{hash}\"");
    Command::new("codesign").arg("--verify").arg(req).arg(path).output().is_ok_and(|o| o.status.success())
}

/// The first signing asks, on the Mac's screen, to let codesign use the key; give up if nobody answers.
fn codesign(path: &std::path::Path, hash: &str) -> bool {
    let Ok(mut child) = Command::new("codesign").args(["--force", "--sign", hash, "--identifier", IDENTIFIER]).arg(path).spawn() else {
        return false;
    };
    for _ in 0..120 {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => std::thread::sleep(std::time::Duration::from_secs(1)),
            Err(_) => break,
        }
    }
    let _ = child.kill();
    false
}

/// The signed copy of the build at `exe`.
fn signed_copy(exe: &Path) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(SIGNED);
    exe.with_file_name(name)
}

/// The build `exe` runs: itself, or the build it's the signed copy of. A restart runs it, so an update lands.
pub fn build_of(exe: &Path) -> PathBuf {
    let name = exe.file_name().unwrap_or_default().to_string_lossy();
    match name.strip_suffix(SIGNED) {
        Some(build) if !build.is_empty() => exe.with_file_name(build),
        _ => exe.to_path_buf(),
    }
}

/// The copy was signed from a build other than the one on disk: it carries its build's modified time.
fn stale(copy: &Path, build: &Path) -> bool {
    let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    modified(build).is_some_and(|b| modified(copy) != Some(b))
}

/// Signs a copy of `build` and moves it into place as its signed copy, stamped with the build's modified time
/// so `stale` can tell it apart from a later build.
fn sign(build: &Path, hash: &str) -> Option<PathBuf> {
    // Signed beside it under another name, then moved into place: a copy still running keeps its file.
    let (copy, tmp) = (signed_copy(build), build.with_extension(format!("signing-{}", std::process::id())));
    let stamp = || {
        let modified = std::fs::metadata(build).and_then(|m| m.modified()).ok()?;
        std::fs::File::options().write(true).open(&tmp).and_then(|f| f.set_modified(modified)).ok()
    };
    let signed = std::fs::copy(build, &tmp).is_ok() && codesign(&tmp, hash) && signed_with(&tmp, hash) && stamp().is_some();
    if !signed || std::fs::rename(&tmp, &copy).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return None;
    }
    Some(copy)
}

/// `loft sign`: signs the build's copy and exits, 0 when it's signed. setup.sh runs it before pointing the
/// background service at the copy.
pub const ARG: &str = "sign";

pub fn cli() -> ! {
    let signed = (|| {
        let exe = std::env::current_exe().ok()?;
        if build_of(&exe) != exe {
            eprintln!("loft sign: this is the signed copy; run it from the build at {}", build_of(&exe).display());
            return None;
        }
        sign(&exe, &identity_hash()?)
    })();
    std::process::exit(if signed.is_some() { 0 } else { 1 })
}

/// Copies the build to its signed copy, signs it, and runs it in this process's place. Returns when there's no
/// "loft" identity (Loft stays ad-hoc signed), when this is an up-to-date signed copy, or when signing fails.
/// A signed copy older than the build runs the build instead, which signs a fresh copy.
pub fn ensure() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let build = build_of(&exe);
    if build != exe {
        if stale(&exe, &build) {
            let err = Command::new(&build).args(std::env::args_os().skip(1)).exec();
            eprintln!("signing: running the newer build failed: {err}");
        }
        return;
    }
    let Some(hash) = identity_hash() else { return };
    let Some(copy) = sign(&exe, &hash) else {
        eprintln!("signing: couldn't sign with the \"{IDENTITY}\" identity; macOS permissions won't survive rebuilds");
        return;
    };
    println!("signed a copy with the \"{IDENTITY}\" identity, running it");
    let err = Command::new(&copy).args(std::env::args_os().skip(1)).exec();
    eprintln!("signing: running the signed copy failed: {err}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_signed_copy_sits_beside_its_build_and_restarts_into_it() {
        let build = Path::new("/x/target/release/loft");
        let copy = signed_copy(build);
        assert_eq!(copy, Path::new("/x/target/release/loft-signed"));
        assert_eq!((build_of(&copy), build_of(build)), (build.to_path_buf(), build.to_path_buf()));
        assert_eq!(build_of(Path::new("/x/-signed")), Path::new("/x/-signed"));
    }

    #[test]
    fn a_copy_is_stale_once_its_build_changes() {
        let dir = std::env::temp_dir().join(format!("loft-signing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (build, copy) = (dir.join("loft"), dir.join("loft-signed"));
        let at = |p: &Path, secs: u64| {
            std::fs::write(p, "x").unwrap();
            let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
            std::fs::File::options().write(true).open(p).unwrap().set_modified(t).unwrap();
        };
        // No build beside it (deleted, or never built): run the copy.
        at(&copy, 1_000);
        assert!(!stale(&copy, &build));
        at(&build, 1_000);
        assert!(!stale(&copy, &build));
        at(&build, 2_000);
        assert!(stale(&copy, &build));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
