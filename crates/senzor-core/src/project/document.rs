use std::fmt;
use std::fs::{File, Metadata, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::format::update_text;
use super::{ProjectError, ProjectFile, Validated};

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
    /// Where relative paths start while the project has no file; `None` means the working
    /// directory.
    untitled_folder: Option<PathBuf>,
    file: ProjectFile,
    saved: ProjectFile,
    validated: Validated,
    /// The model before each undo step, the latest last; at most [`HISTORY`].
    undo: Vec<Step>,
    /// The models undone, the latest last; a new edit clears them.
    redo: Vec<Step>,
    /// The group of the latest step while more edits in it may join it.
    open_group: Option<u64>,
    /// The file's modification time and size when last looked at.
    disk: Option<DiskStamp>,
    /// Another project found in the file and not yet taken or kept, with the file's text.
    on_disk: Option<(ProjectFile, String)>,
    /// The file's text as last read or written, and the project it holds. Saving changes only
    /// what was edited in it, so comments and formatting stay.
    source: Option<Arc<Source>>,
    /// The text known to be in the file; a `source` text that differs from it, such as a new
    /// comment, is an unsaved change.
    on_file: Option<String>,
    /// Counts the changes to the project and to the text saving builds on.
    revision: u64,
    /// A `#:schema` line the file started with, which editors read to check and complete it.
    /// Saving keeps it as the first line even when the file is written in the plain form.
    schema_line: Option<String>,
}

/// A file's text and the project it holds.
#[derive(Debug, PartialEq)]
struct Source {
    text: String,
    file: ProjectFile,
}

impl Source {
    fn new(text: String, file: ProjectFile) -> Option<Arc<Self>> {
        Some(Arc::new(Self { text, file }))
    }
}

/// One undo or redo step: the project and the text saving builds on.
#[derive(Debug, Clone)]
struct Step {
    file: ProjectFile,
    source: Option<Arc<Source>>,
}

/// What tells that a file changed without reading it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct DiskStamp {
    modified: std::time::SystemTime,
    len: u64,
}

impl DiskStamp {
    fn of(path: &Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        Some(Self {
            modified: metadata.modified().ok()?,
            len: metadata.len(),
        })
    }
}

/// How many undo steps a document keeps.
const HISTORY: usize = 100;

impl Document {
    pub fn new_empty() -> Self {
        Self::from_file(None, None, ProjectFile::default())
    }

    /// A project that has no file yet and nothing to save, such as an example.
    pub fn untitled(file: ProjectFile) -> Self {
        Self::from_file(None, None, file)
    }

    /// [`Self::untitled`], with relative paths (TLS and `.proto` files) resolved against
    /// `folder` until the project is saved, such as an example's own files.
    pub fn untitled_in(file: ProjectFile, folder: PathBuf) -> Self {
        Self::from_file(None, Some(folder), file)
    }

    pub fn open(path: &Path) -> Result<Self, OpenError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| OpenError::Read(format!("cannot read {}: {e}", path.display())))?;
        let file = ProjectFile::parse(&text).map_err(OpenError::Parse)?;
        let mut doc = Self::from_file(Some(path.to_path_buf()), None, file.clone());
        doc.schema_line = schema_line(&text);
        doc.on_file = Some(text.clone());
        doc.source = Source::new(text, file);
        Ok(doc)
    }

    fn from_file(
        path: Option<PathBuf>,
        untitled_folder: Option<PathBuf>,
        file: ProjectFile,
    ) -> Self {
        let validated = file.validate(folder_of(path.as_deref(), untitled_folder.as_deref()));
        let disk = path.as_deref().and_then(DiskStamp::of);
        Self {
            path,
            untitled_folder,
            saved: file.clone(),
            file,
            validated,
            undo: Vec::new(),
            redo: Vec::new(),
            open_group: None,
            disk,
            on_disk: None,
            source: None,
            on_file: None,
            revision: 0,
            schema_line: None,
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn file(&self) -> &ProjectFile {
        &self.file
    }

    /// The folder that relative paths in the project (TLS and `.proto` files) start from: the
    /// project file's folder or, while the project has no file, the folder it was opened in
    /// ([`Self::untitled_in`]) or the working directory.
    pub fn folder(&self) -> &Path {
        folder_of(self.path.as_deref(), self.untitled_folder.as_deref())
    }

    pub fn validated(&self) -> &Validated {
        &self.validated
    }

    /// True when the model differs from what was last opened or saved (undoing an edit clears it),
    /// or when the text to save differs from the file's only in comments or layout.
    pub fn is_dirty(&self) -> bool {
        self.file != self.saved
            || self
                .source
                .as_ref()
                .is_some_and(|source| self.on_file.as_ref() != Some(&source.text))
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

    /// Applies `f` and re-validates. Returns whether the model changed; a change is an undo step.
    pub fn edit(&mut self, f: impl FnOnce(&mut ProjectFile)) -> bool {
        self.edit_grouped(None, f)
    }

    /// As [`Document::edit`], but consecutive changed edits in the same `group` make one undo
    /// step; the GUI groups the keystrokes typed into one text field.
    pub fn edit_grouped(&mut self, group: Option<u64>, f: impl FnOnce(&mut ProjectFile)) -> bool {
        let before = self.file.clone();
        f(&mut self.file);
        if self.file == before {
            return false;
        }
        let joins = group.is_some() && group == self.open_group;
        if !joins {
            self.push_undo(before);
            self.open_group = group;
        } else if self.undo.last().map(|step| &step.file) == Some(&self.file) {
            // Typed and taken back: the step would undo nothing.
            self.undo.pop();
            self.open_group = None;
        }
        self.redo.clear();
        self.revalidate();
        true
    }

    /// Makes `before` and the current source an undo step.
    fn push_undo(&mut self, before: ProjectFile) {
        self.undo.push(Step {
            file: before,
            source: self.source.clone(),
        });
        if self.undo.len() > HISTORY {
            self.undo.remove(0);
        }
    }

    /// The current project and source as a step, replaced by `step`'s.
    fn swap(&mut self, step: Step) -> Step {
        Step {
            file: std::mem::replace(&mut self.file, step.file),
            source: std::mem::replace(&mut self.source, step.source),
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Reverts the latest undo step; false when there is none.
    pub fn undo(&mut self) -> bool {
        let Some(before) = self.undo.pop() else {
            return false;
        };
        let current = self.swap(before);
        self.redo.push(current);
        self.open_group = None;
        self.revalidate();
        true
    }

    /// Applies the latest undone step again; false when there is none.
    pub fn redo(&mut self) -> bool {
        let Some(after) = self.redo.pop() else {
            return false;
        };
        let current = self.swap(after);
        self.undo.push(current);
        self.open_group = None;
        self.revalidate();
        true
    }

    fn revalidate(&mut self) {
        self.validated = self.file.validate(self.folder());
        self.revision += 1;
    }

    /// A number that changes whenever the project or [`Document::text_to_save`] may have; views
    /// that show a costly rendering of the document keep it until this changes.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Whether the file now holds another valid project than the one last opened or saved, not
    /// yet reported. A file that holds the same project, written differently, becomes the text
    /// the next save builds on. Reads the file only when its modification time or size changed; a file
    /// that does not parse is left for the next check.
    pub fn check_disk(&mut self) -> bool {
        let Some(path) = self.path.as_deref() else {
            return false;
        };
        let stamp = DiskStamp::of(path);
        if stamp.is_none() || stamp == self.disk {
            return false;
        }
        let Some((found, text)) = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| Some((ProjectFile::parse(&text).ok()?, text)))
        else {
            return false;
        };
        self.disk = stamp;
        if found == self.saved {
            // The same project written differently, such as a changed comment: the next save
            // builds on this text.
            self.on_file = Some(text.clone());
            self.source = Source::new(text, found);
            self.revision += 1;
            return false;
        }
        self.on_disk = Some((found, text));
        true
    }

    /// Takes the project [`Document::check_disk`] found as an edit (Undo returns to the one
    /// before) and as what is saved.
    pub fn reload(&mut self) {
        if let Some((found, text)) = self.on_disk.take() {
            self.edit(|file| *file = found.clone());
            self.on_file = Some(text.clone());
            self.source = Source::new(text, found.clone());
            self.saved = found;
        }
    }

    /// Keeps the current project over the one [`Document::check_disk`] found, which becomes
    /// what is saved: the current one now counts as unsaved changes.
    pub fn keep_mine(&mut self) {
        if let Some((found, text)) = self.on_disk.take() {
            self.on_file = Some(text.clone());
            self.source = Source::new(text, found.clone());
            self.saved = found;
            self.revision += 1;
        }
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
        self.validated = self.file.validate(self.folder());
        Ok(())
    }

    /// Replaces the project with the one `text` holds, as one undo step, also when only comments
    /// or layout differ; saving then writes `text` as it is. Returns whether anything changed.
    /// Text that does not parse as a project changes nothing.
    pub fn edit_text(&mut self, text: &str) -> Result<bool, Vec<ProjectError>> {
        let file = ProjectFile::parse(text)?;
        if file == self.file && text == self.text_to_save() {
            return Ok(false);
        }
        // One step, also when only comments or layout changed.
        let before = std::mem::replace(&mut self.file, file.clone());
        self.push_undo(before);
        self.redo.clear();
        self.open_group = None;
        self.source = Source::new(text.to_string(), file);
        self.revalidate();
        Ok(true)
    }

    /// What saving writes: the file's text with the edits made since it was read or written, or
    /// the plain form for a project that has no text yet.
    pub fn text_to_save(&self) -> String {
        let toml = match &self.source {
            Some(source) => update_text(&source.text, &source.file, &self.file),
            None => self.file.to_toml_string(),
        };
        match &self.schema_line {
            Some(line) if !toml.starts_with(line.as_str()) => format!("{line}\n{toml}"),
            _ => toml,
        }
    }

    fn write(&mut self, path: &Path) -> Result<(), String> {
        let content = self.text_to_save();
        write_replacing(path, &content)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        self.on_file = Some(content.clone());
        self.source = Source::new(content, self.file.clone());
        self.revision += 1;
        self.saved = self.file.clone();
        self.disk = DiskStamp::of(path);
        self.on_disk = None;
        Ok(())
    }
}

/// The file's first line when it is a `#:schema` directive.
fn schema_line(text: &str) -> Option<String> {
    let first = text.lines().next()?.trim_end();
    first.starts_with("#:schema").then(|| first.to_string())
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

/// Where relative paths start: the folder of the project file at `path`; without a file,
/// `untitled` or else the working directory.
fn folder_of<'a>(path: Option<&'a Path>, untitled: Option<&'a Path>) -> &'a Path {
    match path {
        Some(path) => match path.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir,
            _ => Path::new("."),
        },
        None => untitled.unwrap_or(Path::new(".")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::DeviceSpec;

    /// An empty folder for one test.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("senzor-doc-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn an_untitled_project_reads_its_files_from_the_folder_it_was_opened_in_until_saved() {
        let dir = temp_dir("untitled-in");
        std::fs::write(
            dir.join("a.proto"),
            "syntax = \"proto3\";\nmessage A { int32 v = 1; }\n",
        )
        .unwrap();
        let file = ProjectFile::parse(
            "[[templates]]\nid = \"t\"\n\
             messages = [{ topic = \"t\", payload = '{\"v\": 1}', encoding = \"protobuf\", \
             proto = \"a.proto\", message = \"A\" }]\n\
             fields.v = { generator = \"constant\", value = 1 }\n",
        )
        .unwrap();
        let doc = Document::untitled_in(file.clone(), dir.clone());
        assert_eq!(doc.folder(), dir);
        assert!(
            doc.validated().errors.is_empty(),
            "{:?}",
            doc.validated().errors
        );
        assert_eq!(
            Document::untitled(file).validated().errors.len(),
            1,
            "the working directory has no a.proto"
        );

        let mut doc = doc;
        let elsewhere = temp_dir("untitled-in-saved").join("p.toml");
        doc.save_as(elsewhere.clone()).unwrap();
        assert_eq!(doc.folder(), elsewhere.parent().unwrap());
        assert_eq!(
            doc.validated().errors.len(),
            1,
            "saved where a.proto is not"
        );
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
    fn an_untitled_document_holds_a_project_without_unsaved_changes() {
        let file = ProjectFile::parse("[[devices]]\nid = \"d\"\ntemplate = \"co2\"\n").unwrap();
        let doc = Document::untitled(file.clone());
        assert_eq!(doc.file(), &file);
        assert_eq!(doc.path(), None);
        assert!(!doc.is_dirty());
        assert!(!doc.can_undo());
        assert!(doc.validated().errors.is_empty());
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
    fn a_schema_line_at_the_top_is_kept_when_saving() {
        let dir = temp_dir("schema-line");
        let path = dir.join("project.toml");
        let directive = "#:schema https://senzor.kankaj.cz/schema/project.json";
        std::fs::write(&path, format!("{directive}\n# a comment\n{PROJECT}")).unwrap();
        let mut doc = open_edited(&path);
        doc.save().unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.starts_with(&format!("{directive}\n")), "{saved}");
        assert!(saved.contains("# a comment"), "other comments are kept too");

        let other = dir.join("other.toml");
        doc.save_as(other.clone()).unwrap();
        assert!(
            std::fs::read_to_string(&other)
                .unwrap()
                .starts_with(directive)
        );
    }

    #[test]
    fn a_file_without_a_schema_line_gets_none() {
        let (_dir, path) = project_file("no-schema-line");
        let mut doc = open_edited(&path);
        doc.save().unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("#:schema"));
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

    fn add(doc: &mut Document, id: &str) -> bool {
        doc.edit(|f| f.devices.push(DeviceSpec::new(id, "co2")))
    }

    fn ids(doc: &Document) -> Vec<&str> {
        doc.file().devices.iter().map(|d| d.id.as_str()).collect()
    }

    #[test]
    fn undo_reverts_the_last_edit_and_redo_applies_it_again() {
        let mut doc = Document::new_empty();
        assert!(!doc.can_undo());
        assert!(!doc.undo(), "nothing to undo");
        add(&mut doc, "a");
        add(&mut doc, "b");

        assert!(doc.undo());
        assert_eq!(ids(&doc), ["a"]);
        assert!(doc.can_redo());
        assert!(doc.undo());
        assert_eq!(ids(&doc), Vec::<&str>::new());
        assert!(!doc.is_dirty(), "back where the project started");
        assert!(
            doc.validated().project.devices.is_empty(),
            "validated again"
        );

        assert!(doc.redo());
        assert_eq!(ids(&doc), ["a"]);
        assert!(doc.redo());
        assert_eq!(ids(&doc), ["a", "b"]);
        assert!(!doc.redo(), "nothing to redo");
    }

    #[test]
    fn a_new_edit_after_undo_drops_what_could_be_redone() {
        let mut doc = Document::new_empty();
        add(&mut doc, "a");
        doc.undo();
        add(&mut doc, "b");
        assert!(!doc.can_redo());
        assert_eq!(ids(&doc), ["b"]);
    }

    #[test]
    fn an_edit_that_changes_nothing_is_no_undo_step() {
        let mut doc = Document::new_empty();
        add(&mut doc, "a");
        assert!(!doc.edit(|_| {}));
        doc.undo();
        assert_eq!(ids(&doc), Vec::<&str>::new());
    }

    #[test]
    fn edits_in_one_group_undo_together_until_another_edit_comes_between() {
        let mut doc = Document::new_empty();
        add(&mut doc, "d");
        let rename = |doc: &mut Document, group, id: &str| {
            doc.edit_grouped(group, |f| f.devices[0].id = id.into())
        };
        rename(&mut doc, Some(1), "d1");
        rename(&mut doc, Some(1), "d12");
        rename(&mut doc, Some(2), "d123");
        add(&mut doc, "e");
        rename(&mut doc, Some(2), "d1234");

        doc.undo();
        assert_eq!(
            ids(&doc),
            ["d123", "e"],
            "group 2 after another edit is its own step"
        );
        doc.undo();
        assert_eq!(ids(&doc), ["d123"]);
        doc.undo();
        assert_eq!(ids(&doc), ["d12"], "group 2 is one step");
        doc.undo();
        assert_eq!(ids(&doc), ["d"], "both edits of group 1 undo together");
    }

    #[test]
    fn typing_after_an_undo_starts_a_new_step() {
        let mut doc = Document::new_empty();
        add(&mut doc, "d");
        doc.edit_grouped(Some(1), |f| f.devices[0].id = "x".into());
        doc.undo();
        doc.edit_grouped(Some(1), |f| f.devices[0].id = "y".into());
        doc.undo();
        assert_eq!(ids(&doc), ["d"]);
        assert!(doc.can_undo());
    }

    #[test]
    fn the_history_keeps_the_latest_hundred_steps() {
        let mut doc = Document::new_empty();
        for n in 0..105 {
            add(&mut doc, &format!("d{n}"));
        }
        let mut undone = 0;
        while doc.undo() {
            undone += 1;
        }
        assert_eq!(undone, 100);
        assert_eq!(doc.file().devices.len(), 5);
    }

    #[test]
    fn a_group_that_ends_where_it_started_leaves_no_undo_step() {
        let mut doc = Document::new_empty();
        add(&mut doc, "d");
        doc.edit_grouped(Some(1), |f| f.devices[0].id = "dx".into());
        doc.edit_grouped(Some(1), |f| f.devices[0].id = "d".into());
        doc.undo();
        assert_eq!(ids(&doc), Vec::<&str>::new(), "the add is the step undone");
    }

    /// A project file in its own folder, opened.
    fn opened(name: &str, src: &str) -> (PathBuf, Document) {
        let path = temp_dir(name).join("project.toml");
        std::fs::write(&path, src).unwrap();
        let doc = Document::open(&path).unwrap();
        (path, doc)
    }

    const ONE: &str = "[[devices]]\nid = \"a\"\ntemplate = \"co2\"\n";
    const TWO: &str = "[[devices]]\nid = \"a\"\ntemplate = \"co2\"\n\n[[devices]]\nid = \"b\"\ntemplate = \"co2\"\n";

    #[test]
    fn a_file_changed_elsewhere_is_reported_once() {
        let (path, mut doc) = opened("disk-changed", ONE);
        assert!(!doc.check_disk(), "nothing changed yet");
        std::fs::write(&path, TWO).unwrap();
        assert!(doc.check_disk());
        assert!(!doc.check_disk(), "reported once");
    }

    #[test]
    fn our_own_save_and_a_rewrite_with_the_same_project_are_no_change() {
        let (path, mut doc) = opened("disk-own-save", ONE);
        add(&mut doc, "b");
        doc.save().unwrap();
        assert!(!doc.check_disk());
        // Same project, other formatting.
        std::fs::write(&path, format!("# a comment\n{TWO}")).unwrap();
        assert!(!doc.check_disk());
    }

    #[test]
    fn a_half_written_file_waits_until_it_parses() {
        let (path, mut doc) = opened("disk-half-written", ONE);
        std::fs::write(&path, "[[devices]\nid = ").unwrap();
        assert!(!doc.check_disk());
        std::fs::write(&path, TWO).unwrap();
        assert!(doc.check_disk());
    }

    #[test]
    fn reload_takes_the_file_as_an_undoable_edit_and_as_saved() {
        let (path, mut doc) = opened("disk-reload", ONE);
        std::fs::write(&path, TWO).unwrap();
        assert!(doc.check_disk());
        doc.reload();
        assert_eq!(ids(&doc), ["a", "b"]);
        assert!(!doc.is_dirty());
        assert_eq!(doc.validated().project.devices.len(), 2);
        doc.undo();
        assert_eq!(ids(&doc), ["a"]);
        assert!(doc.is_dirty(), "the file holds two devices now");
    }

    #[test]
    fn comments_survive_saves_and_a_reload_from_disk() {
        let commented =
            "# Office sensors.\n[[devices]]\nid = \"a\"  # the first desk\ntemplate = \"co2\"\n";
        let (path, mut doc) = opened("disk-comments", commented);
        doc.edit(|f| f.devices[0].count = 3);
        doc.save().unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert_eq!(saved, format!("{commented}count = 3\n"));

        // Someone changes only a comment: the same project, nothing to reload, but Senzor's
        // next save keeps their comment.
        let theirs = saved.replace("# Office sensors.", "# Office sensors, second floor.");
        std::fs::write(&path, &theirs).unwrap();
        assert!(!doc.check_disk(), "the same project: nothing to reload");
        doc.edit(|f| f.devices[0].count = 4);
        doc.save().unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(
            saved.starts_with("# Office sensors, second floor.\n"),
            "{saved}"
        );

        // Then they change the project too; Senzor reloads it and saves an edit over it.
        std::fs::write(&path, saved.replace("count = 4", "count = 6  # more desks")).unwrap();
        assert!(doc.check_disk());
        doc.reload();
        doc.edit(|f| f.devices[0].autostart = false);
        doc.save().unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains("# the first desk\n"), "{saved}");
        assert!(saved.contains("count = 6  # more desks\n"), "{saved}");
        assert!(saved.contains("autostart = false\n"), "{saved}");
    }

    #[test]
    fn a_whole_text_is_one_edit_and_saved_as_written() {
        let (path, mut doc) = opened("edit-text", ONE);
        let text = "# Hand-written.\n[[devices]]\nid = \"z\"   # renamed\ntemplate = \"co2\"\n";
        doc.edit_text(text).unwrap();
        assert_eq!(ids(&doc), ["z"]);
        assert!(doc.is_dirty());
        assert_eq!(doc.text_to_save(), text);
        doc.save().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);

        doc.undo();
        assert_eq!(ids(&doc), ["a"], "one undo step");
        let errors = doc.edit_text("[[devices]\n").unwrap_err();
        assert!(errors[0].to_string().starts_with("line 1"), "{errors:?}");
        assert_eq!(ids(&doc), ["a"], "text that does not parse changes nothing");
    }

    #[test]
    fn a_changed_comment_is_an_unsaved_change() {
        let (_path, mut doc) = opened("edit-comment", ONE);
        let text = format!("# A note.\n{}", doc.text_to_save());
        assert!(doc.edit_text(&text).unwrap());
        assert!(
            doc.is_dirty(),
            "the same project, but the file would change"
        );
        assert_eq!(doc.text_to_save(), text);
        doc.undo();
        assert!(!doc.is_dirty(), "undo takes the comment back");
        assert!(!doc.text_to_save().contains("# A note."));
        doc.redo();
        doc.save().unwrap();
        assert!(!doc.is_dirty());
        assert!(
            !doc.edit_text(&text).unwrap(),
            "the same text again changes nothing"
        );
    }

    #[test]
    fn keeping_mine_makes_it_the_unsaved_version() {
        let (path, mut doc) = opened("disk-keep", ONE);
        std::fs::write(&path, TWO).unwrap();
        assert!(doc.check_disk());
        doc.keep_mine();
        assert_eq!(ids(&doc), ["a"]);
        assert!(doc.is_dirty());
        doc.save().unwrap();
        assert!(!doc.check_disk(), "our save is no change");
        assert!(!std::fs::read_to_string(&path).unwrap().contains("\"b\""));
    }
}
