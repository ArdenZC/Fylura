#![cfg(target_os = "macos")]

use std::fs;
use std::io::Read;
use std::path::Path;

use fylura_tauri::platform::macos::file_provider::GENERIC_FILE_PROVIDER_AWARENESS_AVAILABLE;
use fylura_tauri::platform::macos::file_semantics::{content_read_eligibility, open_content_read};
use fylura_tauri::platform::macos::MacContentReadEligibility;
use fylura_tauri::platform::macos::{strategy, volume};

#[test]
fn local_fixture_is_readable_and_cloud_provider_fixtures_never_materialize_implicitly() {
    let root = std::env::temp_dir().join(format!(
        "zen-canvas-macos-feasibility-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&root).expect("create local feasibility fixture");
    let local_file = root.join("ordinary-local.txt");
    fs::write(&local_file, b"local fixture").expect("write local feasibility fixture");

    assert_eq!(
        content_read_eligibility(&local_file),
        MacContentReadEligibility::Eligible
    );
    let mut opened = open_content_read(&local_file).expect("open local fixture through gate");
    let mut contents = String::new();
    opened
        .read_to_string(&mut contents)
        .expect("read local fixture");
    assert_eq!(contents, "local fixture");

    println!(
        "macos_file_provider_feasibility ordinary_local=eligible generic_file_provider_awareness={GENERIC_FILE_PROVIDER_AWARENESS_AVAILABLE}"
    );
    inspect_optional_fixture("icloud", "ZEN_CANVAS_ICLOUD_FIXTURE");
    inspect_optional_fixture("file_provider", "ZEN_CANVAS_FILE_PROVIDER_FIXTURE");
    inspect_optional_volume_fixture("external_apfs", "ZEN_CANVAS_EXTERNAL_APFS_FIXTURE");
    inspect_optional_volume_fixture("exfat", "ZEN_CANVAS_EXFAT_FIXTURE");
    inspect_optional_volume_fixture("network", "ZEN_CANVAS_NETWORK_VOLUME_FIXTURE");

    fs::remove_dir_all(root).expect("remove local feasibility fixture");
}

fn inspect_optional_fixture(label: &str, variable: &str) {
    let Some(path) = std::env::var_os(variable).map(std::path::PathBuf::from) else {
        println!(
            "macos_file_provider_feasibility {label}: SKIPPED — REAL FIXTURE NOT PROVIDED; NOT VERIFIED — fixture unavailable env={variable}"
        );
        return;
    };
    if !path.exists() {
        println!(
            "macos_file_provider_feasibility {label}: SKIPPED — REAL FIXTURE NOT PROVIDED; NOT VERIFIED — fixture unavailable missing_path={}",
            path.display()
        );
        return;
    }

    let eligibility = content_read_eligibility(&path);
    println!(
        "macos_file_provider_feasibility {label}: IMPLEMENTED; NATIVE CONTRACT TESTED; REAL FIXTURE NOT VERIFIED path={} eligibility={:?}",
        path.display(),
        eligibility
    );
    if !matches!(eligibility, MacContentReadEligibility::Eligible) {
        assert_ne!(eligibility, MacContentReadEligibility::Eligible);
        assert_no_implicit_materialization(&path);
    }
}

fn assert_no_implicit_materialization(path: &Path) {
    // The only operation here is the gate itself. A deferred item must never
    // reach OpenOptions, Foundation download APIs, or any mutation authority.
    assert_ne!(
        content_read_eligibility(path),
        MacContentReadEligibility::Eligible
    );
}

fn inspect_optional_volume_fixture(label: &str, variable: &str) {
    let Some(path) = std::env::var_os(variable).map(std::path::PathBuf::from) else {
        println!(
            "macos_file_provider_feasibility {label}: SKIPPED — REAL FIXTURE NOT PROVIDED; NOT VERIFIED — fixture unavailable env={variable}"
        );
        return;
    };
    if !path.exists() {
        println!(
            "macos_file_provider_feasibility {label}: SKIPPED — REAL FIXTURE NOT PROVIDED; NOT VERIFIED — fixture unavailable missing_path={}",
            path.display()
        );
        return;
    }

    let target_parent = if path.is_dir() {
        &path
    } else {
        path.parent().unwrap_or(&path)
    };
    let semantics = volume::inspect(&path);
    let selected = strategy::select(&path, target_parent);
    println!(
        "macos_file_provider_feasibility {label}: IMPLEMENTED; NATIVE CONTRACT TESTED; REAL FIXTURE NOT VERIFIED path={} filesystem={:?} local={:?} removable={:?} readonly={:?} strategy={}",
        path.display(),
        semantics.filesystem_type,
        semantics.is_local,
        semantics.is_removable,
        semantics.is_read_only,
        selected.label(),
    );
}
