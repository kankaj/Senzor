//! Helpers shared by the CLI test binaries.

use std::ops::Deref;
use std::path::{Path, PathBuf};

/// The binary under test, as Cargo names it.
#[allow(dead_code)] // Not every test binary runs it by name.
pub const BIN: &str = "senzor-cli";

/// A project file in a folder of its own under the system temp dir. The folder is removed when
/// the value is dropped, also when the test fails.
pub struct TempProject {
    dir: PathBuf,
    path: PathBuf,
}

impl TempProject {
    /// Writes `content` to `project.toml` in a fresh folder named after `prefix`, the process ID
    /// and `name`.
    pub fn new(prefix: &str, name: &str, content: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("{prefix}-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("project.toml");
        std::fs::write(&path, content).unwrap();
        Self { dir, path }
    }

    /// The folder that holds the project file.
    #[allow(dead_code)] // Not every test binary needs it.
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

impl Deref for TempProject {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
