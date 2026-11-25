//! Local Markdown workspaces and conflict-checked file saves.

use std::{
    fs::{self, DirEntry, Metadata, Permissions},
    io,
    path::{Path, PathBuf},
    time::SystemTime,
};

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod publication;
mod search;

pub use search::{SearchHit, SearchLocation, SearchResults};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    root: PathBuf,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    identity: (u64, u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub path: PathBuf,
    pub kind: EntryKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Folder,
    Markdown,
    File,
}

impl Workspace {
    pub fn open(path: &Path) -> io::Result<Self> {
        let root = path.canonicalize()?;
        let metadata = fs::metadata(&root)?;
        if !metadata.is_dir() {
            return Err(io::Error::new(io::ErrorKind::NotADirectory, "not a folder"));
        }
        Ok(Self {
            root,
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            identity: publication::identity(&metadata),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Return the device and inode captured when the workspace was opened, without I/O.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub fn root_identity(&self) -> (u64, u64) {
        self.identity
    }

    /// List only regular files directly in the workspace. Do not follow sidebar symlinks.
    pub fn markdown_files(&self) -> io::Result<Vec<PathBuf>> {
        self.check_root()?;
        let mut files = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();
            if is_markdown(&path) && entry.file_type()?.is_file() {
                files.push(path);
            }
        }
        self.check_root()?;
        files.sort_unstable();
        Ok(files)
    }

    /// List one folder, with folders first. Do not follow symbolic links.
    pub fn directory_entries(&self, path: &Path) -> io::Result<Vec<DirectoryEntry>> {
        let path = self.root.join(path);
        self.check_direct_path(&path)?;
        let mut entries = Vec::new();
        for entry in fs::read_dir(&path)? {
            entries.extend(classify_entry(&entry?)?);
        }
        self.check_direct_path(&path)?;
        entries.sort_unstable_by(|a, b| {
            (a.kind != EntryKind::Folder, &a.path).cmp(&(b.kind != EntryKind::Folder, &b.path))
        });
        Ok(entries)
    }

    /// Resolve an absolute or workspace-relative Markdown file path without reading the file.
    pub fn resolve_document(&self, path: &Path) -> io::Result<PathBuf> {
        let path = self.resolve_markdown_path(path)?;
        file_metadata(&path)?;
        Ok(path)
    }

    /// Open a Markdown file by absolute path or by a path relative to the workspace root.
    pub fn read_document(&self, path: &Path) -> io::Result<DocumentFile> {
        let path = self.resolve_markdown_path(path)?;
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let parent = path.parent().expect("an absolute document has a parent");
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let parent_identity = publication::identity(&fs::metadata(parent)?);
        let metadata = file_metadata(&path)?;
        let stamp = Stamp::read(&metadata)?;
        let source = fs::read_to_string(&path)?;
        if self.resolve_markdown_path(&path)? != path
            || Stamp::read(&file_metadata(&path)?)? != stamp
        {
            return Err(external_change());
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        if publication::identity(&fs::metadata(parent)?) != parent_identity {
            return Err(publication::folder_moved());
        }
        Ok(DocumentFile {
            directory: self.clone(),
            path,
            source,
            stamp,
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            parent_identity,
        })
    }

    /// Check that the workspace root still points to the opened directory.
    pub fn check_root(&self) -> io::Result<()> {
        let canonical = self.root.canonicalize()?;
        let metadata = fs::metadata(&self.root)?;
        let same_directory = canonical == self.root && metadata.is_dir();
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let same_directory = same_directory && publication::identity(&metadata) == self.identity;
        if !same_directory {
            return Err(io::Error::other("the workspace folder has moved"));
        }
        Ok(())
    }

    fn resolve(&self, path: &Path) -> io::Result<PathBuf> {
        self.check_root()?;
        let canonical = self.root.join(path).canonicalize()?;
        if !canonical.starts_with(&self.root) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the path is outside the workspace",
            ));
        }
        Ok(canonical)
    }

    fn check_direct_path(&self, path: &Path) -> io::Result<()> {
        if self.resolve(path)? != path {
            return Err(io::Error::other("not a direct workspace path"));
        }
        Ok(())
    }

    fn resolve_markdown_path(&self, path: &Path) -> io::Result<PathBuf> {
        check_extension(path)?;
        let path = self.resolve(path)?;
        check_extension(&path)?;
        Ok(path)
    }
}

/// The last version read from or saved to disk. The editor keeps its own editable source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentFile {
    directory: Workspace,
    path: PathBuf,
    source: String,
    stamp: Stamp,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    parent_identity: (u64, u64),
}

impl DocumentFile {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// Write a new file without replacing an existing entry, even if it appears during Save.
    /// If publication cannot be verified, the draft may remain in the original folder.
    /// If no workspace is selected, scope subsequent saves to the chosen file's parent folder.
    pub fn create(workspace: Option<&Workspace>, path: &Path, source: String) -> io::Result<Self> {
        let mut path = path.to_owned();
        if path.extension().is_none() {
            path.set_extension("md");
        }
        check_extension(&path)?;
        let name = path
            .file_name()
            .ok_or_else(|| io::Error::other("missing file name"))?;
        let parent = path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let directory = match workspace {
            Some(workspace) => workspace.clone(),
            None => Workspace::open(parent)?,
        };
        let parent = if workspace.is_some() {
            directory.resolve(parent)?
        } else {
            directory.root.clone()
        };
        let path = parent.join(name);
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let (stamp, parent_identity) = publication::create(&directory, &path, &source)?;
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let stamp = unsupported_publication(&directory, &path, &source)?;
        Ok(Self {
            directory,
            path,
            source,
            stamp,
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            parent_identity,
        })
    }

    /// Sync the draft before exchanging it with the file on disk (macOS and Linux).
    /// Check the original parent folder and verify the published bytes before updating the snapshot.
    /// On a late conflict the draft may be at the document path, but the displaced version
    /// is kept for recovery and the saved snapshot stays unchanged.
    pub fn save(&mut self, source: &str) -> io::Result<()> {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let stamp = publication::save(self, source, || {})?;
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let stamp = unsupported_publication(&self.directory, &self.path, source)?;
        source.clone_into(&mut self.source);
        self.stamp = stamp;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    modified: SystemTime,
    len: u64,
    permissions: Permissions,
    #[cfg(unix)]
    identity: (u64, u64),
    #[cfg(unix)]
    owner: (u32, u32),
    #[cfg(unix)]
    changed: (i64, i64),
}

impl Stamp {
    fn read(metadata: &Metadata) -> io::Result<Self> {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt as _;

        Ok(Self {
            modified: metadata.modified()?,
            len: metadata.len(),
            permissions: metadata.permissions(),
            #[cfg(unix)]
            identity: (metadata.dev(), metadata.ino()),
            #[cfg(unix)]
            owner: (metadata.uid(), metadata.gid()),
            #[cfg(unix)]
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        })
    }
}

fn classify_entry(entry: &DirEntry) -> io::Result<Option<DirectoryEntry>> {
    let file_type = entry.file_type()?;
    if file_type.is_dir()
        && entry
            .file_name()
            .as_encoded_bytes()
            .starts_with(b".gpui-save-")
    {
        // Private recovery folders are not workspaces to browse.
        return Ok(None);
    }
    let path = entry.path();
    let kind = if file_type.is_dir() {
        EntryKind::Folder
    } else if file_type.is_file() && is_markdown(&path) {
        EntryKind::Markdown
    } else if file_type.is_file() {
        EntryKind::File
    } else {
        return Ok(None);
    };
    Ok(Some(DirectoryEntry { path, kind }))
}

fn is_markdown(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "md")
}

fn check_extension(path: &Path) -> io::Result<()> {
    if !is_markdown(path) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "choose a .md file",
        ));
    }
    Ok(())
}

fn file_metadata(path: &Path) -> io::Result<Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    Ok(metadata)
}

fn external_change() -> io::Error {
    io::Error::other("the file changed on disk; reopen it before saving")
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn unsupported_publication(_: &Workspace, _: &Path, _: &str) -> io::Result<Stamp> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "workspace file publication requires macOS or Linux",
    ))
}
