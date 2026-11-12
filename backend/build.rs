//! Embeds the commit this binary is built from as `LOFT_COMMIT`, so the update status can say what is running.
use std::process::Command;

fn main() {
    let git = |args: &[&str]| Command::new("git").args(args).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    println!("cargo:rustc-env=LOFT_COMMIT={}", git(&["rev-parse", "HEAD"]).unwrap_or_default());
    // Rebuild when HEAD moves (a checkout, a pull), including from a worktree.
    if let Some(p) = git(&["rev-parse", "--git-path", "HEAD"]) {
        println!("cargo:rerun-if-changed={p}");
    }
    if let Some(p) = git(&["rev-parse", "--git-path", "refs/heads"]) {
        println!("cargo:rerun-if-changed={p}");
    }
}
