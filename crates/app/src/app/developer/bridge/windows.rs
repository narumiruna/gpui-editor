//! Local named pipes and per-user discovery on Windows.

use std::{
    env,
    ffi::c_void,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    mem::MaybeUninit,
    os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Path, PathBuf},
    ptr,
};

use windows_sys::Win32::{
    Foundation::{
        ERROR_BROKEN_PIPE, ERROR_NO_DATA, ERROR_PIPE_CONNECTED, ERROR_PIPE_LISTENING,
        INVALID_HANDLE_VALUE, LocalFree,
    },
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SDDL_REVISION_1,
        },
        GetTokenInformation, SECURITY_ATTRIBUTES, SECURITY_MAX_SID_SIZE, TOKEN_QUERY, TOKEN_USER,
        TokenUser,
    },
    Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_DELETE_ON_CLOSE, FILE_FLAG_FIRST_PIPE_INSTANCE,
        PIPE_ACCESS_DUPLEX, ReadFile,
    },
    System::{
        Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, PIPE_NOWAIT, PIPE_READMODE_BYTE,
            PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
        },
        Threading::{GetCurrentProcess, OpenProcessToken},
    },
};

use super::invalid;

pub(super) struct Listener {
    _marker: File,
    waiting: File,
    name: Vec<u16>,
    security: Vec<u16>,
}

impl Listener {
    pub(super) fn start() -> io::Result<Self> {
        let local = env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or_else(|| invalid("Cannot locate Local AppData"))?;
        Self::listen(&discovery_base(&local)?)
    }

    pub(super) fn listen(base: &Path) -> io::Result<Self> {
        let sid = user_sid()?;
        let security: Vec<u16> = format!("D:P(A;;GA;;;{sid})\0").encode_utf16().collect();
        let (marker, waiting, name) = tempfile::Builder::new()
            .prefix("i-")
            .rand_bytes(24)
            .disable_cleanup(true)
            .make_in(base, |path| {
                let name: Vec<u16> = pipe_path(path)?.encode_utf16().chain([0]).collect();
                let waiting = create_pipe(&name, &security, FILE_FLAG_FIRST_PIPE_INSTANCE)?;
                let marker = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .custom_flags(FILE_FLAG_DELETE_ON_CLOSE)
                    .open(path)?;
                Ok((marker, waiting, name))
            })?
            .into_file();
        Ok(Self {
            _marker: marker,
            waiting,
            name,
            security,
        })
    }

    pub(super) fn accept(&mut self) -> io::Result<Stream> {
        // SAFETY: The handle is a synchronous nonblocking server pipe and stays open.
        let connected = unsafe { ConnectNamedPipe(self.waiting.as_raw_handle(), ptr::null_mut()) };
        let closed = if connected == 0 {
            let error = io::Error::last_os_error();
            match error.raw_os_error().map(|code| code as u32) {
                Some(ERROR_PIPE_CONNECTED) => false,
                Some(ERROR_NO_DATA) => true,
                Some(ERROR_PIPE_LISTENING) => return Err(io::ErrorKind::WouldBlock.into()),
                _ => return Err(error),
            }
        } else {
            return Err(io::ErrorKind::WouldBlock.into());
        };
        // Keep at least one handle open to reserve the name from discovery to shutdown.
        let next = create_pipe(&self.name, &self.security, 0)?;
        let previous = std::mem::replace(&mut self.waiting, next);
        if closed {
            return Err(io::ErrorKind::Interrupted.into());
        }
        Ok(Stream(previous))
    }
}

fn discovery_base(local: &Path) -> io::Result<PathBuf> {
    check_directory(local)?;
    let base = local.join("gpui-pi");
    match fs::create_dir(&base) {
        Ok(()) => (),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error),
    }
    check_directory(&base)?;
    Ok(base)
}

fn check_directory(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Unsafe Developer Mode discovery directory",
        ));
    }
    Ok(())
}

fn pipe_path(marker: &Path) -> io::Result<String> {
    let name = marker
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| invalid("Invalid Developer Mode instance name"))?;
    Ok(format!(r"\\.\pipe\gpui-editor-pi-{name}"))
}

fn create_pipe(name: &[u16], sddl: &[u16], flags: u32) -> io::Result<File> {
    let mut descriptor = ptr::null_mut();
    // SAFETY: The SDDL string is NUL-terminated and the output pointer is writable.
    status(unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        )
    })?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    // SAFETY: The NUL-terminated name and security descriptor live through the call.
    let handle = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX | flags,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
            3,
            64 * 1024,
            64 * 1024,
            0,
            &attributes,
        )
    };
    let result = if handle == INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: The successful call returned an owned handle.
        Ok(unsafe { File::from_raw_handle(handle) })
    };
    // SAFETY: The conversion allocated this descriptor with LocalAlloc.
    unsafe { LocalFree(descriptor) };
    result
}

fn user_sid() -> io::Result<String> {
    let mut token = ptr::null_mut();
    // SAFETY: The process pseudo-handle is valid and the output pointer is writable.
    status(unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) })?;
    // SAFETY: The successful call returned an owned token handle.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    #[repr(C)]
    struct User {
        header: TOKEN_USER,
        sid: [u8; SECURITY_MAX_SID_SIZE as usize],
    }
    let mut info = MaybeUninit::<User>::uninit();
    let mut used = 0;
    // SAFETY: The aligned buffer has room for the token user header and any SID.
    status(unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            info.as_mut_ptr().cast::<c_void>(),
            size_of::<User>() as u32,
            &mut used,
        )
    })?;
    let mut sid = ptr::null_mut();
    // SAFETY: The call initialized the token user header and its SID in info.
    status(unsafe { ConvertSidToStringSidW((*info.as_ptr()).header.User.Sid, &mut sid) })?;
    // SAFETY: Conversion returned a NUL-terminated wide string owned by this function.
    let sid_text = unsafe {
        let length = (0..).find(|&index| *sid.add(index) == 0).unwrap();
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(sid, length));
        LocalFree(sid.cast());
        text
    };
    Ok(sid_text)
}

fn status(result: i32) -> io::Result<()> {
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub(super) struct Stream(File);

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut count = 0;
        // SAFETY: This is a synchronous handle and both output buffers are valid.
        let result = unsafe {
            ReadFile(
                self.0.as_raw_handle(),
                buf.as_mut_ptr(),
                buf.len().min(u32::MAX as usize) as u32,
                &mut count,
                ptr::null_mut(),
            )
        };
        if result != 0 {
            return Ok(count as usize);
        }
        // File::read reports both errors as EOF. Only a broken pipe is EOF here.
        let error = io::Error::last_os_error();
        match error.raw_os_error().map(|code| code as u32) {
            Some(ERROR_NO_DATA) => Err(io::ErrorKind::WouldBlock.into()),
            Some(ERROR_BROKEN_PIPE) => Ok(0),
            _ => Err(error),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.0.write(buf) {
            Ok(0) if !buf.is_empty() => Err(io::ErrorKind::WouldBlock.into()),
            result => result,
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
pub(super) fn test_directory() -> io::Result<tempfile::TempDir> {
    tempfile::tempdir()
}

#[cfg(test)]
pub(super) fn test_client(base: &Path) -> io::Result<Stream> {
    use windows_sys::Win32::{
        Foundation::ERROR_PIPE_BUSY,
        Storage::FileSystem::{SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT},
        System::Pipes::SetNamedPipeHandleState,
    };
    let path = fs::read_dir(base)?
        .next()
        .ok_or(io::ErrorKind::NotFound)??
        .path();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .security_qos_flags(SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION)
        .open(pipe_path(&path)?);
    let file = match file {
        Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        result => result?,
    };
    let mode = PIPE_NOWAIT | PIPE_READMODE_BYTE;
    // SAFETY: The pipe handle and mode are valid. The optional arguments are absent.
    status(unsafe {
        SetNamedPipeHandleState(file.as_raw_handle(), &mode, ptr::null(), ptr::null())
    })?;
    Ok(Stream(file))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_discovery_marker_when_listener_closes() {
        let temp = test_directory().unwrap();
        let base = discovery_base(temp.path()).unwrap();
        let listener = Listener::listen(&base).unwrap();
        let marker = fs::read_dir(&base).unwrap().next().unwrap().unwrap();
        assert!(marker.file_type().unwrap().is_file());
        assert_eq!(marker.file_name().len(), 26);
        drop(listener);
        assert!(fs::read_dir(base).unwrap().next().is_none());
    }
}
