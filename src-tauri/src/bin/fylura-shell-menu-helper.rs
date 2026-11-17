#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
fn main() {
    std::process::exit(fylura_tauri::platform::windows::shell_context_menu::run_helper());
}

#[cfg(not(windows))]
fn main() {
    eprintln!("The Fylura Shell menu helper is available on Windows only.");
    std::process::exit(2);
}
