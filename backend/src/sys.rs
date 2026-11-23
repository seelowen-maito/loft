//! The few system calls Loft makes that differ between Unix and Windows.

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
