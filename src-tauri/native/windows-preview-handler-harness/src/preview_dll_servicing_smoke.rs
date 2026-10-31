#![cfg_attr(not(windows), allow(dead_code))]

#[cfg(windows)]
mod windows_smoke {
    use std::{
        error::Error,
        ffi::OsStr,
        fs::{self, OpenOptions},
        os::windows::ffi::OsStrExt,
        path::{Path, PathBuf},
        process::Command,
    };

    use windows::{
        core::PCSTR,
        core::PCWSTR,
        Win32::{
            Foundation::{CloseHandle, FreeLibrary, ERROR_SHARING_VIOLATION, HANDLE, HMODULE},
            Storage::FileSystem::{
                CreateFileW, MoveFileExW, DELETE, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_WRITE,
                FILE_SHARE_NONE, MOVE_FILE_FLAGS, OPEN_EXISTING,
            },
            System::LibraryLoader::GetProcAddress,
            System::LibraryLoader::LoadLibraryW,
        },
    };

    const DLL_NAME: &str = "fylura_windows_preview_handler.dll";
    fn wide(path: &Path) -> Vec<u16> {
        OsStr::new(path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    fn win32_error_code(error: &windows::core::Error) -> u32 {
        let hresult = error.code().0 as u32;
        if hresult & 0xffff_0000 == 0x8007_0000 {
            hresult & 0xffff
        } else {
            hresult
        }
    }

    struct LoadedLibrary(HMODULE);

    impl Drop for LoadedLibrary {
        fn drop(&mut self) {
            let _ = unsafe { FreeLibrary(self.0) };
        }
    }

    struct SmokeFixture {
        root: PathBuf,
        canonical: PathBuf,
        retired_dir: PathBuf,
        retired: Option<PathBuf>,
    }

    impl SmokeFixture {
        fn new(root: PathBuf) -> Result<Self, Box<dyn Error>> {
            if root.as_os_str().is_empty() || root.parent().is_none() {
                return Err("fixture root must be a dedicated child path".into());
            }
            let canonical = root.join("native").join(DLL_NAME);
            let retired_dir = root
                .parent()
                .expect("validated fixture parent")
                .join(".zen-canvas-retired");
            fs::create_dir_all(canonical.parent().expect("canonical parent"))?;
            Ok(Self {
                root,
                canonical,
                retired_dir,
                retired: None,
            })
        }

        fn reserve_retired_path(&mut self) -> Result<PathBuf, Box<dyn Error>> {
            fs::create_dir_all(&self.retired_dir)?;
            for serial in 0..32u32 {
                let candidate = self.retired_dir.join(format!(
                    "w4-04-preview-dll-servicing-{}-{serial}.tmp",
                    std::process::id()
                ));
                match OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&candidate)
                {
                    Ok(file) => {
                        drop(file);
                        fs::remove_file(&candidate)?;
                        self.retired = Some(candidate.clone());
                        return Ok(candidate);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => return Err(error.into()),
                }
            }
            Err("could not reserve a unique retirement placeholder".into())
        }
    }

    impl Drop for SmokeFixture {
        fn drop(&mut self) {
            if let Some(retired) = self.retired.as_ref() {
                let _ = fs::remove_file(retired);
            }
            let _ = fs::remove_dir(&self.retired_dir);
            // The caller provides a dedicated disposable fixture root.  Only
            // that exact root is recursively cleaned; the retirement parent
            // is never recursively touched.
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn probe_mapped_file(path: &Path) -> Result<u32, Box<dyn Error>> {
        let path_wide = wide(path);
        let result = unsafe {
            CreateFileW(
                PCWSTR(path_wide.as_ptr()),
                FILE_GENERIC_WRITE.0 | DELETE.0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None::<HANDLE>,
            )
        };
        match result {
            Ok(handle) => {
                let _ = unsafe { CloseHandle(handle) };
                Err("mapped DLL unexpectedly allowed write/delete access with zero sharing".into())
            }
            Err(error) => Ok(win32_error_code(&error)),
        }
    }

    fn move_without_replace(source: &Path, destination: &Path) -> Result<(), Box<dyn Error>> {
        let source_wide = wide(source);
        let destination_wide = wide(destination);
        unsafe {
            MoveFileExW(
                PCWSTR(source_wide.as_ptr()),
                PCWSTR(destination_wide.as_ptr()),
                MOVE_FILE_FLAGS(0),
            )?;
        }
        Ok(())
    }

    pub fn load_only(source: &Path) -> Result<(), Box<dyn Error>> {
        if source.file_name().and_then(|name| name.to_str()) != Some(DLL_NAME) {
            return Err(format!("source must be named {DLL_NAME}: {}", source.display()).into());
        }
        let source_wide = wide(source);
        let loaded = LoadedLibrary(unsafe { LoadLibraryW(PCWSTR(source_wide.as_ptr()))? });
        let export =
            unsafe { GetProcAddress(loaded.0, PCSTR(c"DllCanUnloadNow".as_ptr() as *const u8)) };
        if export.is_none() {
            return Err("replacement DLL loaded but DllCanUnloadNow was not exported".into());
        }
        println!("W4-04 mapped Preview DLL servicing child LoadLibrary: PASS");
        Ok(())
    }

    pub fn run(source: &Path, root: PathBuf) -> Result<(), Box<dyn Error>> {
        if source.file_name().and_then(|name| name.to_str()) != Some(DLL_NAME) {
            return Err(format!("source must be named {DLL_NAME}: {}", source.display()).into());
        }
        if !source.is_file() {
            return Err(format!("source DLL is missing: {}", source.display()).into());
        }

        let source_bytes = fs::read(source)?;
        let mut fixture = SmokeFixture::new(root)?;
        fs::copy(source, &fixture.canonical)?;
        let canonical_wide = wide(&fixture.canonical);
        let loaded = LoadedLibrary(unsafe { LoadLibraryW(PCWSTR(canonical_wide.as_ptr()))? });

        let probe_error = probe_mapped_file(&fixture.canonical)?;
        if probe_error != ERROR_SHARING_VIOLATION.0 {
            return Err(format!(
                "mapped DLL probe returned Win32 error {probe_error}, expected ERROR_SHARING_VIOLATION (32)"
            )
            .into());
        }

        let retired = fixture.reserve_retired_path()?;
        move_without_replace(&fixture.canonical, &retired)?;
        if fixture.canonical.exists() || !retired.is_file() {
            return Err("exact retirement move did not produce the expected post-state".into());
        }

        fs::copy(source, &fixture.canonical)?;
        if fs::read(&fixture.canonical)? != source_bytes {
            return Err("replacement bytes do not match the packaged source DLL".into());
        }
        if !fixture.canonical.is_file() || !retired.is_file() {
            return Err("canonical replacement or retired recovery bytes disappeared".into());
        }

        let child = Command::new(std::env::current_exe()?)
            .arg("--load-only")
            .arg(&fixture.canonical)
            .status()?;
        if !child.success() {
            return Err(format!(
                "a new process could not LoadLibrary the canonical replacement (status {child})"
            )
            .into());
        }

        // This is the uninstall-style proof: the old image remains mapped in
        // this process while the canonical replacement and the product root
        // are removed.  The retired path is outside that root by construction.
        fs::remove_file(&fixture.canonical)?;
        fs::remove_dir_all(&fixture.root)?;
        if fixture.root.exists() {
            return Err(
                "product-equivalent fixture root remained after uninstall-style removal".into(),
            );
        }

        drop(loaded);
        fs::remove_file(&retired)?;
        fixture.retired = None;
        if fixture.canonical.exists() || fixture.root.exists() || retired.exists() {
            return Err("mapped-DLL servicing smoke cleanup left an unexpected file".into());
        }

        println!(
            "W4-04 mapped Preview DLL servicing smoke: CreateFileW error {probe_error}, rename-without-replace, canonical replacement, and cleanup: PASS"
        );
        Ok(())
    }
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os();
    let _program = args.next();
    let first = args.next().ok_or(
        "usage: fylura-windows-preview-dll-servicing-smoke <preview-handler.dll> [fixture-root]",
    )?;
    if first == std::ffi::OsStr::new("--load-only") {
        let source = args.next().map(std::path::PathBuf::from).ok_or(
            "usage: fylura-windows-preview-dll-servicing-smoke --load-only <preview-handler.dll>",
        )?;
        return windows_smoke::load_only(&source);
    }
    let source = std::path::PathBuf::from(first);
    let root = args
        .next()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_dir()
                .expect("current directory")
                .join(".tmp-tests")
                .join("w4-04-preview-dll-servicing-smoke")
        });
    windows_smoke::run(&source, root)
}

#[cfg(not(windows))]
fn main() {
    eprintln!("fylura-windows-preview-dll-servicing-smoke requires Windows");
}
