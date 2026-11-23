//! Files and folders only the user may read: 0600 and 0700 on Unix. On Windows these keep the default permissions
//! for now, as plain files and folders.

use std::fs::OpenOptions;
use std::io::Result;
use std::path::Path;

/// Makes `path`, a file, readable by the user only.
pub fn file(path: &Path) -> Result<()> {
    set_mode(path, 0o600)
}

/// Makes `path`, a folder, openable by the user only.
pub fn dir(path: &Path) -> Result<()> {
    set_mode(path, 0o700)
}

/// Creates the folder `path` (with its parents when `recursive`), openable by the user only from the start.
pub fn create_dir(path: &Path, recursive: bool) -> Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(recursive);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(path)
}

/// Options that create a file readable by the user only from its first byte.
pub fn options() -> OpenOptions {
    #[allow(unused_mut)]
    let mut o = OpenOptions::new();
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o
}

/// Makes `path` executable: 0700 on Unix. Windows runs a file by its extension, so there it does nothing.
pub fn executable(path: &Path) -> Result<()> {
    set_mode(path, 0o700)
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<()> {
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(path: &Path, _mode: u32) -> Result<()> {
    std::fs::metadata(path).map(|_| ())
}
