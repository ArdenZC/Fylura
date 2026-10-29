#[cfg(target_os = "macos")]
fn main() {
    if let Err(error) = zen_canvas_tauri::global_index::macos::run_qualification() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("smart_lean_native_qa_requires_macos");
    std::process::exit(2);
}
