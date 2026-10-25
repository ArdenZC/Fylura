use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileOperationResult {
    pub operation: String,
    pub source_path: String,
    pub target_path: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExecuteMovesRequest {
    pub operations: Vec<OperationPreviewRequest>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExecuteMovesByIdRequest {
    pub operations: Vec<OperationSelection>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OperationSelection {
    pub id: String,
    #[serde(alias = "fileId")]
    pub file_id: String,
    #[serde(alias = "operationFingerprint")]
    pub operation_fingerprint: String,
    #[serde(alias = "expectedRevision")]
    pub expected_revision: String,
    #[serde(default, alias = "newName")]
    pub new_name: Option<String>,
    #[serde(default, alias = "destinationDirectory")]
    pub destination_directory: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OperationPreviewRequest {
    pub id: String,
    #[serde(alias = "fileId")]
    pub file_id: String,
    #[serde(alias = "operationType")]
    pub operation_type: String,
    #[serde(alias = "sourcePath")]
    pub source_path: String,
    #[serde(alias = "targetPath")]
    pub target_path: String,
    #[serde(alias = "oldName")]
    pub old_name: String,
    #[serde(alias = "newName")]
    pub new_name: String,
    #[serde(default, alias = "isExecutable")]
    pub is_executable: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OperationLogDto {
    pub id: String,
    pub batch_id: String,
    pub operation_type: String,
    pub source_path: String,
    pub target_path: String,
    pub old_name: String,
    pub new_name: String,
    pub status: String,
    pub error_message: Option<String>,
    pub created_at: String,
    pub can_undo: bool,
    pub path_before: String,
    pub path_after: String,
    pub name_before: String,
    pub name_after: String,
    pub can_restore: bool,
    pub restored_at: Option<String>,
    pub restore_status: String,
    pub restore_error: Option<String>,
    #[serde(default)]
    pub source_size: Option<u64>,
    #[serde(default)]
    pub source_modified_ns: Option<String>,
    #[serde(default)]
    pub source_platform_file_id: Option<String>,
    #[serde(default)]
    pub source_platform_volume_id: Option<String>,
    #[serde(default)]
    pub source_quick_hash: Option<String>,
    #[serde(default)]
    pub source_full_hash: Option<String>,
    #[serde(default)]
    pub target_platform_file_id: Option<String>,
    #[serde(default)]
    pub target_platform_volume_id: Option<String>,
    #[serde(default)]
    pub target_full_hash: Option<String>,
    #[serde(default)]
    pub source_claim_path: Option<String>,
    #[serde(default = "default_operation_phase")]
    pub operation_phase: String,
    #[serde(default)]
    pub claim_created_at: Option<String>,
    #[serde(default)]
    pub claim_platform_file_id: Option<String>,
    #[serde(default)]
    pub claim_platform_volume_id: Option<String>,
    #[serde(default)]
    pub claim_full_hash: Option<String>,
    #[serde(default)]
    pub restore_claim_path: Option<String>,
    #[serde(default = "default_restore_phase")]
    pub restore_phase: String,
    #[serde(default)]
    pub restore_claim_created_at: Option<String>,
    #[serde(default)]
    pub restore_claim_platform_file_id: Option<String>,
    #[serde(default)]
    pub restore_claim_platform_volume_id: Option<String>,
    #[serde(default)]
    pub restore_claim_full_hash: Option<String>,
}

fn default_operation_phase() -> String {
    "completed".to_string()
}

fn default_restore_phase() -> String {
    "idle".to_string()
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecuteMovesResult {
    pub logs: Vec<OperationLogDto>,
    pub batch_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RestoreMovesRequest {
    pub logs: Vec<OperationLogDto>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RestoreMovesByIdRequest {
    #[serde(alias = "logIds")]
    pub log_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RestoreMovesResult {
    pub logs: Vec<OperationLogDto>,
    pub restored: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RecoveryActionRequest {
    #[serde(alias = "logId")]
    pub log_id: String,
    pub action: String,
    #[serde(default, alias = "targetPath")]
    pub target_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecoveryActionResult {
    pub original_log: OperationLogDto,
    pub action_log: OperationLogDto,
    pub target_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MaterializeProviderRequest {
    #[serde(alias = "previewId")]
    pub preview_id: String,
    #[serde(alias = "fileId")]
    pub file_id: String,
    #[serde(alias = "operationFingerprint")]
    pub operation_fingerprint: String,
    /// The preview revision is duplicated explicitly in the request so a
    /// retry cannot accidentally be treated as a path-only download. The
    /// current implementation binds it to the operation fingerprint.
    #[serde(alias = "expectedRevision")]
    pub expected_revision: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterializeProviderResult {
    pub preview_id: String,
    pub file_id: String,
    pub materialization: String,
    pub next_operation_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OperationProgressPayload {
    pub kind: String,
    pub batch_id: String,
    pub processed: u64,
    pub total: u64,
    pub current_path: String,
}

#[derive(Clone, Default)]
pub struct OperationCancellationToken {
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) running: Arc<AtomicBool>,
}

impl OperationCancellationToken {
    pub(crate) fn begin(&self) -> Result<OperationRunGuard, String> {
        self.running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .map_err(|_| "Another file operation is already running.".to_string())?;
        self.cancel.store(false, Ordering::Release);
        Ok(OperationRunGuard {
            running: Arc::clone(&self.running),
        })
    }

    pub fn cancel_for_lifecycle(&self) {
        self.cancel.store(true, Ordering::Release);
    }
}

pub(crate) struct OperationRunGuard {
    running: Arc<AtomicBool>,
}

impl Drop for OperationRunGuard {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
    }
}
