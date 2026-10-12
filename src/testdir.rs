//! Temporary folders for tests, removed when dropped (also when the test fails).

use std::{ops::Deref, path::Path};

pub struct TestDir(tempfile::TempDir);

/// A new empty folder in the system's temporary directory, named
/// `touchbinux-<name>-<random>`.
pub fn new(name: &str) -> TestDir {
    let dir = tempfile::Builder::new()
        .prefix(&format!("touchbinux-{name}-"))
        .tempdir()
        .expect("creating a temporary folder");
    TestDir(dir)
}

impl Deref for TestDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        self.0.path()
    }
}

impl AsRef<Path> for TestDir {
    fn as_ref(&self) -> &Path {
        self.0.path()
    }
}
