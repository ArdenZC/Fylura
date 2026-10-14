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
#[cfg(target_os = "windows")]
const EXPECTED_BINARY_SHA256_VARIABLE: &str = "T10_B04_EXPECTED_BINARY_SHA256";
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
fn valid_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(any(target_os = "windows", test))]
fn ntfs_is_verified(value: Option<&str>) -> bool {
    value.is_some_and(|filesystem| filesystem.eq_ignore_ascii_case("NTFS"))
}

#[cfg(any(target_os = "windows", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileIdentity {
    volume_serial_number: u64,
    file_id: [u8; 16],
}

#[cfg(any(target_os = "windows", test))]
fn identity_is_known(identity: Option<&FileIdentity>) -> bool {
    identity.is_some_and(|identity| {
        identity.volume_serial_number != 0
            && identity.volume_serial_number != u64::MAX
            && identity.file_id.iter().any(|byte| *byte != 0)
            && !identity.file_id.iter().all(|byte| *byte == u8::MAX)
    })
}

#[cfg(any(target_os = "windows", test))]
fn identities_match(expected: Option<&FileIdentity>, observed: Option<&FileIdentity>) -> bool {
    identity_is_known(expected) && identity_is_known(observed) && expected == observed
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProbeStatus {
    NotExecuted,
    FileOperationObserved,
    SafetyRejection,
    EnvironmentUnavailable,
    ProbeIncomplete,
    CleanupIncomplete,
}

impl ProbeStatus {
    #[cfg(target_os = "windows")]
    fn as_str(self) -> &'static str {
        match self {
            Self::NotExecuted => "NOT_EXECUTED",
            Self::FileOperationObserved => "FILE_OPERATION_OBSERVED",
            Self::SafetyRejection => "SAFETY_REJECTION",
            Self::EnvironmentUnavailable => "ENVIRONMENT_UNAVAILABLE",
            Self::ProbeIncomplete => "PROBE_INCOMPLETE",
            Self::CleanupIncomplete => "CLEANUP_INCOMPLETE",
        }
    }

    fn exit_code(self) -> u8 {
        match self {
            Self::NotExecuted | Self::FileOperationObserved => 0,
            Self::SafetyRejection => 2,
            Self::EnvironmentUnavailable => 3,
            Self::ProbeIncomplete => 4,
            Self::CleanupIncomplete => 5,
        }
    }
}

fn status_from_name(status: &str) -> ProbeStatus {
    match status {
        "NOT_EXECUTED" => ProbeStatus::NotExecuted,
        "FILE_OPERATION_OBSERVED" => ProbeStatus::FileOperationObserved,
        "ENVIRONMENT_UNAVAILABLE" => ProbeStatus::EnvironmentUnavailable,
        "PROBE_INCOMPLETE" => ProbeStatus::ProbeIncomplete,
        "CLEANUP_INCOMPLETE" => ProbeStatus::CleanupIncomplete,
        _ => ProbeStatus::SafetyRejection,
    }
}

#[cfg(any(target_os = "windows", test))]
fn final_probe_status(primary: ProbeStatus, cleanup_complete: bool) -> ProbeStatus {
    if cleanup_complete {
        primary
    } else {
        ProbeStatus::CleanupIncomplete
    }
}

#[cfg(any(target_os = "windows", test))]
fn rename_identity_gate_allows(
    identity_at_creation: Option<&FileIdentity>,
    identity_at_path: Option<&FileIdentity>,
) -> bool {
    identities_match(identity_at_creation, identity_at_path)
}

#[cfg(any(target_os = "windows", test))]
fn required_phase_outcome<'a>(
    observations: &'a [Value],
    phase: &str,
    case_id: Option<&str>,
) -> Option<&'a str> {
    observations.iter().find_map(|observation| {
        if observation["phase"].as_str() != Some(phase) {
            return None;
        }
        if let Some(expected_case) = case_id
            && observation["case_id"].as_str() != Some(expected_case)
        {
            return None;
        }
        observation["result"].as_str()
    })
}

#[cfg(any(target_os = "windows", test))]
fn required_phases_complete(observations: &[Value]) -> bool {
    let directory_cases = [
        "A_FILE_OPEN_BASELINE",
        "B_BACKUP_SEMANTICS_GENERIC_READ",
        "C_BACKUP_SEMANTICS_GENERIC_WRITE",
    ];
    for case_id in directory_cases {
        match required_phase_outcome(observations, "directory_open", Some(case_id)) {
            Some("SUCCESS") => {
                if !matches!(
                    required_phase_outcome(observations, "directory_sync", Some(case_id)),
                    Some("SUCCESS" | "ERROR")
                ) {
                    return false;
                }
            }
            Some("ERROR") => {}
            _ => return false,
        }
    }

    if required_phase_outcome(observations, "small_file_open_create_new", None) != Some("SUCCESS")
        || required_phase_outcome(observations, "file_identity_at_creation", None)
            != Some("API_RETURN_OBSERVED")
        || required_phase_outcome(observations, "small_file_write", None) != Some("SUCCESS")
        || required_phase_outcome(observations, "small_file_sync", None) != Some("SUCCESS")
        || required_phase_outcome(observations, "file_identity_before_rename", None)
            != Some("API_RETURN_OBSERVED")
        || required_phase_outcome(observations, "source_reopen_before_rename", None)
            != Some("SUCCESS")
        || required_phase_outcome(observations, "same_directory_rename", None)
            != Some("API_RETURN_OBSERVED")
        || required_phase_outcome(observations, "readonly_reopen_after_rename", None)
            != Some("SUCCESS")
        || required_phase_outcome(observations, "file_identity_after_rename", None)
            != Some("API_RETURN_OBSERVED")
        || required_phase_outcome(observations, "readonly_reopen_content_check", None)
            != Some("API_RETURN_OBSERVED")
    {
        return false;
    }

    let Some(post_rename_identity) = observations
        .iter()
        .find(|observation| observation["phase"] == "file_identity_after_rename")
    else {
        return false;
    };
    let Some(pre_rename_identity) = observations
        .iter()
        .find(|observation| observation["phase"] == "file_identity_before_rename")
    else {
        return false;
    };
    let Some(content_check) = observations
        .iter()
        .find(|observation| observation["phase"] == "readonly_reopen_content_check")
    else {
        return false;
    };
    pre_rename_identity["creation_before_rename_identity_matches"] == true
        && post_rename_identity["before_after_identity_matches"] == true
        && content_check["payload_matches"] == true
        && content_check["before_after_sha256_matches"] == true
}

#[cfg(any(target_os = "windows", test))]
fn aggregate_probe_status(observations: &[Value]) -> (ProbeStatus, &'static str) {
    if observations
        .iter()
        .any(|observation| observation["result"] == "SAFETY_REJECTION")
    {
        return (ProbeStatus::SafetyRejection, "SAFETY_CHECK_REJECTED");
    }
    if observations
        .iter()
        .any(|observation| observation["result"] == "ENVIRONMENT_UNAVAILABLE")
    {
        return (
            ProbeStatus::EnvironmentUnavailable,
            "REQUIRED_CAPABILITY_UNAVAILABLE",
        );
    }
    if observations
        .iter()
        .any(|observation| observation["result"] == "NOT_ATTEMPTED")
    {
        return (ProbeStatus::ProbeIncomplete, "REQUIRED_PHASE_NOT_ATTEMPTED");
    }
    if !required_phases_complete(observations) {
        return (
            ProbeStatus::ProbeIncomplete,
            "REQUIRED_PHASE_MISSING_OR_FAILED",
        );
    }
    (
        ProbeStatus::FileOperationObserved,
        "ALL_REQUIRED_PHASES_COMPLETE_API_RETURNS_ONLY",
    )
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
    primary_status: &str,
    reason: Option<&str>,
    source_head: Option<&str>,
    source_tree: Option<&str>,
    binary_sha256: Option<&str>,
    os_build: Option<&str>,
    filesystem: Option<&str>,
    free_bytes: Option<u64>,
    observations: &[Value],
    cleanup: Option<&str>,
) {
    let cleanup_failure = observations
        .iter()
        .find(|observation| observation["result"] == "CLEANUP_INCOMPLETE");
    let record = json!({
        "schema": "zen-canvas-t10-b04-ntfs-probe-v1",
        "status": status,
        "primary_status": primary_status,
        "exit_code": status_from_name(status).exit_code(),
        "reason": reason,
        "primary_outcome": {
            "status": primary_status,
            "reason": reason,
        },
        "cleanup_failure": cleanup_failure,
        "source": {"head": source_head, "tree": source_tree, "binary_sha256": binary_sha256},
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
fn main() -> std::process::ExitCode {
    if !native_windows_probe_allowed(std::env::consts::OS) {
        emit_record(
            "NOT_EXECUTED",
            "NOT_EXECUTED",
            Some("NON_WINDOWS_BUILD_NO_NATIVE_PROBE"),
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            Some("NO_SANDBOX_CREATED"),
        );
        return std::process::ExitCode::SUCCESS;
    }
    if consent(std::env::var(CONSENT_VARIABLE).ok().as_deref()) == Consent::Disabled {
        emit_record(
            "NOT_EXECUTED",
            "NOT_EXECUTED",
            Some("CONSENT_GATE_NOT_SET_TO_1"),
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            Some("NO_SANDBOX_CREATED"),
        );
        return std::process::ExitCode::SUCCESS;
    }

    if std::env::args_os().len() != 1 {
        emit_record(
            "SAFETY_REJECTION",
            "SAFETY_REJECTION",
            Some("UNEXPECTED_ARGUMENTS"),
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            Some("NO_SANDBOX_CREATED"),
        );
        return std::process::ExitCode::from(ProbeStatus::SafetyRejection.exit_code());
    }

    std::process::ExitCode::from(windows_probe::run())
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
        "NOT_EXECUTED",
        Some(reason),
        None,
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
        binary_sha256: Option<String>,
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
                binary_sha256: None,
                os_build: None,
                filesystem: None,
                free_bytes: None,
                observations: Vec::new(),
            }
        }

        fn record(&self, status: &str, reason: Option<&str>, cleanup: Option<&str>) {
            self.record_with_primary(status, status, reason, cleanup);
        }

        fn record_with_primary(
            &self,
            status: &str,
            primary_status: &str,
            reason: Option<&str>,
            cleanup: Option<&str>,
        ) {
            emit_record(
                status,
                primary_status,
                reason,
                self.source_head.as_deref(),
                self.source_tree.as_deref(),
                self.binary_sha256.as_deref(),
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

    #[derive(Default)]
    struct OwnedFile {
        was_created: bool,
        identity: Option<FileIdentity>,
    }

    fn preflight_failure(context: &Context, status: &str, reason: &str) -> u8 {
        context.record(status, Some(reason), Some("NO_SANDBOX_CREATED"));
        status_from_name(status).exit_code()
    }

    pub(super) fn run() -> u8 {
        let start = Instant::now();
        let mut context = Context::new();

        let source = match verify_source_identity() {
            Ok(source) => source,
            Err(reason) => {
                return preflight_failure(&context, "SAFETY_REJECTION", reason);
            }
        };
        context.source_head = Some(source.0);
        context.source_tree = Some(source.1);

        let binary_sha256 = match verify_binary_identity() {
            Ok(digest) => digest,
            Err(reason) => {
                return preflight_failure(&context, "SAFETY_REJECTION", reason);
            }
        };
        context.binary_sha256 = Some(binary_sha256);

        let runner_temp = match std::env::var_os("RUNNER_TEMP") {
            Some(path) => PathBuf::from(path),
            None => {
                return preflight_failure(
                    &context,
                    "ENVIRONMENT_UNAVAILABLE",
                    "RUNNER_TEMP_MISSING",
                );
            }
        };
        if !runner_temp.is_absolute() {
            return preflight_failure(&context, "SAFETY_REJECTION", "RUNNER_TEMP_NOT_ABSOLUTE");
        }
        if let Err(reason) = ensure_non_reparse_components(&runner_temp) {
            return preflight_failure(&context, "SAFETY_REJECTION", reason);
        }
        let canonical_temp = match fs::canonicalize(&runner_temp) {
            Ok(path) => path,
            Err(_) => {
                return preflight_failure(
                    &context,
                    "ENVIRONMENT_UNAVAILABLE",
                    "RUNNER_TEMP_UNRESOLVED",
                );
            }
        };
        if !canonical_temp.is_dir() {
            return preflight_failure(&context, "SAFETY_REJECTION", "RUNNER_TEMP_NOT_DIRECTORY");
        }
        let canonical_text = canonical_temp.to_string_lossy().into_owned();
        let protected_roots = match protected_roots() {
            Some(roots) => roots,
            None => {
                return preflight_failure(
                    &context,
                    "ENVIRONMENT_UNAVAILABLE",
                    "USER_OR_SYSTEM_ROOT_UNAVAILABLE",
                );
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
            return preflight_failure(
                &context,
                "SAFETY_REJECTION",
                "RUNNER_TEMP_OUTSIDE_SAFE_SCOPE",
            );
        }

        let security = match security_info(&canonical_temp) {
            Ok(info) => info,
            Err(_) => {
                return preflight_failure(
                    &context,
                    "ENVIRONMENT_UNAVAILABLE",
                    "RUNNER_IDENTITY_OR_OWNER_UNAVAILABLE",
                );
            }
        };
        if security.is_elevated {
            return preflight_failure(&context, "SAFETY_REJECTION", "ELEVATED_TOKEN_REJECTED");
        }
        if security.current_user_sid != security.owner_sid {
            return preflight_failure(&context, "SAFETY_REJECTION", "RUNNER_TEMP_OWNER_MISMATCH");
        }

        let os_build = match windows_build() {
            Some(build) => build,
            None => {
                return preflight_failure(
                    &context,
                    "ENVIRONMENT_UNAVAILABLE",
                    "WINDOWS_BUILD_UNAVAILABLE",
                );
            }
        };
        context.os_build = Some(os_build);

        let drive = match local_drive_designator(&canonical_temp) {
            Some(drive) => drive,
            None => {
                return preflight_failure(
                    &context,
                    "SAFETY_REJECTION",
                    "RUNNER_TEMP_NOT_LOCAL_DRIVE",
                );
            }
        };
        let filesystem = match filesystem_name(&drive) {
            Some(filesystem) if ntfs_is_verified(Some(&filesystem)) => filesystem,
            Some(_) => {
                return preflight_failure(
                    &context,
                    "ENVIRONMENT_UNAVAILABLE",
                    "FILESYSTEM_NOT_NTFS",
                );
            }
            None => {
                return preflight_failure(
                    &context,
                    "ENVIRONMENT_UNAVAILABLE",
                    "FILESYSTEM_TYPE_UNAVAILABLE",
                );
            }
        };
        context.filesystem = Some(filesystem);

        let free_bytes = match free_bytes(&drive) {
            Some(free) if free >= MAX_FILE_BYTES => free,
            Some(_) => {
                return preflight_failure(
                    &context,
                    "ENVIRONMENT_UNAVAILABLE",
                    "STORAGE_BUDGET_UNAVAILABLE",
                );
            }
            None => {
                return preflight_failure(
                    &context,
                    "ENVIRONMENT_UNAVAILABLE",
                    "FREE_SPACE_UNAVAILABLE",
                );
            }
        };
        context.free_bytes = Some(free_bytes);

        let expected_payload_bytes = FILE_PAYLOAD.len() as u64;
        if !budget_is_valid(expected_payload_bytes, 1, start.elapsed()) {
            return preflight_failure(&context, "SAFETY_REJECTION", "STATIC_PROBE_BUDGET_EXCEEDED");
        }

        let run_id = format!(
            "b04-ntfs-probe-{}-{}",
            std::process::id(),
            start.elapsed().as_nanos()
        );
        let sandbox = canonical_temp.join(run_id);
        if path_exists(&sandbox) {
            return preflight_failure(&context, "SAFETY_REJECTION", "SANDBOX_NAME_ALREADY_EXISTS");
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
            return ProbeStatus::EnvironmentUnavailable.exit_code();
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
                    context.observations.push(json!({
                        "phase": "cleanup_sandbox_remove",
                        "result": "CLEANUP_INCOMPLETE",
                        "reason": "SANDBOX_OWNER_RECHECK_FAILED; sandbox left untouched",
                    }));
                }
                let primary_status = if cleanup_state == "CLEANUP_COMPLETE" {
                    ProbeStatus::SafetyRejection
                } else {
                    ProbeStatus::CleanupIncomplete
                };
                context.record_with_primary(
                    if cleanup_state == "CLEANUP_COMPLETE" {
                        ProbeStatus::SafetyRejection.as_str()
                    } else {
                        ProbeStatus::CleanupIncomplete.as_str()
                    },
                    ProbeStatus::SafetyRejection.as_str(),
                    Some("SANDBOX_OWNER_OR_TOKEN_RECHECK_FAILED"),
                    Some(cleanup_state),
                );
                return primary_status.exit_code();
            }
        }
        if ensure_non_reparse_components(&sandbox).is_err() {
            if !remove_empty_verified_sandbox(&sandbox) {
                cleanup_state = "CLEANUP_INCOMPLETE";
                context.observations.push(json!({
                    "phase": "cleanup_sandbox_remove",
                    "result": "CLEANUP_INCOMPLETE",
                    "reason": "SANDBOX_REPARSE_RECHECK_FAILED; sandbox left untouched",
                }));
            }
            let final_status = if cleanup_state == "CLEANUP_COMPLETE" {
                ProbeStatus::SafetyRejection
            } else {
                ProbeStatus::CleanupIncomplete
            };
            context.record_with_primary(
                if cleanup_state == "CLEANUP_COMPLETE" {
                    ProbeStatus::SafetyRejection.as_str()
                } else {
                    ProbeStatus::CleanupIncomplete.as_str()
                },
                ProbeStatus::SafetyRejection.as_str(),
                Some("SANDBOX_REPARSE_POINT_REJECTED"),
                Some(cleanup_state),
            );
            return final_status.exit_code();
        }

        probe_directory_baseline(&sandbox, start, &mut context.observations);
        probe_directory_flagged_read(&sandbox, start, &mut context.observations);
        probe_directory_flagged_write(&sandbox, start, &mut context.observations);

        let source_name = "regular-before-rename.bin";
        let destination_name = "regular-after-rename.bin";
        let source_path = sandbox.join(source_name);
        let destination_path = sandbox.join(destination_name);
        let mut owned_file = OwnedFile::default();
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
            &owned_file,
            &mut context.observations,
        );
        let (primary_status, reason) = aggregate_probe_status(&context.observations);
        let final_status =
            final_probe_status(primary_status, cleanup_state != "CLEANUP_INCOMPLETE");
        context.record_with_primary(
            final_status.as_str(),
            primary_status.as_str(),
            Some(reason),
            Some(cleanup_state),
        );
        final_status.exit_code()
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

    fn verify_binary_identity() -> Result<String, &'static str> {
        const MAX_EXECUTABLE_HASH_BYTES: u64 = 256 * 1024 * 1024;
        let expected = std::env::var(EXPECTED_BINARY_SHA256_VARIABLE)
            .map_err(|_| "EXPECTED_BINARY_SHA256_MISSING")?;
        if !valid_sha256_hex(&expected) {
            return Err("EXPECTED_BINARY_SHA256_INVALID");
        }
        let executable = std::env::current_exe().map_err(|_| "EXECUTABLE_PATH_UNAVAILABLE")?;
        let mut file = File::open(executable).map_err(|_| "EXECUTABLE_OPEN_FAILED")?;
        let mut hasher = Sha256::new();
        let mut total = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file
                .read(&mut buffer)
                .map_err(|_| "EXECUTABLE_HASH_READ_FAILED")?;
            if read == 0 {
                break;
            }
            total = total.saturating_add(read as u64);
            if total > MAX_EXECUTABLE_HASH_BYTES {
                return Err("EXECUTABLE_HASH_SIZE_LIMIT_EXCEEDED");
            }
            hasher.update(&buffer[..read]);
        }
        let actual = format!("{:x}", hasher.finalize());
        if !actual.eq_ignore_ascii_case(&expected) {
            return Err("EXECUTABLE_SHA256_MISMATCH");
        }
        Ok(actual)
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
        let mut sync_observation = rust_io_observation(
            "directory_sync",
            "std::fs::File::sync_all",
            requested_access,
            file.sync_all(),
            json!({
                "case_id": case_id,
                "custom_flags": flags,
                "semantics": "API return only; namespace durability is unproven",
            }),
        );
        sync_observation["case_id"] = json!(case_id);
        observations.push(sync_observation);
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
        owned_file: &mut OwnedFile,
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
                owned_file.was_created = true;
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
                "file_identity_at_creation",
                "30_SECOND_WALL_TIME_BUDGET_EXHAUSTED",
            ));
            return;
        }
        let created_identity = match file_identity(&file) {
            Ok(identity) => {
                owned_file.identity = Some(identity);
                if !identity_is_known(Some(&identity)) {
                    observations.push(json!({
                        "phase": "file_identity_at_creation",
                        "api": "GetFileInformationByHandleEx(FILE_ID_INFO) on newly created handle",
                        "result": "SAFETY_REJECTION",
                        "reason": "ZERO_OR_PLACEHOLDER_FILE_ID",
                        "identity": identity_json(&identity),
                    }));
                    return;
                }
                observations.push(json!({
                    "phase": "file_identity_at_creation",
                    "api": "GetFileInformationByHandleEx(FILE_ID_INFO) on newly created handle",
                    "result": "API_RETURN_OBSERVED",
                    "identity": identity_json(&identity),
                }));
                identity
            }
            Err(code) => {
                observations.push(json!({
                    "phase": "file_identity_at_creation",
                    "api": "GetFileInformationByHandleEx(FILE_ID_INFO) on newly created handle",
                    "result": "ENVIRONMENT_UNAVAILABLE",
                    "win32_error_code": code,
                    "rust_raw_os_error": null,
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

        if budget_expired(start) {
            observations.push(skipped_observation(
                "file_identity_before_rename",
                "30_SECOND_WALL_TIME_BUDGET_EXHAUSTED",
            ));
            return;
        }
        let current_handle_identity = match file_identity(&file) {
            Ok(identity) => identity,
            Err(code) => {
                observations.push(json!({
                    "phase": "file_identity_before_rename",
                    "api": "GetFileInformationByHandleEx(FILE_ID_INFO) on open handle",
                    "result": "ENVIRONMENT_UNAVAILABLE",
                    "win32_error_code": code,
                    "rust_raw_os_error": null,
                }));
                return;
            }
        };
        let handle_identity_matches =
            rename_identity_gate_allows(Some(&created_identity), Some(&current_handle_identity));
        if !identity_is_known(Some(&current_handle_identity)) || !handle_identity_matches {
            observations.push(json!({
                "phase": "file_identity_before_rename",
                "api": "GetFileInformationByHandleEx(FILE_ID_INFO) on open handle",
                "result": "SAFETY_REJECTION",
                "reason": "CREATED_HANDLE_IDENTITY_CHANGED_OR_INVALID",
                "identity": identity_json(&current_handle_identity),
                "creation_before_rename_identity_matches": false,
            }));
            return;
        }

        let source_handle = match File::open(source) {
            Ok(handle) => {
                observations.push(json!({
                    "phase": "source_reopen_before_rename",
                    "api": "std::fs::File::open",
                    "requested_access": "read-only",
                    "result": "SUCCESS",
                    "file_name": "regular-before-rename.bin",
                    "win32_error_code": null,
                    "rust_raw_os_error": null,
                }));
                handle
            }
            Err(error) => {
                let raw = error.raw_os_error();
                observations.push(json!({
                    "phase": "source_reopen_before_rename",
                    "api": "std::fs::File::open",
                    "requested_access": "read-only",
                    "result": "ENVIRONMENT_UNAVAILABLE",
                    "error_kind": format!("{:?}", error.kind()),
                    "win32_error_code": raw.and_then(|code| u32::try_from(code).ok()),
                    "rust_raw_os_error": raw,
                }));
                return;
            }
        };
        if budget_expired(start) {
            observations.push(skipped_observation(
                "file_identity_before_rename",
                "30_SECOND_WALL_TIME_BUDGET_EXHAUSTED",
            ));
            return;
        }
        let path_identity = match file_identity(&source_handle) {
            Ok(identity) => identity,
            Err(code) => {
                observations.push(json!({
                    "phase": "file_identity_before_rename",
                    "api": "GetFileInformationByHandleEx(FILE_ID_INFO) on reopened source path",
                    "result": "ENVIRONMENT_UNAVAILABLE",
                    "win32_error_code": code,
                    "rust_raw_os_error": null,
                }));
                return;
            }
        };
        let creation_before_rename_identity_matches =
            rename_identity_gate_allows(Some(&created_identity), Some(&path_identity));
        if !identity_is_known(Some(&path_identity)) || !creation_before_rename_identity_matches {
            observations.push(json!({
                "phase": "file_identity_before_rename",
                "api": "GetFileInformationByHandleEx(FILE_ID_INFO) on reopened source path",
                "result": "SAFETY_REJECTION",
                "reason": "SOURCE_PATH_IDENTITY_MISMATCH_OR_INVALID; rename not issued",
                "identity": identity_json(&path_identity),
                "creation_before_rename_identity_matches": false,
            }));
            return;
        }
        observations.push(json!({
            "phase": "file_identity_before_rename",
            "api": "GetFileInformationByHandleEx(FILE_ID_INFO) on reopened source path",
            "result": "API_RETURN_OBSERVED",
            "identity": identity_json(&path_identity),
            "creation_before_rename_identity_matches": true,
            "rename_gate": "AUTHORIZED_BY_NONZERO_NONPLACEHOLDER_MATCHING_IDENTITY",
        }));

        if path_exists(destination) {
            observations.push(json!({
                "phase": "rename_destination_precheck",
                "result": "SAFETY_REJECTION",
                "reason": "DESTINATION_ALREADY_EXISTS",
            }));
            return;
        }
        if !rename_identity_gate_allows(Some(&created_identity), Some(&path_identity)) {
            observations.push(json!({
                "phase": "same_directory_rename",
                "result": "SAFETY_REJECTION",
                "reason": "IDENTITY_GATE_DENIED; rename not issued",
                "win32_error_code": null,
                "rust_raw_os_error": null,
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
        drop(source_handle);
        drop(file);
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
                let matches = identities_match(Some(&created_identity), Some(&after_identity));
                let valid = identity_is_known(Some(&after_identity));
                observations.push(json!({
                    "phase": "file_identity_after_rename",
                    "api": "GetFileInformationByHandleEx(FILE_ID_INFO) on read-only reopened handle",
                    "result": if !valid { "SAFETY_REJECTION" } else if matches { "API_RETURN_OBSERVED" } else { "SAFETY_REJECTION" },
                    "reason": if !valid { Some("ZERO_OR_PLACEHOLDER_FILE_ID") } else if matches { None } else { Some("IDENTITY_CHANGED_AFTER_RENAME; follow-on read not issued") },
                    "identity": identity_json(&after_identity),
                    "before_after_identity_matches": matches,
                }));
                if !valid || !matches {
                    return;
                }
            }
            Err(code) => {
                observations.push(json!({
                    "phase": "file_identity_after_rename",
                    "api": "GetFileInformationByHandleEx(FILE_ID_INFO) on read-only reopened handle",
                    "result": "ENVIRONMENT_UNAVAILABLE",
                    "win32_error_code": code,
                    "rust_raw_os_error": null,
                }));
                return;
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
                let payload_matches = bytes.as_slice() == FILE_PAYLOAD;
                observations.push(json!({
                    "phase": "readonly_reopen_content_check",
                    "api": "std::io::Read::read_to_end bounded to 1 MiB + 1",
                    "result": if payload_matches && digest_matches { "API_RETURN_OBSERVED" } else { "SAFETY_REJECTION" },
                    "reason": if payload_matches && digest_matches { None } else { Some("READ_CONTENT_OR_HASH_MISMATCH") },
                    "file_bytes": read,
                    "sha256": digest,
                    "payload_matches": payload_matches,
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

    fn cleanup_io_failure(phase: &str, api: &str, error: io::Error, reason: &str) -> Value {
        let raw = error.raw_os_error();
        json!({
            "phase": phase,
            "api": api,
            "result": "CLEANUP_INCOMPLETE",
            "reason": reason,
            "error_kind": format!("{:?}", error.kind()),
            "win32_error_code": raw.and_then(|code| u32::try_from(code).ok()),
            "rust_raw_os_error": raw,
        })
    }

    fn cleanup_known_paths(
        sandbox: &Path,
        source: &Path,
        destination: &Path,
        owned_file: &OwnedFile,
        observations: &mut Vec<Value>,
    ) -> &'static str {
        let mut complete = true;
        let mut found_probe_file = false;
        let paths = [
            ("regular-before-rename.bin", source),
            ("regular-after-rename.bin", destination),
        ];

        for (name, path) in paths {
            let path_metadata = match fs::symlink_metadata(path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => {
                    complete = false;
                    observations.push(cleanup_io_failure(
                        "cleanup_file_inspect",
                        "std::fs::symlink_metadata",
                        error,
                        "PATH_INSPECTION_FAILED; left untouched",
                    ));
                    continue;
                }
                Ok(metadata) => metadata,
            };
            found_probe_file = true;
            if !path_metadata.file_type().is_file()
                || path_metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            {
                complete = false;
                observations.push(json!({
                    "phase": "cleanup_file_identity_check",
                    "result": "CLEANUP_INCOMPLETE",
                    "file_name": name,
                    "reason": "UNEXPECTED_FILE_TYPE_OR_REPARSE_POINT; left untouched",
                }));
                continue;
            }

            let Some(expected_identity) = owned_file.identity.as_ref() else {
                complete = false;
                observations.push(json!({
                    "phase": "cleanup_file_identity_check",
                    "result": "CLEANUP_INCOMPLETE",
                    "file_name": name,
                    "reason": "OWNED_IDENTITY_UNAVAILABLE; file left untouched",
                }));
                continue;
            };
            if !owned_file.was_created || !identity_is_known(Some(expected_identity)) {
                complete = false;
                observations.push(json!({
                    "phase": "cleanup_file_identity_check",
                    "result": "CLEANUP_INCOMPLETE",
                    "file_name": name,
                    "reason": "NO_VALID_CREATION_IDENTITY; file left untouched",
                }));
                continue;
            }

            let file = match File::open(path) {
                Ok(file) => file,
                Err(error) => {
                    complete = false;
                    observations.push(cleanup_io_failure(
                        "cleanup_file_open_for_identity",
                        "std::fs::File::open",
                        error,
                        "IDENTITY_HANDLE_UNAVAILABLE; file left untouched",
                    ));
                    continue;
                }
            };
            let handle_metadata = match file.metadata() {
                Ok(metadata)
                    if metadata.file_type().is_file()
                        && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0 =>
                {
                    metadata
                }
                Ok(_) => {
                    complete = false;
                    observations.push(json!({
                        "phase": "cleanup_file_identity_check",
                        "result": "CLEANUP_INCOMPLETE",
                        "file_name": name,
                        "reason": "OPEN_HANDLE_NOT_A_REGULAR_FILE; left untouched",
                    }));
                    continue;
                }
                Err(error) => {
                    complete = false;
                    observations.push(cleanup_io_failure(
                        "cleanup_file_handle_metadata",
                        "std::fs::File::metadata",
                        error,
                        "OPEN_HANDLE_METADATA_UNAVAILABLE; file left untouched",
                    ));
                    continue;
                }
            };
            let _ = handle_metadata;
            let observed_identity = match file_identity(&file) {
                Ok(identity) => identity,
                Err(code) => {
                    complete = false;
                    observations.push(json!({
                        "phase": "cleanup_file_identity_check",
                        "api": "GetFileInformationByHandleEx(FILE_ID_INFO)",
                        "result": "CLEANUP_INCOMPLETE",
                        "file_name": name,
                        "win32_error_code": code,
                        "rust_raw_os_error": null,
                        "reason": "IDENTITY_QUERY_FAILED; file left untouched",
                    }));
                    continue;
                }
            };
            if !identity_is_known(Some(&observed_identity))
                || !identities_match(Some(expected_identity), Some(&observed_identity))
            {
                complete = false;
                observations.push(json!({
                    "phase": "cleanup_file_identity_check",
                    "api": "GetFileInformationByHandleEx(FILE_ID_INFO)",
                    "result": "CLEANUP_INCOMPLETE",
                    "file_name": name,
                    "expected_identity": identity_json(expected_identity),
                    "observed_identity": identity_json(&observed_identity),
                    "reason": "OBJECT_IDENTITY_MISMATCH; file left untouched",
                }));
                continue;
            }

            let path_binding = match File::open(path) {
                Ok(file) => file,
                Err(error) => {
                    complete = false;
                    observations.push(cleanup_io_failure(
                        "cleanup_file_reopen_for_remove",
                        "std::fs::File::open",
                        error,
                        "FINAL_PATH_HANDLE_UNAVAILABLE; file left untouched",
                    ));
                    continue;
                }
            };
            let path_binding_metadata = match path_binding.metadata() {
                Ok(metadata)
                    if metadata.file_type().is_file()
                        && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0 =>
                {
                    metadata
                }
                Ok(_) => {
                    complete = false;
                    observations.push(json!({
                        "phase": "cleanup_file_identity_check",
                        "result": "CLEANUP_INCOMPLETE",
                        "file_name": name,
                        "reason": "FINAL_PATH_HANDLE_NOT_A_REGULAR_FILE; left untouched",
                    }));
                    continue;
                }
                Err(error) => {
                    complete = false;
                    observations.push(cleanup_io_failure(
                        "cleanup_file_final_handle_metadata",
                        "std::fs::File::metadata",
                        error,
                        "FINAL_PATH_HANDLE_METADATA_UNAVAILABLE; file left untouched",
                    ));
                    continue;
                }
            };
            let _ = path_binding_metadata;
            let path_identity = match file_identity(&path_binding) {
                Ok(identity) => identity,
                Err(code) => {
                    complete = false;
                    observations.push(json!({
                        "phase": "cleanup_file_identity_check",
                        "api": "GetFileInformationByHandleEx(FILE_ID_INFO) on final path handle",
                        "result": "CLEANUP_INCOMPLETE",
                        "file_name": name,
                        "win32_error_code": code,
                        "rust_raw_os_error": null,
                        "reason": "FINAL_PATH_IDENTITY_UNAVAILABLE; file left untouched",
                    }));
                    continue;
                }
            };
            if !identities_match(Some(expected_identity), Some(&path_identity)) {
                complete = false;
                observations.push(json!({
                    "phase": "cleanup_file_identity_check",
                    "api": "GetFileInformationByHandleEx(FILE_ID_INFO) on final path handle",
                    "result": "CLEANUP_INCOMPLETE",
                    "file_name": name,
                    "expected_identity": identity_json(expected_identity),
                    "observed_identity": identity_json(&path_identity),
                    "reason": "FINAL_PATH_HANDLE_IDENTITY_MISMATCH; file left untouched",
                }));
                continue;
            }
            observations.push(json!({
                "phase": "cleanup_file_identity_check",
                "api": "GetFileInformationByHandleEx(FILE_ID_INFO) on final path handle",
                "result": "API_RETURN_OBSERVED",
                "file_name": name,
                "identity_match": true,
                "deletion_policy": "remove exact path while verified handle remains open",
            }));
            if let Err(error) = fs::remove_file(path) {
                complete = false;
                observations.push(cleanup_io_failure(
                    "cleanup_file_remove",
                    "std::fs::remove_file",
                    error,
                    "IDENTITY_VERIFIED_BUT_REMOVE_FAILED",
                ));
            } else {
                observations.push(json!({
                    "phase": "cleanup_file_remove",
                    "result": "API_RETURN_OBSERVED",
                    "file_name": name,
                    "identity_match_required": true,
                    "identity_match": true,
                    "path_scope": "exact generated direct child only",
                }));
            }
        }

        if owned_file.was_created && !found_probe_file {
            complete = false;
            observations.push(json!({
                "phase": "cleanup_file_identity_check",
                "result": "CLEANUP_INCOMPLETE",
                "reason": "CREATED_OBJECT_NOT_FOUND_AT_EITHER_EXPECTED_NAME",
            }));
        }

        match fs::symlink_metadata(sandbox) {
            Ok(metadata)
                if metadata.is_dir()
                    && !metadata.file_type().is_symlink()
                    && metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0 =>
            {
                if let Err(error) = fs::remove_dir(sandbox) {
                    complete = false;
                    observations.push(cleanup_io_failure(
                        "cleanup_sandbox_remove",
                        "std::fs::remove_dir",
                        error,
                        "EMPTY_UNIQUE_SANDBOX_REMOVE_FAILED; no recursive deletion",
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

    fn identity_with(byte: u8) -> FileIdentity {
        FileIdentity {
            volume_serial_number: 17,
            file_id: [byte; 16],
        }
    }

    fn complete_observations() -> Vec<Value> {
        let mut observations = vec![
            serde_json::json!({"phase":"directory_open","case_id":"A_FILE_OPEN_BASELINE","result":"ERROR","win32_error_code":5,"rust_raw_os_error":Some(5)}),
            serde_json::json!({"phase":"directory_open","case_id":"B_BACKUP_SEMANTICS_GENERIC_READ","result":"SUCCESS"}),
            serde_json::json!({"phase":"directory_sync","case_id":"B_BACKUP_SEMANTICS_GENERIC_READ","result":"ERROR","win32_error_code":5,"rust_raw_os_error":Some(5)}),
            serde_json::json!({"phase":"directory_open","case_id":"C_BACKUP_SEMANTICS_GENERIC_WRITE","result":"SUCCESS"}),
            serde_json::json!({"phase":"directory_sync","case_id":"C_BACKUP_SEMANTICS_GENERIC_WRITE","result":"SUCCESS"}),
            serde_json::json!({"phase":"small_file_open_create_new","result":"SUCCESS"}),
            serde_json::json!({"phase":"file_identity_at_creation","result":"API_RETURN_OBSERVED"}),
            serde_json::json!({"phase":"small_file_write","result":"SUCCESS"}),
            serde_json::json!({"phase":"small_file_sync","result":"SUCCESS"}),
            serde_json::json!({"phase":"file_identity_before_rename","result":"API_RETURN_OBSERVED","creation_before_rename_identity_matches":true}),
            serde_json::json!({"phase":"source_reopen_before_rename","result":"SUCCESS"}),
            serde_json::json!({"phase":"same_directory_rename","result":"API_RETURN_OBSERVED"}),
            serde_json::json!({"phase":"readonly_reopen_after_rename","result":"SUCCESS"}),
            serde_json::json!({"phase":"file_identity_after_rename","result":"API_RETURN_OBSERVED","before_after_identity_matches":true}),
            serde_json::json!({"phase":"readonly_reopen_content_check","result":"API_RETURN_OBSERVED","payload_matches":true,"before_after_sha256_matches":true}),
        ];
        observations.shrink_to_fit();
        observations
    }

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
    fn missing_zero_and_placeholder_identity_never_issue_rename() {
        let zero_file_id = FileIdentity {
            volume_serial_number: 17,
            file_id: [0; 16],
        };
        let placeholder_file_id = FileIdentity {
            volume_serial_number: 17,
            file_id: [u8::MAX; 16],
        };
        let zero_volume = FileIdentity {
            volume_serial_number: 0,
            file_id: [1; 16],
        };
        let placeholder_volume = FileIdentity {
            volume_serial_number: u64::MAX,
            file_id: [1; 16],
        };
        let created_identity = identity_with(1);
        let changed_identity = identity_with(2);
        let rejected = [
            None,
            Some(&zero_file_id),
            Some(&placeholder_file_id),
            Some(&zero_volume),
            Some(&placeholder_volume),
            Some(&changed_identity),
        ];
        for identity in rejected {
            let mut rename_issued = false;
            if rename_identity_gate_allows(Some(&created_identity), identity) {
                rename_issued = true;
            }
            assert!(!rename_issued);
        }
        assert!(rename_identity_gate_allows(
            Some(&created_identity),
            Some(&created_identity)
        ));
    }

    #[test]
    fn changed_post_rename_identity_blocks_follow_on_and_aggregates_rejection() {
        let expected = identity_with(1);
        let changed = identity_with(2);
        assert!(!identities_match(Some(&expected), Some(&changed)));

        let mut observations = complete_observations();
        observations.retain(|item| item["phase"] != "readonly_reopen_content_check");
        let after = observations
            .iter_mut()
            .find(|item| item["phase"] == "file_identity_after_rename")
            .unwrap();
        after["result"] = serde_json::json!("SAFETY_REJECTION");
        after["before_after_identity_matches"] = serde_json::json!(false);
        assert_eq!(
            aggregate_probe_status(&observations).0,
            ProbeStatus::SafetyRejection
        );
    }

    #[test]
    fn missing_identity_query_blocks_rename_and_is_unavailable() {
        let mut observations = complete_observations();
        observations.retain(|item| item["phase"] != "same_directory_rename");
        let before = observations
            .iter_mut()
            .find(|item| item["phase"] == "file_identity_before_rename")
            .unwrap();
        before["result"] = serde_json::json!("ENVIRONMENT_UNAVAILABLE");
        before["win32_error_code"] = serde_json::json!(5);
        assert_eq!(
            aggregate_probe_status(&observations).0,
            ProbeStatus::EnvironmentUnavailable
        );
    }

    #[test]
    fn directory_api_errors_are_observations_when_required_phases_complete() {
        let observations = complete_observations();
        assert_eq!(
            aggregate_probe_status(&observations),
            (
                ProbeStatus::FileOperationObserved,
                "ALL_REQUIRED_PHASES_COMPLETE_API_RETURNS_ONLY"
            )
        );
        assert_eq!(ProbeStatus::FileOperationObserved.exit_code(), 0);
    }

    #[test]
    fn wrong_content_digest_is_a_safety_rejection() {
        let mut observations = complete_observations();
        let content = observations
            .iter_mut()
            .find(|item| item["phase"] == "readonly_reopen_content_check")
            .unwrap();
        content["result"] = serde_json::json!("SAFETY_REJECTION");
        content["before_after_sha256_matches"] = serde_json::json!(false);
        assert_eq!(
            aggregate_probe_status(&observations).0,
            ProbeStatus::SafetyRejection
        );
    }

    #[test]
    fn skipped_or_missing_required_phase_is_probe_incomplete() {
        let mut skipped = complete_observations();
        skipped.push(serde_json::json!({"phase":"directory_sync","case_id":"C_BACKUP_SEMANTICS_GENERIC_WRITE","result":"NOT_ATTEMPTED"}));
        assert_eq!(
            aggregate_probe_status(&skipped).0,
            ProbeStatus::ProbeIncomplete
        );

        let mut missing = complete_observations();
        missing.retain(|item| item["phase"] != "readonly_reopen_content_check");
        assert_eq!(
            aggregate_probe_status(&missing).0,
            ProbeStatus::ProbeIncomplete
        );
    }

    #[test]
    fn cleanup_failure_does_not_promote_or_erase_primary_status() {
        let observations = complete_observations();
        let (primary, _) = aggregate_probe_status(&observations);
        assert_eq!(primary, ProbeStatus::FileOperationObserved);
        assert_eq!(
            final_probe_status(primary, false),
            ProbeStatus::CleanupIncomplete
        );
        assert_ne!(ProbeStatus::CleanupIncomplete.exit_code(), 0);
    }

    #[test]
    fn source_and_owner_preflight_failures_return_nonzero_statuses() {
        let source_rejected = final_probe_status(status_from_name("SAFETY_REJECTION"), true);
        let owner_unavailable =
            final_probe_status(status_from_name("ENVIRONMENT_UNAVAILABLE"), true);
        assert_eq!(source_rejected, ProbeStatus::SafetyRejection);
        assert_eq!(owner_unavailable, ProbeStatus::EnvironmentUnavailable);
        assert_ne!(source_rejected.exit_code(), 0);
        assert_ne!(owner_unavailable.exit_code(), 0);
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
    fn executable_identity_requires_full_sha256_hex() {
        assert!(valid_sha256_hex(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        ));
        assert!(!valid_sha256_hex("short"));
        assert!(!valid_sha256_hex(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdefg"
        ));
    }

    #[test]
    fn non_windows_never_enables_native_probe() {
        assert!(!native_windows_probe_allowed("linux"));
        assert!(!native_windows_probe_allowed("macos"));
        assert!(native_windows_probe_allowed("windows"));
    }
}
