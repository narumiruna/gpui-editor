use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

#[cfg(windows)]
use std::os::windows::fs::{MetadataExt as _, OpenOptionsExt as _};
#[cfg(unix)]
use std::{
    fs::Permissions,
    os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _},
};

#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
#[cfg(windows)]
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

pub(super) fn path(name: &str) -> Option<PathBuf> {
    #[cfg(unix)]
    let directory = std::env::home_dir()
        .filter(|home| home.is_absolute())
        .map(|home| home.join("Library/Application Support/GPUI Editor"));
    #[cfg(windows)]
    let directory = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|directory| directory.is_absolute())
        .map(|directory| directory.join("GPUI Editor"));
    directory.map(|directory| directory.join(name))
}

pub(super) fn open(path: &Path) -> io::Result<Option<File>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    #[cfg(windows)]
    options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS);
    let file = options.open(path);
    let file = match file {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let metadata = file.metadata()?;
    #[cfg(windows)]
    reject_reparse_point(&metadata)?;
    if !metadata.is_file() {
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
    #[cfg(unix)]
    let created = fs::DirBuilder::new().mode(0o700).create(directory);
    #[cfg(windows)]
    let created = fs::create_dir(directory);
    match created {
        Ok(()) => (),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error),
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW);
    #[cfg(windows)]
    // Keep the directory from being renamed or removed while the draft is published.
    options
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .share_mode(0x1 | 0x2); // FILE_SHARE_READ | FILE_SHARE_WRITE, without FILE_SHARE_DELETE.
    let directory_file = options.open(directory)?;
    #[cfg(unix)]
    if directory_file.metadata()?.permissions().mode() & 0o7777 != 0o700 {
        directory_file.set_permissions(Permissions::from_mode(0o700))?;
    }
    #[cfg(windows)]
    {
        let metadata = directory_file.metadata()?;
        reject_reparse_point(&metadata)?;
        if !metadata.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not a state directory",
            ));
        }
        drop(open(path)?);
    }

    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    #[cfg(unix)]
    temporary
        .as_file()
        .set_permissions(Permissions::from_mode(0o600))?;
    write(temporary.as_file_mut())?;
    temporary.as_file().sync_all()?;
    #[cfg(windows)]
    // Reject a destination that changed to a reparse point or non-regular file during writing.
    drop(open(path)?);
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(windows)]
fn reject_reparse_point(metadata: &fs::Metadata) -> io::Result<()> {
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "state must not be a reparse point",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    #[test]
    fn rejects_non_regular_state() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.json");
        fs::create_dir(&path).unwrap();
        assert!(open(&path).is_err());
        assert!(write_atomically(&path, |file| file.write_all(b"state")).is_err());
        assert!(path.is_dir());
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn uses_local_app_data_for_state() {
        let expected = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .filter(|directory| directory.is_absolute())
            .map(|directory| directory.join("GPUI Editor/session.json"));
        assert_eq!(path("session.json"), expected);
    }

    #[cfg(windows)]
    #[test]
    fn rejects_junctions_without_requiring_the_symlink_privilege() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        fs::create_dir(&target).unwrap();
        let junction = temp.path().join("state.json");
        let output = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&junction)
            .arg(&target)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(
            open(&junction).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(write_atomically(&junction, |file| file.write_all(b"new")).is_err());
        assert!(
            write_atomically(&junction.join("state.json"), |file| file.write_all(b"new")).is_err()
        );
        assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
    }

    #[cfg(windows)]
    #[test]
    fn rejects_reparse_state_files_and_directories() {
        use std::os::windows::fs::{symlink_dir, symlink_file};

        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target.json");
        fs::write(&target, b"original").unwrap();
        let link = temp.path().join("state.json");
        match symlink_file(&target, &link) {
            Ok(()) => (),
            // Creating symbolic links requires Developer Mode or the symlink privilege.
            Err(error) if error.raw_os_error() == Some(1314) => {
                eprintln!("symlink checks skipped: the symlink privilege is not available");
                return;
            }
            Err(error) => panic!("cannot create state symlink: {error}"),
        }
        assert_eq!(open(&link).unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert!(write_atomically(&link, |file| file.write_all(b"new")).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"original");
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );

        let changed = temp.path().join("changed.json");
        let result = write_atomically(&changed, |file| {
            file.write_all(b"new")?;
            symlink_file(&target, &changed)
        });
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read(&target).unwrap(), b"original");
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 3);

        fs::remove_file(&target).unwrap();
        assert!(open(&link).is_err());
        assert!(write_atomically(&link, |file| file.write_all(b"new")).is_err());
        assert!(!target.exists());

        let directory = temp.path().join("target");
        fs::create_dir(&directory).unwrap();
        let link = temp.path().join("linked-directory");
        symlink_dir(&directory, &link).unwrap();
        assert!(write_atomically(&link.join("state.json"), |file| file.write_all(b"new")).is_err());
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);
    }
}
