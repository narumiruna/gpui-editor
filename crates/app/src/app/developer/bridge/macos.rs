use std::{
    ffi::OsStr,
    fs::{self, DirBuilder, Permissions},
    io,
    os::{
        fd::AsRawFd,
        unix::{
            ffi::OsStrExt,
            fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
            net::UnixListener,
        },
    },
    path::{Path, PathBuf},
};

use super::invalid;

pub(super) use std::os::unix::net::UnixStream as Stream;

pub(super) struct Listener {
    socket: UnixListener,
    _instance: tempfile::TempDir,
}

impl Listener {
    pub(super) fn start() -> io::Result<Self> {
        Self::listen(&discovery_base(&darwin_temp_dir()?)?)
    }

    pub(super) fn listen(base: &Path) -> io::Result<Self> {
        let instance = tempfile::Builder::new()
            .prefix("i-")
            .permissions(Permissions::from_mode(0o700))
            .tempdir_in(base)?;
        check_private(instance.path(), false)?;
        let path = instance.path().join("s");
        // Darwin's 104-byte sun_path must also fit the terminating NUL.
        if path.as_os_str().as_bytes().len() >= 104 {
            return Err(invalid("Developer socket path is too long"));
        }
        let listener = UnixListener::bind(&path)?;
        fs::set_permissions(&path, Permissions::from_mode(0o600))?;
        check_private(&path, true)?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            socket: listener,
            _instance: instance,
        })
    }

    pub(super) fn accept(&mut self) -> io::Result<Stream> {
        let (socket, _) = self.socket.accept()?;
        if !same_user(&socket) || socket.set_nonblocking(true).is_err() {
            return Err(io::ErrorKind::Interrupted.into());
        }
        Ok(socket)
    }
}

fn darwin_temp_dir() -> io::Result<PathBuf> {
    let mut buffer = [0_u8; libc::PATH_MAX as usize];
    // SAFETY: confstr writes at most buffer.len() bytes to this writable buffer.
    let size = unsafe {
        libc::confstr(
            libc::_CS_DARWIN_USER_TEMP_DIR,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
        )
    };
    if !(2..=buffer.len()).contains(&size) || buffer[size - 1] != 0 {
        return Err(invalid("Cannot locate the Darwin user temporary directory"));
    }
    let path = Path::new(OsStr::from_bytes(&buffer[..size - 1]));
    if !path.is_absolute() {
        return Err(invalid("Darwin user temporary directory is not absolute"));
    }
    // Strip the trailing slash before checking that the final component is not
    // a symlink. Canonicalize system aliases such as /var for discovery.
    let path: PathBuf = path.components().collect();
    check_private(&path, false)?;
    path.canonicalize()
}

fn discovery_base(temp: &Path) -> io::Result<PathBuf> {
    check_private(temp, false)?;
    let base = temp.join("gpui-pi");
    match DirBuilder::new().mode(0o700).create(&base) {
        Ok(()) => (),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => (),
        Err(err) => return Err(err),
    }
    check_private(&base, false)?;
    Ok(base)
}

fn check_private(path: &Path, socket: bool) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    let (correct_type, mode) = if socket {
        (meta.file_type().is_socket(), 0o600)
    } else {
        (meta.is_dir(), 0o700)
    };
    if !correct_type || meta.uid() != effective_uid() || meta.mode() & 0o7777 != mode {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Unsafe Developer Mode discovery directory or socket",
        ));
    }
    Ok(())
}

fn effective_uid() -> libc::uid_t {
    // SAFETY: geteuid takes no arguments and has no memory preconditions.
    unsafe { libc::geteuid() }
}

fn same_user(socket: &Stream) -> bool {
    let (mut uid, mut gid) = (0, 0);
    // SAFETY: the descriptor stays open for this call, and both outputs are writable.
    let result = unsafe { libc::getpeereid(socket.as_raw_fd(), &mut uid, &mut gid) };
    result == 0 && uid == effective_uid()
}

#[cfg(test)]
pub(super) fn test_directory() -> io::Result<tempfile::TempDir> {
    tempfile::Builder::new()
        .permissions(Permissions::from_mode(0o700))
        .tempdir_in(darwin_temp_dir()?)
}

#[cfg(test)]
pub(super) fn test_client(base: &Path) -> io::Result<Stream> {
    let path = fs::read_dir(base)?
        .next()
        .ok_or(io::ErrorKind::NotFound)??
        .path()
        .join("s");
    let stream = Stream::connect(path)?;
    stream.set_nonblocking(true)?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn discovery_rejects_symlinks_and_bad_modes_and_removes_only_its_instance() {
        let temp = test_directory().unwrap();
        let base = discovery_base(temp.path()).unwrap();
        let listener = Listener::listen(&base).unwrap();
        let instance = listener._instance.path().to_path_buf();
        check_private(&instance, false).unwrap();
        check_private(&instance.join("s"), true).unwrap();
        let sibling = tempfile::tempdir_in(&base).unwrap();
        let other = test_directory().unwrap();
        let link = other.path().join("gpui-pi");
        symlink(&base, &link).unwrap();
        assert_eq!(
            discovery_base(other.path()).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        fs::remove_file(&link).unwrap();
        fs::create_dir(&link).unwrap();
        fs::set_permissions(&link, Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            discovery_base(other.path()).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        drop(listener);
        assert!(!instance.exists());
        assert!(sibling.path().is_dir());
    }
}
