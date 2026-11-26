#![cfg(windows)]

use std::{
    fs::{self, File, FileTimes, OpenOptions},
    io,
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle},
    path::Path,
    ptr,
};

use gpui_editor_workspace::Workspace;
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::{
        Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1},
        DACL_SECURITY_INFORMATION, GetFileSecurityW, GetSecurityDescriptorControl,
        GetSecurityDescriptorDacl, PROTECTED_DACL_SECURITY_INFORMATION, SE_DACL_PROTECTED,
        SetFileSecurityW,
    },
    Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, GetFileInformationByHandle,
    },
};

#[test]
fn root_identity_is_the_windows_volume_and_file_index() {
    let folder = tempfile::tempdir().unwrap();
    let workspace = Workspace::open(folder.path()).unwrap();
    let root = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(workspace.root())
        .unwrap();
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: The root handle is live and info is a correctly sized output buffer.
    assert_ne!(
        unsafe { GetFileInformationByHandle(root.as_raw_handle(), &mut info) },
        0
    );
    assert_eq!(
        workspace.root_identity(),
        (
            u64::from(info.dwVolumeSerialNumber),
            (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        )
    );
}

#[test]
fn replacing_a_file_with_matching_bytes_and_time_is_still_a_stale_save() {
    let folder = tempfile::tempdir().unwrap();
    let workspace = Workspace::open(folder.path()).unwrap();
    let path = workspace.root().join("document.md");
    fs::write(&path, "original").unwrap();
    let mut document = workspace.read_document(&path).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let replacement = workspace.root().join("replacement.tmp");
    fs::write(&replacement, "original").unwrap();
    File::options()
        .write(true)
        .open(&replacement)
        .unwrap()
        .set_times(FileTimes::new().set_modified(modified))
        .unwrap();
    fs::rename(replacement, &path).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);

    assert!(document.save("draft").is_err());
    assert_eq!(document.source(), "original");
    assert_eq!(fs::read_to_string(&path).unwrap(), "original");
    assert_eq!(fs::read_dir(workspace.root()).unwrap().count(), 1);
}

#[test]
fn read_only_file_is_not_replaced() {
    let folder = tempfile::tempdir().unwrap();
    let workspace = Workspace::open(folder.path()).unwrap();
    let path = workspace.root().join("read-only.md");
    fs::write(&path, "original").unwrap();
    let permissions = fs::metadata(&path).unwrap().permissions();
    let mut read_only = permissions.clone();
    read_only.set_readonly(true);
    fs::set_permissions(&path, read_only).unwrap();
    let mut document = workspace.read_document(&path).unwrap();
    let result = document.save("draft");
    fs::set_permissions(&path, permissions).unwrap();

    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(document.source(), "original");
    assert_eq!(fs::read_to_string(&path).unwrap(), "original");
    assert_eq!(fs::read_dir(workspace.root()).unwrap().count(), 1);
}

#[test]
fn save_preserves_an_explicit_protected_dacl() {
    let folder = tempfile::tempdir().unwrap();
    let workspace = Workspace::open(folder.path()).unwrap();
    let path = workspace.root().join("protected.md");
    fs::write(&path, "original").unwrap();
    let wide_path: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // Deny execute for Everyone and give full access only to the owner. This needs
    // no privileged account and differs from the staging file's inherited DACL.
    let descriptor: Vec<u16> = "D:P(D;;0x20;;;WD)(A;;FA;;;OW)\0".encode_utf16().collect();
    let mut security = ptr::null_mut();
    // SAFETY: Both input and output pointers remain valid for the conversion.
    assert_ne!(
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                descriptor.as_ptr(),
                SDDL_REVISION_1,
                &mut security,
                ptr::null_mut(),
            )
        },
        0
    );
    // SAFETY: The path is NUL-terminated and security is a live security descriptor.
    let result = unsafe {
        SetFileSecurityW(
            wide_path.as_ptr(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            security,
        )
    };
    // SAFETY: The conversion allocated security with LocalAlloc.
    unsafe { LocalFree(security) };
    assert_ne!(result, 0, "{}", io::Error::last_os_error());
    let before = dacl(&path);

    let mut document = workspace.read_document(&path).unwrap();
    document.save("saved").unwrap();
    assert_eq!(dacl(&path), before);
    assert_eq!(fs::read_to_string(&path).unwrap(), "saved");
    assert_eq!(document.source(), "saved");
}

fn dacl(path: &Path) -> (bool, Vec<u8>) {
    let path: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut needed = 0;
    // SAFETY: This call only determines the required buffer size.
    unsafe {
        GetFileSecurityW(
            path.as_ptr(),
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            0,
            &mut needed,
        )
    };
    assert!(needed > 0, "{}", io::Error::last_os_error());
    let mut bytes = vec![0; needed as usize];
    // SAFETY: bytes has the size returned by the first call and is writable.
    assert_ne!(
        unsafe {
            GetFileSecurityW(
                path.as_ptr(),
                DACL_SECURITY_INFORMATION,
                bytes.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        },
        0
    );
    let mut present = 0;
    let mut acl = ptr::null_mut();
    let mut defaulted = 0;
    let mut control = 0;
    let mut revision = 0;
    // SAFETY: bytes contains a valid self-relative descriptor. The returned ACL
    // remains in that buffer until it is copied below.
    unsafe {
        assert_ne!(
            GetSecurityDescriptorDacl(
                bytes.as_mut_ptr().cast(),
                &mut present,
                &mut acl,
                &mut defaulted
            ),
            0
        );
        assert_ne!(
            GetSecurityDescriptorControl(bytes.as_mut_ptr().cast(), &mut control, &mut revision),
            0
        );
        assert_ne!(present, 0);
        assert!(!acl.is_null());
        // Compare the actual ACL and protection state. ReplaceFileW can update the
        // descriptor's automatic-inheritance bookkeeping without changing the ACL.
        (
            control & SE_DACL_PROTECTED != 0,
            std::slice::from_raw_parts(acl.cast::<u8>(), (*acl).AclSize as usize).to_vec(),
        )
    }
}
