use serde_json::{json, Value};
use std::error::Error;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use zen_canvas_tauri::db::Database;

#[cfg(windows)]
use zen_canvas_tauri::global_index::{
    models::PROVIDER_WINDOWS_MFT_USN, windows::volumes::discover_windows_volumes,
};

#[cfg(windows)]
fn prepare_profile(profile_root: PathBuf, target_mount_path: &str) -> Result<(), Box<dyn Error>> {
    if !profile_root.is_absolute() {
        return Err("profile root must be absolute".into());
    }
    let target_mount_path = target_mount_path.trim_end_matches(['\\', '/']);
    let database = Database::open(profile_root.join("zen-canvas.sqlite3"))?;
    let mut target_source = None;

    for mut source in discover_windows_volumes()? {
        let is_target = source
            .volume
            .mount_path
            .trim_end_matches(['\\', '/'])
            .eq_ignore_ascii_case(target_mount_path);
        if is_target {
            if source.volume.drive_kind != "fixed"
                || !source.volume.filesystem_type.eq_ignore_ascii_case("NTFS")
                || source.volume.provider != PROVIDER_WINDOWS_MFT_USN
            {
                return Err(format!(
                    "qualification target is not a supported fixed NTFS source: mount={} kind={} filesystem={} provider={}",
                    source.volume.mount_path,
                    source.volume.drive_kind,
                    source.volume.filesystem_type,
                    source.volume.provider,
                )
                .into());
            }
            target_source = Some(source.volume.clone());
        }
        // Seed the isolated QA profile through the existing durable source
        // setting. The candidate coordinator will rediscover every volume,
        // while only this task-owned test volume remains enabled.
        source.volume.enabled = is_target;
        database.upsert_global_volume(&source.volume)?;
    }

    let target_source = target_source.ok_or_else(|| {
        format!(
            "qualification target was not discovered as a drive-letter volume: {target_mount_path}"
        )
    })?;
    let target_source_id = target_source.id.clone();
    // This probe runs only after the Windows qualification script creates and
    // validates its task-owned disposable NTFS VHD. Grant consent to that
    // exact source; all other discovered volumes remain disabled/unapproved.
    database.authorize_global_volume(&target_source_id, true, &target_source)?;
    let enabled_sources = database
        .list_global_volumes()?
        .into_iter()
        .filter(|volume| volume.enabled)
        .collect::<Vec<_>>();
    if enabled_sources.len() != 1 || enabled_sources[0].id != target_source_id {
        return Err(format!(
            "isolated QA profile must enable exactly the qualification volume: target={} enabled={:?}",
            target_source_id,
            enabled_sources
                .iter()
                .map(|volume| (&volume.id, &volume.mount_path))
                .collect::<Vec<_>>(),
        )
        .into());
    }

    println!(
        "{}",
        serde_json::to_string(&json!({
            "prepared": true,
            "targetSourceId": target_source_id,
            "targetMountPath": enabled_sources[0].mount_path,
            "enabledSourceCount": enabled_sources.len(),
            "enabledSourceIds": enabled_sources.iter().map(|volume| &volume.id).collect::<Vec<_>>(),
        }))?
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments
        .first()
        .is_some_and(|argument| argument == "--prepare-profile")
    {
        if arguments.len() != 3 {
            return Err("usage: zb05-global-index-qa --prepare-profile <absolute-profile-root> <drive-letter-mount>".into());
        }
        #[cfg(windows)]
        {
            return prepare_profile(PathBuf::from(&arguments[1]), &arguments[2]);
        }
        #[cfg(not(windows))]
        {
            return Err("profile preparation is supported only on Windows".into());
        }
    }
    if arguments
        .first()
        .is_some_and(|a| a == "--qualify-smart-lean-direct")
    {
        if arguments.len() != 5 {
            return Err("usage: --qualify-smart-lean-direct <profile> <source-id> <included-file> <excluded-file>".into());
        }
        #[cfg(windows)]
        {
            let result = zen_canvas_tauri::global_index::windows::smart_lean_qa::run(
                std::path::Path::new(&arguments[1]),
                &arguments[2],
                std::path::Path::new(&arguments[3]),
                std::path::Path::new(&arguments[4]),
            )?;
            println!("{}", serde_json::to_string(&result)?);
            return Ok(());
        }
        #[cfg(not(windows))]
        {
            return Err("Direct qualification requires Windows".into());
        }
    }
    if !arguments.is_empty() {
        return Err("unexpected QA probe arguments".into());
    }

    let profile_root = std::env::var_os("ZC_NATIVE_QA_PROFILE_ROOT")
        .map(PathBuf::from)
        .ok_or("ZC_NATIVE_QA_PROFILE_ROOT is required for the isolated probe")?;
    if !profile_root.is_absolute() {
        return Err("ZC_NATIVE_QA_PROFILE_ROOT must be absolute".into());
    }
    let database = Database::open(profile_root.join("zen-canvas.sqlite3"))?;
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());

    for line in stdin.lock().lines() {
        let line = line?;
        let request: Value = serde_json::from_str(&line)?;
        let query = request
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let volumes = database
            .list_global_volumes()?
            .into_iter()
            .map(|volume| {
                json!({
                    "id": volume.id,
                    "mountPath": volume.mount_path,
                    "filesystemType": volume.filesystem_type,
                    "driveKind": volume.drive_kind,
                    "enabled": volume.enabled,
                    "provider": volume.provider,
                    "status": volume.index_status,
                    "lastError": volume.last_error,
                    "entryCount": volume.entry_count,
                    "lastFullIndexAt": volume.last_full_index_at,
                })
            })
            .collect::<Vec<_>>();
        let results = if query.is_empty() {
            Vec::new()
        } else {
            database
                .search_global_entries(query, 25, 0)?
                .into_iter()
                .map(|result| json!({"name": result.name, "path": result.path}))
                .collect::<Vec<_>>()
        };
        serde_json::to_writer(
            &mut stdout,
            &json!({"volumes": volumes, "results": results, "coverage": database.global_index_coverage()?}),
        )?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }

    Ok(())
}
