use std::fmt;
use std::fs::{File, Metadata, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use senzor_core::project::{ProjectError, ProjectFile, Validated};

/// Why a project file could not be opened.
#[derive(Debug, Clone, PartialEq)]
pub enum OpenError {
    Read(String),
    /// TOML syntax or schema errors: the file must be fixed in a text editor.
    Parse(Vec<ProjectError>),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenError::Read(message) => write!(f, "{message}"),
            OpenError::Parse(errors) => {
                for (i, error) in errors.iter().enumerate() {
                    if i > 0 {
                        writeln!(f)?;
                    }
                    write!(f, "{error}")?;
                }
                Ok(())
            }
        }
    }
}

/// The project being edited: the file model, where it lives, whether it changed, and its validation.
pub struct Document {
    path: Option<PathBuf>,
    file: ProjectFile,
    saved: ProjectFile,
    validated: Validated,
}

impl Document {
    pub fn new_empty() -> Self {
        Self::from_file(None, ProjectFile::default())
    }

    pub fn open(path: &Path) -> Result<Self, OpenError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| OpenError::Read(format!("cannot read {}: {e}", path.display())))?;
        let file = ProjectFile::parse(&text).map_err(OpenError::Parse)?;
        Ok(Self::from_file(Some(path.to_path_buf()), file))
    }

    fn from_file(path: Option<PathBuf>, file: ProjectFile) -> Self {
        let validated = validate_in_folder(&file, path.as_deref());
        Self {
            path,
            saved: file.clone(),
            file,
            validated,
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn file(&self) -> &ProjectFile {
        &self.file
    }

    pub fn validated(&self) -> &Validated {
        &self.validated
    }

    /// True when the model differs from what was last opened or saved (undoing an edit clears it).
    pub fn is_dirty(&self) -> bool {
        self.file != self.saved
    }

    /// The name of the project's file, once it has one.
    pub fn file_name(&self) -> Option<String> {
        self.path
            .as_deref()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
    }

    /// File name (or "untitled"), with `*` when there are unsaved changes.
    pub fn title(&self) -> String {
        let name = self.file_name().unwrap_or_else(|| "untitled".to_string());
        if self.is_dirty() {
            format!("{name}*")
        } else {
            name
        }
    }

    /// Applies `f` and re-validates. Returns whether the model changed.
    pub fn edit(&mut self, f: impl FnOnce(&mut ProjectFile)) -> bool {
        let before = self.file.clone();
        f(&mut self.file);
        let changed = self.file != before;
        if changed {
            self.validated = validate_in_folder(&self.file, self.path.as_deref());
        }
        changed
    }

    pub fn save(&mut self) -> Result<(), String> {
        let path = self
            .path
            .clone()
            .ok_or_else(|| "the project has no file name yet; use Save as".to_string())?;
        self.write(&path)
    }

    /// Writes to `path` and makes it the document's file. Relative TLS paths now resolve against it.
    pub fn save_as(&mut self, path: PathBuf) -> Result<(), String> {
        self.write(&path)?;
        self.path = Some(path);
        self.validated = validate_in_folder(&self.file, self.path.as_deref());
        Ok(())
    }

    fn write(&mut self, path: &Path) -> Result<(), String> {
        write_replacing(path, &self.file.to_toml_string())
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        self.saved = self.file.clone();
        Ok(())
    }
}

/// How many names [`create_temp`] tries before giving up.
const TEMP_ATTEMPTS: u32 = 100;

/// Writes `content` to a temporary file next to `path` and renames it over `path`, so a crash
/// mid-write leaves the old project or the new one, never a truncated file. The new file gets
/// the old one's permissions before anything is written to it (it may hold a password).
///
/// A symlink stays a link: its target gets the content (a dangling link is written through,
/// which creates the target). A file with other hard links, or in a folder where no temporary
/// file can be created, is rewritten in place instead. A read-only file is not written.
fn write_replacing(path: &Path, content: &str) -> io::Result<()> {
    let target = match std::fs::symlink_metadata(path) {
        Ok(link) if link.file_type().is_symlink() => match std::fs::canonicalize(path) {
            Ok(target) => target,
            Err(_) => return write_in_place(path, content),
        },
        _ => path.to_path_buf(),
    };
    if target.file_name().is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a file name",
        ));
    }
    let existing = std::fs::metadata(&target).ok();
    if let Some(old) = &existing {
        if old.permissions().readonly() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the file is read-only",
            ));
        }
        if has_other_hard_links(old) {
            return write_in_place(&target, content);
        }
    }
    let (temp, mut file) = match create_temp(&target) {
        Ok(created) => created,
        // A writable file in a folder that is not: no temporary file can go next to it.
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied && existing.is_some() => {
            return write_in_place(&target, content);
        }
        Err(e) => return Err(e),
    };
    let result = (|| {
        if let Some(old) = &existing {
            file.set_permissions(old.permissions())?;
        }
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temp, &target)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// Creates the temporary file for `target` under the first free name. A name that is already
/// taken, by a leftover file or by a symlink, is skipped without opening it.
fn create_temp(target: &Path) -> io::Result<(PathBuf, File)> {
    for candidate in temp_candidates(target) {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free name for a temporary file in its folder",
    ))
}

/// Names to try, in order, for the temporary file that replaces `target`.
fn temp_candidates(target: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    let pid = std::process::id();
    (0..TEMP_ATTEMPTS).map(move |n| target.with_file_name(format!(".{name}.{pid}.{n}.tmp")))
}

/// Truncates `path` (creating it if missing) and writes `content` into the same file.
fn write_in_place(path: &Path, content: &str) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    file.write_all(content.as_bytes())?;
    file.sync_all()
}

#[cfg(unix)]
fn has_other_hard_links(file: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    file.nlink() > 1
}

#[cfg(not(unix))]
fn has_other_hard_links(_file: &Metadata) -> bool {
    false
}

/// Validates `file` with relative paths (TLS files) resolved against the folder of `path`, or
/// against the working directory while the project has no file.
fn validate_in_folder(file: &ProjectFile, path: Option<&Path>) -> Validated {
    let folder = match path.and_then(Path::parent) {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    file.validate(folder)
}

#[cfg(test)]
mod tests {
    use super::*;
    use senzor_core::project::DeviceSpec;

    /// An empty folder for one test.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("senzor-doc-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const PROJECT: &str = r#"
[[devices]]
id = "office"
template = "co2"
count = 2
"#;

    /// A folder for one test with `project.toml` holding [`PROJECT`]; returns both.
    fn project_file(name: &str) -> (PathBuf, PathBuf) {
        let dir = temp_dir(name);
        let path = dir.join("project.toml");
        std::fs::write(&path, PROJECT).unwrap();
        (dir, path)
    }

    /// Opens `path` and changes the broker host, so there is something to save.
    fn open_edited(path: &Path) -> Document {
        let mut doc = Document::open(path).unwrap();
        doc.edit(|f| f.broker.host = "mqtt.local".into());
        doc
    }

    #[test]
    fn new_document_is_clean_and_untitled() {
        let doc = Document::new_empty();
        assert!(!doc.is_dirty());
        assert_eq!(doc.title(), "untitled");
        assert!(doc.validated().errors.is_empty());
        assert!(doc.validated().project.devices.is_empty());
    }

    #[test]
    fn open_validates_and_titles_with_the_file_name() {
        let (_, path) = project_file("open");
        let doc = Document::open(&path).unwrap();
        assert_eq!(doc.title(), "project.toml");
        assert_eq!(doc.validated().project.instances().count(), 2);
    }

    #[test]
    fn open_reports_parse_errors() {
        let path = temp_dir("parse").join("broken.toml");
        std::fs::write(&path, "[broker]\nhots = 1\n").unwrap();
        match Document::open(&path) {
            Err(OpenError::Parse(errors)) => {
                assert!(errors[0].to_string().starts_with("line 2, column 1"))
            }
            Err(other) => panic!("unexpected error {other:?}"),
            Ok(_) => panic!("a broken file must not open"),
        }
    }

    #[test]
    fn open_reports_missing_files() {
        assert!(matches!(
            Document::open(Path::new("/nonexistent/project.toml")),
            Err(OpenError::Read(_))
        ));
    }

    #[test]
    fn edit_revalidates_and_marks_dirty_until_reverted() {
        let mut doc = Document::new_empty();
        let changed = doc.edit(|f| f.devices.push(DeviceSpec::new("c", "co2")));
        assert!(changed);
        assert!(doc.is_dirty());
        assert_eq!(doc.title(), "untitled*");
        assert_eq!(doc.validated().project.instances().count(), 1);

        assert!(doc.edit(|f| f.devices.clear()));
        assert!(!doc.is_dirty(), "back to the saved model");
        assert!(!doc.edit(|_| {}), "no-op edit reports no change");
    }

    #[test]
    fn a_failed_save_reports_the_error_and_stays_unsaved() {
        let mut doc = Document::new_empty();
        doc.edit(|f| f.broker.port = 1884);
        let error = doc
            .save_as(PathBuf::from("/nonexistent/dir/project.toml"))
            .unwrap_err();
        assert!(
            error.starts_with("cannot write /nonexistent/dir/project.toml"),
            "{error}"
        );
        assert!(doc.is_dirty());
        assert_eq!(doc.path(), None);
    }

    /// A new inode shows the file was swapped in by a rename: the old file was never rewritten
    /// in place, so a crash mid-write cannot leave it truncated.
    #[cfg(unix)]
    #[test]
    fn saving_over_a_file_swaps_in_a_complete_new_one_and_leaves_no_temporary_file() {
        use std::os::unix::fs::MetadataExt;

        let (dir, path) = project_file("atomic");
        let old_inode = std::fs::metadata(&path).unwrap().ino();
        let mut doc = open_edited(&path);

        doc.save().unwrap();

        assert_ne!(std::fs::metadata(&path).unwrap().ino(), old_inode);
        assert_eq!(Document::open(&path).unwrap().file(), doc.file());
        assert_eq!(file_names(&dir), ["project.toml"]);
    }

    /// The file is replaced by a new one, which must not loosen a file kept private because it
    /// holds a broker password.
    #[cfg(unix)]
    #[test]
    fn saving_over_a_file_keeps_its_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let (_, path) = project_file("permissions");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let mut doc = open_edited(&path);

        doc.save().unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    fn file_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[cfg(unix)]
    #[test]
    fn saving_through_a_symlink_writes_its_target_and_keeps_the_link() {
        let dir = temp_dir("symlink");
        let target = dir.join("real.toml");
        std::fs::write(&target, PROJECT).unwrap();
        let link = dir.join("project.toml");
        std::os::unix::fs::symlink("real.toml", &link).unwrap();
        let mut doc = open_edited(&link);

        doc.save().unwrap();

        let link_type = std::fs::symlink_metadata(&link).unwrap().file_type();
        assert!(link_type.is_symlink());
        assert_eq!(Document::open(&target).unwrap().file(), doc.file());
        assert_eq!(file_names(&dir), ["project.toml", "real.toml"]);
    }

    #[cfg(unix)]
    #[test]
    fn saving_through_a_dangling_symlink_creates_its_target() {
        let dir = temp_dir("dangling-symlink");
        let link = dir.join("project.toml");
        std::os::unix::fs::symlink("real.toml", &link).unwrap();
        let mut doc = Document::new_empty();
        doc.edit(|f| f.broker.host = "mqtt.local".into());

        doc.save_as(link.clone()).unwrap();

        let link_type = std::fs::symlink_metadata(&link).unwrap().file_type();
        assert!(link_type.is_symlink());
        let target = dir.join("real.toml");
        assert_eq!(Document::open(&target).unwrap().file(), doc.file());
    }

    #[cfg(unix)]
    #[test]
    fn saving_a_file_with_another_hard_link_updates_both_names() {
        let (dir, path) = project_file("hard-link");
        let other = dir.join("other.toml");
        std::fs::hard_link(&path, &other).unwrap();
        let mut doc = open_edited(&path);

        doc.save().unwrap();

        assert_eq!(Document::open(&path).unwrap().file(), doc.file());
        assert_eq!(Document::open(&other).unwrap().file(), doc.file());
        assert_eq!(file_names(&dir), ["other.toml", "project.toml"]);
    }

    /// Gives a folder back its write permission when dropped, even if the test panics, so it
    /// can be removed.
    #[cfg(unix)]
    struct WritableAgain<'a>(&'a Path);

    #[cfg(unix)]
    impl Drop for WritableAgain<'_> {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(self.0, std::fs::Permissions::from_mode(0o755));
        }
    }

    #[test]
    fn a_file_already_named_like_the_temporary_file_is_left_alone() {
        let (_, path) = project_file("temp-name-taken");
        let taken = temp_candidates(&path).next().unwrap();
        std::fs::write(&taken, "not the save's").unwrap();
        let mut doc = open_edited(&path);

        doc.save().unwrap();

        assert_eq!(std::fs::read_to_string(&taken).unwrap(), "not the save's");
        assert_eq!(Document::open(&path).unwrap().file(), doc.file());
    }

    /// Following the link would let anyone who can write the folder redirect the save into a
    /// file elsewhere.
    #[cfg(unix)]
    #[test]
    fn a_symlink_named_like_the_temporary_file_is_not_followed() {
        let victim = temp_dir("temp-symlink-victim").join("victim.txt");
        std::fs::write(&victim, "victim").unwrap();
        let (_, path) = project_file("temp-symlink");
        let taken = temp_candidates(&path).next().unwrap();
        std::os::unix::fs::symlink(&victim, &taken).unwrap();
        let mut doc = open_edited(&path);

        doc.save().unwrap();

        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "victim");
        assert_eq!(std::fs::read_link(&taken).unwrap(), victim);
        let saved_type = std::fs::symlink_metadata(&path).unwrap().file_type();
        assert!(saved_type.is_file());
        assert_eq!(Document::open(&path).unwrap().file(), doc.file());
    }

    /// No temporary file can be created next to the project, so it is rewritten in place.
    #[cfg(unix)]
    #[test]
    fn saving_a_writable_file_in_a_read_only_folder_rewrites_it() {
        use std::os::unix::fs::PermissionsExt;

        let (dir, path) = project_file("read-only-folder");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let mut doc = open_edited(&path);
        let _restore = WritableAgain(&dir);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let probe = dir.join("probe");
        if std::fs::File::create(&probe).is_ok() {
            let _ = std::fs::remove_file(&probe);
            eprintln!("skipped: running as root, so a read-only folder still takes new files");
            return;
        }

        assert_eq!(doc.save(), Ok(()));

        assert_eq!(Document::open(&path).unwrap().file(), doc.file());
        assert_eq!(file_names(&dir), ["project.toml"]);
    }

    #[test]
    fn a_read_only_file_is_not_overwritten() {
        let (dir, path) = project_file("read-only-file");
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions).unwrap();
        let mut doc = open_edited(&path);

        let error = doc.save().unwrap_err();

        assert_eq!(
            error,
            format!("cannot write {}: the file is read-only", path.display())
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), PROJECT);
        assert!(doc.is_dirty());
        assert_eq!(file_names(&dir), ["project.toml"]);
    }

    #[test]
    fn save_requires_a_path_and_save_as_writes_canonical_toml() {
        let mut doc = Document::new_empty();
        doc.edit(|f| f.broker.host = "mqtt.local".into());
        assert!(doc.save().is_err());

        let path = temp_dir("save").join("saved.toml");
        doc.save_as(path.clone()).unwrap();
        assert!(!doc.is_dirty());
        assert_eq!(doc.title(), "saved.toml");
        let reopened = Document::open(&path).unwrap();
        assert_eq!(reopened.file(), doc.file());
    }
}
