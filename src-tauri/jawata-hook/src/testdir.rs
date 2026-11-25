//! A test's own temporary folder, removed when its guard goes out of scope, so also when the test fails.
//!
//! Test code only, and one file for both crates: the unit tests and the integration tests of `jawata-studio` and of
//! `jawata-hook` include it. It uses `std` alone, because the hook takes no dependency for a test's convenience.
//! A test makes no folder under the temporary folder by hand: before this, each run of the suite left some 500
//! entries (3.4 GB, mostly copies of the hook binary) behind.

// Each program that includes this file uses a part of it.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Owns a folder and removes it on drop. Dereferences to the path it stands for.
///
/// Bind it to a name that lives as long as the folder is needed: `TestDir::new("x").join("y")` removes the folder
/// at the end of that statement. [`TestDir::child`] keeps the guard with the inner path.
#[derive(Debug)]
pub struct TestDir {
    root: PathBuf,
    path: PathBuf,
}

// Marked as test code for the scan that forbids panicking constructs on the hook's fire-time path.
#[cfg(test)]
impl TestDir {
    /// A new, empty folder under the platform's temporary folder whose name starts with `label`.
    pub fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "{label}-{}-{nanos}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("a test's temporary folder");
        Self::adopt(dir)
    }

    /// Takes over a folder that the code under test made, so that it is removed with the guard.
    pub fn adopt(dir: PathBuf) -> Self {
        Self { root: dir.clone(), path: dir }
    }

    /// The same guard standing for a path inside its folder. Nothing is created.
    pub fn child(mut self, name: impl AsRef<Path>) -> Self {
        self.path.push(name);
        self
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl std::ops::Deref for TestDir {
    type Target = PathBuf;

    fn deref(&self) -> &PathBuf {
        &self.path
    }
}

impl AsRef<Path> for TestDir {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<std::ffi::OsStr> for TestDir {
    fn as_ref(&self) -> &std::ffi::OsStr {
        self.path.as_os_str()
    }
}
