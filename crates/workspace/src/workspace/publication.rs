//! Descriptor-relative workspace publication on macOS and Linux.

use std::{
    ffi::{CStr, CString, OsStr},
    fs::{File, Metadata},
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Component, Path},
};

mod acl;
use acl::Acl;

use super::{DocumentFile, Stamp, Workspace, external_change};

const VERSION: &CStr = c"version";

pub(super) fn create(
    workspace: &Workspace,
    path: &Path,
    source: &str,
) -> io::Result<(Stamp, (u64, u64))> {
    let (parent, name, directory) = destination(workspace, path)?;
    let parent_identity = identity(&directory.0.metadata()?);
    let stage = Stage::new(&directory, parent)?;
    let mut draft = stage.new_file()?;
    draft.write_all(source.as_bytes())?;
    draft.sync_all()?;
    let draft_stamp = Stamp::read(&draft.metadata()?)?;
    check_parent(workspace, parent, &directory)?;
    // SAFETY: Both directory descriptors and basename pointers are valid. linkat without
    // flags publishes the complete file and fails if any entry already has the name.
    status(unsafe {
        libc::linkat(
            stage.dir.fd(),
            VERSION.as_ptr(),
            directory.fd(),
            name.as_ptr(),
            0,
        )
    })?;
    // Removing the staging link can update ctime. Verify and take the final stamp afterward.
    drop(stage);
    let verified = (|| {
        let stamp = check_published(&directory, &name, &draft_stamp, source)?;
        check_parent(workspace, parent, &directory)?;
        Ok((stamp, parent_identity))
    })();
    verified.map_err(|error: io::Error| {
        io::Error::other(format!(
            "could not verify the new file at {} ({error}); the draft may be in the original destination folder if it moved",
            path.display()
        ))
    })
}

pub(super) fn save(
    document: &DocumentFile,
    source: &str,
    before_exchange: impl FnOnce(),
) -> io::Result<Stamp> {
    let workspace = &document.directory;
    let (parent, name, directory) = destination(workspace, &document.path)?;
    if identity(&directory.0.metadata()?) != document.parent_identity {
        return Err(folder_moved());
    }
    let mut original = directory.open_file(&name, libc::O_RDWR)?;
    let metadata = check_original(document, &original)?;
    let acl = Acl::read(&original)?;

    let mut stage = Stage::new(&directory, parent)?;
    let mut draft = stage.new_file()?;
    draft.write_all(source.as_bytes())?;
    let owner = draft.metadata()?;
    if (owner.uid(), owner.gid()) != (metadata.uid(), metadata.gid()) {
        // SAFETY: The descriptor is live and both IDs came from the original file.
        status(unsafe { libc::fchown(draft.as_raw_fd(), metadata.uid(), metadata.gid()) })?;
    }
    acl.apply(&draft)?;
    draft.set_permissions(metadata.permissions())?;
    if !acl.matches(&draft)? {
        return Err(io::Error::other(
            "the replacement ACL differs from the original",
        ));
    }
    draft.sync_all()?;
    let draft_stamp = Stamp::read(&draft.metadata()?)?;
    if draft_stamp.owner != document.stamp.owner
        || draft_stamp.permissions != document.stamp.permissions
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "could not preserve file ownership and mode",
        ));
    }

    if !same_bytes(&mut original, &document.source)? {
        return Err(external_change());
    }
    // Reopen without create or truncate. This checks actual write access, including ACLs,
    // even if a different owner or group has write bits. It also checks the current entry.
    check_original(document, &directory.open_file(&name, libc::O_RDWR)?)?;
    check_parent(workspace, parent, &directory)?;
    before_exchange();
    exchange(&stage.dir, VERSION, &directory, &name)?;
    // From here, stage no longer holds our draft. Keep the displaced entry on every error.
    stage.keep = true;
    let verified = (|| {
        let mut displaced = stage.dir.open_file(VERSION, libc::O_RDONLY)?;
        let before = Stamp::read(&displaced.metadata()?)?;
        if !same_after_rename(&before, &document.stamp)
            || !same_bytes(&mut displaced, &document.source)?
            || !acl.matches(&displaced)?
            || before != Stamp::read(&displaced.metadata()?)?
        {
            return Err(external_change());
        }
        let stamp = check_published(&directory, &name, &draft_stamp, source)?;
        check_parent(workspace, parent, &directory)?;
        Ok(stamp)
    })();
    match verified {
        Ok(stamp) => {
            stage.keep = false;
            Ok(stamp)
        }
        Err(error) => Err(io::Error::other(format!(
            "could not verify the exchange ({error}); the draft may be at the document path; \
             the displaced entry was kept at {} (in the original destination folder if it moved)",
            parent
                .join(OsStr::from_bytes(stage.name.to_bytes()))
                .join("version")
                .display()
        ))),
    }
}

fn check_original(document: &DocumentFile, file: &File) -> io::Result<Metadata> {
    let metadata = file.metadata()?;
    // Keep the editor's explicit read-only mode policy even when run as root.
    if metadata.permissions().readonly() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the file is read-only",
        ));
    }
    if Stamp::read(&metadata)? != document.stamp {
        return Err(external_change());
    }
    Ok(metadata)
}

fn check_published(directory: &Dir, name: &CStr, draft: &Stamp, source: &str) -> io::Result<Stamp> {
    let mut published = directory.open_file(name, libc::O_RDONLY)?;
    let stamp = Stamp::read(&published.metadata()?)?;
    if !same_after_rename(&stamp, draft) || !same_bytes(&mut published, source)? {
        return Err(external_change());
    }
    // Reopen the named entry to confirm its stamp still matches after reading the bytes.
    let current = directory.open_file(name, libc::O_RDONLY)?;
    if stamp != Stamp::read(&current.metadata()?)? {
        return Err(external_change());
    }
    Ok(stamp)
}

fn same_bytes(file: &mut File, expected: &str) -> io::Result<bool> {
    let mut bytes = Vec::new();
    file.take(expected.len() as u64 + 1)
        .read_to_end(&mut bytes)?;
    Ok(bytes == expected.as_bytes())
}

fn same_after_rename(actual: &Stamp, expected: &Stamp) -> bool {
    // Exchange can change ctime even if no file data or permissions changed.
    actual.identity == expected.identity
        && actual.modified == expected.modified
        && actual.len == expected.len
        && actual.permissions == expected.permissions
        && actual.owner == expected.owner
}

pub(super) fn open_search_file(workspace: &Workspace, path: &Path) -> io::Result<File> {
    let (_, name, directory) = destination(workspace, path)?;
    directory.open_file(&name, libc::O_RDONLY)
}

fn destination<'a>(workspace: &Workspace, path: &'a Path) -> io::Result<(&'a Path, CString, Dir)> {
    let parent = path.parent().expect("an absolute document has a parent");
    let name = basename(path.file_name().expect("a document has a name"))?;
    Ok((parent, name, workspace_parent(workspace, parent)?))
}

fn workspace_parent(workspace: &Workspace, parent: &Path) -> io::Result<Dir> {
    let relative = parent
        .strip_prefix(&workspace.root)
        .map_err(io::Error::other)?;
    let root = Dir(File::open("/")?);
    let root = root.descend(workspace.root.strip_prefix("/").map_err(io::Error::other)?)?;
    if identity(&root.0.metadata()?) != workspace.identity {
        return Err(folder_moved());
    }
    root.descend(relative)
}

fn check_parent(workspace: &Workspace, parent: &Path, held: &Dir) -> io::Result<()> {
    // Re-walk the canonical path without following symlinks and compare directory identities.
    // A later rename of the held directory can still move it, but cannot redirect our writes.
    let current = workspace_parent(workspace, parent).map_err(|_| folder_moved())?;
    if identity(&current.0.metadata()?) != identity(&held.0.metadata()?) {
        return Err(folder_moved());
    }
    Ok(())
}

pub(super) fn folder_moved() -> io::Error {
    io::Error::other("the workspace or destination folder has moved")
}

pub(super) fn identity(metadata: &Metadata) -> (u64, u64) {
    (metadata.dev(), metadata.ino())
}

fn basename(name: &OsStr) -> io::Result<CString> {
    CString::new(name.as_bytes()).map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))
}

struct Dir(File);

impl Dir {
    fn fd(&self) -> libc::c_int {
        self.0.as_raw_fd()
    }

    fn descend(mut self, path: &Path) -> io::Result<Self> {
        for component in path.components() {
            let Component::Normal(component) = component else {
                return Err(folder_moved());
            };
            self = self.open_dir(&basename(component)?)?;
        }
        Ok(self)
    }

    fn open_dir(&self, name: &CStr) -> io::Result<Self> {
        #[cfg(target_os = "macos")]
        let access = libc::O_SEARCH;
        #[cfg(target_os = "linux")]
        let access = libc::O_PATH;
        self.open(name, access | libc::O_DIRECTORY, 0).map(Self)
    }

    fn open_file(&self, name: &CStr, flags: libc::c_int) -> io::Result<File> {
        // Nonblocking open avoids waiting for a FIFO if a regular file was replaced.
        let file = self.open(name, flags | libc::O_NONBLOCK, 0)?;
        if !file.metadata()?.is_file() {
            return Err(external_change());
        }
        Ok(file)
    }

    fn open(&self, name: &CStr, flags: libc::c_int, mode: libc::c_uint) -> io::Result<File> {
        // SAFETY: name is NUL-terminated, fd is live; mode has the C variadic integer width.
        let fd = unsafe {
            libc::openat(
                self.fd(),
                name.as_ptr(),
                flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                mode,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: A successful openat gives this function sole ownership of a new fd.
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    fn unlink(&self, name: &CStr, flags: libc::c_int) -> io::Result<()> {
        // SAFETY: fd is live, name is NUL-terminated and used only for this call.
        status(unsafe { libc::unlinkat(self.fd(), name.as_ptr(), flags) })
    }
}

/// A private directory makes the staged name and displaced entry independent of changes to
/// the document's parent path. Only the directory's own removal uses its name in the parent.
struct Stage<'a> {
    parent: &'a Dir,
    dir: Dir,
    name: CString,
    keep: bool,
}

impl<'a> Stage<'a> {
    fn new(parent: &'a Dir, display_path: &Path) -> io::Result<Self> {
        // Use tempfile only to generate unique basenames; it must not clean up by path.
        tempfile::Builder::new()
            .prefix(".gpui-save-")
            .rand_bytes(12)
            .disable_cleanup(true)
            .make_in(display_path, |path| {
                let name = basename(path.file_name().expect("tempfile generates basenames"))?;
                // SAFETY: parent fd is live and name is NUL-terminated.
                status(unsafe { libc::mkdirat(parent.fd(), name.as_ptr(), 0o700) })?;
                // Open for ACL access as well as relative operations.
                let dir = match parent.open(&name, libc::O_RDONLY | libc::O_DIRECTORY, 0) {
                    Ok(file) => Dir(file),
                    Err(error) => {
                        // Best effort: only remove an empty directory under the held parent.
                        let _ = parent.unlink(&name, libc::AT_REMOVEDIR);
                        return Err(error);
                    }
                };
                let meta = dir.0.metadata()?;
                // SAFETY: geteuid takes no arguments and has no side effects.
                if meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "staging folder is not private",
                    ));
                }
                let stage = Self {
                    parent,
                    dir,
                    name,
                    keep: false,
                };
                Acl::clear(&stage.dir.0)?;
                Ok(stage)
            })
            .map(tempfile::NamedTempFile::into_file)
    }

    fn new_file(&self) -> io::Result<File> {
        self.dir
            .open(VERSION, libc::O_RDWR | libc::O_CREAT | libc::O_EXCL, 0o600)
    }
}

impl Drop for Stage<'_> {
    fn drop(&mut self) {
        if self.keep {
            return;
        }
        let _ = self.dir.unlink(VERSION, 0);
        let same_dir = self.parent.open_dir(&self.name).and_then(|current| {
            Ok(identity(&current.0.metadata()?) == identity(&self.dir.0.metadata()?))
        });
        if matches!(same_dir, Ok(true)) {
            let _ = self.parent.unlink(&self.name, libc::AT_REMOVEDIR);
        }
    }
}

fn exchange(from: &Dir, from_name: &CStr, to: &Dir, to_name: &CStr) -> io::Result<()> {
    // SAFETY: Both directory descriptors and basename pointers are valid for the call.
    let result = unsafe {
        #[cfg(target_os = "macos")]
        {
            libc::renameatx_np(
                from.fd(),
                from_name.as_ptr(),
                to.fd(),
                to_name.as_ptr(),
                libc::RENAME_SWAP,
            )
        }
        #[cfg(target_os = "linux")]
        {
            libc::renameat2(
                from.fd(),
                from_name.as_ptr(),
                to.fd(),
                to_name.as_ptr(),
                libc::RENAME_EXCHANGE,
            )
        }
    };
    // No overwrite-only fallback: an unsupported exchange must leave the original alone.
    status(result)
}

fn status(result: libc::c_int) -> io::Result<()> {
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn late_external_saves_keep_the_displaced_version() {
        for replace in [false, true] {
            let folder = tempfile::tempdir().unwrap();
            let workspace = Workspace::open(folder.path()).unwrap();
            let path = workspace.root().join("document.md");
            fs::write(&path, "original").unwrap();
            let document = workspace.read_document(&path).unwrap();

            let error = save(&document, "editor draft", || {
                if replace {
                    let competing_path = workspace.root().join("external.tmp");
                    fs::write(&competing_path, "competing version").unwrap();
                    fs::rename(competing_path, &path).unwrap();
                } else {
                    fs::write(&path, "competing version").unwrap();
                }
            })
            .unwrap_err();

            let recovery = fs::read_dir(workspace.root())
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .find(|path| {
                    path.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with(".gpui-save-")
                })
                .unwrap()
                .join("version");
            assert!(
                error.to_string().contains(recovery.to_str().unwrap()),
                "{error}"
            );
            assert_eq!(fs::read_to_string(recovery).unwrap(), "competing version");
            assert_eq!(fs::read_to_string(&path).unwrap(), "editor draft");
            assert_eq!(document.source(), "original");
        }
    }

    #[test]
    fn a_held_parent_must_still_match_the_workspace_path() {
        let folder = tempfile::tempdir().unwrap();
        let workspace = Workspace::open(folder.path()).unwrap();
        let parent = workspace.root().join("nested");
        fs::create_dir(&parent).unwrap();
        let held = workspace_parent(&workspace, &parent).unwrap();
        check_parent(&workspace, &parent, &held).unwrap();

        fs::rename(&parent, workspace.root().join("moved")).unwrap();
        fs::create_dir(&parent).unwrap();
        assert!(check_parent(&workspace, &parent, &held).is_err());
    }
}
