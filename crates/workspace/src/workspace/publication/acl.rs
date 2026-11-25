//! Read and apply access ACLs without resolving the document path again.

#[cfg(target_os = "macos")]
mod platform {
    use std::{ffi::c_void, fs::File, io, os::fd::AsRawFd, ptr::NonNull};

    use super::super::status;

    // Darwin's <sys/acl.h> defines acl_t as an opaque pointer. libc does not expose it.
    unsafe extern "C" {
        fn acl_get_fd(fd: libc::c_int) -> *mut c_void;
        fn acl_set_fd(fd: libc::c_int, acl: *mut c_void) -> libc::c_int;
        fn acl_init(count: libc::c_int) -> *mut c_void;
        fn acl_free(acl: *mut c_void) -> libc::c_int;
        fn acl_size(acl: *mut c_void) -> libc::ssize_t;
        fn acl_copy_ext(buf: *mut c_void, acl: *mut c_void, len: libc::ssize_t) -> libc::ssize_t;
    }

    struct Handle(NonNull<c_void>);

    impl Handle {
        fn bytes(&self) -> io::Result<Vec<u8>> {
            // SAFETY: The ACL is live; the buffer passed to acl_copy_ext is its reported size.
            let size = unsafe { acl_size(self.0.as_ptr()) };
            if size < 0 {
                return Err(io::Error::last_os_error());
            }
            let mut bytes = vec![0; size as usize];
            if unsafe { acl_copy_ext(bytes.as_mut_ptr().cast(), self.0.as_ptr(), size) } < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(bytes)
        }

        fn apply(&self, file: &File) -> io::Result<()> {
            // SAFETY: The ACL and target descriptor are live through the call.
            status(unsafe { acl_set_fd(file.as_raw_fd(), self.0.as_ptr()) })
        }
    }

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: This handle owns the allocation from acl_get_fd or acl_init.
            unsafe { acl_free(self.0.as_ptr()) };
        }
    }

    pub(in crate::workspace::publication) struct Acl(Option<Handle>);

    impl Acl {
        pub(in crate::workspace::publication) fn read(file: &File) -> io::Result<Self> {
            // SAFETY: The descriptor is live; a non-null result is a newly allocated ACL.
            if let Some(acl) = NonNull::new(unsafe { acl_get_fd(file.as_raw_fd()) }) {
                return Ok(Self(Some(Handle(acl))));
            }
            let error = io::Error::last_os_error();
            match error.raw_os_error() {
                Some(libc::ENOENT | libc::ENOTSUP) => Ok(Self(None)),
                _ => Err(error),
            }
        }

        pub(in crate::workspace::publication) fn clear(file: &File) -> io::Result<()> {
            if Self::read(file)?.0.is_some() {
                // SAFETY: acl_init(0) allocates an empty ACL; setting it removes entries.
                let empty =
                    NonNull::new(unsafe { acl_init(0) }).ok_or_else(io::Error::last_os_error)?;
                Handle(empty).apply(file)?;
            }
            Ok(())
        }

        pub(in crate::workspace::publication) fn apply(&self, file: &File) -> io::Result<()> {
            match &self.0 {
                Some(acl) => acl.apply(file),
                None => Self::clear(file),
            }
        }

        pub(in crate::workspace::publication) fn matches(&self, file: &File) -> io::Result<bool> {
            let other = Self::read(file)?;
            match (&self.0, other.0) {
                (None, None) => Ok(true),
                (Some(left), Some(right)) => Ok(left.bytes()? == right.bytes()?),
                _ => Ok(false),
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::{ffi::CStr, fs::File, io, os::fd::AsRawFd, ptr};

    use super::super::status;

    const ACCESS: &CStr = c"system.posix_acl_access";

    pub(in crate::workspace::publication) struct Acl(Option<Vec<u8>>);

    impl Acl {
        pub(in crate::workspace::publication) fn read(file: &File) -> io::Result<Self> {
            let fd = file.as_raw_fd();
            // SAFETY: A null buffer with size 0 retrieves the required allocation size.
            let size = unsafe { libc::fgetxattr(fd, ACCESS.as_ptr(), ptr::null_mut(), 0) };
            if size < 0 {
                let error = io::Error::last_os_error();
                return match error.raw_os_error() {
                    Some(libc::ENODATA | libc::ENOTSUP) => Ok(Self(None)),
                    _ => Err(error),
                };
            }
            let mut bytes = vec![0; size as usize];
            // SAFETY: The buffer is writable for its full length, and the descriptor is live.
            let size = unsafe {
                libc::fgetxattr(fd, ACCESS.as_ptr(), bytes.as_mut_ptr().cast(), bytes.len())
            };
            if size < 0 {
                return Err(io::Error::last_os_error());
            }
            bytes.truncate(size as usize);
            Ok(Self(Some(bytes)))
        }

        pub(in crate::workspace::publication) fn clear(file: &File) -> io::Result<()> {
            // SAFETY: The descriptor and attribute name are valid for the call.
            let result = status(unsafe { libc::fremovexattr(file.as_raw_fd(), ACCESS.as_ptr()) });
            match result {
                Err(error)
                    if matches!(error.raw_os_error(), Some(libc::ENODATA | libc::ENOTSUP)) =>
                {
                    Ok(())
                }
                other => other,
            }
        }

        pub(in crate::workspace::publication) fn apply(&self, file: &File) -> io::Result<()> {
            let Some(bytes) = &self.0 else {
                return Self::clear(file);
            };
            // SAFETY: The descriptor, name, and read-only ACL buffer are valid for the call.
            status(unsafe {
                libc::fsetxattr(
                    file.as_raw_fd(),
                    ACCESS.as_ptr(),
                    bytes.as_ptr().cast(),
                    bytes.len(),
                    0,
                )
            })
        }

        pub(in crate::workspace::publication) fn matches(&self, file: &File) -> io::Result<bool> {
            Ok(self.0 == Self::read(file)?.0)
        }
    }
}

pub(super) use platform::Acl;
