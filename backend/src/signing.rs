//! macOS keys privacy permissions (e.g. Full Disk Access) to a binary's code signature.
//! Cargo builds are ad-hoc signed, so every rebuild would lose them. When the keychain has a code-signing
//! identity named "loft", sign our own executable with it at startup, so permissions survive updates.

use std::os::unix::process::CommandExt;
use std::process::Command;

const IDENTITY: &str = "loft";
const IDENTIFIER: &str = "dev.loft";

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

/// Sign the running executable if needed, then restart into the signed copy. Returns if nothing to do.
pub fn ensure() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let Some(hash) = identity_hash() else { return };
    let Ok(exe) = std::env::current_exe() else { return };
    if signed_with(&exe, &hash) {
        return;
    }
    // Sign a copy and swap it in: rewriting the running binary in place would crash this process.
    let tmp = exe.with_extension("signing");
    let signed = std::fs::copy(&exe, &tmp).is_ok() && codesign(&tmp, &hash) && signed_with(&tmp, &hash);
    if !signed || std::fs::rename(&tmp, &exe).is_err() {
        let _ = std::fs::remove_file(&tmp);
        eprintln!("signing: couldn't sign with the \"{IDENTITY}\" identity; macOS permissions won't survive rebuilds");
        return;
    }
    println!("signed with the \"{IDENTITY}\" identity, restarting");
    let err = Command::new(&exe).args(std::env::args_os().skip(1)).exec();
    eprintln!("signing: restart failed: {err}");
}
