//! macOS keys privacy permissions (e.g. Full Disk Access) to a binary's code signature.
//! Cargo builds are ad-hoc signed, so every rebuild would lose them. When the keychain has a code-signing
//! identity named "loft", Loft runs as a copy of its build signed with it, so permissions survive updates.
//!
//! A copy, not the build itself: an update builds over the build while Loft keeps running until its agents are
//! idle, and macOS can't check a running binary whose file changed ("the code on disk does not match what is
//! running"), so meanwhile it would ask again for everything Loft was allowed.

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

/// Copies the build to its signed copy, signs it, and runs it in this process's place. Returns when there's no
/// "loft" identity (Loft stays ad-hoc signed), when this is the signed copy, or when signing fails.
pub fn ensure() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    if build_of(&exe) != exe {
        return;
    }
    let Some(hash) = identity_hash() else { return };
    // Signed beside it under another name, then moved into place: a copy still running keeps its file.
    let (copy, tmp) = (signed_copy(&exe), exe.with_extension("signing"));
    let signed = std::fs::copy(&exe, &tmp).is_ok() && codesign(&tmp, &hash) && signed_with(&tmp, &hash);
    if !signed || std::fs::rename(&tmp, &copy).is_err() {
        let _ = std::fs::remove_file(&tmp);
        eprintln!("signing: couldn't sign with the \"{IDENTITY}\" identity; macOS permissions won't survive rebuilds");
        return;
    }
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
}
