//! Local Markdown workspaces and conflict-checked file saves.

use std::{
    fs::{self, Metadata, Permissions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};

use tempfile::NamedTempFile;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    pub fn open(path: &Path) -> io::Result<Self> {
        let root = path.canonicalize()?;
        if !fs::metadata(&root)?.is_dir() {
            return Err(io::Error::new(io::ErrorKind::NotADirectory, "not a folder"));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
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
        files.sort_unstable();
        Ok(files)
    }

    /// Open a Markdown file by absolute path or by a path relative to the workspace root.
    pub fn read_document(&self, path: &Path) -> io::Result<DocumentFile> {
        let path = self.resolve_document(path)?;
        let metadata = file_metadata(&path)?;
        let stamp = Stamp::read(&metadata)?;
        let source = fs::read_to_string(&path)?;
        if self.resolve_document(&path)? != path || Stamp::read(&file_metadata(&path)?)? != stamp {
            return Err(external_change());
        }
        Ok(DocumentFile {
            directory: self.clone(),
            path,
            source,
            stamp,
        })
    }

    fn check_root(&self) -> io::Result<()> {
        if self.root.canonicalize()? != self.root || !fs::metadata(&self.root)?.is_dir() {
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

    fn resolve_document(&self, path: &Path) -> io::Result<PathBuf> {
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
}

impl DocumentFile {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// Write a new file without replacing an existing entry, even if it appears during Save.
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
        let (temp, stamp) = write_temp(&parent, &source, None)?;
        if directory.resolve(&parent)? != parent {
            return Err(io::Error::other("the destination folder has moved"));
        }
        temp.persist_noclobber(&path).map_err(|error| error.error)?;
        Ok(Self {
            directory,
            path,
            source,
            stamp,
        })
    }

    /// Replace an existing file only after the complete new version is written and synced.
    /// Leave the saved snapshot intact if writing or the final conflict check fails.
    pub fn save(&mut self, source: &str) -> io::Result<()> {
        let metadata = self.checked_metadata()?;
        let permissions = metadata.permissions();
        if permissions.readonly() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the file is read-only",
            ));
        }
        let parent = self.path.parent().expect("a document has a parent folder");
        let (temp, stamp) = write_temp(parent, source, Some(permissions))?;
        // Check both bytes and metadata again after writing the temp file. A portable rename
        // cannot compare and replace atomically, so minimize the gap before the final rename.
        self.checked_metadata()?;
        if fs::read(&self.path)? != self.source.as_bytes() {
            return Err(external_change());
        }
        self.checked_metadata()?;
        temp.persist(&self.path).map_err(|error| error.error)?;
        source.clone_into(&mut self.source);
        self.stamp = stamp;
        Ok(())
    }

    fn checked_metadata(&self) -> io::Result<Metadata> {
        if self.directory.resolve_document(&self.path)? != self.path {
            return Err(external_change());
        }
        let metadata = file_metadata(&self.path)?;
        if Stamp::read(&metadata)? != self.stamp {
            return Err(external_change());
        }
        Ok(metadata)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    modified: SystemTime,
    len: u64,
    permissions: Permissions,
    #[cfg(unix)]
    identity: (u64, u64),
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
        })
    }
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

fn write_temp(
    parent: &Path,
    source: &str,
    permissions: Option<Permissions>,
) -> io::Result<(NamedTempFile, Stamp)> {
    let mut temp = NamedTempFile::new_in(parent)?;
    temp.write_all(source.as_bytes())?;
    if let Some(permissions) = permissions {
        temp.as_file().set_permissions(permissions)?;
    }
    temp.as_file().sync_all()?;
    let stamp = Stamp::read(&temp.as_file().metadata()?)?;
    Ok((temp, stamp))
}
