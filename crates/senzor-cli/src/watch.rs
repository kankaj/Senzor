//! Notices when the project file is saved, for `run --watch`.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How often the file is looked at.
const POLL: Duration = Duration::from_secs(1);
/// How long the file must stay the same before it is read: an editor may save in several
/// writes (truncate, then write; or write a copy, then rename it over the file).
const SETTLE: Duration = Duration::from_millis(200);

/// What tells that the file changed without reading it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stamp {
    modified: SystemTime,
    len: u64,
}

impl Stamp {
    /// `None` while the file is missing or empty: in the middle of a save, not yet written.
    pub fn of(path: &Path) -> Option<Self> {
        Self::from_metadata(&std::fs::metadata(path).ok()?)
    }

    /// [`Stamp::of`] without blocking the async runtime.
    async fn of_async(path: &Path) -> Option<Self> {
        Self::from_metadata(&tokio::fs::metadata(path).await.ok()?)
    }

    fn from_metadata(metadata: &std::fs::Metadata) -> Option<Self> {
        (metadata.len() > 0).then_some(())?;
        Some(Self {
            modified: metadata.modified().ok()?,
            len: metadata.len(),
        })
    }
}

/// Looks at a project file once a second and hands out each new content once it is saved.
pub struct FileWatch {
    path: PathBuf,
    /// The stamp of the content last read.
    seen: Option<Stamp>,
    /// The content last handed out, or loaded at the start.
    text: String,
    poll: Duration,
    settle: Duration,
}

impl FileWatch {
    /// Watches `path`, which had `stamp` before `text` was read from it.
    pub fn new(path: &Path, stamp: Option<Stamp>, text: String) -> Self {
        Self {
            path: path.to_path_buf(),
            seen: stamp,
            text,
            poll: POLL,
            settle: SETTLE,
        }
    }

    /// Waits until the file holds a content other than the last one, and returns it.
    pub async fn changed(&mut self) -> String {
        loop {
            tokio::time::sleep(self.poll).await;
            let Some(text) = self.settled().await else {
                continue;
            };
            if text != self.text {
                self.text.clone_from(&text);
                return text;
            }
        }
    }

    /// The file's content if its stamp changed, once the stamp has held still for `settle`.
    async fn settled(&mut self) -> Option<String> {
        let mut stamp = Stamp::of_async(&self.path).await?;
        if Some(stamp) == self.seen {
            return None;
        }
        loop {
            tokio::time::sleep(self.settle).await;
            let again = Stamp::of_async(&self.path).await?;
            if again == stamp {
                break;
            }
            stamp = again;
        }
        // A save that starts after the last look changes the stamp again; the next poll sees it.
        let text = tokio::fs::read_to_string(&self.path).await.ok()?;
        self.seen = Some(stamp);
        (!text.is_empty()).then_some(text)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    /// A file in a folder of its own, removed when dropped.
    struct TempFile(PathBuf);

    impl TempFile {
        fn new(name: &str, content: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("senzor-cli-watch-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("project.toml");
            std::fs::write(&path, content).unwrap();
            Self(path)
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
        }
    }

    fn watch(file: &TempFile, settle: Duration) -> FileWatch {
        let text = std::fs::read_to_string(&file.0).unwrap();
        let mut watch = FileWatch::new(&file.0, Stamp::of(&file.0), text);
        watch.poll = Duration::from_millis(20);
        watch.settle = settle;
        watch
    }

    async fn changed(watch: &mut FileWatch) -> String {
        tokio::time::timeout(Duration::from_secs(5), watch.changed())
            .await
            .expect("a change within 5 s")
    }

    #[tokio::test]
    async fn a_save_in_several_writes_is_read_once_it_is_complete() {
        let file = TempFile::new("several-writes", "old");
        let mut watch = watch(&file, Duration::from_millis(300));
        let path = file.0.clone();
        let writer = std::thread::spawn(move || {
            let mut out = std::fs::File::create(&path).unwrap();
            for part in 0..10 {
                writeln!(out, "part {part}").unwrap();
                out.flush().unwrap();
                std::thread::sleep(Duration::from_millis(30));
            }
        });
        let text = changed(&mut watch).await;
        writer.join().unwrap();
        assert_eq!(text.lines().count(), 10, "{text}");
    }

    #[tokio::test]
    async fn the_same_content_saved_again_is_no_change() {
        let file = TempFile::new("same", "one");
        let mut watch = watch(&file, Duration::from_millis(20));
        std::fs::write(&file.0, "one").unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        std::fs::write(&file.0, "two").unwrap();
        assert_eq!(changed(&mut watch).await, "two");
    }

    #[tokio::test]
    async fn a_missing_or_empty_file_is_waited_for() {
        let file = TempFile::new("missing", "one");
        let mut watch = watch(&file, Duration::from_millis(20));
        std::fs::remove_file(&file.0).unwrap();
        let waiting = tokio::time::timeout(Duration::from_millis(200), watch.changed()).await;
        assert!(waiting.is_err(), "no change while missing");
        std::fs::write(&file.0, "").unwrap();
        let waiting = tokio::time::timeout(Duration::from_millis(200), watch.changed()).await;
        assert!(waiting.is_err(), "no change while empty");
        std::fs::write(&file.0, "two").unwrap();
        assert_eq!(changed(&mut watch).await, "two");
    }
}
