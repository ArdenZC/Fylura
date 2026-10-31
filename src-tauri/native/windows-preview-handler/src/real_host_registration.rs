#![cfg(windows)]

//! Test-only Explorer/prevhost registration runner.
//!
//! This binary is intentionally separate from the controlled, registry-free
//! harness. It creates a fresh disposable fixture root, installs the narrow
//! HKCU registration guard, refreshes the shell association cache, opens
//! Explorer, and waits for the operator to finish real-host checks. Cleanup is
//! performed before the process exits and is retried by the registration
//! guard during unwind.

use std::{
    env,
    error::Error,
    fs,
    io::{self, Write},
    path::Path,
    process::Command,
};

use fylura_windows_preview_handler::test_registration::{
    self, Registration, FRIENDLY_NAME, TEST_EXTENSION, TEST_PROGID,
};
use windows::Win32::{
    System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    },
    UI::Shell::{IPreviewHandler, SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST},
};

const FIXTURE_A_NAME: &str = "zen-w4-03-v2-a.zcv2preview";
const FIXTURE_B_NAME: &str = "zen-w4-03-v2-b.zcv2preview";
const FIXTURE_A_CONTENT: &[u8] = b"Fylura W4-03 v2 real-host fixture A\r\nselection A\r\n";
const FIXTURE_B_CONTENT: &[u8] = b"Fylura W4-03 v2 real-host fixture B\r\nselection B\r\n";

struct ComApartment;

impl ComApartment {
    fn initialize() -> Result<Self, Box<dyn Error>> {
        let status = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if !status.is_ok() {
            return Err(format!("CoInitializeEx failed: {status:?}").into());
        }
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args().skip(1);
    match arguments.next().as_deref() {
        Some("run") => {
            let dll = arguments
                .next()
                .ok_or("usage: real-host run <handler.dll> <fresh-fixture-root>")?;
            let fixture_root = arguments
                .next()
                .ok_or("usage: real-host run <handler.dll> <fresh-fixture-root>")?;
            if arguments.next().is_some() {
                return Err("usage: real-host run <handler.dll> <fresh-fixture-root>".into());
            }
            run(Path::new(&dll), Path::new(&fixture_root))
        }
        Some("help") | None => {
            println!("usage: real-host run <handler.dll> <fresh-fixture-root>");
            Ok(())
        }
        Some(command) => Err(format!("unknown command: {command}").into()),
    }
}

fn run(dll_path: &Path, fixture_root: &Path) -> Result<(), Box<dyn Error>> {
    let dll_path = fs::canonicalize(dll_path)?;
    if fixture_root.exists() {
        return Err(format!(
            "fixture root must be absent before the real-host run: {}",
            fixture_root.display()
        )
        .into());
    }
    fs::create_dir_all(fixture_root)?;

    let fixture_a = fixture_root.join(FIXTURE_A_NAME);
    let fixture_b = fixture_root.join(FIXTURE_B_NAME);
    fs::write(&fixture_a, FIXTURE_A_CONTENT)?;
    fs::write(&fixture_b, FIXTURE_B_CONTENT)?;

    let run_result = run_registered(&dll_path, fixture_root, &fixture_a, &fixture_b);

    let cleanup_result = cleanup_fixture_root(fixture_root);
    if let Err(error) = &cleanup_result {
        eprintln!("FIXTURE cleanup FAILED: {error}");
    }

    run_result?;
    cleanup_result?;
    Ok(())
}

fn run_registered(
    dll_path: &Path,
    fixture_root: &Path,
    fixture_a: &Path,
    fixture_b: &Path,
) -> Result<(), Box<dyn Error>> {
    let _com = ComApartment::initialize()?;
    let mut registration = Registration::install(dll_path)?;
    println!("REAL_HOST registration: INSTALLED");
    println!("REAL_HOST handler DLL: {}", dll_path.display());
    println!(
        "REAL_HOST test CLSID: {}",
        test_registration::clsid_string()
    );
    println!("REAL_HOST extension: {TEST_EXTENSION}");
    println!("REAL_HOST ProgID: {TEST_PROGID}");
    println!("REAL_HOST friendly name: {FRIENDLY_NAME}");
    println!(
        "REAL_HOST PreviewHandlers list value: {} = REG_SZ {FRIENDLY_NAME}",
        test_registration::clsid_string()
    );
    println!("REAL_HOST low-integrity opt-out: NOT SET");
    println!("REAL_HOST fixture root: {}", fixture_root.display());
    print_fixture("A", fixture_a)?;
    print_fixture("B", fixture_b)?;

    println!("REAL_HOST registry key creations:");
    for path in registration.created_key_paths() {
        println!("  HKCU\\{path}");
    }
    println!("REAL_HOST registry value mutations:");
    for mutation in registration.mutation_lines() {
        println!("  {mutation}");
    }

    let handler: IPreviewHandler =
        unsafe { CoCreateInstance(&test_registration::TEST_CLSID, None, CLSCTX_INPROC_SERVER)? };
    println!("REAL_HOST CoCreateInstance from HKCU registration: PASS");
    drop(handler);

    unsafe {
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
    }
    println!("REAL_HOST shell association refresh: SENT");

    Command::new("explorer.exe")
        .arg(format!("/select,{}", fixture_a.display()))
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("failed to launch Explorer: {error}"))?;

    println!("REAL_HOST Explorer launch: SENT");
    println!("REAL_HOST next actions:");
    println!("  1. Enable Explorer Preview pane and select fixture A.");
    println!("  2. Verify prevhost.exe loaded this DLL and the preview is visible.");
    println!("  3. Switch A -> B, resize/focus the pane, and exercise keyboard input.");
    println!("  4. While previewing, write/rename/move/delete the disposable fixtures.");
    println!("  5. Close Explorer or select a non-fixture before confirming cleanup.");
    print!("REAL_HOST press Enter after recording observations: ");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;

    let cleanup_report = registration.cleanup()?;
    println!("REAL_HOST registry cleanup: PASS");
    for line in cleanup_report {
        println!("  {line}");
    }
    println!("REAL_HOST registration residue: exact created keys absent (PASS)");
    Ok(())
}

fn print_fixture(label: &str, path: &Path) -> Result<(), Box<dyn Error>> {
    let metadata = fs::metadata(path)?;
    println!(
        "REAL_HOST fixture {label}: path={} name={} extension={} bytes={} regular_file={}",
        path.display(),
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("<invalid>"),
        path.extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("<none>"),
        metadata.len(),
        metadata.is_file()
    );
    Ok(())
}

fn cleanup_fixture_root(fixture_root: &Path) -> io::Result<()> {
    if fixture_root.exists() {
        fs::remove_dir_all(fixture_root)?;
    }
    if fixture_root.exists() {
        return Err(io::Error::other(format!(
            "fixture root remained after cleanup: {}",
            fixture_root.display()
        )));
    }
    Ok(())
}
