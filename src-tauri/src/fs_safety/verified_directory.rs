use super::{platform_support, PathGuardError};
use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryIdentity {
    pub volume_id: String,
    pub file_id: String,
}

pub struct VerifiedDirectory {
    path: PathBuf,
    identity: DirectoryIdentity,
    handle: File,
}

impl std::fmt::Debug for VerifiedDirectory {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VerifiedDirectory")
            .field("path", &self.path)
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

impl VerifiedDirectory {
    pub fn open_existing(path: &Path) -> Result<Self, PathGuardError> {
        platform_support::ensure_supported_file_mutation()
            .map_err(super::path_guard::map_platform_error)?;
        if !path.is_absolute() {
            return Err(PathGuardError::UnsafePath);
        }

        #[cfg(windows)]
        {
            open_windows(path)
        }

        #[cfg(target_os = "macos")]
        {
            open_macos(path)
        }

        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = path;
            Err(PathGuardError::UnsupportedPlatformLinux)
        }
    }

    pub fn open_or_create(path: &Path) -> Result<Self, PathGuardError> {
        platform_support::ensure_supported_file_mutation()
            .map_err(super::path_guard::map_platform_error)?;

        #[cfg(windows)]
        {
            open_windows_relative(path, true)
        }

        #[cfg(not(windows))]
        {
            super::path_guard::create_directory_chain_no_links(path)?;
            Self::open_existing(path)
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn identity(&self) -> &DirectoryIdentity {
        &self.identity
    }

    #[cfg(windows)]
    pub(crate) fn handle(&self) -> &File {
        &self.handle
    }

    #[cfg(target_os = "macos")]
    pub(crate) fn raw_fd(&self) -> std::os::fd::RawFd {
        use std::os::fd::AsRawFd;

        self.handle.as_raw_fd()
    }

    pub fn sync(&self) -> io::Result<()> {
        self.handle.sync_all()
    }

    /// Creates one child directory relative to this verified directory handle.
    /// It fails when the name already exists and returns the identity captured
    /// from the newly created directory handle.
    pub(crate) fn create_child_directory(
        &self,
        name: &std::ffi::OsStr,
    ) -> Result<VerifiedDirectory, PathGuardError> {
        if name.is_empty()
            || Path::new(name).components().count() != 1
            || Path::new(name)
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
        {
            return Err(PathGuardError::UnsafePath);
        }
        #[cfg(windows)]
        {
            create_windows_child_directory(self, name)
        }
        #[cfg(target_os = "macos")]
        {
            create_macos_child_directory(self, name)
        }
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = (self, name);
            Err(PathGuardError::UnsupportedPlatformLinux)
        }
    }

    pub fn ensure_unchanged(&self) -> Result<(), PathGuardError> {
        let current = Self::open_existing(&self.path)?;
        if current.identity != self.identity {
            return Err(PathGuardError::IdentityChanged);
        }
        Ok(())
    }
}

#[cfg(windows)]
fn create_windows_child_directory(
    parent: &VerifiedDirectory,
    name: &std::ffi::OsStr,
) -> Result<VerifiedDirectory, PathGuardError> {
    use std::{
        mem,
        os::windows::ffi::OsStrExt,
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    };
    use windows_sys::{
        Wdk::{
            Foundation::OBJECT_ATTRIBUTES,
            Storage::FileSystem::{
                NtCreateFile, FILE_CREATE, FILE_DIRECTORY_FILE, FILE_OPEN_REPARSE_POINT,
                FILE_SYNCHRONOUS_IO_NONALERT,
            },
        },
        Win32::{
            Foundation::{RtlNtStatusToDosError, OBJ_CASE_INSENSITIVE, UNICODE_STRING},
            Storage::FileSystem::{
                FILE_ADD_SUBDIRECTORY, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES,
                FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES,
                SYNCHRONIZE,
            },
            System::IO::IO_STATUS_BLOCK,
        },
    };
    let mut wide = name.encode_wide().collect::<Vec<_>>();
    if wide.is_empty() || wide.contains(&0) || wide.len() > (u16::MAX as usize / 2) {
        return Err(PathGuardError::UnsafePath);
    }
    let unicode = UNICODE_STRING {
        Length: (wide.len() * mem::size_of::<u16>()) as u16,
        MaximumLength: (wide.len() * mem::size_of::<u16>()) as u16,
        Buffer: wide.as_mut_ptr(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.handle.as_raw_handle(),
        ObjectName: &unicode,
        Attributes: OBJ_CASE_INSENSITIVE,
        SecurityDescriptor: std::ptr::null(),
        SecurityQualityOfService: std::ptr::null(),
    };
    let mut raw = std::ptr::null_mut();
    let mut io_status = unsafe { mem::zeroed::<IO_STATUS_BLOCK>() };
    let status = unsafe {
        NtCreateFile(
            &mut raw,
            FILE_LIST_DIRECTORY
                | FILE_ADD_SUBDIRECTORY
                | FILE_READ_ATTRIBUTES
                | FILE_WRITE_ATTRIBUTES
                | SYNCHRONIZE,
            &attributes,
            &mut io_status,
            std::ptr::null(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_CREATE,
            FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            std::ptr::null(),
            0,
        )
    };
    if status < 0 {
        return Err(PathGuardError::Io(io::Error::from_raw_os_error(unsafe {
            RtlNtStatusToDosError(status)
        }
            as i32)));
    }
    let handle = unsafe { File::from(OwnedHandle::from_raw_handle(raw)) };
    inspect_windows_directory(&handle)?;
    let identity = windows_directory_identity(&handle)?;
    Ok(VerifiedDirectory {
        path: parent.path.join(name),
        identity,
        handle,
    })
}

#[cfg(target_os = "macos")]
fn create_macos_child_directory(
    parent: &VerifiedDirectory,
    name: &std::ffi::OsStr,
) -> Result<VerifiedDirectory, PathGuardError> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt, os::unix::io::FromRawFd};
    let name = CString::new(name.as_bytes()).map_err(|_| PathGuardError::UnsafePath)?;
    if unsafe { libc::mkdirat(parent.raw_fd(), name.as_ptr(), 0o755) } != 0 {
        return Err(PathGuardError::Io(io::Error::last_os_error()));
    }
    let fd = unsafe {
        libc::openat(
            parent.raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(PathGuardError::Io(io::Error::last_os_error()));
    }
    let handle = unsafe { File::from_raw_fd(fd) };
    let identity = macos_directory_identity(&handle)?;
    parent.sync()?;
    Ok(VerifiedDirectory {
        path: parent
            .path
            .join(std::ffi::OsStr::from_bytes(name.as_bytes())),
        identity,
        handle,
    })
}

#[cfg(windows)]
fn open_windows(path: &Path) -> Result<VerifiedDirectory, PathGuardError> {
    open_windows_relative(path, false)
}

#[cfg(windows)]
fn open_windows_relative(
    path: &Path,
    create_missing: bool,
) -> Result<VerifiedDirectory, PathGuardError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(PathGuardError::UnsafePath);
    }

    let mut root_path = PathBuf::new();
    let mut names = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(_) | std::path::Component::RootDir => {
                root_path.push(component.as_os_str());
            }
            std::path::Component::Normal(name) => names.push(name.to_os_string()),
            _ => return Err(PathGuardError::UnsafePath),
        }
    }
    if root_path.as_os_str().is_empty() {
        return Err(PathGuardError::UnsafePath);
    }

    let root = open_windows_root(&root_path)?;
    let mut current = root;
    let mut current_path = root_path;
    for name in names {
        let next = if create_missing {
            open_or_create_windows_child(&current, &name)?
        } else {
            open_windows_child(&current, &name)?
        };
        inspect_windows_directory(&next)?;
        current_path.push(&name);
        current = next;
    }

    let identity = windows_directory_identity(&current)?;
    Ok(VerifiedDirectory {
        path: current_path,
        identity,
        handle: current,
    })
}

#[cfg(windows)]
fn open_windows_root(path: &Path) -> Result<File, PathGuardError> {
    use std::os::windows::io::{FromRawHandle, OwnedHandle};
    use windows_sys::Win32::Foundation::{GetLastError, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ADD_SUBDIRECTORY, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES, OPEN_EXISTING, SYNCHRONIZE,
    };
    let wide = super::source_claim::windows_wide_path(path);
    let raw = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_LIST_DIRECTORY
                | FILE_ADD_SUBDIRECTORY
                | FILE_READ_ATTRIBUTES
                | FILE_WRITE_ATTRIBUTES
                | SYNCHRONIZE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(PathGuardError::Io(io::Error::from_raw_os_error(
            unsafe { GetLastError() } as i32,
        )));
    }
    let handle = unsafe { File::from(OwnedHandle::from_raw_handle(raw)) };
    inspect_windows_directory(&handle)?;
    Ok(handle)
}

#[cfg(windows)]
fn open_windows_child(parent: &File, name: &std::ffi::OsStr) -> Result<File, PathGuardError> {
    open_windows_child_with_disposition(parent, name, false)
}

#[cfg(windows)]
fn open_or_create_windows_child(
    parent: &File,
    name: &std::ffi::OsStr,
) -> Result<File, PathGuardError> {
    open_windows_child_with_disposition(parent, name, true)
}

#[cfg(windows)]
fn open_windows_child_with_disposition(
    parent: &File,
    name: &std::ffi::OsStr,
    create_missing: bool,
) -> Result<File, PathGuardError> {
    use std::{
        mem,
        os::windows::ffi::OsStrExt,
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    };
    use windows_sys::{
        Wdk::{
            Foundation::OBJECT_ATTRIBUTES,
            Storage::FileSystem::{
                NtCreateFile, FILE_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_IF,
                FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT,
            },
        },
        Win32::{
            Foundation::{RtlNtStatusToDosError, OBJ_CASE_INSENSITIVE, UNICODE_STRING},
            Storage::FileSystem::{
                FILE_ADD_SUBDIRECTORY, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES,
                FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES,
                SYNCHRONIZE,
            },
            System::IO::IO_STATUS_BLOCK,
        },
    };
    let mut wide = name.encode_wide().collect::<Vec<_>>();
    if wide.is_empty() || wide.contains(&0) || wide.len() > (u16::MAX as usize / 2) {
        return Err(PathGuardError::UnsafePath);
    }
    let unicode = UNICODE_STRING {
        Length: (wide.len() * mem::size_of::<u16>()) as u16,
        MaximumLength: (wide.len() * mem::size_of::<u16>()) as u16,
        Buffer: wide.as_mut_ptr(),
    };
    let attributes = OBJECT_ATTRIBUTES {
        Length: mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.as_raw_handle(),
        ObjectName: &unicode,
        Attributes: OBJ_CASE_INSENSITIVE,
        SecurityDescriptor: std::ptr::null(),
        SecurityQualityOfService: std::ptr::null(),
    };
    let mut raw = std::ptr::null_mut();
    let mut io_status = unsafe { mem::zeroed::<IO_STATUS_BLOCK>() };
    let status = unsafe {
        NtCreateFile(
            &mut raw,
            FILE_LIST_DIRECTORY
                | FILE_ADD_SUBDIRECTORY
                | FILE_READ_ATTRIBUTES
                | FILE_WRITE_ATTRIBUTES
                | SYNCHRONIZE,
            &attributes,
            &mut io_status,
            std::ptr::null(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            if create_missing {
                FILE_OPEN_IF
            } else {
                FILE_OPEN
            },
            FILE_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            std::ptr::null(),
            0,
        )
    };
    if status < 0 {
        return Err(PathGuardError::Io(io::Error::from_raw_os_error(unsafe {
            RtlNtStatusToDosError(status)
        }
            as i32)));
    }
    Ok(unsafe { File::from(OwnedHandle::from_raw_handle(raw)) })
}

#[cfg(windows)]
fn inspect_windows_directory(handle: &File) -> Result<(), PathGuardError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY,
        FILE_ATTRIBUTE_REPARSE_POINT,
    };
    let mut info = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
    if unsafe { GetFileInformationByHandle(handle.as_raw_handle(), &mut info) } == 0 {
        return Err(PathGuardError::Io(io::Error::last_os_error()));
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(PathGuardError::ReparsePoint);
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
        return Err(PathGuardError::UnsafePath);
    }
    Ok(())
}

#[cfg(windows)]
fn windows_directory_identity(handle: &File) -> Result<DirectoryIdentity, PathGuardError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let mut info = unsafe { std::mem::zeroed::<BY_HANDLE_FILE_INFORMATION>() };
    if unsafe { GetFileInformationByHandle(handle.as_raw_handle(), &mut info) } == 0 {
        return Err(PathGuardError::Io(io::Error::last_os_error()));
    }
    Ok(DirectoryIdentity {
        volume_id: info.dwVolumeSerialNumber.to_string(),
        file_id: (u64::from(info.nFileIndexHigh) << 32 | u64::from(info.nFileIndexLow)).to_string(),
    })
}

#[cfg(target_os = "macos")]
fn open_macos(path: &Path) -> Result<VerifiedDirectory, PathGuardError> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt, os::unix::io::FromRawFd};

    let resolved = super::path_guard::prepare_macos_path(path)?;
    let name =
        CString::new(resolved.as_os_str().as_bytes()).map_err(|_| PathGuardError::UnsafePath)?;
    let fd = unsafe {
        libc::open(
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(PathGuardError::Io(io::Error::last_os_error()));
    }
    let handle = unsafe { File::from_raw_fd(fd) };
    let identity = macos_directory_identity(&handle)?;
    Ok(VerifiedDirectory {
        path: resolved,
        identity,
        handle,
    })
}

#[cfg(target_os = "macos")]
fn macos_directory_identity(handle: &File) -> Result<DirectoryIdentity, PathGuardError> {
    use std::os::unix::io::AsRawFd;
    let mut stat = unsafe { std::mem::zeroed::<libc::stat>() };
    if unsafe { libc::fstat(handle.as_raw_fd(), &mut stat) } != 0 {
        return Err(PathGuardError::Io(io::Error::last_os_error()));
    }
    if stat.st_mode & libc::S_IFMT != libc::S_IFDIR {
        return Err(PathGuardError::UnsafePath);
    }
    Ok(DirectoryIdentity {
        volume_id: stat.st_dev.to_string(),
        file_id: stat.st_ino.to_string(),
    })
}
