//! Source-only Windows NTFS API probe for the B04 directory-sync question.
//!
//! This binary is inert unless `T10_B04_PROBE_ENABLE=1` is explicitly set.
//! Even then, it refuses to touch the filesystem until source identity,
//! runner identity, NTFS, the temporary root, and the bounded storage budget
//! have been checked. It reports API behavior only; it cannot prove power-loss
//! persistence or cross-file atomicity.

use serde_json::{Value, json};
#[cfg(target_os = "windows")]
use sha2::{Digest, Sha256};
use std::time::Duration;
#[cfg(target_os = "windows")]
use std::time::Instant;

#[cfg(target_os = "windows")]
const CONSENT_VARIABLE: &str = "T10_B04_PROBE_ENABLE";
#[cfg(target_os = "windows")]
const EXPECTED_HEAD_VARIABLE: &str = "T10_B04_EXPECTED_HEAD";
#[cfg(target_os = "windows")]
const EXPECTED_TREE_VARIABLE: &str = "T10_B04_EXPECTED_TREE";
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_DISPOSABLE_FILES: u8 = 10;
const MAX_WALL_TIME: Duration = Duration::from_secs(30);
#[cfg(any(target_os = "windows", test))]
const FILE_PAYLOAD: &[u8] = b"Zen Canvas B04 NTFS file sync and rename probe\n";

#[cfg(any(target_os = "windows", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Consent {
    Disabled,
    Enabled,
}

#[cfg(any(target_os = "windows", test))]
fn consent(value: Option<&str>) -> Consent {
    match value {
        Some("1") => Consent::Enabled,
        _ => Consent::Disabled,
    }
}

#[cfg(any(target_os = "windows", test))]
fn valid_git_object_id(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(any(target_os = "windows", test))]
fn ntfs_is_verified(value: Option<&str>) -> bool {
    value.is_some_and(|filesystem| filesystem.eq_ignore_ascii_case("NTFS"))
}

#[cfg(any(target_os = "windows", test))]
fn identity_is_known<T>(identity: Option<T>) -> bool {
    identity.is_some()
}

#[cfg(any(target_os = "windows", test))]
fn budget_is_valid(file_bytes: u64, file_count: u8, elapsed: Duration) -> bool {
    file_bytes <= MAX_FILE_BYTES && file_count <= MAX_DISPOSABLE_FILES && elapsed <= MAX_WALL_TIME
}

#[cfg(any(target_os = "windows", test))]
fn normalized_windows_path(value: &str) -> String {
    let mut normalized = value.replace('/', "\\").to_ascii_lowercase();
    if let Some(without_device_prefix) = normalized.strip_prefix("\\\\?\\") {
        normalized = without_device_prefix.to_string();
    }
    while normalized.ends_with('\\') && normalized.len() > 3 {
        normalized.pop();
    }
    normalized
}

#[cfg(any(target_os = "windows", test))]
fn is_local_absolute_windows_path(value: &str) -> bool {
    let normalized = normalized_windows_path(value);
    let bytes = normalized.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && bytes[2] == b'\\'
        && !normalized.starts_with("\\\\")
        && !normalized
            .split('\\')
            .any(|part| part == "." || part == "..")
}

#[cfg(any(target_os = "windows", test))]
fn is_windows_drive_root(value: &str) -> bool {
    let normalized = normalized_windows_path(value);
    (normalized.len() == 2
        && normalized.as_bytes()[0].is_ascii_alphabetic()
        && normalized.as_bytes()[1] == b':')
        || (normalized.len() == 3
            && normalized.as_bytes()[0].is_ascii_alphabetic()
            && normalized.as_bytes()[1] == b':'
            && normalized.as_bytes()[2] == b'\\')
}

#[cfg(any(target_os = "windows", test))]
fn path_is_within(candidate: &str, root: &str) -> bool {
    let candidate = normalized_windows_path(candidate);
    let root = normalized_windows_path(root);
    candidate == root || candidate.starts_with(&(root + "\\"))
}

#[cfg(any(target_os = "windows", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScopeError {
    NotLocalAbsolute,
    VolumeRoot,
    ProtectedPath,
    ReparsePath,
}

#[cfg(any(target_os = "windows", test))]
fn validate_temp_scope(
    candidate: &str,
    protected_roots: &[&str],
    reparse_free: bool,
) -> Result<(), ScopeError> {
    if !is_local_absolute_windows_path(candidate) {
        return Err(ScopeError::NotLocalAbsolute);
    }
    if is_windows_drive_root(candidate) {
        return Err(ScopeError::VolumeRoot);
    }
    if protected_roots
        .iter()
        .any(|root| path_is_within(candidate, root) || path_is_within(root, candidate))
    {
        return Err(ScopeError::ProtectedPath);
    }
    if !reparse_free {
        return Err(ScopeError::ReparsePath);
    }
    Ok(())
}

fn native_windows_probe_allowed(os: &str) -> bool {
    os.eq_ignore_ascii_case("windows")
}

fn emit_record(
    status: &str,
    reason: Option<&str>,
    source_head: Option<&str>,
    source_tree: Option<&str>,
    os_build: Option<&str>,
    filesystem: Option<&str>,
    free_bytes: Option<u64>,
    observations: &[Value],
    cleanup: Option<&str>,
) {
    let record = json!({
        "schema": "zen-canvas-t10-b04-ntfs-probe-v1",
        "status": status,
        "reason": reason,
        "source": {"head": source_head, "tree": source_tree},
        "host": {
            "os": std::env::consts::OS,
            "build": os_build,
            "filesystem": filesystem,
            "free_bytes_before_probe": free_bytes,
        },
        "sandbox": "RUNNER_TEMP/<unique-direct-child>",
        "observations": observations,
        "cleanup": cleanup,
        "durability_claim": "NONE_API_RETURN_OBSERVED_ONLY",
        "limits": {
            "max_combined_file_bytes": MAX_FILE_BYTES,
            "max_disposable_files": MAX_DISPOSABLE_FILES,
            "max_wall_time_seconds": MAX_WALL_TIME.as_secs(),
            "real_enospc": "DEFERRED_NOT_TESTED",
            "physical_power_loss": "DEFERRED_NOT_TESTED",
        }
    });
    println!("{record}");
}

#[cfg(target_os = "windows")]
fn main() {
    if !native_windows_probe_allowed(std::env::consts::OS) {
        emit_record(
            "NOT_EXECUTED",
            Some("NON_WINDOWS_BUILD_NO_NATIVE_PROBE"),
            None,
            None,
            None,
            None,
            None,
            &[],
            Some("NO_SANDBOX_CREATED"),
        );
        return;
    }
    if consent(std::env::var(CONSENT_VARIABLE).ok().as_deref()) == Consent::Disabled {
        emit_record(
            "NOT_EXECUTED",
            Some("CONSENT_GATE_NOT_SET_TO_1"),
            None,
            None,
            None,
            None,
            None,
            &[],
            Some("NO_SANDBOX_CREATED"),
        );
        return;
    }

    if std::env::args_os().len() != 1 {
        emit_record(
            "SAFETY_REJECTION",
            Some("UNEXPECTED_ARGUMENTS"),
            None,
            None,
            None,
            None,
            None,
            &[],
            Some("NO_SANDBOX_CREATED"),
        );
        return;
    }

    windows_probe::run();
}

#[cfg(not(target_os = "windows"))]
fn main() {
    // This branch intentionally does not inspect runner paths or create files.
    let reason = if native_windows_probe_allowed(std::env::consts::OS) {
        "PLATFORM_GUARD_REJECTED"
    } else {
        "NON_WINDOWS_BUILD_NO_NATIVE_PROBE"
    };
    emit_record(
        "NOT_EXECUTED",
        Some(reason),
        None,
        None,
        None,
        None,
        None,
        &[],
        Some("NO_SANDBOX_CREATED"),
    );
}

#[cfg(target_os = "windows")]
mod windows_probe {
    use super::*;
    use std::ffi::OsStr;
    use std::fs::{self, File, OpenOptions};
    use std::io::{self, Read, Write};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use std::os::windows::io::AsRawHandle;
    use std::path::{Component, Path, PathBuf, Prefix};
    use std::process::{Command, Output};

    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;
    const FILE_ID_INFO_CLASS: u32 = 18;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
        fn GetFileInformationByHandleEx(
            file: *mut std::ffi::c_void,
            information_class: u32,
            information: *mut std::ffi::c_void,
            buffer_size: u32,
        ) -> i32;
        fn GetLastError() -> u32;
    }

    #[repr(C)]
    struct NativeFileIdInfo {
        volume_serial_number: u64,
        file_id: [u8; 16],
    }

    struct Context {
        source_head: Option<String>,
        source_tree: Option<String>,
        os_build: Option<String>,
        filesystem: Option<String>,
        free_bytes: Option<u64>,
        observations: Vec<Value>,
    }

    impl Context {
        fn new() -> Self {
            Self {
                source_head: None,
                source_tree: None,
                os_build: None,
                filesystem: None,
                free_bytes: None,
                observations: Vec::new(),
            }
        }

        fn record(&self, status: &str, reason: Option<&str>, cleanup: Option<&str>) {
            emit_record(
                status,
                reason,
                self.source_head.as_deref(),
                self.source_tree.as_deref(),
                self.os_build.as_deref(),
                self.filesystem.as_deref(),
                self.free_bytes,
                &self.observations,
                cleanup,
            );
        }
    }

    struct SecurityInfo {
        current_user_sid: String,
        owner_sid: String,
        is_elevated: bool,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    struct FileIdentity {
        volume_serial_number: u64,
        file_id: [u8; 16],
    }

    #[derive(Clone, Copy)]
    enum OwnedFileLocation {
        None,
        BeforeRename,
        AfterRename,
    }

    fn preflight_failure(context: &Context, status: &str, reason: &str) {
        context.record(status, Some(reason), Some("NO_SANDBOX_CREATED"));
    }

    pub(super) fn run() {
        let start = Instant::now();
        let mut context = Context::new();

        let source = match verify_source_identity() {
            Ok(source) => source,
            Err(reason) => {
                preflight_failure(&context, "SAFETY_REJECTION", reason);
                return;
            }
        };
        context.source_head = Some(source.0);
        context.source_tree = Some(source.1);

        let runner_temp = match std::env::var_os("RUNNER_TEMP") {
            Some(path) => PathBuf::from(path),
            None => {
                preflight_failure(&context, "ENVIRONMENT_UNAVAILABLE", "RUNNER_TEMP_MISSING");
                return;
            }
        };
        if !runner_temp.is_absolute() {
            preflight_failure(&context, "SAFETY_REJECTION", "RUNNER_TEMP_NOT_ABSOLUTE");
            return;
        }
        if let Err(reason) = ensure_non_reparse_components(&runner_temp) {
            preflight_failure(&context, "SAFETY_REJECTION", reason);
            return;
        }
        let canonical_temp = match fs::canonicalize(&runner_temp) {
            Ok(path) => path,
            Err(_) => {
                preflight_failure(
                    &context,
                    "ENVIRONMENT_UNAVAILABLE",
                    "RUNNER_TEMP_UNRESOLVED",
                );
                return;
            }
        };
        if !canonical_temp.is_dir() {
            preflight_failure(&context, "SAFETY_REJECTION", "RUNNER_TEMP_NOT_DIRECTORY");
            return;
        }
        let canonical_text = canonical_temp.to_string_lossy().into_owned();
        let protected_roots = match protected_roots() {
            Some(roots) => roots,
            None => {
                preflight_failure(
                    &context,
                    "ENVIRONMENT_UNAVAILABLE",
                    "USER_OR_SYSTEM_ROOT_UNAVAILABLE",
                );
                return;
            }
        };
        let protected_text = protected_roots
            .iter()
            .map(|root| root.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let protected_refs = protected_text
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        if validate_temp_scope(&canonical_text, &protected_refs, true).is_err() {
            preflight_failure(
                &context,
                "SAFETY_REJECTION",
                "RUNNER_TEMP_OUTSIDE_SAFE_SCOPE",
            );
            return;
        }

        let security = match security_info(&canonical_temp) {
            Ok(info) => info,
            Err(_) => {
                preflight_failure(
                    &context,
                    "ENVIRONMENT_UNAVAILABLE",
                    "RUNNER_IDENTITY_OR_OWNER_UNAVAILABLE",
                );
                return;
            }
        };
        if security.is_elevated {
            preflight_failure(&context, "SAFETY_REJECTION", "ELEVATED_TOKEN_REJECTED");
            return;
        }
        if security.current_user_sid != security.owner_sid {
            preflight_failure(&context, "SAFETY_REJECTION", "RUNNER_TEMP_OWNER_MISMATCH");
            return;
        }

        let os_build = match windows_build() {
            Some(build) => build,
            None => {
                preflight_failure(
                    &context,
                    "ENVIRONMENT_UNAVAILABLE",
                    "WINDOWS_BUILD_UNAVAILABLE",
                );
                return;
            }
        };
        context.os_build = Some(os_build);

        let drive = match local_drive_designator(&canonical_temp) {
            Some(drive) => drive,
            None => {
                preflight_failure(&context, "SAFETY_REJECTION", "RUNNER_TEMP_NOT_LOCAL_DRIVE");
                return;
            }
        };
        let filesystem = match filesystem_name(&drive) {
            Some(filesystem) if ntfs_is_verified(Some(&filesystem)) => filesystem,
            Some(_) => {
                preflight_failure(&context, "ENVIRONMENT_UNAVAILABLE", "FILESYSTEM_NOT_NTFS");
                return;
            }
            None => {
                preflight_failure(
                    &context,
                    "ENVIRONMENT_UNAVAILABLE",
                    "FILESYSTEM_TYPE_UNAVAILABLE",
                );
                return;
            }
        };
        context.filesystem = Some(filesystem);

        let free_bytes = match free_bytes(&drive) {
            Some(free) if free >= MAX_FILE_BYTES => free,
            Some(_) => {
                preflight_failure(
                    &context,
                    "ENVIRONMENT_UNAVAILABLE",
                    "STORAGE_BUDGET_UNAVAILABLE",
                );
                return;
            }
            None => {
                preflight_failure(
                    &context,
                    "ENVIRONMENT_UNAVAILABLE",
                    "FREE_SPACE_UNAVAILABLE",
                );
                return;
            }
        };
        context.free_bytes = Some(free_bytes);

        let expected_payload_bytes = FILE_PAYLOAD.len() as u64;
        if !budget_is_valid(expected_payload_bytes, 1, start.elapsed()) {
            preflight_failure(&context, "SAFETY_REJECTION", "STATIC_PROBE_BUDGET_EXCEEDED");
            return;
        }

        let run_id = format!(
            "b04-ntfs-probe-{}-{}",
            std::process::id(),
            start.elapsed().as_nanos()
        );
        let sandbox = canonical_temp.join(run_id);
        if path_exists(&sandbox) {
            preflight_failure(&context, "SAFETY_REJECTION", "SANDBOX_NAME_ALREADY_EXISTS");
            return;
        }
        if let Err(error) = fs::create_dir(&sandbox) {
            context.observations.push(rust_io_observation(
                "sandbox_create",
                "std::fs::create_dir",
                "RUNNER_TEMP direct child; create-only",
                Err(error),
                json!({}),
            ));
            context.record(
                "ENVIRONMENT_UNAVAILABLE",
                Some("SANDBOX_CREATE_FAILED"),
                Some("NO_SANDBOX_CREATED_OR_EMPTY"),
            );
            return;
        }

        let mut cleanup_state = "CLEANUP_COMPLETE";
        let sandbox_security = security_info(&sandbox);
        match sandbox_security {
            Ok(info)
                if info.current_user_sid == security.current_user_sid
                    && info.owner_sid == security.current_user_sid
                    && !info.is_elevated => {}
            _ => {
                if !remove_empty_verified_sandbox(&sandbox) {
                    cleanup_state = "CLEANUP_INCOMPLETE";
                }
                context.record(
                    if cleanup_state == "CLEANUP_COMPLETE" {
                        "SAFETY_REJECTION"
                    } else {
                        "CLEANUP_INCOMPLETE"
                    },
                    Some("SANDBOX_OWNER_OR_TOKEN_RECHECK_FAILED"),
                    Some(cleanup_state),
                );
                return;
            }
        }
        if ensure_non_reparse_components(&sandbox).is_err() {
            if !remove_empty_verified_sandbox(&sandbox) {
                cleanup_state = "CLEANUP_INCOMPLETE";
            }
            context.record(
                if cleanup_state == "CLEANUP_COMPLETE" {
                    "SAFETY_REJECTION"
                } else {
                    "CLEANUP_INCOMPLETE"
                },
                Some("SANDBOX_REPARSE_POINT_REJECTED"),
                Some(cleanup_state),
            );
            return;
        }

        probe_directory_baseline(&sandbox, start, &mut context.observations);
        probe_directory_flagged_read(&sandbox, start, &mut context.observations);
        probe_directory_flagged_write(&sandbox, start, &mut context.observations);

        let source_name = "regular-before-rename.bin";
        let destination_name = "regular-after-rename.bin";
        let source_path = sandbox.join(source_name);
        let destination_path = sandbox.join(destination_name);
        let mut owned_file = OwnedFileLocation::None;
        if !budget_expired(start) {
            probe_small_file_and_rename(
                &source_path,
                &destination_path,
                start,
                &mut context.observations,
                &mut owned_file,
            );
        } else {
            context.observations.push(skipped_observation(
                "small_file_sync_and_rename",
                "30_SECOND_WALL_TIME_BUDGET_EXHAUSTED",
            ));
        }

        cleanup_state = cleanup_known_paths(
            &sandbox,
            &source_path,
            &destination_path,
            owned_file,
            &mut context.observations,
        );
        let final_status = if cleanup_state == "CLEANUP_INCOMPLETE" {
            "CLEANUP_INCOMPLETE"
        } else {
            "FILE_OPERATION_OBSERVED"
        };
        context.record(final_status, None, Some(cleanup_state));
    }

    fn verify_source_identity() -> Result<(String, String), &'static str> {
        let expected_head =
            std::env::var(EXPECTED_HEAD_VARIABLE).map_err(|_| "EXPECTED_SOURCE_HEAD_MISSING")?;
        let expected_tree =
            std::env::var(EXPECTED_TREE_VARIABLE).map_err(|_| "EXPECTED_SOURCE_TREE_MISSING")?;
        if !valid_git_object_id(&expected_head) || !valid_git_object_id(&expected_tree) {
            return Err("EXPECTED_SOURCE_ID_INVALID");
        }

        let current_dir = std::env::current_dir().map_err(|_| "SOURCE_ROOT_UNAVAILABLE")?;
        let top_level =
            git_value(&["rev-parse", "--show-toplevel"]).ok_or("SOURCE_GIT_ROOT_UNAVAILABLE")?;
        let canonical_top = fs::canonicalize(top_level).map_err(|_| "SOURCE_ROOT_UNRESOLVED")?;
        let canonical_current =
            fs::canonicalize(current_dir).map_err(|_| "SOURCE_ROOT_UNRESOLVED")?;
        if canonical_top != canonical_current {
            return Err("MUST_RUN_FROM_SOURCE_ROOT");
        }

        let head =
            git_value(&["rev-parse", "--verify", "HEAD"]).ok_or("SOURCE_HEAD_UNAVAILABLE")?;
        let tree = git_value(&["rev-parse", "--verify", "HEAD^{tree}"])
            .ok_or("SOURCE_TREE_UNAVAILABLE")?;
        if !valid_git_object_id(&head) || !valid_git_object_id(&tree) {
            return Err("SOURCE_ID_INVALID");
        }
        let status = git_value(&["status", "--porcelain", "--untracked-files=all"])
            .ok_or("SOURCE_STATUS_UNAVAILABLE")?;
        if !status.is_empty() {
            return Err("SOURCE_TREE_NOT_CLEAN");
        }
        if head != expected_head || tree != expected_tree {
            return Err("SOURCE_ID_DOES_NOT_MATCH_OWNER_APPROVED_EXPECTATION");
        }
        Ok((head, tree))
    }

    fn git_value(arguments: &[&str]) -> Option<String> {
        let output = Command::new("git").args(arguments).output().ok()?;
        if !output.status.success() {
            return None;
        }
        let value = String::from_utf8(output.stdout).ok()?.trim().to_string();
        Some(value)
    }

    fn protected_roots() -> Option<Vec<PathBuf>> {
        let system_root = std::env::var_os("SystemRoot").or_else(|| std::env::var_os("windir"))?;
        let user_root = std::env::var_os("USERPROFILE")?;
        let mut paths = vec![PathBuf::from(system_root), PathBuf::from(user_root)];
        paths.extend(
            [
                "SystemRoot",
                "windir",
                "ProgramFiles",
                "ProgramFiles(x86)",
                "ProgramData",
                "GITHUB_WORKSPACE",
            ]
            .into_iter()
            .filter_map(std::env::var_os)
            .map(PathBuf::from),
        );
        paths
            .into_iter()
            .map(fs::canonicalize)
            .collect::<Result<Vec<_>, _>>()
            .ok()
    }

    fn ensure_non_reparse_components(path: &Path) -> Result<(), &'static str> {
        let mut current = PathBuf::new();
        for component in path.components() {
            current.push(component.as_os_str());
            let metadata =
                fs::symlink_metadata(&current).map_err(|_| "PATH_COMPONENT_UNVERIFIED")?;
            if metadata.file_type().is_symlink()
                || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            {
                return Err("REPARSE_POINT_IN_PATH");
            }
        }
        Ok(())
    }

    fn path_exists(path: &Path) -> bool {
        match fs::symlink_metadata(path) {
            Ok(_) => true,
            Err(error) => error.kind() != io::ErrorKind::NotFound,
        }
    }

    fn remove_empty_verified_sandbox(sandbox: &Path) -> bool {
        let Ok(metadata) = fs::symlink_metadata(sandbox) else {
            return false;
        };
        metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0
            && fs::remove_dir(sandbox).is_ok()
    }

    fn security_info(path: &Path) -> io::Result<SecurityInfo> {
        const SCRIPT: &str = r#"$id = [Security.Principal.WindowsIdentity]::GetCurrent(); $principal = New-Object Security.Principal.WindowsPrincipal($id); $admin = $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator); $owner = (Get-Acl -LiteralPath $env:T10_B04_PROBE_SECURITY_PATH).GetOwner([Security.Principal.SecurityIdentifier]).Value; '{0}|{1}|{2}' -f $id.User.Value, $admin, $owner"#;
        let output = Command::new("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                SCRIPT,
            ])
            .env("T10_B04_PROBE_SECURITY_PATH", path)
            .output()?;
        if !output.status.success() {
            return Err(io::Error::other("security identity query failed"));
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let line = text
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("");
        let fields = line.trim().split('|').collect::<Vec<_>>();
        if fields.len() != 3 || !fields[0].starts_with("S-1-") || !fields[2].starts_with("S-1-") {
            return Err(io::Error::other("security identity output unavailable"));
        }
        let is_administrator = match fields[1].trim().to_ascii_lowercase().as_str() {
            "true" => true,
            "false" => false,
            _ => return Err(io::Error::other("administrator state unavailable")),
        };
        let groups = Command::new("whoami.exe")
            .args(["/groups", "/fo", "csv", "/nh"])
            .output()?;
        if !groups.status.success() {
            return Err(io::Error::other("integrity level query failed"));
        }
        let groups_text = String::from_utf8_lossy(&groups.stdout);
        let high_or_system_integrity =
            groups_text.contains("S-1-16-12288") || groups_text.contains("S-1-16-16384");
        Ok(SecurityInfo {
            current_user_sid: fields[0].to_string(),
            owner_sid: fields[2].to_string(),
            is_elevated: is_administrator || high_or_system_integrity,
        })
    }

    fn windows_build() -> Option<String> {
        let output = Command::new("cmd.exe")
            .args(["/d", "/c", "ver"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        extract_version_number(&text)
    }

    fn extract_version_number(text: &str) -> Option<String> {
        let mut version = String::new();
        let mut started = false;
        for character in text.chars() {
            if character.is_ascii_digit() {
                started = true;
                version.push(character);
            } else if started && character == '.' {
                version.push(character);
            } else if started {
                break;
            }
        }
        if version.contains('.')
            && version
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte == b'.')
        {
            Some(version)
        } else {
            None
        }
    }

    fn local_drive_designator(path: &Path) -> Option<String> {
        let prefix = path.components().find_map(|component| match component {
            Component::Prefix(prefix) => Some(prefix.kind()),
            _ => None,
        })?;
        match prefix {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                Some(format!("{}:", letter as char))
            }
            _ => None,
        }
    }

    fn command_output(program: &str, arguments: &[&str]) -> Option<Output> {
        let output = Command::new(program).args(arguments).output().ok()?;
        output.status.success().then_some(output)
    }

    fn filesystem_name(drive: &str) -> Option<String> {
        let output = command_output("fsutil.exe", &["fsinfo", "volumeinfo", drive])?;
        let text = String::from_utf8_lossy(&output.stdout);
        text.lines().find_map(|line| {
            let (label, value) = line.split_once(':')?;
            label
                .trim()
                .eq_ignore_ascii_case("File System Name")
                .then(|| value.trim().to_string())
        })
    }

    fn free_bytes(drive: &str) -> Option<u64> {
        let output = command_output("fsutil.exe", &["volume", "diskfree", drive])?;
        let text = String::from_utf8_lossy(&output.stdout);
        let mut total_free = None;
        let mut quota_free = None;
        for line in text.lines() {
            let Some((label, value)) = line.split_once(':') else {
                continue;
            };
            let Some(parsed) = value
                .split_whitespace()
                .next()
                .and_then(|value| value.parse::<u64>().ok())
            else {
                continue;
            };
            if label.trim().eq_ignore_ascii_case("Total free bytes") {
                total_free = Some(parsed);
            } else if label.trim().eq_ignore_ascii_case("Total quota free bytes") {
                quota_free = Some(parsed);
            }
        }
        Some(total_free?.min(quota_free?))
    }

    fn budget_expired(start: Instant) -> bool {
        start.elapsed() > MAX_WALL_TIME
    }

    fn skipped_observation(phase: &str, reason: &str) -> Value {
        json!({
            "phase": phase,
            "result": "NOT_ATTEMPTED",
            "reason": reason,
            "win32_error_code": null,
            "rust_raw_os_error": null,
        })
    }

    fn rust_io_observation(
        phase: &str,
        api: &str,
        requested_access: &str,
        result: io::Result<()>,
        details: Value,
    ) -> Value {
        match result {
            Ok(()) => json!({
                "phase": phase,
                "api": api,
                "requested_access": requested_access,
                "result": "SUCCESS",
                "error_kind": null,
                "win32_error_code": null,
                "rust_raw_os_error": null,
                "details": details,
            }),
            Err(error) => {
                let raw = error.raw_os_error();
                json!({
                    "phase": phase,
                    "api": api,
                    "requested_access": requested_access,
                    "result": "ERROR",
                    "error_kind": format!("{:?}", error.kind()),
                    "win32_error_code": raw.and_then(|code| u32::try_from(code).ok()),
                    "rust_raw_os_error": raw,
                    "details": details,
                })
            }
        }
    }

    fn directory_access_case(
        case_id: &str,
        api: &str,
        requested_access: &str,
        flags: Option<u32>,
        start: Instant,
        observations: &mut Vec<Value>,
        open: impl FnOnce() -> io::Result<File>,
    ) {
        if budget_expired(start) {
            observations.push(skipped_observation(
                case_id,
                "30_SECOND_WALL_TIME_BUDGET_EXHAUSTED",
            ));
            return;
        }
        let file = match open() {
            Ok(file) => {
                observations.push(json!({
                    "phase": "directory_open",
                    "case_id": case_id,
                    "api": api,
                    "requested_access": requested_access,
                    "custom_flags": flags,
                    "result": "SUCCESS",
                    "error_kind": null,
                    "win32_error_code": null,
                    "rust_raw_os_error": null,
                }));
                file
            }
            Err(error) => {
                let raw = error.raw_os_error();
                observations.push(json!({
                    "phase": "directory_open",
                    "case_id": case_id,
                    "api": api,
                    "requested_access": requested_access,
                    "custom_flags": flags,
                    "result": "ERROR",
                    "error_kind": format!("{:?}", error.kind()),
                    "win32_error_code": raw.and_then(|code| u32::try_from(code).ok()),
                    "rust_raw_os_error": raw,
                }));
                return;
            }
        };
        if budget_expired(start) {
            observations.push(skipped_observation(
                "directory_sync",
                "30_SECOND_WALL_TIME_BUDGET_EXHAUSTED",
            ));
            return;
        }
        observations.push(rust_io_observation(
            "directory_sync",
            "std::fs::File::sync_all",
            requested_access,
            file.sync_all(),
            json!({
                "case_id": case_id,
                "custom_flags": flags,
                "semantics": "API return only; namespace durability is unproven",
            }),
        ));
    }

    fn probe_directory_baseline(path: &Path, start: Instant, observations: &mut Vec<Value>) {
        directory_access_case(
            "A_FILE_OPEN_BASELINE",
            "std::fs::File::open",
            "Rust File::open default access; no custom flags",
            None,
            start,
            observations,
            || File::open(path),
        );
    }

    fn probe_directory_flagged_read(path: &Path, start: Instant, observations: &mut Vec<Value>) {
        directory_access_case(
            "B_BACKUP_SEMANTICS_GENERIC_READ",
            "std::fs::OpenOptionsExt + CreateFileW via Rust std",
            "GENERIC_READ (0x80000000)",
            Some(FILE_FLAG_BACKUP_SEMANTICS),
            start,
            observations,
            || {
                let mut options = OpenOptions::new();
                options.access_mode(GENERIC_READ);
                options.custom_flags(FILE_FLAG_BACKUP_SEMANTICS);
                options.open(path)
            },
        );
    }

    fn probe_directory_flagged_write(path: &Path, start: Instant, observations: &mut Vec<Value>) {
        directory_access_case(
            "C_BACKUP_SEMANTICS_GENERIC_WRITE",
            "std::fs::OpenOptionsExt + CreateFileW via Rust std",
            "GENERIC_WRITE (0x40000000)",
            Some(FILE_FLAG_BACKUP_SEMANTICS),
            start,
            observations,
            || {
                let mut options = OpenOptions::new();
                options.access_mode(GENERIC_WRITE);
                options.custom_flags(FILE_FLAG_BACKUP_SEMANTICS);
                options.open(path)
            },
        );
    }

    fn file_identity(file: &File) -> Result<FileIdentity, u32> {
        let mut native = NativeFileIdInfo {
            volume_serial_number: 0,
            file_id: [0; 16],
        };
        let buffer_size = std::mem::size_of::<NativeFileIdInfo>() as u32;
        // SAFETY: `file` owns a live Windows handle for the duration of this
        // call, `native` is writable storage with the documented FILE_ID_INFO
        // size/layout, and the class value is FileIdInfo (18). On failure the
        // Win32 error is read immediately below.
        let success = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FILE_ID_INFO_CLASS,
                &mut native as *mut NativeFileIdInfo as *mut std::ffi::c_void,
                buffer_size,
            )
        };
        if success == 0 {
            // SAFETY: This immediately follows the failed Win32 API call.
            return Err(unsafe { GetLastError() });
        }
        Ok(FileIdentity {
            volume_serial_number: native.volume_serial_number,
            file_id: native.file_id,
        })
    }

    fn identity_json(identity: &FileIdentity) -> Value {
        let file_id = identity
            .file_id
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        json!({
            "volume_serial_number": identity.volume_serial_number,
            "file_id_128_hex": file_id,
        })
    }

    fn probe_small_file_and_rename(
        source: &Path,
        destination: &Path,
        start: Instant,
        observations: &mut Vec<Value>,
        owned_file: &mut OwnedFileLocation,
    ) {
        if path_exists(source) || path_exists(destination) {
            observations.push(json!({
                "phase": "file_path_precheck",
                "result": "SAFETY_REJECTION",
                "reason": "GENERATED_NAME_NOT_ABSENT",
            }));
            return;
        }
        if FILE_PAYLOAD.len() as u64 > MAX_FILE_BYTES {
            observations.push(skipped_observation(
                "small_file_create",
                "FILE_BUDGET_EXCEEDED",
            ));
            return;
        }
        if budget_expired(start) {
            observations.push(skipped_observation(
                "small_file_create",
                "30_SECOND_WALL_TIME_BUDGET_EXHAUSTED",
            ));
            return;
        }

        let mut file = match OpenOptions::new().write(true).create_new(true).open(source) {
            Ok(file) => {
                *owned_file = OwnedFileLocation::BeforeRename;
                observations.push(json!({
                    "phase": "small_file_open_create_new",
                    "api": "std::fs::OpenOptions::create_new",
                    "result": "SUCCESS",
                    "file_name": "regular-before-rename.bin",
                    "file_bytes": FILE_PAYLOAD.len(),
                    "sha256": format!("{:x}", Sha256::digest(FILE_PAYLOAD)),
                    "win32_error_code": null,
                    "rust_raw_os_error": null,
                }));
                file
            }
            Err(error) => {
                let raw = error.raw_os_error();
                observations.push(json!({
                    "phase": "small_file_open_create_new",
                    "api": "std::fs::OpenOptions::create_new",
                    "result": "ERROR",
                    "error_kind": format!("{:?}", error.kind()),
                    "win32_error_code": raw.and_then(|code| u32::try_from(code).ok()),
                    "rust_raw_os_error": raw,
                }));
                return;
            }
        };

        if budget_expired(start) {
            observations.push(skipped_observation(
                "small_file_write",
                "30_SECOND_WALL_TIME_BUDGET_EXHAUSTED",
            ));
            return;
        }
        let write_result = file.write_all(FILE_PAYLOAD);
        observations.push(rust_io_observation(
            "small_file_write",
            "std::io::Write::write_all",
            "write-only newly created file",
            write_result,
            json!({
                "file_bytes": FILE_PAYLOAD.len(),
                "sha256": format!("{:x}", Sha256::digest(FILE_PAYLOAD)),
            }),
        ));
        if observations
            .last()
            .is_some_and(|item| item["result"] == "ERROR")
        {
            return;
        }

        if budget_expired(start) {
            observations.push(skipped_observation(
                "small_file_sync",
                "30_SECOND_WALL_TIME_BUDGET_EXHAUSTED",
            ));
            return;
        }
        let sync_result = file.sync_all();
        observations.push(rust_io_observation(
            "small_file_sync",
            "std::fs::File::sync_all",
            "write-capable regular-file handle",
            sync_result,
            json!({
                "file_bytes": FILE_PAYLOAD.len(),
                "sha256": format!("{:x}", Sha256::digest(FILE_PAYLOAD)),
            }),
        ));
        if observations
            .last()
            .is_some_and(|item| item["result"] == "ERROR")
        {
            return;
        }

        let before_identity = if budget_expired(start) {
            observations.push(skipped_observation(
                "file_identity_before_rename",
                "30_SECOND_WALL_TIME_BUDGET_EXHAUSTED",
            ));
            None
        } else {
            match file_identity(&file) {
                Ok(identity) => {
                    observations.push(json!({
                        "phase": "file_identity_before_rename",
                        "api": "GetFileInformationByHandleEx(FILE_ID_INFO) on open handle",
                        "result": "API_RETURN_OBSERVED",
                        "identity": identity_json(&identity),
                    }));
                    Some(identity)
                }
                Err(code) => {
                    observations.push(json!({
                        "phase": "file_identity_before_rename",
                        "api": "GetFileInformationByHandleEx(FILE_ID_INFO) on open handle",
                        "result": "ENVIRONMENT_UNAVAILABLE",
                        "win32_error_code": code,
                        "rust_raw_os_error": null,
                    }));
                    None
                }
            }
        };
        drop(file);

        if path_exists(destination) {
            observations.push(json!({
                "phase": "rename_destination_precheck",
                "result": "SAFETY_REJECTION",
                "reason": "DESTINATION_ALREADY_EXISTS",
            }));
            return;
        }
        if budget_expired(start) {
            observations.push(skipped_observation(
                "same_directory_rename",
                "30_SECOND_WALL_TIME_BUDGET_EXHAUSTED",
            ));
            return;
        }
        match move_file_write_through(source, destination) {
            Ok(()) => {
                *owned_file = OwnedFileLocation::AfterRename;
                observations.push(json!({
                    "phase": "same_directory_rename",
                    "api": "MoveFileExW",
                    "flags": {"MOVEFILE_WRITE_THROUGH": MOVEFILE_WRITE_THROUGH},
                    "destination_policy": "no MOVEFILE_REPLACE_EXISTING; no copy/cross-volume fallback",
                    "result": "API_RETURN_OBSERVED",
                    "win32_error_code": null,
                    "rust_raw_os_error": null,
                    "durability_claim": "NONE_API_RETURN_OBSERVED_ONLY",
                }));
            }
            Err(MoveFileFailure::Win32(code)) => observations.push(json!({
                "phase": "same_directory_rename",
                "api": "MoveFileExW",
                "flags": {"MOVEFILE_WRITE_THROUGH": MOVEFILE_WRITE_THROUGH},
                "destination_policy": "no MOVEFILE_REPLACE_EXISTING; no copy/cross-volume fallback",
                "result": "ERROR",
                "win32_error_code": code,
                "rust_raw_os_error": null,
                "durability_claim": "NONE_API_RETURN_OBSERVED_ONLY",
            })),
            Err(MoveFileFailure::PathEncoding) => observations.push(json!({
                "phase": "same_directory_rename",
                "api": "MoveFileExW",
                "flags": {"MOVEFILE_WRITE_THROUGH": MOVEFILE_WRITE_THROUGH},
                "result": "SAFETY_REJECTION",
                "reason": "PATH_ENCODING_REJECTED_BEFORE_WIN32_CALL",
                "win32_error_code": null,
                "rust_raw_os_error": null,
            })),
        }
        if observations
            .last()
            .is_some_and(|item| item["result"] != "API_RETURN_OBSERVED")
        {
            return;
        }

        if budget_expired(start) {
            observations.push(skipped_observation(
                "readonly_reopen_and_identity",
                "30_SECOND_WALL_TIME_BUDGET_EXHAUSTED",
            ));
            return;
        }
        let reopened = match File::open(destination) {
            Ok(file) => {
                observations.push(json!({
                    "phase": "readonly_reopen_after_rename",
                    "api": "std::fs::File::open",
                    "requested_access": "read-only",
                    "result": "SUCCESS",
                    "file_name": "regular-after-rename.bin",
                    "win32_error_code": null,
                    "rust_raw_os_error": null,
                }));
                file
            }
            Err(error) => {
                let raw = error.raw_os_error();
                observations.push(json!({
                    "phase": "readonly_reopen_after_rename",
                    "api": "std::fs::File::open",
                    "requested_access": "read-only",
                    "result": "ERROR",
                    "error_kind": format!("{:?}", error.kind()),
                    "win32_error_code": raw.and_then(|code| u32::try_from(code).ok()),
                    "rust_raw_os_error": raw,
                }));
                return;
            }
        };

        if budget_expired(start) {
            observations.push(skipped_observation(
                "file_identity_after_rename",
                "30_SECOND_WALL_TIME_BUDGET_EXHAUSTED",
            ));
            observations.push(skipped_observation(
                "readonly_reopen_content_check",
                "30_SECOND_WALL_TIME_BUDGET_EXHAUSTED",
            ));
            return;
        }
        match file_identity(&reopened) {
            Ok(after_identity) => {
                let matches = before_identity.as_ref().is_some_and(|before| {
                    before.volume_serial_number == after_identity.volume_serial_number
                        && before.file_id == after_identity.file_id
                });
                observations.push(json!({
                    "phase": "file_identity_after_rename",
                    "api": "GetFileInformationByHandleEx(FILE_ID_INFO) on read-only reopened handle",
                    "result": if identity_is_known(before_identity.as_ref()) { "API_RETURN_OBSERVED" } else { "ENVIRONMENT_UNAVAILABLE" },
                    "identity": identity_json(&after_identity),
                    "before_after_identity_matches": before_identity.as_ref().map(|_| matches),
                }));
            }
            Err(code) => {
                observations.push(json!({
                    "phase": "file_identity_after_rename",
                    "api": "GetFileInformationByHandleEx(FILE_ID_INFO) on read-only reopened handle",
                    "result": "ENVIRONMENT_UNAVAILABLE",
                    "win32_error_code": code,
                    "rust_raw_os_error": null,
                }));
            }
        }

        if budget_expired(start) {
            observations.push(skipped_observation(
                "readonly_reopen_content_check",
                "30_SECOND_WALL_TIME_BUDGET_EXHAUSTED",
            ));
            return;
        }
        let mut bounded = reopened.take(MAX_FILE_BYTES + 1);
        let mut bytes = Vec::new();
        match bounded.read_to_end(&mut bytes) {
            Ok(read) if read as u64 <= MAX_FILE_BYTES => {
                let digest = format!("{:x}", Sha256::digest(&bytes));
                let expected_digest = format!("{:x}", Sha256::digest(FILE_PAYLOAD));
                let digest_matches = digest == expected_digest;
                observations.push(json!({
                    "phase": "readonly_reopen_content_check",
                    "api": "std::io::Read::read_to_end bounded to 1 MiB + 1",
                    "result": "API_RETURN_OBSERVED",
                    "file_bytes": read,
                    "sha256": digest,
                    "payload_matches": bytes.as_slice() == FILE_PAYLOAD,
                    "before_after_sha256_matches": digest_matches,
                }));
            }
            Ok(read) => observations.push(json!({
                "phase": "readonly_reopen_content_check",
                "result": "SAFETY_REJECTION",
                "file_bytes": read,
                "reason": "READ_LIMIT_EXCEEDED",
            })),
            Err(error) => {
                let raw = error.raw_os_error();
                observations.push(json!({
                    "phase": "readonly_reopen_content_check",
                    "result": "ERROR",
                    "error_kind": format!("{:?}", error.kind()),
                    "win32_error_code": raw.and_then(|code| u32::try_from(code).ok()),
                    "rust_raw_os_error": raw,
                }));
            }
        }
    }

    enum MoveFileFailure {
        Win32(u32),
        PathEncoding,
    }

    fn move_file_write_through(source: &Path, destination: &Path) -> Result<(), MoveFileFailure> {
        let source_wide =
            nul_terminated_wide(source.as_os_str()).map_err(|_| MoveFileFailure::PathEncoding)?;
        let destination_wide = nul_terminated_wide(destination.as_os_str())
            .map_err(|_| MoveFileFailure::PathEncoding)?;
        // SAFETY: Both paths are generated direct children of the verified,
        // newly-created sandbox; they contain no embedded NUL; the destination
        // was checked absent; and flags omit REPLACE_EXISTING and COPY_ALLOWED.
        // The paths are in the same directory/volume. GetLastError is read
        // immediately after a failed Win32 call.
        let result = unsafe {
            MoveFileExW(
                source_wide.as_ptr(),
                destination_wide.as_ptr(),
                MOVEFILE_WRITE_THROUGH,
            )
        };
        if result != 0 {
            Ok(())
        } else {
            // SAFETY: This follows the failed Win32 API call immediately.
            Err(MoveFileFailure::Win32(unsafe { GetLastError() }))
        }
    }

    fn nul_terminated_wide(value: &OsStr) -> io::Result<Vec<u16>> {
        let mut wide = value.encode_wide().collect::<Vec<_>>();
        if wide.contains(&0) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "embedded NUL"));
        }
        wide.push(0);
        Ok(wide)
    }

    fn cleanup_known_paths(
        sandbox: &Path,
        source: &Path,
        destination: &Path,
        owned_file: OwnedFileLocation,
        observations: &mut Vec<Value>,
    ) -> &'static str {
        let mut complete = true;
        let owned_path = match owned_file {
            OwnedFileLocation::None => None,
            OwnedFileLocation::BeforeRename => Some(("regular-before-rename.bin", source)),
            OwnedFileLocation::AfterRename => Some(("regular-after-rename.bin", destination)),
        };
        if let Some((name, path)) = owned_path {
            match fs::symlink_metadata(path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => {
                    complete = false;
                    observations.push(rust_io_observation(
                        "cleanup_file_inspect",
                        "std::fs::symlink_metadata",
                        "exact generated child only",
                        Err(error),
                        json!({"file_name": name}),
                    ));
                }
                Ok(metadata)
                    if metadata.file_type().is_file()
                        && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0 =>
                {
                    if let Err(error) = fs::remove_file(path) {
                        complete = false;
                        observations.push(rust_io_observation(
                            "cleanup_file_remove",
                            "std::fs::remove_file",
                            "exact generated regular file only",
                            Err(error),
                            json!({"file_name": name}),
                        ));
                    }
                }
                Ok(_) => {
                    complete = false;
                    observations.push(json!({
                        "phase": "cleanup_file_remove",
                        "result": "CLEANUP_INCOMPLETE",
                        "file_name": name,
                        "reason": "UNEXPECTED_FILE_TYPE_OR_REPARSE_POINT; left untouched",
                    }));
                }
            }
        }

        match fs::symlink_metadata(sandbox) {
            Ok(metadata)
                if metadata.is_dir()
                    && !metadata.file_type().is_symlink()
                    && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0 =>
            {
                if let Err(error) = fs::remove_dir(sandbox) {
                    complete = false;
                    observations.push(rust_io_observation(
                        "cleanup_sandbox_remove",
                        "std::fs::remove_dir",
                        "empty unique sandbox directory only; no recursion",
                        Err(error),
                        json!({"sandbox": "RUNNER_TEMP/<unique-direct-child>"}),
                    ));
                }
            }
            _ => {
                complete = false;
                observations.push(json!({
                    "phase": "cleanup_sandbox_remove",
                    "result": "CLEANUP_INCOMPLETE",
                    "reason": "SANDBOX_TYPE_CHANGED_OR_UNVERIFIED; left untouched",
                }));
            }
        }
        if complete {
            "CLEANUP_COMPLETE"
        } else {
            "CLEANUP_INCOMPLETE"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_nonexact_consent_keeps_probe_disabled() {
        assert_eq!(consent(None), Consent::Disabled);
        assert_eq!(consent(Some("0")), Consent::Disabled);
        assert_eq!(consent(Some("true")), Consent::Disabled);
        assert_eq!(consent(Some("1")), Consent::Enabled);
    }

    #[test]
    fn system_and_user_roots_are_rejected() {
        assert_eq!(
            validate_temp_scope("C:\\Windows\\Temp", &["C:\\Windows"], true),
            Err(ScopeError::ProtectedPath)
        );
        assert_eq!(
            validate_temp_scope(
                "C:\\Users\\runner\\AppData\\Local\\Temp",
                &["C:\\Users\\runner"],
                true
            ),
            Err(ScopeError::ProtectedPath)
        );
        assert_eq!(
            validate_temp_scope("D:\\", &[], true),
            Err(ScopeError::VolumeRoot)
        );
    }

    #[test]
    fn reparse_escape_is_rejected() {
        assert_eq!(
            validate_temp_scope("D:\\a\\_temp", &[], false),
            Err(ScopeError::ReparsePath)
        );
        assert_eq!(
            validate_temp_scope("D:\\a\\..\\private", &[], true),
            Err(ScopeError::NotLocalAbsolute)
        );
    }

    #[test]
    fn missing_or_wrong_filesystem_is_not_ntfs() {
        assert!(!ntfs_is_verified(None));
        assert!(!ntfs_is_verified(Some("ReFS")));
        assert!(ntfs_is_verified(Some("ntfs")));
    }

    #[test]
    fn oversize_file_count_or_duration_is_rejected() {
        assert!(budget_is_valid(
            FILE_PAYLOAD.len() as u64,
            1,
            Duration::from_secs(1)
        ));
        assert!(!budget_is_valid(
            MAX_FILE_BYTES + 1,
            1,
            Duration::from_secs(1)
        ));
        assert!(!budget_is_valid(
            1,
            MAX_DISPOSABLE_FILES + 1,
            Duration::from_secs(1)
        ));
        assert!(!budget_is_valid(
            1,
            1,
            MAX_WALL_TIME + Duration::from_millis(1)
        ));
    }

    #[test]
    fn unknown_object_identity_is_not_accepted() {
        assert!(!identity_is_known::<u64>(None));
        assert!(identity_is_known(Some(7_u64)));
    }

    #[test]
    fn source_identity_requires_sha1_length_and_hex() {
        assert!(valid_git_object_id(
            "0123456789abcdef0123456789abcdef01234567"
        ));
        assert!(!valid_git_object_id("not-a-commit"));
        assert!(!valid_git_object_id(
            "0123456789abcdef0123456789abcdef0123456g"
        ));
    }

    #[test]
    fn non_windows_never_enables_native_probe() {
        assert!(!native_windows_probe_allowed("linux"));
        assert!(!native_windows_probe_allowed("macos"));
        assert!(native_windows_probe_allowed("windows"));
    }
}
