//! The one place tests make temp folders: removed when dropped, even when the test panics.
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once};

pub struct TempDir(PathBuf);

impl TempDir {
    /// A new empty `loft-<name>-<uuid>` folder in the system temp folder.
    pub fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("loft-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
}

impl std::ops::Deref for TempDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

thread_local! { static KEPT: RefCell<Vec<TempDir>> = const { RefCell::new(Vec::new()) }; }

/// A folder that lives until this test's thread ends, for helpers that return a state or path with nothing to hold a guard.
pub fn for_this_test(name: &str) -> PathBuf {
    let dir = TempDir::new(name);
    let path = dir.0.clone();
    KEPT.with(|k| k.borrow_mut().push(dir));
    path
}

static PROCESS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
static EXIT_HOOK: Once = Once::new();

extern "C" fn remove_process_dirs() {
    for d in PROCESS.lock().unwrap_or_else(|e| e.into_inner()).drain(..) {
        let _ = std::fs::remove_dir_all(d);
    }
}

/// A folder shared by every test in the process (set up once), removed when the process exits.
pub fn for_this_process(name: &str) -> PathBuf {
    let dir = TempDir::new(name);
    let path = dir.0.clone();
    std::mem::forget(dir);
    PROCESS.lock().unwrap().push(path.clone());
    EXIT_HOOK.call_once(|| unsafe {
        libc::atexit(remove_process_dirs);
    });
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panicking_test_still_removes_its_folder() {
        let path = std::thread::spawn(|| {
            let dir = for_this_test("panics");
            let path = dir.clone();
            std::fs::write(dir.join("f"), "x").unwrap();
            std::panic::resume_unwind(Box::new(path));
        })
        .join()
        .unwrap_err()
        .downcast::<PathBuf>()
        .unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn a_dropped_guard_removes_its_folder() {
        let dir = TempDir::new("guard");
        let path = dir.to_path_buf();
        assert!(path.exists());
        drop(dir);
        assert!(!path.exists());
    }
}
