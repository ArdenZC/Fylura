#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn main() {
    if let Err(error) =
        fylura_tauri::platform::macos::native_preview::run_native_preview_lifecycle_harness()
    {
        eprintln!("macos_native_preview_lifecycle_failed:{error}");
        std::process::exit(1);
    }
    println!("macos_native_preview_lifecycle_passed");
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
fn main() {
    eprintln!("macos_native_preview_lifecycle_requires_apple_silicon_macos");
    std::process::exit(2);
}
