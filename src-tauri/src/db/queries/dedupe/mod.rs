use super::super::*;
use super::{current_unix_seconds, trim_trailing_path_separators};
use crate::fs_safety::PhysicalFileIdentity;
use crate::ids::new_job_id;
use rusqlite::{
    params, params_from_iter, Connection, OptionalExtension, Transaction, TransactionBehavior,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;

mod projection;

use projection::{
    add_representative_paths, dedupe_run_from_row, fingerprint_from_row, group_from_row,
    group_member_from_row, parse_group_cursor, query_dedupe_run, query_fingerprint,
    DEDUPE_RUN_SELECT, FINGERPRINT_SELECT, GROUP_SELECT,
};

pub const PREHASH_MIN_SIZE: i64 = 1024 * 1024;
pub const PREHASH_SAMPLE_BYTES: usize = 4096;
pub const DEDUPE_ERROR_DETAIL_LIMIT: i64 = 1000;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DedupeScopeRequest {
    pub kind: String,
    #[serde(default)]
    pub root_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartDedupeRunRequest {
    pub scope: DedupeScopeRequest,
    #[serde(default)]
    pub request_key: Option<String>,
    #[serde(default)]
    pub parent_scan_session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DedupeRunDto {
    pub id: String,
    pub request_key: String,
    pub request_attempt: i64,
    pub parent_scan_session_id: Option<String>,
    pub scope: Value,
    pub scope_snapshot: Value,
    pub scope_hash: String,
    pub scope_snapshot_hash: String,
    pub publication_mode: String,
    pub status: String,
    pub phase: String,
    pub revision: i64,
    pub cancel_requested: bool,
    pub rerun_required: bool,
    pub candidate_files: i64,
    pub candidate_physical_objects: i64,
    pub candidate_bytes: i64,
    pub identity_verified_files: i64,
    pub identity_unknown_files: i64,
    pub hardlink_aliases: i64,
    pub prehashed_files: i64,
    pub prehash_pruned_files: i64,
    pub full_hashed_files: i64,
    pub duplicate_groups: i64,
    pub duplicate_members: i64,
    pub exact_reclaimable_bytes: i64,
    pub potential_reclaimable_bytes: i64,
    pub processed_files: i64,
    pub processed_bytes: i64,
    pub total_bytes: i64,
    pub warning_count: i64,
    pub error_count: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub last_checkpoint_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DedupeGroupDto {
    pub id: String,
    pub size_each: i64,
    pub full_hash: String,
    pub full_hash_algorithm: String,
    pub full_hash_version: i64,
    pub member_count: i64,
    pub physical_copy_count: i64,
    pub hardlink_alias_count: i64,
    pub exact_reclaimable_bytes: Option<i64>,
    pub potential_reclaimable_bytes: i64,
    pub reclaimable_confidence: String,
    pub status: String,
    pub last_built_run_id: String,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_verified_at: i64,
    pub representative_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DedupeGroupMemberDto {
    pub group_id: String,
    pub file_id: String,
    pub path_snapshot: String,
    pub physical_key: Option<String>,
    pub identity_status: String,
    pub is_hardlink_alias: bool,
    pub size: i64,
    pub modified_ns: Option<i64>,
    pub verified_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DedupeGroupPageDto {
    pub groups: Vec<DedupeGroupDto>,
    pub next_cursor: Option<String>,
    pub limit: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct DedupeAdmission {
    pub run: DedupeRunDto,
    pub created: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct DedupeCandidate {
    pub file_id: String,
    pub path: String,
    pub size: i64,
    pub mtime: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct FingerprintCas {
    pub file_id: String,
    pub path_snapshot: String,
    pub size: i64,
    pub indexed_mtime: i64,
    pub modified_ns: Option<i64>,
    pub physical_key: Option<String>,
    pub expected_revision: i64,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct FingerprintRow {
    pub file_id: String,
    pub path_snapshot: String,
    pub identity_status: String,
    pub platform_kind: String,
    pub platform_volume_id: Option<String>,
    pub platform_file_id: Option<String>,
    pub physical_key: Option<String>,
    pub link_count: Option<i64>,
    pub size: i64,
    pub modified_ns: Option<i64>,
    pub prehash: Option<String>,
    pub prehash_algorithm: String,
    pub prehash_version: i64,
    pub prehash_sample_bytes: i64,
    pub full_hash: Option<String>,
    pub full_hash_algorithm: String,
    pub full_hash_version: i64,
    pub fingerprint_status: String,
    pub captured_at: i64,
    pub prehashed_at: Option<i64>,
    pub full_hashed_at: Option<i64>,
    pub last_verified_at: i64,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub revision: i64,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DedupeCheckpoint {
    pub phase: String,
    pub candidate_files: i64,
    pub candidate_physical_objects: i64,
    pub candidate_bytes: i64,
    pub identity_verified_files: i64,
    pub identity_unknown_files: i64,
    pub hardlink_aliases: i64,
    pub prehashed_files: i64,
    pub prehash_pruned_files: i64,
    pub full_hashed_files: i64,
    pub duplicate_groups: i64,
    pub duplicate_members: i64,
    pub exact_reclaimable_bytes: i64,
    pub potential_reclaimable_bytes: i64,
    pub processed_files: i64,
    pub processed_bytes: i64,
    pub total_bytes: i64,
    pub warning_count: i64,
    pub error_count: i64,
}

#[derive(Debug, Clone)]
pub(crate) struct BuiltGroup {
    pub id: String,
    pub size_each: i64,
    pub full_hash: String,
    pub members: Vec<BuiltMember>,
    pub physical_copy_count: i64,
    pub hardlink_alias_count: i64,
    pub exact_reclaimable_bytes: Option<i64>,
    pub potential_reclaimable_bytes: i64,
    pub reclaimable_confidence: String,
}

#[derive(Debug, Clone)]
pub(crate) struct BuiltMember {
    pub file_id: String,
    pub path_snapshot: String,
    pub physical_key: Option<String>,
    pub identity_status: String,
    pub is_hardlink_alias: bool,
    pub size: i64,
    pub modified_ns: Option<i64>,
    pub verified_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublishOutcome {
    Completed,
    CompletedWithWarnings,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScopeSpec {
    kind: String,
    #[serde(default)]
    root_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GroupCursor {
    potential_reclaimable_bytes: i64,
    size_each: i64,
    full_hash: String,
    id: String,
}

impl Database {
    pub(crate) fn start_dedupe_run(
        &self,
        request: &StartDedupeRunRequest,
    ) -> Result<DedupeAdmission, DbError> {
        let request_key = request
            .request_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| new_job_id("dedupe-request"));
        self.admit_dedupe_run(
            &request.scope,
            &request_key,
            request.parent_scan_session_id.as_deref(),
            false,
        )
    }

    pub(crate) fn start_dedupe_run_for_scan_session(
        &self,
        session_id: &str,
    ) -> Result<DedupeAdmission, DbError> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            "SELECT DISTINCT effective_root_id FROM scan_session_roots
             WHERE session_id = ?1 AND resolution = 'effective' AND effective_root_id IS NOT NULL
             ORDER BY effective_index, effective_root_id",
        )?;
        let root_ids = statement
            .query_map(params![session_id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        if root_ids.is_empty() {
            return Err(DbError::Validation(
                "The scan session has no effective managed roots for dedupe.".to_string(),
            ));
        }
        // A scan-triggered run is the sole authoritative publication path.
        // The effective roots are still validated above for dispatch
        // ownership, but publication always re-reads every enabled managed
        // File Library root so a partial session can never publish a local
        // duplicate universe.
        let _ = root_ids;
        let scope = DedupeScopeRequest {
            kind: "allManagedFileLibrary".to_string(),
            root_ids: Vec::new(),
        };
        let request_key = format!("scan-session:{session_id}");
        let existing = conn
            .query_row(
                "SELECT status, error_code, request_attempt FROM dedupe_runs WHERE request_key = ?1 ORDER BY request_attempt DESC LIMIT 1",
                params![request_key],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, i64>(2)?)),
            )
            .optional()?;
        let force_retry = existing
            .as_ref()
            .is_some_and(|(status, error_code, attempt)| {
                matches!(
                    status.as_str(),
                    "failed" | "interrupted" | "cancelled" | "completed_with_warnings"
                ) && !(error_code.as_deref() == Some("scope_changed_retry_exhausted")
                    && *attempt >= 3)
            });
        self.admit_dedupe_run(&scope, &request_key, Some(session_id), force_retry)
    }

    pub(crate) fn retry_dedupe_run(&self, run_id: &str) -> Result<DedupeAdmission, DbError> {
        let run = self.get_dedupe_run(run_id)?;
        if matches!(run.status.as_str(), "queued" | "running" | "cancelling") {
            return Err(DbError::Validation(
                "An active dedupe run cannot be retried.".to_string(),
            ));
        }
        let scope = scope_request_from_value(&run.scope)?;
        self.admit_dedupe_run(
            &scope,
            &run.request_key,
            run.parent_scan_session_id.as_deref(),
            true,
        )
    }

    fn admit_dedupe_run(
        &self,
        scope_request: &DedupeScopeRequest,
        request_key: &str,
        parent_scan_session_id: Option<&str>,
        force_retry: bool,
    ) -> Result<DedupeAdmission, DbError> {
        if request_key.len() > 256 {
            return Err(DbError::Validation(
                "Dedupe request key is too long.".to_string(),
            ));
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (scope_spec, scope_hash) = canonical_scope(&tx, scope_request)?;
        let publication_mode = if scope_spec.kind == "all_managed_file_library" {
            "authoritative"
        } else {
            "diagnostic"
        };
        let scope_json = serde_json::to_string(&scope_spec)?;
        let (root_snapshot_json, _root_snapshot_hash) = scope_snapshot(&tx, &scope_spec.root_ids)?;
        let snapshot_json_value = json!({
            "scopeHash": scope_hash,
            "roots": serde_json::from_str::<Value>(&root_snapshot_json)?,
        });
        let snapshot_json = serde_json::to_string(&snapshot_json_value)?;
        let snapshot_hash = blake3::hash(snapshot_json.as_bytes()).to_hex().to_string();

        let existing = tx
            .query_row(
                &format!("{DEDUPE_RUN_SELECT} WHERE request_key = ?1 ORDER BY request_attempt DESC LIMIT 1"),
                params![request_key],
                dedupe_run_from_row,
            )
            .optional()?;
        if let Some(existing) = existing {
            if !force_retry && existing.scope_hash == scope_hash {
                tx.commit()?;
                return Ok(DedupeAdmission {
                    run: existing,
                    created: false,
                });
            }
        }

        let active = tx
            .query_row(
                &format!("{DEDUPE_RUN_SELECT} WHERE scope_hash = ?1 AND status IN ('queued', 'running', 'cancelling') ORDER BY created_at DESC LIMIT 1"),
                params![scope_hash],
                dedupe_run_from_row,
            )
            .optional()?;
        if let Some(active) = active {
            if !force_retry {
                tx.execute(
                    "UPDATE dedupe_runs SET rerun_required = 1, revision = revision + 1, updated_at = ?1 WHERE id = ?2 AND rerun_required = 0",
                    params![current_unix_seconds(), active.id],
                )?;
                let run = query_dedupe_run(&tx, &active.id)?;
                tx.commit()?;
                return Ok(DedupeAdmission {
                    run,
                    created: false,
                });
            }
            return Err(DbError::Validation(
                "A dedupe run for this managed scope is still active.".to_string(),
            ));
        }

        let request_attempt = tx.query_row(
            "SELECT COALESCE(MAX(request_attempt), 0) + 1 FROM dedupe_runs WHERE request_key = ?1",
            params![request_key],
            |row| row.get::<_, i64>(0),
        )?;
        let now = current_unix_seconds();
        let id = new_job_id("dedupe-run");
        tx.execute(
            r#"
            INSERT INTO dedupe_runs (
                id, request_key, request_attempt, parent_scan_session_id,
                scope_json, scope_hash, scope_snapshot_json, scope_snapshot_hash,
                publication_mode, status, phase, revision, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'queued', 'collecting', 1, ?10, ?10)
            "#,
            params![
                id,
                request_key,
                request_attempt,
                parent_scan_session_id,
                scope_json,
                scope_hash,
                snapshot_json,
                snapshot_hash,
                publication_mode,
                now,
            ],
        )?;
        let run = query_dedupe_run(&tx, &id)?;
        tx.commit()?;
        Ok(DedupeAdmission { run, created: true })
    }

    pub(crate) fn get_dedupe_run(&self, run_id: &str) -> Result<DedupeRunDto, DbError> {
        let conn = self.conn()?;
        query_dedupe_run(&conn, run_id)
    }

    pub(crate) fn list_dedupe_runs(&self, limit: usize) -> Result<Vec<DedupeRunDto>, DbError> {
        let conn = self.conn()?;
        let limit = i64::try_from(limit.clamp(1, 200)).unwrap_or(200);
        let mut statement = conn.prepare(&format!(
            "{DEDUPE_RUN_SELECT} ORDER BY created_at DESC, id DESC LIMIT ?1"
        ))?;
        let result = statement
            .query_map(params![limit], dedupe_run_from_row)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(DbError::from);
        result
    }

    pub(crate) fn get_active_dedupe_run(&self) -> Result<Option<DedupeRunDto>, DbError> {
        let conn = self.conn()?;
        conn.query_row(
            &format!(
                "{DEDUPE_RUN_SELECT} WHERE status IN ('queued', 'running', 'cancelling') ORDER BY created_at DESC LIMIT 1"
            ),
            [],
            dedupe_run_from_row,
        )
        .optional()
        .map_err(DbError::from)
    }

    pub(crate) fn claim_dedupe_run(&self, run_id: &str) -> Result<Option<DedupeRunDto>, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run = query_dedupe_run(&tx, run_id)?;
        if run.status != "queued" {
            tx.commit()?;
            return Ok(None);
        }
        let now = current_unix_seconds();
        let changed = tx.execute(
            "UPDATE dedupe_runs SET status = 'running', phase = 'collecting', started_at = COALESCE(started_at, ?1), last_checkpoint_at = ?1, revision = revision + 1, updated_at = ?1 WHERE id = ?2 AND status = 'queued' AND revision = ?3",
            params![now, run_id, run.revision],
        )?;
        if changed != 1 {
            tx.commit()?;
            return Ok(None);
        }
        let updated = query_dedupe_run(&tx, run_id)?;
        tx.commit()?;
        Ok(Some(updated))
    }

    pub(crate) fn request_dedupe_cancellation(
        &self,
        run_id: &str,
    ) -> Result<DedupeRunDto, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run = query_dedupe_run(&tx, run_id)?;
        if matches!(
            run.status.as_str(),
            "completed" | "completed_with_warnings" | "cancelled" | "failed" | "interrupted"
        ) {
            tx.commit()?;
            return Ok(run);
        }
        let now = current_unix_seconds();
        let changed = tx.execute(
            "UPDATE dedupe_runs SET status = 'cancelling', cancel_requested = 1, revision = revision + 1, updated_at = ?1 WHERE id = ?2 AND revision = ?3 AND status IN ('queued', 'running', 'cancelling')",
            params![now, run_id, run.revision],
        )?;
        if changed != 1 {
            return Err(DbError::Validation(
                "Dedupe cancellation lost durable revision ownership.".to_string(),
            ));
        }
        let updated = query_dedupe_run(&tx, run_id)?;
        tx.commit()?;
        Ok(updated)
    }

    pub(crate) fn is_dedupe_cancel_requested(&self, run_id: &str) -> Result<bool, DbError> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT cancel_requested FROM dedupe_runs WHERE id = ?1",
            params![run_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|value| value != 0)
        .map_err(DbError::from)
    }

    pub(crate) fn checkpoint_dedupe_run(
        &self,
        run_id: &str,
        expected_revision: i64,
        checkpoint: &DedupeCheckpoint,
    ) -> Result<DedupeRunDto, DbError> {
        let now = current_unix_seconds();
        let conn = self.conn()?;
        let changed = conn.execute(
            r#"
            UPDATE dedupe_runs
            SET phase = ?1,
                candidate_files = ?2,
                candidate_physical_objects = ?3,
                candidate_bytes = ?4,
                identity_verified_files = ?5,
                identity_unknown_files = ?6,
                hardlink_aliases = ?7,
                prehashed_files = ?8,
                prehash_pruned_files = ?9,
                full_hashed_files = ?10,
                duplicate_groups = ?11,
                duplicate_members = ?12,
                exact_reclaimable_bytes = ?13,
                potential_reclaimable_bytes = ?14,
                processed_files = ?15,
                processed_bytes = ?16,
                total_bytes = ?17,
                warning_count = ?18,
                error_count = ?19,
                last_checkpoint_at = ?20,
                revision = revision + 1,
                updated_at = ?20
            WHERE id = ?21 AND revision = ?22
              AND status IN ('queued', 'running', 'cancelling')
            "#,
            params![
                checkpoint.phase,
                checkpoint.candidate_files,
                checkpoint.candidate_physical_objects,
                checkpoint.candidate_bytes,
                checkpoint.identity_verified_files,
                checkpoint.identity_unknown_files,
                checkpoint.hardlink_aliases,
                checkpoint.prehashed_files,
                checkpoint.prehash_pruned_files,
                checkpoint.full_hashed_files,
                checkpoint.duplicate_groups,
                checkpoint.duplicate_members,
                checkpoint.exact_reclaimable_bytes,
                checkpoint.potential_reclaimable_bytes,
                checkpoint.processed_files,
                checkpoint.processed_bytes,
                checkpoint.total_bytes,
                checkpoint.warning_count,
                checkpoint.error_count,
                now,
                run_id,
                expected_revision,
            ],
        )?;
        if changed != 1 {
            return Err(DbError::Validation(
                "Dedupe checkpoint rejected because the run revision or lease is stale."
                    .to_string(),
            ));
        }
        query_dedupe_run(&conn, run_id)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_dedupe_error(
        &self,
        run_id: &str,
        expected_revision: i64,
        file_id: Option<&str>,
        path: &str,
        phase: &str,
        error_code: &str,
        error_message: &str,
    ) -> Result<DedupeRunDto, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let error_count: i64 = tx.query_row(
            "SELECT error_count FROM dedupe_runs WHERE id = ?1 AND revision = ?2 AND status IN ('queued', 'running', 'cancelling')",
            params![run_id, expected_revision],
            |row| row.get(0),
        )?;
        if error_count < DEDUPE_ERROR_DETAIL_LIMIT {
            tx.execute(
                "INSERT INTO dedupe_run_errors (id, run_id, file_id, path_snapshot, phase, error_code, error_message, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![new_job_id("dedupe-error"), run_id, file_id, path, phase, error_code, error_message, current_unix_seconds()],
            )?;
        }
        let truncated_warning = if error_count + 1 == DEDUPE_ERROR_DETAIL_LIMIT {
            1
        } else {
            0
        };
        let stored_error_code = if error_count + 1 >= DEDUPE_ERROR_DETAIL_LIMIT {
            "errors_truncated"
        } else {
            error_code
        };
        let stored_error_message = if error_count + 1 >= DEDUPE_ERROR_DETAIL_LIMIT {
            "The per-run error detail limit was reached; subsequent errors are aggregate-only."
        } else {
            error_message
        };
        let changed = tx.execute(
            "UPDATE dedupe_runs SET error_count = error_count + 1, warning_count = warning_count + ?1, error_code = ?2, error_message = ?3, revision = revision + 1, updated_at = ?4 WHERE id = ?5 AND revision = ?6 AND status IN ('queued', 'running', 'cancelling')",
            params![truncated_warning, stored_error_code, stored_error_message, current_unix_seconds(), run_id, expected_revision],
        )?;
        if changed != 1 {
            return Err(DbError::Validation(
                "Dedupe error recording lost durable revision ownership.".to_string(),
            ));
        }
        let updated = query_dedupe_run(&tx, run_id)?;
        tx.commit()?;
        Ok(updated)
    }

    pub(crate) fn record_dedupe_warning(
        &self,
        run_id: &str,
        expected_revision: i64,
        phase: &str,
        error_code: &str,
        error_message: &str,
    ) -> Result<DedupeRunDto, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE dedupe_runs SET warning_count = warning_count + 1, error_code = ?1, error_message = ?2, last_checkpoint_at = ?3, revision = revision + 1, updated_at = ?3 WHERE id = ?4 AND revision = ?5 AND status IN ('queued', 'running', 'cancelling')",
            params![error_code, format!("{phase}: {error_message}"), current_unix_seconds(), run_id, expected_revision],
        )?;
        if changed != 1 {
            return Err(DbError::Validation(
                "Dedupe warning could not claim the active run revision.".to_string(),
            ));
        }
        let updated = query_dedupe_run(&tx, run_id)?;
        tx.commit()?;
        Ok(updated)
    }

    pub(crate) fn get_fingerprint(&self, file_id: &str) -> Result<Option<FingerprintRow>, DbError> {
        let conn = self.conn()?;
        query_fingerprint(&conn, file_id)
    }

    pub(crate) fn find_cached_fingerprint_by_physical(
        &self,
        identity: &PhysicalFileIdentity,
        exclude_file_id: &str,
    ) -> Result<Option<FingerprintRow>, DbError> {
        let Some(physical_key) = identity.physical_key.as_deref() else {
            return Ok(None);
        };
        let conn = self.conn()?;
        conn.query_row(
            &format!("{FINGERPRINT_SELECT} WHERE physical_key = ?1 AND file_id <> ?2 AND size = ?3 AND modified_ns IS ?4 AND identity_status IN ('verified', 'stale', 'missing') AND prehash_algorithm = 'blake3-head-tail' AND prehash_version = 1 AND full_hash_algorithm = 'blake3' AND full_hash_version = 1 AND fingerprint_status IN ('prehash_complete', 'complete', 'stale', 'missing') AND EXISTS (SELECT 1 FROM files WHERE files.id = file_fingerprints.file_id AND files.path = file_fingerprints.path_snapshot AND files.is_dir = 0 AND files.size = file_fingerprints.size) ORDER BY full_hashed_at DESC, file_id LIMIT 1"),
            params![physical_key, exclude_file_id, i64::try_from(identity.size).unwrap_or(i64::MAX), identity.modified_ns],
            fingerprint_from_row,
        )
        .optional()
        .map_err(DbError::from)
    }

    pub(crate) fn upsert_physical_identity(
        &self,
        candidate: &DedupeCandidate,
        identity: &PhysicalFileIdentity,
    ) -> Result<FingerprintRow, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old = query_fingerprint(&tx, &candidate.file_id)?;
        let same_identity = old.as_ref().is_some_and(|row| {
            row.size == i64::try_from(identity.size).unwrap_or(i64::MAX)
                && row.modified_ns == identity.modified_ns
                && row.physical_key == identity.physical_key
                && row.prehash_algorithm == "blake3-head-tail"
                && row.prehash_version == 1
                && row.full_hash_algorithm == "blake3"
                && row.full_hash_version == 1
        });
        let prehash = same_identity
            .then(|| old.as_ref().and_then(|row| row.prehash.clone()))
            .flatten();
        let full_hash = same_identity
            .then(|| old.as_ref().and_then(|row| row.full_hash.clone()))
            .flatten();
        let prehashed_at = same_identity
            .then(|| old.as_ref().and_then(|row| row.prehashed_at))
            .flatten();
        let full_hashed_at = same_identity
            .then(|| old.as_ref().and_then(|row| row.full_hashed_at))
            .flatten();
        let fingerprint_status = if full_hash.is_some() {
            "complete"
        } else if prehash.is_some() {
            "prehash_complete"
        } else {
            "identity_only"
        };
        let now = current_unix_seconds();
        tx.execute(
            r#"
            INSERT INTO file_fingerprints (
                file_id, path_snapshot, identity_status, platform_kind,
                platform_volume_id, platform_file_id, physical_key, link_count,
                size, modified_ns, prehash, prehash_algorithm, prehash_version,
                prehash_sample_bytes, full_hash, full_hash_algorithm, full_hash_version,
                fingerprint_status, captured_at, prehashed_at, full_hashed_at,
                last_verified_at, error_code, error_message, revision
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'blake3-head-tail', 1, 4096, ?12, 'blake3', 1, ?13, ?14, ?15, ?16, ?14, NULL, NULL, 1)
            ON CONFLICT(file_id) DO UPDATE SET
                path_snapshot = excluded.path_snapshot,
                identity_status = excluded.identity_status,
                platform_kind = excluded.platform_kind,
                platform_volume_id = excluded.platform_volume_id,
                platform_file_id = excluded.platform_file_id,
                physical_key = excluded.physical_key,
                link_count = excluded.link_count,
                size = excluded.size,
                modified_ns = excluded.modified_ns,
                prehash = excluded.prehash,
                prehash_algorithm = excluded.prehash_algorithm,
                prehash_version = excluded.prehash_version,
                prehash_sample_bytes = excluded.prehash_sample_bytes,
                full_hash = excluded.full_hash,
                full_hash_algorithm = excluded.full_hash_algorithm,
                full_hash_version = excluded.full_hash_version,
                fingerprint_status = excluded.fingerprint_status,
                captured_at = excluded.captured_at,
                prehashed_at = excluded.prehashed_at,
                full_hashed_at = excluded.full_hashed_at,
                last_verified_at = excluded.last_verified_at,
                error_code = NULL,
                error_message = NULL,
                revision = file_fingerprints.revision + 1
            "#,
            params![
                candidate.file_id,
                candidate.path,
                if identity.physical_key.is_some() { "verified" } else { "path_only" },
                identity.platform_kind.as_str(),
                identity.platform_volume_id,
                identity.platform_file_id,
                identity.physical_key,
                identity.link_count.and_then(|value| i64::try_from(value).ok()),
                i64::try_from(identity.size).unwrap_or(i64::MAX),
                identity.modified_ns,
                prehash,
                full_hash,
                fingerprint_status,
                now,
                prehashed_at,
                full_hashed_at,
            ],
        )?;
        let row = query_fingerprint(&tx, &candidate.file_id)?.ok_or_else(|| {
            DbError::Validation("Fingerprint identity upsert did not return a row.".to_string())
        })?;
        super::library::bump_library_query_revision_in_transaction(&tx)?;
        tx.commit()?;
        Ok(row)
    }

    pub(crate) fn record_fingerprint_error(
        &self,
        candidate: &DedupeCandidate,
        identity_status: &str,
        fingerprint_status: &str,
        error_code: &str,
        error_message: &str,
    ) -> Result<(), DbError> {
        if !matches!(identity_status, "unsupported" | "missing" | "error")
            || !matches!(fingerprint_status, "unsupported" | "missing" | "error")
        {
            return Err(DbError::Validation(
                "Invalid durable fingerprint error status.".to_string(),
            ));
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM files WHERE id = ?1)",
            params![candidate.file_id],
            |row| row.get::<_, i64>(0).map(|value| value != 0),
        )?;
        if !exists {
            tx.commit()?;
            return Ok(());
        }
        invalidate_file_in_transaction(&tx, &candidate.file_id, "stale")?;
        let now = current_unix_seconds();
        tx.execute(
            r#"
            INSERT INTO file_fingerprints (
                file_id, path_snapshot, identity_status, platform_kind,
                size, prehash_algorithm, prehash_version, prehash_sample_bytes,
                full_hash_algorithm, full_hash_version, fingerprint_status,
                captured_at, last_verified_at, error_code, error_message, revision
            ) VALUES (?1, ?2, ?3, '', ?4, 'blake3-head-tail', 1, 4096,
                      'blake3', 1, ?5, ?6, ?6, ?7, ?8, 1)
            ON CONFLICT(file_id) DO UPDATE SET
                path_snapshot = excluded.path_snapshot,
                identity_status = excluded.identity_status,
                platform_kind = excluded.platform_kind,
                platform_volume_id = NULL,
                platform_file_id = NULL,
                physical_key = NULL,
                link_count = NULL,
                size = excluded.size,
                modified_ns = NULL,
                prehash = NULL,
                prehashed_at = NULL,
                full_hash = NULL,
                full_hashed_at = NULL,
                fingerprint_status = excluded.fingerprint_status,
                captured_at = excluded.captured_at,
                last_verified_at = excluded.last_verified_at,
                error_code = excluded.error_code,
                error_message = excluded.error_message,
                revision = file_fingerprints.revision + 1
            "#,
            params![
                candidate.file_id,
                candidate.path,
                identity_status,
                candidate.size,
                fingerprint_status,
                now,
                error_code,
                error_message,
            ],
        )?;
        super::library::bump_library_query_revision_in_transaction(&tx)?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn copy_cached_fingerprint_for_rename(
        &self,
        candidate: &DedupeCandidate,
        identity: &PhysicalFileIdentity,
        cached: &FingerprintRow,
    ) -> Result<Option<FingerprintRow>, DbError> {
        if identity.physical_key.is_none()
            || cached.physical_key != identity.physical_key
            || cached.size != i64::try_from(identity.size).unwrap_or(i64::MAX)
            || cached.modified_ns != identity.modified_ns
        {
            return Ok(None);
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE file_fingerprints SET prehash = ?1, prehashed_at = ?2, full_hash = ?3, full_hashed_at = ?4, fingerprint_status = CASE WHEN ?3 IS NOT NULL AND ?3 <> '' THEN 'complete' WHEN ?1 IS NOT NULL AND ?1 <> '' THEN 'prehash_complete' ELSE 'identity_only' END, revision = revision + 1, last_verified_at = ?9, error_code = NULL, error_message = NULL WHERE file_id = ?5 AND size = ?6 AND modified_ns IS ?7 AND physical_key = ?8",
            params![cached.prehash, cached.prehashed_at, cached.full_hash, cached.full_hashed_at, candidate.file_id, cached.size, cached.modified_ns, cached.physical_key, current_unix_seconds()],
        )?;
        tx.execute(
            "UPDATE files SET content_hash = CASE WHEN ?1 IS NOT NULL AND ?1 <> '' THEN ?1 ELSE '' END WHERE id = ?2 AND path = ?3 AND size = ?4 AND mtime = ?5 AND is_dir = 0 AND is_stale = 0",
            params![cached.full_hash, candidate.file_id, candidate.path, candidate.size, candidate.mtime],
        )?;
        let row = query_fingerprint(&tx, &candidate.file_id)?;
        tx.commit()?;
        Ok(row)
    }

    pub(crate) fn save_prehash(
        &self,
        entries: &[FingerprintCas],
        prehash: &str,
    ) -> Result<usize, DbError> {
        if entries.is_empty() {
            return Ok(0);
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = current_unix_seconds();
        let mut updated = 0;
        for entry in entries {
            updated += tx.execute(
                r#"
                UPDATE file_fingerprints
                SET prehash = ?1,
                    prehashed_at = ?2,
                    fingerprint_status = CASE
                        WHEN full_hash IS NOT NULL AND full_hash <> '' THEN 'complete'
                        ELSE 'prehash_complete'
                    END,
                    last_verified_at = ?2,
                    revision = revision + 1
                WHERE file_id = ?3
                  AND path_snapshot = ?4
                  AND size = ?5
                  AND modified_ns IS ?6
                  AND physical_key IS ?7
                  AND revision = ?8
                  AND identity_status IN ('verified', 'path_only')
                  AND EXISTS (
                      SELECT 1 FROM files
                      WHERE files.id = file_fingerprints.file_id
                        AND files.path = ?4
                        AND files.size = ?5
                        AND files.mtime = ?9
                        AND files.is_dir = 0
                        AND files.is_stale = 0
                  )
                "#,
                params![
                    prehash,
                    now,
                    entry.file_id,
                    entry.path_snapshot,
                    entry.size,
                    entry.modified_ns,
                    entry.physical_key,
                    entry.expected_revision,
                    entry.indexed_mtime,
                ],
            )?;
        }
        tx.commit()?;
        Ok(updated)
    }

    pub(crate) fn save_full_hash(
        &self,
        entries: &[(FingerprintCas, String)],
    ) -> Result<usize, DbError> {
        if entries.is_empty() {
            return Ok(0);
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = current_unix_seconds();
        let mut updated = 0;
        for (entry, hash) in entries {
            let changed = tx.execute(
                r#"
                UPDATE file_fingerprints
                SET full_hash = ?1,
                    full_hashed_at = ?2,
                    fingerprint_status = 'complete',
                    last_verified_at = ?2,
                    revision = revision + 1
                WHERE file_id = ?3
                  AND path_snapshot = ?4
                  AND size = ?5
                  AND modified_ns IS ?6
                  AND physical_key IS ?7
                  AND revision = ?8
                  AND identity_status IN ('verified', 'path_only')
                  AND EXISTS (
                      SELECT 1 FROM files
                      WHERE files.id = file_fingerprints.file_id
                        AND files.path = ?4
                        AND files.size = ?5
                        AND files.mtime = ?9
                        AND files.is_dir = 0
                        AND files.is_stale = 0
                  )
                "#,
                params![
                    hash,
                    now,
                    entry.file_id,
                    entry.path_snapshot,
                    entry.size,
                    entry.modified_ns,
                    entry.physical_key,
                    entry.expected_revision,
                    entry.indexed_mtime,
                ],
            )?;
            if changed == 1 {
                tx.execute(
                    "UPDATE files SET content_hash = ?1 WHERE id = ?2 AND path = ?3 AND size = ?4 AND mtime = ?5 AND is_dir = 0 AND is_stale = 0",
                    params![hash, entry.file_id, entry.path_snapshot, entry.size, entry.indexed_mtime],
                )?;
                updated += 1;
            }
        }
        tx.commit()?;
        Ok(updated)
    }

    pub(crate) fn collect_dedupe_candidates(
        &self,
        scope: &Value,
    ) -> Result<Vec<DedupeCandidate>, DbError> {
        let spec = scope_request_from_value(scope)?;
        let conn = self.conn()?;
        let (where_clause, params) = scope_file_filter(&conn, &spec)?;
        let sql = format!(
            "WITH scoped_files AS (SELECT f.id, f.path, f.size, f.mtime FROM files AS f WHERE f.is_dir = 0 AND f.is_stale = 0 AND f.size > 0 AND {where_clause}), candidate_sizes AS (SELECT size FROM scoped_files GROUP BY size HAVING COUNT(*) > 1) SELECT f.id, f.path, f.size, f.mtime FROM scoped_files AS f JOIN candidate_sizes AS candidate ON candidate.size = f.size ORDER BY f.size ASC, f.path COLLATE NOCASE ASC"
        );
        let mut statement = conn.prepare(&sql)?;
        let result = statement
            .query_map(params_from_iter(params.iter()), |row| {
                Ok(DedupeCandidate {
                    file_id: row.get(0)?,
                    path: row.get(1)?,
                    size: row.get(2)?,
                    mtime: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(DbError::from);
        result
    }

    pub(crate) fn publish_dedupe_groups(
        &self,
        run_id: &str,
        groups: &[BuiltGroup],
        checkpoint: &DedupeCheckpoint,
    ) -> Result<PublishOutcome, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run = query_dedupe_run(&tx, run_id)?;
        if run.cancel_requested || run.status == "cancelling" {
            mark_dedupe_terminal_tx(
                &tx,
                run_id,
                run.revision,
                "cancelled",
                "cancelled",
                Some("cancelled"),
                Some("Dedupe run cancelled before group publication."),
                checkpoint,
            )?;
            tx.commit()?;
            return Ok(PublishOutcome::Cancelled);
        }
        if run.publication_mode != "authoritative" {
            let mut warning_checkpoint = checkpoint.clone();
            warning_checkpoint.warning_count = warning_checkpoint.warning_count.saturating_add(1);
            mark_dedupe_terminal_tx(
                &tx,
                run_id,
                run.revision,
                "completed_with_warnings",
                "completed",
                Some("diagnostic_only"),
                Some("Explicit-root dedupe runs are diagnostic only and cannot publish or replace the global duplicate authority."),
                &warning_checkpoint,
            )?;
            tx.commit()?;
            return Ok(PublishOutcome::CompletedWithWarnings);
        }
        let scope = scope_request_from_value(&run.scope)?;
        let (scope_spec, current_scope_hash) = canonical_scope(
            &tx,
            &DedupeScopeRequest {
                kind: "all_managed_file_library".to_string(),
                root_ids: Vec::new(),
            },
        )?;
        let (root_snapshot_json, _root_snapshot_hash) = scope_snapshot(&tx, &scope_spec.root_ids)?;
        let current_snapshot_json = serde_json::to_string(&json!({
            "scopeHash": current_scope_hash,
            "roots": serde_json::from_str::<Value>(&root_snapshot_json)?,
        }))?;
        let snapshot_hash = blake3::hash(current_snapshot_json.as_bytes())
            .to_hex()
            .to_string();
        if snapshot_hash != run.scope_snapshot_hash {
            let now = current_unix_seconds();
            let auto_retry = run.parent_scan_session_id.is_some() && run.request_attempt < 3;
            let dispatch_state = if auto_retry { "pending" } else { "suppressed" };
            let error_code = if auto_retry {
                "scope_changed_during_run"
            } else {
                "scope_changed_retry_exhausted"
            };
            let error_message = if auto_retry {
                "The managed index changed while duplicate groups were being built; a follow-up attempt was scheduled."
            } else {
                "The managed index changed during three consecutive duplicate attempts; manual attention is required."
            };
            tx.execute(
                "UPDATE dedupe_runs SET status = 'completed_with_warnings', phase = 'completed', rerun_required = ?1, warning_count = warning_count + 1, error_code = ?2, error_message = ?3, finished_at = ?4, last_checkpoint_at = ?4, revision = revision + 1, updated_at = ?4 WHERE id = ?5 AND revision = ?6 AND status IN ('running', 'cancelling')",
                params![bool_to_i64(auto_retry), error_code, error_message, now, run_id, run.revision],
            )?;
            if let Some(session_id) = run.parent_scan_session_id.as_deref() {
                tx.execute(
                    "UPDATE scan_sessions SET dedupe_dispatch_state = ?1, dedupe_job_id = NULL, dedupe_last_error = ?2, revision = revision + 1, updated_at = ?3 WHERE id = ?4 AND dedupe_requested = 1 AND status IN ('completed', 'completed_with_warnings')",
                    params![dispatch_state, error_message, now, session_id],
                )?;
            }
            tx.commit()?;
            return Ok(PublishOutcome::CompletedWithWarnings);
        }

        if scope.kind != "all_managed_file_library" {
            return Err(DbError::Validation(
                "Authoritative dedupe runs must use the all-managed scope.".to_string(),
            ));
        }
        let total_roots: i64 = tx.query_row(
            "SELECT COUNT(*) FROM scan_roots WHERE enabled = 1 AND source_kind = 'file_library'",
            [],
            |row| row.get(0),
        )?;
        let unhealthy_roots: i64 = tx.query_row(
            "SELECT COUNT(*) FROM scan_roots WHERE enabled = 1 AND source_kind = 'file_library' AND (health_status <> 'healthy' OR needs_reconciliation <> 0 OR watcher_rule_recovery_required <> 0 OR watcher_revision <> watcher_applied_revision)",
            [],
            |row| row.get(0),
        )?;
        if total_roots == 0 || unhealthy_roots > 0 {
            let mut warning_checkpoint = checkpoint.clone();
            warning_checkpoint.warning_count = warning_checkpoint.warning_count.saturating_add(1);
            bump_dedupe_authority_tx(&tx, "rebuild_required")?;
            mark_dedupe_terminal_tx(
                &tx,
                run_id,
                run.revision,
                "completed_with_warnings",
                "completed",
                Some("dedupe_authority_unhealthy"),
                Some("At least one enabled managed root is unhealthy or requires reconciliation; the global duplicate authority was not replaced."),
                &warning_checkpoint,
            )?;
            tx.commit()?;
            return Ok(PublishOutcome::CompletedWithWarnings);
        }
        let old_group_ids = tx
            .prepare("SELECT id FROM duplicate_groups WHERE status = 'active'")?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let new_group_ids = groups
            .iter()
            .map(|group| group.id.as_str())
            .collect::<HashSet<_>>();
        let now = current_unix_seconds();
        for group in groups {
            tx.execute(
                r#"
                INSERT INTO duplicate_groups (
                    id, size_each, full_hash, full_hash_algorithm, full_hash_version,
                    member_count, physical_copy_count, hardlink_alias_count,
                    exact_reclaimable_bytes, potential_reclaimable_bytes,
                    reclaimable_confidence, status, last_built_run_id, revision,
                    created_at, updated_at, last_verified_at
                ) VALUES (?1, ?2, ?3, 'blake3', 1, ?4, ?5, ?6, ?7, ?8, ?9, 'active', ?10, 1, ?11, ?11, ?11)
                ON CONFLICT(size_each, full_hash, full_hash_algorithm, full_hash_version)
                DO UPDATE SET
                    id = excluded.id,
                    member_count = excluded.member_count,
                    physical_copy_count = excluded.physical_copy_count,
                    hardlink_alias_count = excluded.hardlink_alias_count,
                    exact_reclaimable_bytes = excluded.exact_reclaimable_bytes,
                    potential_reclaimable_bytes = excluded.potential_reclaimable_bytes,
                    reclaimable_confidence = excluded.reclaimable_confidence,
                    status = 'active',
                    last_built_run_id = excluded.last_built_run_id,
                    revision = duplicate_groups.revision + 1,
                    updated_at = excluded.updated_at,
                    last_verified_at = excluded.last_verified_at
                "#,
                params![
                    group.id,
                    group.size_each,
                    group.full_hash,
                    i64::try_from(group.members.len()).unwrap_or(i64::MAX),
                    group.physical_copy_count,
                    group.hardlink_alias_count,
                    group.exact_reclaimable_bytes,
                    group.potential_reclaimable_bytes,
                    group.reclaimable_confidence,
                    run_id,
                    now,
                ],
            )?;
            tx.execute(
                "DELETE FROM duplicate_group_members WHERE group_id = ?1",
                params![group.id],
            )?;
            for member in &group.members {
                tx.execute(
                    "INSERT INTO duplicate_group_members (group_id, file_id, path_snapshot, physical_key, identity_status, is_hardlink_alias, size, modified_ns, verified_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![group.id, member.file_id, member.path_snapshot, member.physical_key, member.identity_status, bool_to_i64(member.is_hardlink_alias), member.size, member.modified_ns, member.verified_at],
                )?;
            }
        }
        for group_id in old_group_ids {
            if new_group_ids.contains(group_id.as_str()) {
                continue;
            }
            tx.execute(
                "UPDATE duplicate_groups SET status = 'stale', revision = revision + 1, updated_at = ?1 WHERE id = ?2 AND status = 'active'",
                params![now, group_id],
            )?;
            invalidate_analysis_findings_for_group_tx(&tx, &group_id)?;
        }
        tx.execute(
            "UPDATE dedupe_authority_state SET revision = revision + 1, status = 'healthy', last_authoritative_run_id = ?1, scope_hash = ?2, updated_at = ?3 WHERE id = 1",
            params![run_id, run.scope_hash, now],
        )?;
        mark_dedupe_terminal_tx(
            &tx,
            run_id,
            run.revision,
            if checkpoint.warning_count > 0 {
                "completed_with_warnings"
            } else {
                "completed"
            },
            "completed",
            None,
            None,
            checkpoint,
        )?;
        super::library::bump_library_query_revision_in_transaction(&tx)?;
        tx.commit()?;
        if checkpoint.warning_count > 0 {
            Ok(PublishOutcome::CompletedWithWarnings)
        } else {
            Ok(PublishOutcome::Completed)
        }
    }

    pub(crate) fn mark_dedupe_terminal(
        &self,
        run_id: &str,
        expected_revision: i64,
        status: &str,
        error_code: Option<&str>,
        error_message: Option<&str>,
        checkpoint: &DedupeCheckpoint,
    ) -> Result<DedupeRunDto, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        mark_dedupe_terminal_tx(
            &tx,
            run_id,
            expected_revision,
            status,
            "completed",
            error_code,
            error_message,
            checkpoint,
        )?;
        let run = query_dedupe_run(&tx, run_id)?;
        tx.commit()?;
        Ok(run)
    }

    pub fn recover_dedupe_runs(&self) -> Result<usize, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = current_unix_seconds();
        let recovered = tx.execute(
            "UPDATE dedupe_runs SET status = 'interrupted', phase = 'completed', error_code = 'interrupted_on_startup', error_message = 'The previous process stopped while duplicate detection was active.', finished_at = ?1, revision = revision + 1, updated_at = ?1 WHERE status IN ('queued', 'running', 'cancelling')",
            params![now],
        )?;
        tx.execute(
            "UPDATE scan_sessions SET dedupe_dispatch_state = 'pending', dedupe_last_error = 'Dedupe run was interrupted and is eligible for retry.', revision = revision + 1, updated_at = ?1 WHERE dedupe_requested = 1 AND dedupe_job_id IN (SELECT id FROM dedupe_runs WHERE status = 'interrupted')",
            params![now],
        )?;
        tx.commit()?;
        Ok(recovered)
    }

    pub fn prune_dedupe_artifacts(&self) -> Result<usize, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let active_run_count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM dedupe_runs WHERE status IN ('queued', 'running', 'cancelling')",
            [],
            |row| row.get(0),
        )?;
        if active_run_count > 0 {
            tx.commit()?;
            return Ok(0);
        }

        let now = current_unix_seconds();
        let fingerprint_cutoff = now.saturating_sub(30 * 24 * 60 * 60);
        let run_cutoff = now.saturating_sub(90 * 24 * 60 * 60);
        let mut remaining = 1000_i64;
        let mut deleted = 0usize;

        if remaining > 0 {
            let changed = tx.execute(
                "DELETE FROM duplicate_groups WHERE rowid IN (SELECT rowid FROM duplicate_groups WHERE status IN ('stale', 'superseded') AND updated_at <= ?1 ORDER BY updated_at, id LIMIT ?2)",
                params![fingerprint_cutoff, remaining],
            )?;
            deleted += changed;
            remaining -= i64::try_from(changed).unwrap_or(remaining);
        }
        if remaining > 0 {
            let changed = tx.execute(
                "DELETE FROM file_fingerprints WHERE rowid IN (SELECT rowid FROM file_fingerprints WHERE fingerprint_status IN ('stale', 'missing', 'unsupported', 'error') AND last_verified_at <= ?1 ORDER BY last_verified_at, file_id LIMIT ?2)",
                params![fingerprint_cutoff, remaining],
            )?;
            deleted += changed;
            remaining -= i64::try_from(changed).unwrap_or(remaining);
        }
        if remaining > 0 {
            let changed = tx.execute(
                "DELETE FROM dedupe_run_errors WHERE rowid IN (SELECT error.rowid FROM dedupe_run_errors AS error JOIN dedupe_runs AS run ON run.id = error.run_id WHERE run.status NOT IN ('queued', 'running', 'cancelling') AND error.created_at <= ?1 ORDER BY error.created_at, error.id LIMIT ?2)",
                params![fingerprint_cutoff, remaining],
            )?;
            deleted += changed;
            remaining -= i64::try_from(changed).unwrap_or(remaining);
        }
        if remaining > 0 {
            let changed = tx.execute(
                r#"
                DELETE FROM dedupe_runs
                WHERE rowid IN (
                    SELECT run.rowid
                    FROM dedupe_runs AS run
                    WHERE run.status NOT IN ('queued', 'running', 'cancelling')
                      AND run.finished_at IS NOT NULL
                      AND run.finished_at <= ?1
                      AND NOT EXISTS (
                          SELECT 1 FROM duplicate_groups AS group_row
                          WHERE group_row.last_built_run_id = run.id
                      )
                    ORDER BY run.finished_at, run.id
                    LIMIT ?2
                )
                "#,
                params![run_cutoff, remaining],
            )?;
            deleted += changed;
        }
        tx.commit()?;
        Ok(deleted)
    }

    pub(crate) fn list_duplicate_groups(
        &self,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<DedupeGroupPageDto, DbError> {
        let limit = i64::try_from(limit.clamp(1, 100)).unwrap_or(100);
        let cursor = cursor.map(parse_group_cursor).transpose()?;
        let conn = self.conn()?;
        let mut params = Vec::<rusqlite::types::Value>::new();
        let cursor_clause = if let Some(cursor) = &cursor {
            params.extend([
                rusqlite::types::Value::Integer(cursor.potential_reclaimable_bytes),
                rusqlite::types::Value::Integer(cursor.size_each),
                rusqlite::types::Value::Text(cursor.full_hash.clone()),
                rusqlite::types::Value::Text(cursor.id.clone()),
            ]);
            "AND (potential_reclaimable_bytes < ?1 OR (potential_reclaimable_bytes = ?1 AND size_each < ?2) OR (potential_reclaimable_bytes = ?1 AND size_each = ?2 AND full_hash > ?3) OR (potential_reclaimable_bytes = ?1 AND size_each = ?2 AND full_hash = ?3 AND id > ?4))"
        } else {
            ""
        };
        params.push(rusqlite::types::Value::Integer(limit + 1));
        let mut statement = conn.prepare(&format!(
            "{GROUP_SELECT} WHERE status = 'active' {cursor_clause} ORDER BY potential_reclaimable_bytes DESC, size_each DESC, full_hash ASC, id ASC LIMIT ?{}",
            params.len()
        ))?;
        let mut groups = {
            let rows = statement
                .query_map(params_from_iter(params.iter()), group_from_row)?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        let has_more = groups.len() > usize::try_from(limit).unwrap_or(100);
        if has_more {
            groups.truncate(usize::try_from(limit).unwrap_or(100));
        }
        let next_cursor = groups
            .last()
            .and_then(|group| {
                has_more.then(|| {
                    serde_json::to_string(&GroupCursor {
                        potential_reclaimable_bytes: group.potential_reclaimable_bytes,
                        size_each: group.size_each,
                        full_hash: group.full_hash.clone(),
                        id: group.id.clone(),
                    })
                    .ok()
                })
            })
            .flatten();
        let groups = groups
            .into_iter()
            .map(|group| add_representative_paths(&conn, group))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(DedupeGroupPageDto {
            groups,
            next_cursor,
            limit,
        })
    }

    pub(crate) fn get_duplicate_group(
        &self,
        group_id: &str,
    ) -> Result<Option<DedupeGroupDto>, DbError> {
        let conn = self.conn()?;
        let group = conn
            .query_row(
                &format!("{GROUP_SELECT} WHERE id = ?1"),
                params![group_id],
                group_from_row,
            )
            .optional()?;
        group
            .map(|group| add_representative_paths(&conn, group))
            .transpose()
    }

    pub(crate) fn list_duplicate_group_members(
        &self,
        group_id: &str,
    ) -> Result<Vec<DedupeGroupMemberDto>, DbError> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            "SELECT group_id, file_id, path_snapshot, physical_key, identity_status, is_hardlink_alias, size, modified_ns, verified_at FROM duplicate_group_members WHERE group_id = ?1 ORDER BY path_snapshot COLLATE NOCASE, file_id",
        )?;
        let result = statement
            .query_map(params![group_id], group_member_from_row)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(DbError::from);
        result
    }

    pub(crate) fn get_file_duplicate_membership(
        &self,
        file_id: &str,
    ) -> Result<Vec<DedupeGroupDto>, DbError> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            &format!("{GROUP_SELECT} WHERE id IN (SELECT group_id FROM duplicate_group_members WHERE file_id = ?1) AND status = 'active'"),
        )?;
        let groups = statement
            .query_map(params![file_id], group_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        groups
            .into_iter()
            .map(|group| add_representative_paths(&conn, group))
            .collect()
    }
}

fn canonical_scope(
    conn: &Connection,
    request: &DedupeScopeRequest,
) -> Result<(ScopeSpec, String), DbError> {
    let kind = match request.kind.trim() {
        "allManagedFileLibrary" | "all_managed_file_library" => "all_managed_file_library",
        "explicitEnabledScanRoots" | "explicit_enabled_scan_roots" => "explicit_enabled_scan_roots",
        _ => {
            return Err(DbError::Validation(
                "Dedupe scope must be a managed File Library scope.".to_string(),
            ))
        }
    };
    let root_ids = if kind == "all_managed_file_library" {
        let mut statement = conn.prepare(
            "SELECT id FROM scan_roots WHERE enabled = 1 AND source_kind = 'file_library' ORDER BY id",
        )?;
        let root_ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        root_ids
    } else {
        let mut root_ids = request
            .root_ids
            .iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect::<Vec<_>>();
        root_ids.sort();
        root_ids.dedup();
        if root_ids.is_empty() {
            return Err(DbError::Validation(
                "At least one enabled managed scan root is required.".to_string(),
            ));
        }
        for root_id in &root_ids {
            let enabled = conn
                .query_row(
                    "SELECT enabled FROM scan_roots WHERE id = ?1 AND source_kind = 'file_library'",
                    params![root_id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?;
            if enabled != Some(1) {
                return Err(DbError::Validation(
                    "Dedupe scope contains an unknown or disabled managed root.".to_string(),
                ));
            }
        }
        root_ids
    };
    if root_ids.is_empty() {
        return Err(DbError::Validation(
            "No enabled managed File Library roots are available for dedupe.".to_string(),
        ));
    }
    let spec = ScopeSpec {
        kind: kind.to_string(),
        root_ids,
    };
    let json = serde_json::to_string(&spec)?;
    Ok((spec, blake3::hash(json.as_bytes()).to_hex().to_string()))
}

fn scope_request_from_value(value: &Value) -> Result<DedupeScopeRequest, DbError> {
    let spec: ScopeSpec = serde_json::from_value(value.clone())?;
    Ok(DedupeScopeRequest {
        kind: spec.kind,
        root_ids: spec.root_ids,
    })
}

fn scope_snapshot(conn: &Connection, root_ids: &[String]) -> Result<(String, String), DbError> {
    let mut values = Vec::with_capacity(root_ids.len());
    for root_id in root_ids {
        let value = conn.query_row(
            "SELECT id, last_successful_generation, watcher_revision, watcher_applied_revision, needs_reconciliation FROM scan_roots WHERE id = ?1 AND enabled = 1 AND source_kind = 'file_library'",
            params![root_id],
            |row| {
                Ok(json!({
                    "rootId": row.get::<_, String>(0)?,
                    "lastSuccessfulGeneration": row.get::<_, Option<i64>>(1)?,
                    "watcherRevision": row.get::<_, i64>(2)?,
                    "watcherAppliedRevision": row.get::<_, i64>(3)?,
                    "needsReconciliation": row.get::<_, i64>(4)? != 0,
                }))
            },
        )?;
        values.push(value);
    }
    let value = Value::Array(values);
    let json = serde_json::to_string(&value)?;
    let hash = blake3::hash(json.as_bytes()).to_hex().to_string();
    Ok((json, hash))
}

fn scope_file_filter(
    conn: &Connection,
    scope: &DedupeScopeRequest,
) -> Result<(String, Vec<rusqlite::types::Value>), DbError> {
    let (spec, _) = canonical_scope(conn, scope)?;
    let mut paths = Vec::new();
    for root_id in &spec.root_ids {
        paths.push(conn.query_row(
            "SELECT normalized_path FROM scan_roots WHERE id = ?1",
            params![root_id],
            |row| row.get::<_, String>(0),
        )?);
    }
    let mut clauses = Vec::new();
    let mut values = Vec::new();
    for path in paths {
        let path = trim_trailing_path_separators(&path).to_string();
        let escaped = path
            .replace('~', "~~")
            .replace('%', "~%")
            .replace('_', "~_");
        clauses.push(
            "(f.path = ? OR f.path LIKE ? ESCAPE '~' OR f.path LIKE ? ESCAPE '~')".to_string(),
        );
        values.push(rusqlite::types::Value::Text(path));
        values.push(rusqlite::types::Value::Text(format!("{escaped}/%")));
        values.push(rusqlite::types::Value::Text(format!("{escaped}\\%")));
    }
    Ok((clauses.join(" OR "), values))
}

#[allow(dead_code)]
fn scoped_active_group_ids(conn: &Connection, scope: &ScopeSpec) -> Result<Vec<String>, DbError> {
    let request = DedupeScopeRequest {
        kind: scope.kind.clone(),
        root_ids: scope.root_ids.clone(),
    };
    let (where_clause, values) = scope_file_filter(conn, &request)?;
    let sql = format!(
        "SELECT DISTINCT member.group_id FROM duplicate_group_members AS member JOIN duplicate_groups AS group_row ON group_row.id = member.group_id JOIN files AS f ON f.id = member.file_id WHERE group_row.status = 'active' AND ({where_clause})"
    );
    let mut statement = conn.prepare(&sql)?;
    let result = statement
        .query_map(params_from_iter(values.iter()), |row| {
            row.get::<_, String>(0)
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(DbError::from);
    result
}

#[allow(clippy::too_many_arguments)]
fn mark_dedupe_terminal_tx(
    tx: &Transaction<'_>,
    run_id: &str,
    expected_revision: i64,
    status: &str,
    phase: &str,
    error_code: Option<&str>,
    error_message: Option<&str>,
    checkpoint: &DedupeCheckpoint,
) -> Result<(), DbError> {
    if !matches!(
        status,
        "completed" | "completed_with_warnings" | "cancelled" | "failed" | "interrupted"
    ) {
        return Err(DbError::Validation(format!(
            "Invalid dedupe terminal status: {status}"
        )));
    }
    let now = current_unix_seconds();
    tx.execute(
        r#"
        UPDATE dedupe_runs
        SET status = ?1,
            phase = ?2,
            candidate_files = ?3,
            candidate_physical_objects = ?4,
            candidate_bytes = ?5,
            identity_verified_files = ?6,
            identity_unknown_files = ?7,
            hardlink_aliases = ?8,
            prehashed_files = ?9,
            prehash_pruned_files = ?10,
            full_hashed_files = ?11,
            duplicate_groups = ?12,
            duplicate_members = ?13,
            exact_reclaimable_bytes = ?14,
            potential_reclaimable_bytes = ?15,
            processed_files = ?16,
            processed_bytes = ?17,
            total_bytes = ?18,
            warning_count = ?19,
            error_count = ?20,
            error_code = COALESCE(?21, error_code),
            error_message = COALESCE(?22, error_message),
            finished_at = ?23,
            last_checkpoint_at = ?23,
            revision = revision + 1,
            updated_at = ?23
        WHERE id = ?24 AND revision = ?25
          AND status IN ('queued', 'running', 'cancelling')
        "#,
        params![
            status,
            phase,
            checkpoint.candidate_files,
            checkpoint.candidate_physical_objects,
            checkpoint.candidate_bytes,
            checkpoint.identity_verified_files,
            checkpoint.identity_unknown_files,
            checkpoint.hardlink_aliases,
            checkpoint.prehashed_files,
            checkpoint.prehash_pruned_files,
            checkpoint.full_hashed_files,
            checkpoint.duplicate_groups,
            checkpoint.duplicate_members,
            checkpoint.exact_reclaimable_bytes,
            checkpoint.potential_reclaimable_bytes,
            checkpoint.processed_files,
            checkpoint.processed_bytes,
            checkpoint.total_bytes,
            checkpoint.warning_count,
            checkpoint.error_count,
            error_code,
            error_message,
            now,
            run_id,
            expected_revision,
        ],
    )?;
    if tx.changes() != 1 {
        return Err(DbError::Validation(
            "Dedupe terminal transition lost durable revision ownership.".to_string(),
        ));
    }
    Ok(())
}

pub(crate) fn invalidate_file_in_transaction(
    tx: &Transaction<'_>,
    file_id: &str,
    stale_status: &str,
) -> Result<(), DbError> {
    let group_ids = tx
        .prepare("SELECT group_id FROM duplicate_group_members WHERE file_id = ?1")?
        .query_map(params![file_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    tx.execute(
        "UPDATE files SET content_hash = '' WHERE id = ?1 AND content_hash <> ''",
        params![file_id],
    )?;
    tx.execute(
        "UPDATE file_fingerprints SET fingerprint_status = ?1, identity_status = CASE WHEN ?1 = 'missing' THEN 'missing' ELSE 'stale' END, revision = revision + 1, error_code = CASE WHEN ?1 = 'missing' THEN 'missing' ELSE 'file_changed' END, error_message = NULL WHERE file_id = ?2 AND (fingerprint_status <> ?1 OR identity_status <> CASE WHEN ?1 = 'missing' THEN 'missing' ELSE 'stale' END)",
        params![stale_status, file_id],
    )?;
    tx.execute(
        "UPDATE duplicate_groups SET status = 'stale', revision = revision + 1, updated_at = ?1 WHERE status = 'active' AND id IN (SELECT group_id FROM duplicate_group_members WHERE file_id = ?2)",
        params![current_unix_seconds(), file_id],
    )?;
    crate::db::invalidate_analysis_findings_for_file_tx(tx, file_id)?;
    for group_id in group_ids {
        crate::db::invalidate_analysis_findings_for_group_tx(tx, &group_id)?;
    }
    crate::db::bump_dedupe_authority_tx(tx, "rebuild_required")?;
    Ok(())
}

pub(crate) fn invalidate_stale_files_in_transaction(tx: &Transaction<'_>) -> Result<(), DbError> {
    let group_ids = tx
        .prepare(
            "SELECT DISTINCT group_id FROM duplicate_group_members WHERE file_id IN (SELECT id FROM files WHERE is_stale = 1)",
        )?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    tx.execute(
        "UPDATE files SET content_hash = '' WHERE is_stale = 1 AND content_hash <> ''",
        [],
    )?;
    tx.execute(
        "UPDATE file_fingerprints SET fingerprint_status = 'missing', identity_status = 'missing', revision = revision + 1, error_code = 'missing' WHERE file_id IN (SELECT id FROM files WHERE is_stale = 1) AND (fingerprint_status <> 'missing' OR identity_status <> 'missing')",
        [],
    )?;
    tx.execute(
        "UPDATE duplicate_groups SET status = 'stale', revision = revision + 1, updated_at = ?1 WHERE status = 'active' AND id IN (SELECT group_id FROM duplicate_group_members WHERE file_id IN (SELECT id FROM files WHERE is_stale = 1))",
        params![current_unix_seconds()],
    )?;
    for group_id in group_ids {
        crate::db::invalidate_analysis_findings_for_group_tx(tx, &group_id)?;
    }
    crate::db::bump_dedupe_authority_tx(tx, "rebuild_required")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::InsertFileRequest;
    use std::{fs, time::Instant};

    #[test]
    fn scope_rejects_arbitrary_paths() {
        let db_path = std::env::temp_dir().join(format!(
            "zen-canvas-dedupe-scope-{}-{}.sqlite3",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let db = Database::open(&db_path).expect("database");
        let error = db
            .start_dedupe_run(&StartDedupeRunRequest {
                scope: DedupeScopeRequest {
                    kind: "arbitraryPath".into(),
                    root_ids: vec![],
                },
                request_key: Some("scope-test".into()),
                parent_scan_session_id: None,
            })
            .expect_err("arbitrary scope rejected");
        assert!(error.to_string().contains("managed File Library"));
        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn path_only_fallback_and_cross_volume_cache_rules_are_fail_closed() {
        let db_path = std::env::temp_dir().join(format!(
            "zen-canvas-dedupe-identity-rules-{}-{}.sqlite3",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let db = Database::open(&db_path).expect("database");
        db.insert_file(InsertFileRequest {
            id: "/tmp/path-only.txt".into(),
            path: "/tmp/path-only.txt".into(),
            name: "path-only.txt".into(),
            extension: "txt".into(),
            size: 4,
            mtime: 11,
            ctime: 11,
            is_dir: false,
            state_code: 0,
        })
        .expect("path-only file row");
        let path_only = db
            .upsert_physical_identity(
                &DedupeCandidate {
                    file_id: "/tmp/path-only.txt".into(),
                    path: "/tmp/path-only.txt".into(),
                    size: 4,
                    mtime: 11,
                },
                &PhysicalFileIdentity {
                    size: 4,
                    modified_ns: None,
                    created_ns: None,
                    platform_kind: crate::fs_safety::PhysicalPlatform::Other,
                    platform_volume_id: None,
                    platform_file_id: None,
                    physical_key: None,
                    link_count: None,
                },
            )
            .expect("path-only identity");
        assert_eq!(path_only.identity_status, "path_only");
        assert!(path_only.physical_key.is_none());

        db.insert_file(InsertFileRequest {
            id: "/tmp/old-volume.txt".into(),
            path: "/tmp/old-volume.txt".into(),
            name: "old-volume.txt".into(),
            extension: "txt".into(),
            size: 4,
            mtime: 11,
            ctime: 11,
            is_dir: false,
            state_code: 0,
        })
        .expect("old volume file row");
        let conn = db.conn().expect("connection");
        conn.execute(
            "INSERT INTO file_fingerprints (file_id, path_snapshot, identity_status, platform_kind, platform_volume_id, platform_file_id, physical_key, size, modified_ns, full_hash, fingerprint_status, captured_at, last_verified_at) VALUES ('/tmp/old-volume.txt', '/tmp/old-volume.txt', 'verified', 'unix', 'old-volume', '1', 'unix:v1:old-volume:1', 4, 11, 'cached-hash', 'complete', 1, 1)",
            [],
        )
        .expect("old volume fingerprint");
        let new_volume = PhysicalFileIdentity {
            size: 4,
            modified_ns: Some(11),
            created_ns: None,
            platform_kind: crate::fs_safety::PhysicalPlatform::Unix,
            platform_volume_id: Some("new-volume".into()),
            platform_file_id: Some("1".into()),
            physical_key: Some("unix:v1:new-volume:1".into()),
            link_count: Some(1),
        };
        assert!(db
            .find_cached_fingerprint_by_physical(&new_volume, "/tmp/new-volume.txt")
            .expect("cross-volume lookup")
            .is_none());

        let same_physical = PhysicalFileIdentity {
            physical_key: Some("unix:v1:old-volume:1".into()),
            platform_volume_id: Some("old-volume".into()),
            ..new_volume.clone()
        };
        assert!(db
            .find_cached_fingerprint_by_physical(&same_physical, "/tmp/renamed.txt")
            .expect("same-volume lookup")
            .is_some());
        conn.execute(
            "UPDATE files SET is_stale = 1 WHERE id = '/tmp/old-volume.txt'",
            [],
        )
        .expect("mark old row stale");
        conn.execute(
            "UPDATE file_fingerprints SET identity_status = 'missing', fingerprint_status = 'missing' WHERE file_id = '/tmp/old-volume.txt'",
            [],
        )
        .expect("retain stale rename cache");
        assert!(db
            .find_cached_fingerprint_by_physical(&same_physical, "/tmp/renamed.txt")
            .expect("stale rename lookup")
            .is_some());
        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn fingerprint_write_cas_rejects_stale_revision_for_prehash_and_full_hash() {
        let db_path = std::env::temp_dir().join(format!(
            "zen-canvas-dedupe-fingerprint-cas-{}-{}.sqlite3",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let db = Database::open(&db_path).expect("database");
        db.insert_file(InsertFileRequest {
            id: "/tmp/cas.txt".into(),
            path: "/tmp/cas.txt".into(),
            name: "cas.txt".into(),
            extension: "txt".into(),
            size: 4,
            mtime: 11,
            ctime: 11,
            is_dir: false,
            state_code: 0,
        })
        .expect("cas file row");
        let identity = PhysicalFileIdentity {
            size: 4,
            modified_ns: Some(22),
            created_ns: None,
            platform_kind: crate::fs_safety::PhysicalPlatform::Unix,
            platform_volume_id: Some("cas-volume".into()),
            platform_file_id: Some("cas-file".into()),
            physical_key: Some("unix:v1:cas-volume:cas-file".into()),
            link_count: Some(1),
        };
        let candidate = DedupeCandidate {
            file_id: "/tmp/cas.txt".into(),
            path: "/tmp/cas.txt".into(),
            size: 4,
            mtime: 11,
        };
        let fingerprint = db
            .upsert_physical_identity(&candidate, &identity)
            .expect("identity row");
        let cas = FingerprintCas {
            file_id: candidate.file_id.clone(),
            path_snapshot: candidate.path.clone(),
            size: candidate.size,
            indexed_mtime: candidate.mtime,
            modified_ns: identity.modified_ns,
            physical_key: identity.physical_key.clone(),
            expected_revision: fingerprint.revision,
        };
        let conn = db.conn().expect("connection");
        conn.execute(
            "UPDATE file_fingerprints SET revision = revision + 1 WHERE file_id = '/tmp/cas.txt'",
            [],
        )
        .expect("advance competing revision");
        let prehash_updated = db
            .save_prehash(std::slice::from_ref(&cas), "prehash")
            .expect("prehash CAS");
        assert_eq!(prehash_updated, 0);
        let full_hash_updated = db
            .save_full_hash(&[(cas, "full-hash".into())])
            .expect("full-hash CAS");
        assert_eq!(full_hash_updated, 0);
        let stored = db
            .get_fingerprint("/tmp/cas.txt")
            .expect("stored fingerprint");
        assert!(stored.and_then(|row| row.full_hash).is_none());
        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn per_run_error_details_are_capped_and_truncation_is_durable() {
        let db_path = std::env::temp_dir().join(format!(
            "zen-canvas-dedupe-error-cap-{}-{}.sqlite3",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let db = Database::open(&db_path).expect("database");
        let conn = db.conn().expect("connection");
        conn.execute(
            "INSERT INTO scan_roots (id, normalized_path, display_name, source_kind, enabled, health_status, needs_reconciliation, created_at, updated_at) VALUES ('error-cap-root', '/tmp/error-cap-root', 'error-cap-root', 'file_library', 1, 'healthy', 0, 1, 1)",
            [],
        )
        .expect("managed root");
        let admission = db
            .start_dedupe_run(&StartDedupeRunRequest {
                scope: DedupeScopeRequest {
                    kind: "explicitEnabledScanRoots".into(),
                    root_ids: vec!["error-cap-root".into()],
                },
                request_key: Some("error-cap-request".into()),
                parent_scan_session_id: None,
            })
            .expect("admission");
        let mut run = admission.run;
        for index in 0..=DEDUPE_ERROR_DETAIL_LIMIT {
            run = db
                .record_dedupe_error(
                    &run.id,
                    run.revision,
                    None,
                    &format!("/tmp/error-{index}"),
                    "full_hashing",
                    "synthetic_error",
                    "synthetic error",
                )
                .expect("record bounded error");
        }
        let details: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM dedupe_run_errors WHERE run_id = ?1",
                [&run.id],
                |row| row.get(0),
            )
            .expect("error details");
        assert_eq!(details, DEDUPE_ERROR_DETAIL_LIMIT);
        assert_eq!(run.error_count, DEDUPE_ERROR_DETAIL_LIMIT + 1);
        assert_eq!(run.warning_count, 1);
        assert_eq!(run.error_code.as_deref(), Some("errors_truncated"));
        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn invalidation_stales_group_and_clears_only_compatibility_in_caller_transaction() {
        let db_path = std::env::temp_dir().join(format!(
            "zen-canvas-dedupe-invalidation-{}-{}.sqlite3",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let db = Database::open(&db_path).expect("database");
        db.insert_file(InsertFileRequest {
            id: "/tmp/fingerprint.txt".into(),
            path: "/tmp/fingerprint.txt".into(),
            name: "fingerprint.txt".into(),
            extension: "txt".into(),
            size: 1,
            mtime: 1,
            ctime: 1,
            is_dir: false,
            state_code: 0,
        })
        .expect("file");
        let conn = db.conn().expect("connection");
        let mut conn = conn;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("transaction");
        tx.execute(
            "INSERT INTO file_fingerprints (file_id, path_snapshot, identity_status, size, fingerprint_status, captured_at, last_verified_at) VALUES ('/tmp/fingerprint.txt', '/tmp/fingerprint.txt', 'path_only', 1, 'complete', 1, 1)",
            [],
        )
        .expect("fingerprint");
        tx.execute(
            "UPDATE files SET content_hash = 'legacy-hash' WHERE id = '/tmp/fingerprint.txt'",
            [],
        )
        .expect("compatibility hash");
        invalidate_file_in_transaction(&tx, "/tmp/fingerprint.txt", "stale").expect("invalidate");
        tx.commit().expect("commit");
        let status: String = conn
            .query_row(
                "SELECT fingerprint_status FROM file_fingerprints WHERE file_id = '/tmp/fingerprint.txt'",
                [],
                |row| row.get(0),
            )
            .expect("status");
        assert_eq!(status, "stale");
        let compatibility_hash: String = conn
            .query_row(
                "SELECT content_hash FROM files WHERE id = '/tmp/fingerprint.txt'",
                [],
                |row| row.get(0),
            )
            .expect("compatibility hash");
        assert!(compatibility_hash.is_empty());
        let mut conn = db.conn().expect("connection for missing transition");
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("missing transaction");
        invalidate_file_in_transaction(&tx, "/tmp/fingerprint.txt", "missing")
            .expect("mark missing");
        tx.commit().expect("missing commit");
        let missing_status: String = conn
            .query_row(
                "SELECT fingerprint_status FROM file_fingerprints WHERE file_id = '/tmp/fingerprint.txt'",
                [],
                |row| row.get(0),
            )
            .expect("missing status");
        assert_eq!(missing_status, "missing");
        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn fingerprint_errors_are_durable_and_clear_unverified_compatibility_state() {
        let db_path = std::env::temp_dir().join(format!(
            "zen-canvas-dedupe-fingerprint-error-{}-{}.sqlite3",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let db = Database::open(&db_path).expect("database");
        db.insert_file(InsertFileRequest {
            id: "/tmp/unsupported-link.txt".into(),
            path: "/tmp/unsupported-link.txt".into(),
            name: "unsupported-link.txt".into(),
            extension: "txt".into(),
            size: 12,
            mtime: 1,
            ctime: 1,
            is_dir: false,
            state_code: 0,
        })
        .expect("file");
        let conn = db.conn().expect("connection");
        conn.execute(
            "UPDATE files SET content_hash = 'legacy-hash' WHERE id = '/tmp/unsupported-link.txt'",
            [],
        )
        .expect("compatibility hash");
        drop(conn);

        db.record_fingerprint_error(
            &DedupeCandidate {
                file_id: "/tmp/unsupported-link.txt".into(),
                path: "/tmp/unsupported-link.txt".into(),
                size: 12,
                mtime: 1,
            },
            "unsupported",
            "unsupported",
            "unsupported_link",
            "symlink or reparse point",
        )
        .expect("durable error");

        let conn = db.conn().expect("connection");
        let (identity_status, fingerprint_status, content_hash): (String, String, String) = conn
            .query_row(
                "SELECT fp.identity_status, fp.fingerprint_status, f.content_hash FROM file_fingerprints AS fp JOIN files AS f ON f.id = fp.file_id WHERE fp.file_id = '/tmp/unsupported-link.txt'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("durable fingerprint error row");
        assert_eq!(identity_status, "unsupported");
        assert_eq!(fingerprint_status, "unsupported");
        assert!(content_hash.is_empty());
        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn request_key_replay_is_idempotent_but_terminal_runs_can_retry_after_restart() {
        let db_path = std::env::temp_dir().join(format!(
            "zen-canvas-dedupe-admission-{}-{}.sqlite3",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let db = Database::open(&db_path).expect("database");
        let conn = db.conn().expect("connection");
        conn.execute(
            "INSERT INTO scan_roots (id, normalized_path, display_name, source_kind, enabled, health_status, needs_reconciliation, created_at, updated_at) VALUES ('admission-root', '/tmp/admission-root', 'admission-root', 'file_library', 1, 'healthy', 0, 1, 1)",
            [],
        )
        .expect("managed root");

        let request = StartDedupeRunRequest {
            scope: DedupeScopeRequest {
                kind: "explicitEnabledScanRoots".into(),
                root_ids: vec!["admission-root".into()],
            },
            request_key: Some("manual-admission".into()),
            parent_scan_session_id: None,
        };
        let first = db.start_dedupe_run(&request).expect("first admission");
        assert!(first.created);
        let replay = db.start_dedupe_run(&request).expect("idempotent replay");
        assert!(!replay.created);
        assert_eq!(replay.run.id, first.run.id);
        assert_eq!(replay.run.request_attempt, 1);

        let second_request = StartDedupeRunRequest {
            request_key: Some("second-admission".into()),
            ..request.clone()
        };
        let coalesced = db
            .start_dedupe_run(&second_request)
            .expect("same-scope admission");
        assert!(!coalesced.created);
        assert_eq!(coalesced.run.id, first.run.id);
        assert!(coalesced.run.rerun_required);

        let claimed = db
            .claim_dedupe_run(&first.run.id)
            .expect("claim")
            .expect("queued run is claimable");
        assert_eq!(claimed.status, "running");
        let checkpoint = DedupeCheckpoint {
            phase: "full_hashing".into(),
            candidate_files: 2,
            ..DedupeCheckpoint::default()
        };
        let checkpointed = db
            .checkpoint_dedupe_run(&claimed.id, claimed.revision, &checkpoint)
            .expect("checkpoint");
        assert!(db
            .checkpoint_dedupe_run(&claimed.id, claimed.revision, &checkpoint)
            .is_err());
        assert!(db
            .claim_dedupe_run(&first.run.id)
            .expect("second claim")
            .is_none());
        let failed = db
            .mark_dedupe_terminal(
                &first.run.id,
                checkpointed.revision,
                "failed",
                Some("test_failure"),
                Some("synthetic failure"),
                &checkpoint,
            )
            .expect("terminal failure");
        assert_eq!(failed.status, "failed");

        let retry = db.retry_dedupe_run(&failed.id).expect("retry failed run");
        assert!(retry.created);
        assert_ne!(retry.run.id, failed.id);
        assert_eq!(retry.run.request_key, "manual-admission");
        assert_eq!(retry.run.request_attempt, 2);
        let current = db
            .start_dedupe_run(&request)
            .expect("current attempt replay");
        assert!(!current.created);
        assert_eq!(current.run.id, retry.run.id);

        let interrupted = db.recover_dedupe_runs().expect("startup recovery");
        assert_eq!(interrupted, 1);
        let recovered = db.get_dedupe_run(&retry.run.id).expect("recovered run");
        assert_eq!(recovered.status, "interrupted");
        let retry_after_restart = db
            .retry_dedupe_run(&recovered.id)
            .expect("retry after restart");
        assert!(retry_after_restart.created);
        assert_eq!(retry_after_restart.run.request_attempt, 3);
        assert_eq!(retry_after_restart.run.request_key, "manual-admission");

        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn cancellation_preserves_completed_fingerprint_and_skips_partial_publication() {
        let db_path = std::env::temp_dir().join(format!(
            "zen-canvas-dedupe-cancel-{}-{}.sqlite3",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let db = Database::open(&db_path).expect("database");
        let conn = db.conn().expect("connection");
        conn.execute(
            "INSERT INTO scan_roots (id, normalized_path, display_name, source_kind, enabled, health_status, needs_reconciliation, created_at, updated_at) VALUES ('cancel-root', '/tmp/cancel-root', 'cancel-root', 'file_library', 1, 'healthy', 0, 1, 1)",
            [],
        )
        .expect("managed root");
        db.insert_file(InsertFileRequest {
            id: "/tmp/cancel-root/file.bin".into(),
            path: "/tmp/cancel-root/file.bin".into(),
            name: "file.bin".into(),
            extension: "bin".into(),
            size: 4,
            mtime: 11,
            ctime: 11,
            is_dir: false,
            state_code: 0,
        })
        .expect("cancel fixture file");
        let admission = db
            .start_dedupe_run(&StartDedupeRunRequest {
                scope: DedupeScopeRequest {
                    kind: "explicitEnabledScanRoots".into(),
                    root_ids: vec!["cancel-root".into()],
                },
                request_key: Some("cancel-request".into()),
                parent_scan_session_id: None,
            })
            .expect("admission");
        let running = db
            .claim_dedupe_run(&admission.run.id)
            .expect("claim")
            .expect("running");
        db.upsert_physical_identity(
            &DedupeCandidate {
                file_id: "/tmp/cancel-root/file.bin".into(),
                path: "/tmp/cancel-root/file.bin".into(),
                size: 4,
                mtime: 11,
            },
            &PhysicalFileIdentity {
                size: 4,
                modified_ns: Some(22),
                created_ns: None,
                platform_kind: crate::fs_safety::PhysicalPlatform::Other,
                platform_volume_id: None,
                platform_file_id: None,
                physical_key: None,
                link_count: None,
            },
        )
        .expect("persist completed identity");
        let cancelling = db
            .request_dedupe_cancellation(&running.id)
            .expect("request cancellation");
        assert_eq!(cancelling.status, "cancelling");
        let cancelled = db
            .mark_dedupe_terminal(
                &running.id,
                cancelling.revision,
                "cancelled",
                Some("cancelled"),
                Some("synthetic cancellation"),
                &DedupeCheckpoint {
                    phase: "full_hashing".into(),
                    identity_verified_files: 1,
                    ..DedupeCheckpoint::default()
                },
            )
            .expect("terminal cancellation");
        assert_eq!(cancelled.status, "cancelled");
        assert!(db
            .get_fingerprint("/tmp/cancel-root/file.bin")
            .expect("retained fingerprint")
            .is_some());
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM duplicate_groups WHERE status = 'active'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("partial groups"),
            0
        );
        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn duplicate_group_cursor_is_keyset_strict_and_rejects_invalid_input() {
        let db_path = std::env::temp_dir().join(format!(
            "zen-canvas-dedupe-cursor-{}-{}.sqlite3",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let db = Database::open(&db_path).expect("database");
        let conn = db.conn().expect("connection");
        conn.execute(
            r#"
            INSERT INTO dedupe_runs (
                id, request_key, scope_json, scope_hash, scope_snapshot_json,
                scope_snapshot_hash, status, phase, revision, created_at, updated_at
            ) VALUES ('cursor-run', 'cursor-request', '{}', 'cursor-scope', '[]',
                      'cursor-snapshot', 'completed', 'completed', 1, 1, 1)
            "#,
            [],
        )
        .expect("cursor run");
        for (index, potential) in [300_i64, 200, 100].into_iter().enumerate() {
            conn.execute(
                r#"
                INSERT INTO duplicate_groups (
                    id, size_each, full_hash, full_hash_algorithm, full_hash_version,
                    member_count, physical_copy_count, hardlink_alias_count,
                    exact_reclaimable_bytes, potential_reclaimable_bytes,
                    reclaimable_confidence, status, last_built_run_id, revision,
                    created_at, updated_at, last_verified_at
                ) VALUES (?1, 10, ?2, 'blake3', 1, 2, 2, 0, ?3, ?3,
                          'exact', 'active', 'cursor-run', 1, 1, 1, 1)
                "#,
                params![
                    format!("cursor-group-{index}"),
                    format!("cursor-hash-{index}"),
                    potential
                ],
            )
            .expect("cursor group");
        }
        db.insert_file(InsertFileRequest {
            id: "/tmp/cursor-member.txt".into(),
            path: "/tmp/cursor-member.txt".into(),
            name: "cursor-member.txt".into(),
            extension: "txt".into(),
            size: 10,
            mtime: 1,
            ctime: 1,
            is_dir: false,
            state_code: 0,
        })
        .expect("cursor member file");
        conn.execute(
            "INSERT INTO duplicate_group_members (group_id, file_id, path_snapshot, physical_key, identity_status, size, verified_at) VALUES ('cursor-group-0', '/tmp/cursor-member.txt', '/tmp/cursor-member.txt', 'unix:v1:cursor:0', 'verified', 10, 1)",
            [],
        )
        .expect("cursor member");
        let memberships = db
            .get_file_duplicate_membership("/tmp/cursor-member.txt")
            .expect("file membership");
        assert_eq!(memberships.len(), 1);
        assert_eq!(memberships[0].id, "cursor-group-0");
        assert_eq!(
            db.list_duplicate_group_members("cursor-group-0")
                .expect("group members")
                .len(),
            1
        );

        let first_page = db
            .list_duplicate_groups(None, 2)
            .expect("first cursor page");
        assert_eq!(first_page.groups.len(), 2);
        let cursor = first_page.next_cursor.clone().expect("next cursor");
        let second_page = db
            .list_duplicate_groups(Some(&cursor), 2)
            .expect("second cursor page");
        assert_eq!(second_page.groups.len(), 1);
        assert!(second_page.next_cursor.is_none());
        assert!(first_page.groups.iter().all(|first| second_page
            .groups
            .iter()
            .all(|second| first.id != second.id)));
        assert!(db.list_duplicate_groups(Some("not-json"), 2).is_err());

        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn scan_scope_change_marks_warning_and_reopens_dispatch_without_replacing_groups() {
        let db_path = std::env::temp_dir().join(format!(
            "zen-canvas-dedupe-snapshot-{}-{}.sqlite3",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let db = Database::open(&db_path).expect("database");
        let conn = db.conn().expect("connection");
        conn.execute(
            "INSERT INTO scan_roots (id, normalized_path, display_name, source_kind, enabled, health_status, needs_reconciliation, created_at, updated_at) VALUES ('snapshot-root', '/tmp/snapshot-root', 'snapshot-root', 'file_library', 1, 'healthy', 0, 1, 1)",
            [],
        )
        .expect("managed root");
        conn.execute(
            "INSERT INTO scan_sessions (id, status, phase, dedupe_requested, dedupe_dispatch_state, created_at, updated_at) VALUES ('snapshot-session', 'completed', 'completed', 1, 'dispatched', 1, 1)",
            [],
        )
        .expect("scan session");
        conn.execute(
            "INSERT INTO scan_session_roots (session_id, requested_index, requested_path, normalized_requested_path, resolution, effective_root_id, effective_path, effective_index, status, created_at, updated_at) VALUES ('snapshot-session', 0, '/tmp/snapshot-root', '/tmp/snapshot-root', 'effective', 'snapshot-root', '/tmp/snapshot-root', 0, 'completed', 1, 1)",
            [],
        )
        .expect("scan session root mapping");

        let admission = db
            .start_dedupe_run(&StartDedupeRunRequest {
                scope: DedupeScopeRequest {
                    kind: "allManagedFileLibrary".into(),
                    root_ids: vec![],
                },
                request_key: Some("scan-session:snapshot-session".into()),
                parent_scan_session_id: Some("snapshot-session".into()),
            })
            .expect("admission");
        let running = db
            .claim_dedupe_run(&admission.run.id)
            .expect("claim")
            .expect("running");
        conn.execute(
            r#"
            INSERT INTO duplicate_groups (
                id, size_each, full_hash, full_hash_algorithm, full_hash_version,
                member_count, physical_copy_count, hardlink_alias_count,
                exact_reclaimable_bytes, potential_reclaimable_bytes,
                reclaimable_confidence, status, last_built_run_id, revision,
                created_at, updated_at, last_verified_at
            ) VALUES ('preexisting-snapshot-group', 4, 'preexisting-hash', 'blake3', 1,
                      2, 2, 0, 4, 4, 'exact', 'active', ?1, 1, 1, 1, 1)
            "#,
            [&running.id],
        )
        .expect("preexisting active group");
        conn.execute(
            "UPDATE scan_roots SET watcher_revision = watcher_revision + 1 WHERE id = 'snapshot-root'",
            [],
        )
        .expect("change scope snapshot");
        let outcome = db
            .publish_dedupe_groups(
                &running.id,
                &[],
                &DedupeCheckpoint {
                    phase: "finalizing".into(),
                    ..DedupeCheckpoint::default()
                },
            )
            .expect("scope warning publication");
        assert_eq!(outcome, PublishOutcome::CompletedWithWarnings);
        let final_run = db.get_dedupe_run(&running.id).expect("final run");
        assert_eq!(final_run.status, "completed_with_warnings");
        assert!(final_run.rerun_required);
        assert_eq!(
            final_run.error_code.as_deref(),
            Some("scope_changed_during_run")
        );
        let preserved_group_status: String = conn
            .query_row(
                "SELECT status FROM duplicate_groups WHERE id = 'preexisting-snapshot-group'",
                [],
                |row| row.get(0),
            )
            .expect("preserved active group");
        assert_eq!(preserved_group_status, "active");
        let session_state: String = conn
            .query_row(
                "SELECT dedupe_dispatch_state FROM scan_sessions WHERE id = 'snapshot-session'",
                [],
                |row| row.get(0),
            )
            .expect("dispatch state");
        assert_eq!(session_state, "pending");

        let retry = db
            .retry_dedupe_run(&final_run.id)
            .expect("follow-up attempt");
        assert!(retry.created);
        assert_eq!(retry.run.request_attempt, 2);

        let second_running = db
            .claim_dedupe_run(&retry.run.id)
            .expect("claim second attempt")
            .expect("second attempt running");
        conn.execute(
            "UPDATE scan_roots SET watcher_revision = watcher_revision + 1 WHERE id = 'snapshot-root'",
            [],
        )
        .expect("change scope snapshot for second attempt");
        db.publish_dedupe_groups(
            &second_running.id,
            &[],
            &DedupeCheckpoint {
                phase: "finalizing".into(),
                ..DedupeCheckpoint::default()
            },
        )
        .expect("second scope warning publication");
        let second_final = db
            .get_dedupe_run(&second_running.id)
            .expect("second final run");
        assert!(second_final.rerun_required);

        let third = db
            .retry_dedupe_run(&second_final.id)
            .expect("third attempt");
        let third_running = db
            .claim_dedupe_run(&third.run.id)
            .expect("claim third attempt")
            .expect("third attempt running");
        conn.execute(
            "UPDATE scan_roots SET watcher_revision = watcher_revision + 1 WHERE id = 'snapshot-root'",
            [],
        )
        .expect("change scope snapshot for third attempt");
        db.publish_dedupe_groups(
            &third_running.id,
            &[],
            &DedupeCheckpoint {
                phase: "finalizing".into(),
                ..DedupeCheckpoint::default()
            },
        )
        .expect("third scope warning publication");
        let third_final = db
            .get_dedupe_run(&third_running.id)
            .expect("third final run");
        assert!(!third_final.rerun_required);
        assert_eq!(
            third_final.error_code.as_deref(),
            Some("scope_changed_retry_exhausted")
        );
        let suppressed_replay = db
            .start_dedupe_run_for_scan_session("snapshot-session")
            .expect("suppressed scan replay");
        assert!(!suppressed_replay.created);
        assert_eq!(suppressed_replay.run.id, third_final.id);

        let _ = fs::remove_file(db_path);
    }

    #[test]
    fn retention_prune_is_bounded_and_skips_while_a_run_is_active() {
        let db_path = std::env::temp_dir().join(format!(
            "zen-canvas-dedupe-prune-{}-{}.sqlite3",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let db = Database::open(&db_path).expect("database");
        let old = current_unix_seconds().saturating_sub(91 * 24 * 60 * 60);
        let conn = db.conn().expect("connection");
        conn.execute(
            "INSERT INTO scan_roots (id, normalized_path, display_name, source_kind, enabled, health_status, needs_reconciliation, created_at, updated_at) VALUES ('prune-root', '/tmp/prune-root', 'prune-root', 'file_library', 1, 'healthy', 0, 1, 1)",
            [],
        )
        .expect("managed root");
        db.insert_file(InsertFileRequest {
            id: "prune-file".into(),
            path: "/tmp/prune-root/prune.txt".into(),
            name: "prune.txt".into(),
            extension: "txt".into(),
            size: 4,
            mtime: 1,
            ctime: 1,
            is_dir: false,
            state_code: 0,
        })
        .expect("prune file");
        conn.execute(
            r#"
            INSERT INTO dedupe_runs (
                id, request_key, request_attempt, scope_json, scope_hash,
                scope_snapshot_json, scope_snapshot_hash, status, phase, revision,
                finished_at, created_at, updated_at
            ) VALUES ('old-prune-run', 'old-prune-request', 1, '{}', 'old-prune-scope', '[]',
                      'old-prune-snapshot', 'completed', 'completed', 1, ?1, ?1, ?1)
            "#,
            params![old],
        )
        .expect("old run");
        conn.execute(
            "INSERT INTO file_fingerprints (file_id, path_snapshot, identity_status, size, fingerprint_status, captured_at, last_verified_at) VALUES ('prune-file', '/tmp/prune-root/prune.txt', 'stale', 4, 'stale', ?1, ?1)",
            params![old],
        )
        .expect("old fingerprint");
        conn.execute(
            "INSERT INTO duplicate_groups (id, size_each, full_hash, member_count, physical_copy_count, exact_reclaimable_bytes, potential_reclaimable_bytes, reclaimable_confidence, status, last_built_run_id, created_at, updated_at, last_verified_at) VALUES ('old-prune-group', 4, 'old-prune-hash', 2, 2, 4, 4, 'exact', 'stale', 'old-prune-run', ?1, ?1, ?1)",
            params![old],
        )
        .expect("old group");
        conn.execute(
            "INSERT INTO duplicate_group_members (group_id, file_id, path_snapshot, identity_status, size, verified_at) VALUES ('old-prune-group', 'prune-file', '/tmp/prune-root/prune.txt', 'stale', 4, ?1)",
            params![old],
        )
        .expect("old member");
        conn.execute(
            "INSERT INTO dedupe_run_errors (id, run_id, path_snapshot, phase, error_code, error_message, created_at) VALUES ('old-prune-error', 'old-prune-run', '', 'full_hashing', 'old', 'old', ?1)",
            params![old],
        )
        .expect("old error");

        let active = db
            .start_dedupe_run(&StartDedupeRunRequest {
                scope: DedupeScopeRequest {
                    kind: "explicitEnabledScanRoots".into(),
                    root_ids: vec!["prune-root".into()],
                },
                request_key: Some("active-prune-request".into()),
                parent_scan_session_id: None,
            })
            .expect("active run");
        assert_eq!(db.prune_dedupe_artifacts().expect("skip prune"), 0);
        db.mark_dedupe_terminal(
            &active.run.id,
            active.run.revision,
            "failed",
            Some("test"),
            Some("finished"),
            &DedupeCheckpoint::default(),
        )
        .expect("finish active run");
        let deleted = db.prune_dedupe_artifacts().expect("prune old rows");
        assert!(deleted >= 3);
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM duplicate_groups WHERE id = 'old-prune-group'",
                [],
                |row| row.get(0),
            )
            .expect("group retention check");
        assert_eq!(remaining, 0);

        let _ = fs::remove_file(db_path);
    }

    #[test]
    #[ignore = "bounded Task 02 repository benchmark; invoked by npm run test:performance"]
    fn performance_task02_repository_100k_and_group_pages() {
        let db_path = std::env::temp_dir().join(format!(
            "zen-canvas-dedupe-performance-{}-{}.sqlite3",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let db = Database::open(&db_path).expect("database");
        let root_path = "/tmp/task02-performance-root";
        let root_id = "task02-performance-root";
        let now = current_unix_seconds();
        {
            let conn = db.conn().expect("connection");
            conn.execute(
                "INSERT INTO scan_roots (id, normalized_path, display_name, source_kind, enabled, health_status, needs_reconciliation, created_at, updated_at) VALUES (?1, ?2, ?3, 'file_library', 1, 'healthy', 0, ?4, ?4)",
                params![root_id, root_path, root_id, now],
            )
            .expect("managed root");
            let tx = conn.unchecked_transaction().expect("seed transaction");
            for index in 0..100_000_i64 {
                let path = format!("{root_path}/file-{index:06}.bin");
                tx.execute(
                    "INSERT INTO files (id, path, name, extension, size, mtime, is_dir, state_code) VALUES (?1, ?1, ?2, 'bin', 4096, 1, 0, 0)",
                    params![path, format!("file-{index:06}.bin")],
                )
                .expect("seed file");
            }
            tx.commit().expect("commit files");
        }

        let scope = json!({
            "kind": "explicit_enabled_scan_roots",
            "root_ids": [root_id]
        });
        let candidate_started = Instant::now();
        let candidates = db
            .collect_dedupe_candidates(&scope)
            .expect("collect 100k candidates");
        let candidate_elapsed = candidate_started.elapsed();
        assert_eq!(candidates.len(), 100_000);

        let fingerprint_started = Instant::now();
        {
            let conn = db.conn().expect("connection");
            let tx = conn
                .unchecked_transaction()
                .expect("fingerprint transaction");
            for index in 0..100_000_i64 {
                let path = format!("{root_path}/file-{index:06}.bin");
                tx.execute(
                    "INSERT INTO file_fingerprints (file_id, path_snapshot, identity_status, size, physical_key, full_hash, fingerprint_status, captured_at, last_verified_at) VALUES (?1, ?1, 'path_only', 4096, NULL, ?2, 'complete', ?3, ?3)",
                    params![path, format!("benchmark-hash-{index:06}"), now],
                )
                .expect("seed fingerprint");
            }
            let indexed_count: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM file_fingerprints WHERE size = 4096 AND fingerprint_status = 'complete'",
                    [],
                    |row| row.get(0),
                )
                .expect("fingerprint indexed query");
            assert_eq!(indexed_count, 100_000);
            let stale_at = now.saturating_sub(31 * 24 * 60 * 60);
            for index in 0..1_000_i64 {
                let path = format!("{root_path}/stale-{index:04}.bin");
                tx.execute(
                    "INSERT INTO files (id, path, name, extension, size, mtime, is_dir, state_code) VALUES (?1, ?1, ?2, 'bin', 8192, 1, 0, 0)",
                    params![path, format!("stale-{index:04}.bin")],
                )
                .expect("seed stale file");
                tx.execute(
                    "INSERT INTO file_fingerprints (file_id, path_snapshot, identity_status, size, fingerprint_status, captured_at, last_verified_at) VALUES (?1, ?1, 'stale', 8192, 'stale', ?2, ?2)",
                    params![path, stale_at],
                )
                .expect("seed stale fingerprint");
            }
            tx.commit().expect("commit fingerprints");
        }
        let fingerprint_elapsed = fingerprint_started.elapsed();

        let admission = db
            .start_dedupe_run(&StartDedupeRunRequest {
                scope: DedupeScopeRequest {
                    kind: "allManagedFileLibrary".into(),
                    root_ids: Vec::new(),
                },
                request_key: Some("task02-performance-run".into()),
                parent_scan_session_id: None,
            })
            .expect("performance run admission");
        let running = db
            .claim_dedupe_run(&admission.run.id)
            .expect("claim performance run")
            .expect("performance run is claimable");
        let publication_started = Instant::now();
        let groups = (0..10_000_i64)
            .map(|index| BuiltGroup {
                id: format!("task02-performance-group-{index:05}"),
                size_each: 4096,
                full_hash: format!("group-hash-{index:05}"),
                members: (0..2_i64)
                    .map(|member_index| {
                        let file_index = index * 2 + member_index;
                        let path = format!("{root_path}/file-{file_index:06}.bin");
                        BuiltMember {
                            file_id: path.clone(),
                            path_snapshot: path,
                            physical_key: None,
                            identity_status: "path_only".into(),
                            is_hardlink_alias: false,
                            size: 4096,
                            modified_ns: None,
                            verified_at: now,
                        }
                    })
                    .collect(),
                physical_copy_count: 2,
                hardlink_alias_count: 0,
                exact_reclaimable_bytes: None,
                potential_reclaimable_bytes: 4096,
                reclaimable_confidence: "estimated".into(),
            })
            .collect::<Vec<_>>();
        let checkpoint = DedupeCheckpoint {
            phase: "finalizing".into(),
            candidate_files: 100_000,
            candidate_bytes: 409_600_000,
            processed_files: 100_000,
            processed_bytes: 409_600_000,
            total_bytes: 409_600_000,
            duplicate_groups: 10_000,
            duplicate_members: 20_000,
            potential_reclaimable_bytes: 40_960_000,
            ..DedupeCheckpoint::default()
        };
        assert_eq!(
            db.publish_dedupe_groups(&running.id, &groups, &checkpoint)
                .expect("publish performance groups"),
            PublishOutcome::Completed
        );
        let publication_elapsed = publication_started.elapsed();

        let page_started = Instant::now();
        let page = db
            .list_duplicate_groups(None, 100)
            .expect("first keyset group page");
        let page_elapsed = page_started.elapsed();
        assert_eq!(page.groups.len(), 100);
        assert!(page.next_cursor.is_some());

        let prune_started = Instant::now();
        let pruned = db
            .prune_dedupe_artifacts()
            .expect("bounded retention prune");
        let prune_elapsed = prune_started.elapsed();
        assert_eq!(pruned, 1_000);

        println!(
            "Task 02 performance: candidate_collection_ms={:.3}, fingerprint_batch_write_and_index_query_ms={:.3}, publication_10k_groups_20k_members_ms={:.3}, keyset_page_100_ms={:.3}, prune_1000_cap_ms={:.3}",
            candidate_elapsed.as_secs_f64() * 1000.0,
            fingerprint_elapsed.as_secs_f64() * 1000.0,
            publication_elapsed.as_secs_f64() * 1000.0,
            page_elapsed.as_secs_f64() * 1000.0,
            prune_elapsed.as_secs_f64() * 1000.0,
        );

        let _ = fs::remove_file(db_path);
    }
}
