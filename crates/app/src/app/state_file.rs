use std::{
    fs::{self, File, OpenOptions, Permissions},
    io,
    os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _},
    path::{Path, PathBuf},
};

pub(super) fn path(name: &str) -> Option<PathBuf> {
    std::env::home_dir()
        .filter(|home| home.is_absolute())
        .map(|home| {
            home.join("Library/Application Support/GPUI Editor")
                .join(name)
        })
}

pub(super) fn open(path: &Path) -> io::Result<Option<File>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path);
    let file = match file {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a regular file",
        ));
    }
    Ok(Some(file))
}

pub(super) fn write_atomically(
    path: &Path,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    let directory = path
        .parent()
        .filter(|directory| !directory.as_os_str().is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing state directory"))?;
    if let Some(parent) = directory
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    match fs::DirBuilder::new().mode(0o700).create(directory) {
        Ok(()) => (),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error),
    }
    let directory_file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(directory)?;
    if directory_file.metadata()?.permissions().mode() & 0o7777 != 0o700 {
        directory_file.set_permissions(Permissions::from_mode(0o700))?;
    }

    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary
        .as_file()
        .set_permissions(Permissions::from_mode(0o600))?;
    write(temporary.as_file_mut())?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}
