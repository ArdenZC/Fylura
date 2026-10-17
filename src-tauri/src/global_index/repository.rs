use super::models::*;
use crate::db::{Database, DbError};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::Serialize;
use std::{
    collections::HashSet,
    path::Path,
    sync::{Mutex, OnceLock},
};

static LOCAL_GLOBAL_INDEX_OPERATIONS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceOperationKind {
    Initial,
    Incremental,
    Rebuild,
}

impl SourceOperationKind {
    fn as_db_value(self) -> &'static str {
        match self {
            Self::Initial => "initial",
            Self::Incremental => "incremental",
            Self::Rebuild => "rebuild",
        }
    }

    fn admitted_status(self) -> &'static str {
        match self {
            Self::Initial | Self::Rebuild => INDEX_STATUS_INDEXING,
            Self::Incremental => INDEX_STATUS_SYNCING,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceOperationRoute {
    Coordinator,
    Service,
    Direct,
}

impl SourceOperationRoute {
    fn as_db_value(self) -> &'static str {
        match self {
            Self::Coordinator => "coordinator",
            Self::Service => "service",
            Self::Direct => "direct",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceOperationOwner {
    pub volume_id: String,
    pub revision: i64,
    pub operation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceOperationState {
    pub revision: i64,
    pub owner_id: Option<String>,
    pub operation_state: Option<String>,
    pub operation_route: Option<String>,
    pub operation_kind: Option<String>,
    pub rebuild_intent_revision: i64,
    pub rebuild_completed_revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GlobalIndexOperationClaimSnapshot {
    revision: i64,
    owner_id: Option<String>,
    owner_pid: Option<i64>,
    owner_started_at: Option<i64>,
    owner_route: Option<String>,
    operation_kind: Option<String>,
    operation_state: Option<String>,
}

impl SourceOperationState {
    pub fn rebuild_pending(&self) -> bool {
        self.rebuild_intent_revision > self.rebuild_completed_revision
    }
}

#[derive(Debug, Clone)]
pub(crate) struct GlobalSearchSnapshot {
    pub results: Vec<GlobalSearchResult>,
    pub source_health: Vec<GlobalSearchSourceHealth>,
    pub source_revision: String,
    pub index_status: GlobalIndexStatus,
}

#[derive(Debug, Serialize)]
struct GlobalSearchRevisionFact {
    source_id: String,
    enabled: bool,
    provider: String,
    status: String,
    last_error: Option<String>,
    updated_at: i64,
    active_entry_count: i64,
    max_last_seen_at: Option<i64>,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GlobalSearchSourceHealthQueryCandidate {
    Original,
    CorrelatedAggregates,
    NarrowAggregate,
    #[cfg(test)]
    GroupByVolumeId,
}

const GLOBAL_SEARCH_SOURCE_HEALTH_SQL: &str = r#"
    WITH active_volume_facts AS (
        SELECT volume_id,
               COUNT(id) AS active_entry_count,
               MAX(last_seen_at) AS max_last_seen_at
        FROM global_entries
        WHERE is_stale = 0
        GROUP BY volume_id
    )
    SELECT gv.id, gv.enabled, gv.provider, gv.index_status, gv.last_error, gv.updated_at,
           COALESCE(active_volume_facts.active_entry_count, 0),
           active_volume_facts.max_last_seen_at
    FROM global_volumes gv
    LEFT JOIN active_volume_facts ON active_volume_facts.volume_id = gv.id
    ORDER BY gv.id ASC
    "#;

#[cfg(test)]
std::thread_local! {
    static GLOBAL_SEARCH_SOURCE_HEALTH_CANDIDATE_OVERRIDE: std::cell::Cell<Option<GlobalSearchSourceHealthQueryCandidate>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
impl GlobalSearchSourceHealthQueryCandidate {
    pub(crate) fn sql(self) -> &'static str {
        match self {
            Self::Original => {
                r#"
                SELECT gv.id, gv.enabled, gv.provider, gv.index_status, gv.last_error, gv.updated_at,
                       COUNT(ge.id), MAX(ge.last_seen_at)
                FROM global_volumes gv
                LEFT JOIN global_entries ge
                  ON ge.volume_id = gv.id AND ge.is_stale = 0
                GROUP BY gv.id, gv.enabled, gv.provider, gv.index_status, gv.last_error, gv.updated_at
                ORDER BY gv.id ASC
                "#
            }
            Self::CorrelatedAggregates => {
                r#"
                SELECT gv.id, gv.enabled, gv.provider, gv.index_status, gv.last_error, gv.updated_at,
                       (SELECT COUNT(ge.id)
                        FROM global_entries ge
                        WHERE ge.volume_id = gv.id AND ge.is_stale = 0),
                       (SELECT MAX(ge.last_seen_at)
                        FROM global_entries ge
                        WHERE ge.volume_id = gv.id AND ge.is_stale = 0)
                FROM global_volumes gv
                ORDER BY gv.id ASC
                "#
            }
            Self::NarrowAggregate => GLOBAL_SEARCH_SOURCE_HEALTH_SQL,
            #[cfg(test)]
            Self::GroupByVolumeId => {
                r#"
                SELECT gv.id, gv.enabled, gv.provider, gv.index_status, gv.last_error, gv.updated_at,
                       COUNT(ge.id), MAX(ge.last_seen_at)
                FROM global_volumes gv
                LEFT JOIN global_entries ge
                  ON ge.volume_id = gv.id AND ge.is_stale = 0
                GROUP BY gv.id
                ORDER BY gv.id ASC
                "#
            }
        }
    }
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiagnosticSourceHealth {
    pub(crate) source_health: Vec<GlobalSearchSourceHealth>,
    pub(crate) source_revision: String,
    pub(crate) revision_facts_json: Vec<u8>,
}

struct GlobalIndexStatusCounts {
    total_entries: i64,
    indexed_volumes: i64,
    ready_volumes: i64,
    pending_volumes: i64,
    spotlight_not_indexed_volumes: i64,
    permission_required_volumes: i64,
    spotlight_unavailable_volumes: i64,
    spotlight_external_not_indexed_volumes: i64,
    fsevents_unavailable_volumes: i64,
    unavailable_volumes: i64,
    error_volumes: i64,
    paused_volumes: i64,
    last_sync_at: Option<i64>,
    last_error: Option<String>,
}

impl Database {
    pub fn upsert_global_volume(&self, volume: &GlobalVolume) -> Result<(), DbError> {
        let conn = self.conn()?;
        conn.execute(
            r#"
            INSERT INTO global_volumes (
                id, platform, stable_volume_id, display_name, mount_path,
                filesystem_type, drive_kind, enabled, provider, index_status,
                last_error, journal_id, journal_cursor, last_full_index_at,
                last_incremental_sync_at, entry_count, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
            ON CONFLICT(stable_volume_id) DO UPDATE SET
                platform = excluded.platform,
                display_name = excluded.display_name,
                mount_path = excluded.mount_path,
                filesystem_type = excluded.filesystem_type,
                drive_kind = excluded.drive_kind,
                provider = excluded.provider,
                operation_revision = operation_revision + CASE WHEN
                    platform IS NOT excluded.platform OR
                    mount_path IS NOT excluded.mount_path OR
                    filesystem_type IS NOT excluded.filesystem_type OR
                    drive_kind IS NOT excluded.drive_kind OR
                    provider IS NOT excluded.provider
                    THEN 1 ELSE 0 END,
                operation_owner_id = CASE WHEN
                    platform IS NOT excluded.platform OR
                    mount_path IS NOT excluded.mount_path OR
                    filesystem_type IS NOT excluded.filesystem_type OR
                    drive_kind IS NOT excluded.drive_kind OR
                    provider IS NOT excluded.provider
                    THEN NULL ELSE operation_owner_id END,
                operation_owner_pid = CASE WHEN
                    platform IS NOT excluded.platform OR
                    mount_path IS NOT excluded.mount_path OR
                    filesystem_type IS NOT excluded.filesystem_type OR
                    drive_kind IS NOT excluded.drive_kind OR
                    provider IS NOT excluded.provider
                    THEN NULL ELSE operation_owner_pid END,
                operation_owner_started_at = CASE WHEN
                    platform IS NOT excluded.platform OR
                    mount_path IS NOT excluded.mount_path OR
                    filesystem_type IS NOT excluded.filesystem_type OR
                    drive_kind IS NOT excluded.drive_kind OR
                    provider IS NOT excluded.provider
                    THEN NULL ELSE operation_owner_started_at END,
                operation_owner_route = CASE WHEN
                    platform IS NOT excluded.platform OR
                    mount_path IS NOT excluded.mount_path OR
                    filesystem_type IS NOT excluded.filesystem_type OR
                    drive_kind IS NOT excluded.drive_kind OR
                    provider IS NOT excluded.provider
                    THEN NULL ELSE operation_owner_route END,
                operation_kind = CASE WHEN
                    platform IS NOT excluded.platform OR
                    mount_path IS NOT excluded.mount_path OR
                    filesystem_type IS NOT excluded.filesystem_type OR
                    drive_kind IS NOT excluded.drive_kind OR
                    provider IS NOT excluded.provider
                    THEN NULL ELSE operation_kind END,
                operation_state = CASE WHEN
                    platform IS NOT excluded.platform OR
                    mount_path IS NOT excluded.mount_path OR
                    filesystem_type IS NOT excluded.filesystem_type OR
                    drive_kind IS NOT excluded.drive_kind OR
                    provider IS NOT excluded.provider
                    THEN NULL ELSE operation_state END,
                updated_at = excluded.updated_at
            "#,
            params![
                volume.id,
                volume.platform,
                volume.stable_volume_id,
                volume.display_name,
                volume.mount_path,
                volume.filesystem_type,
                volume.drive_kind,
                bool_to_i64(volume.enabled),
                volume.provider,
                volume.index_status,
                volume.last_error,
                volume.journal_id,
                volume.journal_cursor,
                volume.last_full_index_at,
                volume.last_incremental_sync_at,
                volume.entry_count,
                volume.created_at,
                volume.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn list_global_volumes(&self) -> Result<Vec<GlobalVolume>, DbError> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            r#"
            SELECT id, platform, stable_volume_id, display_name, mount_path,
                   filesystem_type, drive_kind, enabled, provider, index_status,
                   last_error, journal_id, journal_cursor, last_full_index_at,
                   last_incremental_sync_at, entry_count, created_at, updated_at
            FROM global_volumes
            ORDER BY mount_path COLLATE NOCASE
            "#,
        )?;
        let rows = statement.query_map([], map_global_volume)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(DbError::from)
    }

    pub fn get_global_volume(&self, id: &str) -> Result<Option<GlobalVolume>, DbError> {
        let conn = self.conn()?;
        conn.query_row(
            r#"
            SELECT id, platform, stable_volume_id, display_name, mount_path,
                   filesystem_type, drive_kind, enabled, provider, index_status,
                   last_error, journal_id, journal_cursor, last_full_index_at,
                   last_incremental_sync_at, entry_count, created_at, updated_at
            FROM global_volumes WHERE id = ?1
            "#,
            params![id],
            map_global_volume,
        )
        .optional()
        .map_err(DbError::from)
    }

    pub(crate) fn global_index_operation_state(
        &self,
        id: &str,
    ) -> Result<Option<SourceOperationState>, DbError> {
        let conn = self.conn()?;
        conn.query_row(
            r#"
            SELECT operation_revision, operation_owner_id, operation_state,
                   operation_owner_route, operation_kind,
                   rebuild_intent_revision, rebuild_completed_revision
            FROM global_volumes WHERE id = ?1
            "#,
            params![id],
            |row| {
                Ok(SourceOperationState {
                    revision: row.get(0)?,
                    owner_id: row.get(1)?,
                    operation_state: row.get(2)?,
                    operation_route: row.get(3)?,
                    operation_kind: row.get(4)?,
                    rebuild_intent_revision: row.get(5)?,
                    rebuild_completed_revision: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(DbError::from)
    }

    /// Persist a coalesced rebuild request without changing committed source health
    /// or the active index. The intent is cleared only by a successful owned rebuild.
    pub(crate) fn request_global_index_rebuild(&self, id: &str) -> Result<bool, DbError> {
        let conn = self.conn()?;
        let changed = conn.execute(
            r#"
            UPDATE global_volumes
            SET rebuild_intent_revision = CASE
                    WHEN rebuild_intent_revision = rebuild_completed_revision
                    THEN rebuild_intent_revision + 1
                    ELSE rebuild_intent_revision
                END
            WHERE id = ?1 AND enabled = 1
            "#,
            params![id],
        )?;
        Ok(changed == 1)
    }

    /// Claim one source only after the caller has real operation admission.
    /// The revision fences every later provider write and terminal result.
    pub(crate) fn claim_global_index_operation(
        &self,
        volume: &GlobalVolume,
        operation_id: &str,
        kind: SourceOperationKind,
        route: SourceOperationRoute,
    ) -> Result<Option<SourceOperationOwner>, DbError> {
        self.claim_global_index_operation_with_process_check(
            volume,
            operation_id,
            kind,
            route,
            process_instance_is_alive,
        )
    }

    #[cfg(test)]
    pub(crate) fn claim_global_index_operation_with_process_check_for_test<F>(
        &self,
        volume: &GlobalVolume,
        operation_id: &str,
        kind: SourceOperationKind,
        route: SourceOperationRoute,
        process_is_alive: F,
    ) -> Result<Option<SourceOperationOwner>, DbError>
    where
        F: FnMut(i64, i64) -> Option<bool>,
    {
        self.claim_global_index_operation_with_process_check(
            volume,
            operation_id,
            kind,
            route,
            process_is_alive,
        )
    }

    fn claim_global_index_operation_with_process_check<F>(
        &self,
        volume: &GlobalVolume,
        operation_id: &str,
        kind: SourceOperationKind,
        route: SourceOperationRoute,
        mut process_is_alive: F,
    ) -> Result<Option<SourceOperationOwner>, DbError>
    where
        F: FnMut(i64, i64) -> Option<bool>,
    {
        let (owner_pid, owner_started_at) = current_process_identity()?;

        // Keep both the pooled connection and its read statement out of the OS
        // process lookup below. The lookup may refresh system process state and
        // must not run while SQLite's IMMEDIATE writer reservation is held.
        let precheck_snapshot = {
            let conn = self.conn()?;
            query_global_index_operation_claim_snapshot(&conn, volume)?
        };
        let Some(precheck_snapshot) = precheck_snapshot else {
            return Ok(None);
        };

        let (existing_owner_locally_active, requested_id_locally_active) = {
            let active_operations = LOCAL_GLOBAL_INDEX_OPERATIONS
                .get_or_init(|| Mutex::new(HashSet::new()))
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (
                precheck_snapshot
                    .owner_id
                    .as_ref()
                    .is_some_and(|owner_id| active_operations.contains(owner_id)),
                active_operations.contains(operation_id),
            )
        };
        if requested_id_locally_active
            || !global_index_operation_snapshot_allows_claim(
                &precheck_snapshot,
                owner_pid,
                owner_started_at,
                existing_owner_locally_active,
                route,
                &mut process_is_alive,
            )
        {
            return Ok(None);
        }

        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current_snapshot = query_global_index_operation_claim_snapshot(&transaction, volume)?;
        if current_snapshot.as_ref() != Some(&precheck_snapshot) {
            transaction.rollback()?;
            return Ok(None);
        }

        let next_revision = precheck_snapshot.revision.checked_add(1).ok_or_else(|| {
            DbError::Validation("global_index_operation_revision_exhausted".into())
        })?;
        let changed = transaction.execute(
            r#"
            UPDATE global_volumes
            SET operation_revision = ?4,
                operation_owner_id = ?5,
                operation_owner_pid = ?6,
                operation_owner_started_at = ?7,
                operation_owner_route = ?8,
                operation_kind = ?9,
                operation_state = 'running',
                index_status = ?11,
                last_error = CASE WHEN index_status = ?12 THEN last_error ELSE NULL END,
                updated_at = ?13
            WHERE id = ?1 AND stable_volume_id = ?2 AND provider = ?3
              AND enabled = 1 AND operation_revision = ?10
              AND operation_owner_id IS ?14
              AND operation_owner_pid IS ?15
              AND operation_owner_started_at IS ?16
              AND operation_owner_route IS ?17
              AND operation_kind IS ?18
              AND operation_state IS ?19
              AND NOT EXISTS (
                  SELECT 1 FROM global_volumes AS other_owner
                  WHERE other_owner.id <> ?1
                    AND other_owner.operation_owner_id = ?5
              )
            "#,
            params![
                volume.id,
                volume.stable_volume_id,
                volume.provider,
                next_revision,
                operation_id,
                owner_pid,
                owner_started_at,
                route.as_db_value(),
                kind.as_db_value(),
                precheck_snapshot.revision,
                kind.admitted_status(),
                INDEX_STATUS_PERMISSION_REQUIRED,
                unix_now(),
                precheck_snapshot.owner_id,
                precheck_snapshot.owner_pid,
                precheck_snapshot.owner_started_at,
                precheck_snapshot.owner_route,
                precheck_snapshot.operation_kind,
                precheck_snapshot.operation_state,
            ],
        )?;
        if changed != 1 {
            transaction.rollback()?;
            return Ok(None);
        }
        if !register_local_global_index_operation(operation_id) {
            transaction.rollback()?;
            return Ok(None);
        }
        if let Err(error) = transaction.commit() {
            unregister_local_global_index_operation(operation_id);
            return Err(DbError::from(error));
        }
        Ok(Some(SourceOperationOwner {
            volume_id: volume.id.clone(),
            revision: next_revision,
            operation_id: operation_id.to_string(),
        }))
    }

    pub(crate) fn mark_global_index_operation_unknown(
        &self,
        owner: &SourceOperationOwner,
    ) -> Result<bool, DbError> {
        let conn = self.conn()?;
        let changed = conn.execute(
            r#"
            UPDATE global_volumes SET operation_state = 'unknown'
            WHERE id = ?1 AND operation_revision = ?2
              AND operation_owner_id = ?3 AND operation_state = 'running'
            "#,
            params![owner.volume_id, owner.revision, owner.operation_id],
        )?;
        if changed == 1 {
            unregister_local_global_index_operation(&owner.operation_id);
        }
        Ok(changed == 1)
    }

    pub(crate) fn forget_local_global_index_operation(&self, owner: &SourceOperationOwner) {
        unregister_local_global_index_operation(&owner.operation_id);
    }

    pub(crate) fn finish_global_index_operation(
        &self,
        owner: &SourceOperationOwner,
        status: &str,
        error: Option<&str>,
        full_index_at: Option<i64>,
        incremental_sync_at: Option<i64>,
        complete_rebuild_intent: bool,
    ) -> Result<bool, DbError> {
        let conn = self.conn()?;
        let changed = conn.execute(
            r#"
            UPDATE global_volumes
            SET index_status = ?4,
                last_error = ?5,
                last_full_index_at = COALESCE(?6, last_full_index_at),
                last_incremental_sync_at = COALESCE(?7, last_incremental_sync_at),
                rebuild_completed_revision = CASE WHEN ?8 = 1
                    THEN rebuild_intent_revision ELSE rebuild_completed_revision END,
                operation_owner_id = NULL,
                operation_owner_pid = NULL,
                operation_owner_started_at = NULL,
                operation_owner_route = NULL,
                operation_kind = NULL,
                operation_state = NULL,
                updated_at = ?9
            WHERE id = ?1 AND operation_revision = ?2
              AND operation_owner_id = ?3 AND operation_state = 'running'
            "#,
            params![
                owner.volume_id,
                owner.revision,
                owner.operation_id,
                status,
                error,
                full_index_at,
                incremental_sync_at,
                bool_to_i64(complete_rebuild_intent),
                unix_now(),
            ],
        )?;
        if changed == 1 {
            unregister_local_global_index_operation(&owner.operation_id);
        }
        Ok(changed == 1)
    }

    pub(crate) fn update_global_volume_checkpoint_owned(
        &self,
        owner: &SourceOperationOwner,
        journal_id: Option<&str>,
        journal_cursor: Option<&str>,
    ) -> Result<bool, DbError> {
        let conn = self.conn()?;
        let changed = conn.execute(
            r#"
            UPDATE global_volumes
            SET journal_id = COALESCE(?4, journal_id),
                journal_cursor = COALESCE(?5, journal_cursor),
                updated_at = ?6
            WHERE id = ?1 AND operation_revision = ?2
              AND operation_owner_id = ?3 AND operation_state = 'running' AND enabled = 1
            "#,
            params![
                owner.volume_id,
                owner.revision,
                owner.operation_id,
                journal_id,
                journal_cursor,
                unix_now(),
            ],
        )?;
        Ok(changed == 1)
    }

    pub(crate) fn update_global_volume_state_owned(
        &self,
        owner: &SourceOperationOwner,
        status: &str,
        error: Option<&str>,
    ) -> Result<bool, DbError> {
        let conn = self.conn()?;
        let changed = conn.execute(
            r#"
            UPDATE global_volumes
            SET index_status = ?4, last_error = ?5, updated_at = ?6
            WHERE id = ?1 AND operation_revision = ?2
              AND operation_owner_id = ?3 AND operation_state = 'running' AND enabled = 1
            "#,
            params![
                owner.volume_id,
                owner.revision,
                owner.operation_id,
                status,
                error,
                unix_now(),
            ],
        )?;
        Ok(changed == 1)
    }

    pub(crate) fn update_global_volume_provider_owned(
        &self,
        owner: &SourceOperationOwner,
        provider: &str,
    ) -> Result<bool, DbError> {
        let conn = self.conn()?;
        let changed = conn.execute(
            r#"
            UPDATE global_volumes
            SET provider = ?4, updated_at = ?5
            WHERE id = ?1 AND operation_revision = ?2
              AND operation_owner_id = ?3 AND operation_state = 'running' AND enabled = 1
            "#,
            params![
                owner.volume_id,
                owner.revision,
                owner.operation_id,
                provider,
                unix_now(),
            ],
        )?;
        Ok(changed == 1)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_global_volume_state(
        &self,
        id: &str,
        status: &str,
        error: Option<&str>,
        journal_id: Option<&str>,
        journal_cursor: Option<&str>,
        full_index_at: Option<i64>,
        incremental_sync_at: Option<i64>,
    ) -> Result<(), DbError> {
        let conn = self.conn()?;
        conn.execute(
            r#"
            UPDATE global_volumes
            SET index_status = ?2,
                last_error = ?3,
                journal_id = COALESCE(?4, journal_id),
                journal_cursor = COALESCE(?5, journal_cursor),
                last_full_index_at = COALESCE(?6, last_full_index_at),
                last_incremental_sync_at = COALESCE(?7, last_incremental_sync_at),
                updated_at = ?8
            WHERE id = ?1
            "#,
            params![
                id,
                status,
                error,
                journal_id,
                journal_cursor,
                full_index_at,
                incremental_sync_at,
                unix_now()
            ],
        )?;
        Ok(())
    }

    pub(crate) fn update_global_volume_state_if_unowned(
        &self,
        id: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<bool, DbError> {
        let conn = self.conn()?;
        let changed = conn.execute(
            r#"
            UPDATE global_volumes
            SET index_status = ?2, last_error = ?3, updated_at = ?4
            WHERE id = ?1 AND operation_owner_id IS NULL
            "#,
            params![id, status, error, unix_now()],
        )?;
        Ok(changed == 1)
    }

    pub fn set_global_volume_enabled(&self, id: &str, enabled: bool) -> Result<(), DbError> {
        let conn = self.conn()?;
        conn.execute(
            r#"
            UPDATE global_volumes
            SET enabled = ?2,
                operation_revision = operation_revision + CASE WHEN enabled != ?2 THEN 1 ELSE 0 END,
                operation_owner_id = CASE WHEN enabled != ?2 THEN NULL ELSE operation_owner_id END,
                operation_owner_pid = CASE WHEN enabled != ?2 THEN NULL ELSE operation_owner_pid END,
                operation_owner_started_at = CASE WHEN enabled != ?2 THEN NULL ELSE operation_owner_started_at END,
                operation_owner_route = CASE WHEN enabled != ?2 THEN NULL ELSE operation_owner_route END,
                operation_kind = CASE WHEN enabled != ?2 THEN NULL ELSE operation_kind END,
                operation_state = CASE WHEN enabled != ?2 THEN NULL ELSE operation_state END,
                updated_at = ?3
            WHERE id = ?1
            "#,
            params![id, bool_to_i64(enabled), unix_now()],
        )?;
        Ok(())
    }

    pub fn update_global_volume_provider(&self, id: &str, provider: &str) -> Result<(), DbError> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE global_volumes SET provider = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, provider, unix_now()],
        )?;
        Ok(())
    }

    pub fn upsert_global_entries_batch(
        &self,
        entries: &[GlobalEntryInput],
    ) -> Result<usize, DbError> {
        self.upsert_global_entries_batch_inner(entries, None)
    }

    pub(crate) fn upsert_global_entries_batch_owned(
        &self,
        entries: &[GlobalEntryInput],
        owner: &SourceOperationOwner,
    ) -> Result<usize, DbError> {
        self.upsert_global_entries_batch_inner(entries, Some(owner))
    }

    fn upsert_global_entries_batch_inner(
        &self,
        entries: &[GlobalEntryInput],
        owner: Option<&SourceOperationOwner>,
    ) -> Result<usize, DbError> {
        if entries.is_empty() {
            return Ok(0);
        }
        let mut conn = self.conn()?;
        // Reserve the WAL writer before reading owner/scope policy state. A
        // DEFERRED read transaction can hold an obsolete snapshot and fail
        // with SQLITE_BUSY_SNAPSHOT when this batch later upgrades to write.
        // This is scoped to the batch's existing atomic unit, with no
        // filesystem/process/network work under the writer reservation.
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(owner) = owner {
            if entries
                .iter()
                .any(|entry| entry.volume_id != owner.volume_id)
            {
                return Err(DbError::Validation(
                    "global_index_operation_foreign_volume_write".into(),
                ));
            }
            ensure_source_operation_owner_tx(&transaction, owner)?;
        }
        let scope_policies = load_enabled_scope_policies(&transaction)?;
        let mut count = 0;
        let mut has_eligible_pending_work = false;
        for entry in entries {
            let entry_id = entry.entry_id();
            transaction.execute(
                r#"
                INSERT INTO global_entries (
                    id, volume_id, platform_file_id, parent_platform_file_id,
                    name, name_normalized, path, path_normalized, extension,
                    is_directory, size, created_at_fs, modified_at_fs,
                    file_attributes, is_hidden, is_system, is_stale,
                    source_provider, last_seen_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, 0, ?17, ?18)
                ON CONFLICT(id) DO UPDATE SET
                    volume_id = excluded.volume_id,
                    platform_file_id = excluded.platform_file_id,
                    parent_platform_file_id = excluded.parent_platform_file_id,
                    name = excluded.name,
                    name_normalized = excluded.name_normalized,
                    path = excluded.path,
                    path_normalized = excluded.path_normalized,
                    extension = excluded.extension,
                    is_directory = excluded.is_directory,
                    size = excluded.size,
                    created_at_fs = excluded.created_at_fs,
                    modified_at_fs = excluded.modified_at_fs,
                    file_attributes = excluded.file_attributes,
                    is_hidden = excluded.is_hidden,
                    is_system = excluded.is_system,
                    is_stale = 0,
                    source_provider = excluded.source_provider,
                    last_seen_at = excluded.last_seen_at
                "#,
                params![
                    entry_id,
                    entry.volume_id,
                    entry.platform_file_id,
                    entry.parent_platform_file_id,
                    entry.name,
                    entry.name.to_lowercase(),
                    entry.path,
                    normalize_path(&entry.path),
                    entry.extension,
                    bool_to_i64(entry.is_directory),
                    entry.size,
                    entry.created_at_fs,
                    entry.modified_at_fs,
                    entry.file_attributes,
                    bool_to_i64(entry.is_hidden),
                    bool_to_i64(entry.is_system),
                    entry.source_provider,
                    entry.last_seen_at,
                ],
            )?;
            has_eligible_pending_work |= enqueue_ai_jobs_for_entry_with_scopes(
                &transaction,
                &entry_id,
                entry,
                &scope_policies,
            )?;
            count += 1;
        }
        transaction.commit()?;
        if has_eligible_pending_work {
            self.notify_managed_ai_work();
        }
        Ok(count)
    }

    pub fn mark_global_entries_stale_for_volume(&self, volume_id: &str) -> Result<usize, DbError> {
        self.mark_global_entries_stale_for_volume_inner(volume_id, None)
    }

    pub(crate) fn mark_global_entries_stale_for_volume_owned(
        &self,
        owner: &SourceOperationOwner,
    ) -> Result<usize, DbError> {
        self.mark_global_entries_stale_for_volume_inner(&owner.volume_id, Some(owner))
    }

    fn mark_global_entries_stale_for_volume_inner(
        &self,
        volume_id: &str,
        owner: Option<&SourceOperationOwner>,
    ) -> Result<usize, DbError> {
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(owner) = owner {
            ensure_source_operation_owner_tx(&transaction, owner)?;
        }
        let now = unix_now();
        let changed = transaction.execute(
            "UPDATE global_entries SET is_stale = 1 WHERE volume_id = ?1 AND is_stale = 0",
            params![volume_id],
        )?;
        transaction.execute(
            "UPDATE managed_entries SET enabled = 0, updated_at = ?2 WHERE global_entry_id IN (SELECT id FROM global_entries WHERE volume_id = ?1)",
            params![volume_id, now],
        )?;
        transaction.execute(
            "UPDATE ai_jobs SET status = 'stale', completed_at = ?2, last_error = 'global_entry_stale' WHERE global_entry_id IN (SELECT id FROM global_entries WHERE volume_id = ?1) AND status IN ('pending', 'running')",
            params![volume_id, now],
        )?;
        transaction.execute(
            "UPDATE ai_job_items SET status = 'stale', updated_at = ?2, last_error = 'global_entry_stale' WHERE global_entry_id IN (SELECT id FROM global_entries WHERE volume_id = ?1) AND status IN ('pending', 'running')",
            params![volume_id, now],
        )?;
        transaction.execute(
            "UPDATE ai_analysis_state SET status = 'stale', last_error = 'global_entry_stale', updated_at = ?2 WHERE global_entry_id IN (SELECT id FROM global_entries WHERE volume_id = ?1)",
            params![volume_id, now],
        )?;
        transaction.execute(
            "UPDATE global_volumes SET entry_count = 0, updated_at = ?2 WHERE id = ?1",
            params![volume_id, now],
        )?;
        transaction.commit()?;
        Ok(changed)
    }

    pub fn mark_global_entry_stale(&self, entry_id: &str) -> Result<bool, DbError> {
        self.mark_global_entry_stale_inner(entry_id, None)
    }

    pub(crate) fn mark_global_entry_stale_owned(
        &self,
        entry_id: &str,
        owner: &SourceOperationOwner,
    ) -> Result<bool, DbError> {
        self.mark_global_entry_stale_inner(entry_id, Some(owner))
    }

    fn mark_global_entry_stale_inner(
        &self,
        entry_id: &str,
        owner: Option<&SourceOperationOwner>,
    ) -> Result<bool, DbError> {
        let mut conn = self.conn()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(owner) = owner {
            ensure_source_operation_owner_tx(&transaction, owner)?;
        }
        let now = unix_now();
        let changed = if let Some(owner) = owner {
            transaction.execute(
                "UPDATE global_entries SET is_stale = 1 WHERE id = ?1 AND volume_id = ?2 AND is_stale = 0",
                params![entry_id, owner.volume_id],
            )?
        } else {
            transaction.execute(
                "UPDATE global_entries SET is_stale = 1 WHERE id = ?1 AND is_stale = 0",
                params![entry_id],
            )?
        };
        let volume_guard = owner.map(|owner| owner.volume_id.as_str());
        transaction.execute(
            "UPDATE managed_entries SET enabled = 0, updated_at = ?2 WHERE global_entry_id = ?1 AND (?3 IS NULL OR global_entry_id IN (SELECT id FROM global_entries WHERE volume_id = ?3))",
            params![entry_id, now, volume_guard],
        )?;
        transaction.execute(
            "UPDATE ai_jobs SET status = 'stale', completed_at = ?2, last_error = 'global_entry_stale' WHERE global_entry_id = ?1 AND (?3 IS NULL OR global_entry_id IN (SELECT id FROM global_entries WHERE volume_id = ?3)) AND status IN ('pending', 'running')",
            params![entry_id, now, volume_guard],
        )?;
        transaction.execute(
            "UPDATE ai_job_items SET status = 'stale', updated_at = ?2, last_error = 'global_entry_stale' WHERE global_entry_id = ?1 AND (?3 IS NULL OR global_entry_id IN (SELECT id FROM global_entries WHERE volume_id = ?3)) AND status IN ('pending', 'running')",
            params![entry_id, now, volume_guard],
        )?;
        transaction.execute(
            "UPDATE ai_analysis_state SET status = 'stale', last_error = 'global_entry_stale', updated_at = ?2 WHERE global_entry_id = ?1 AND (?3 IS NULL OR global_entry_id IN (SELECT id FROM global_entries WHERE volume_id = ?3))",
            params![entry_id, now, volume_guard],
        )?;
        transaction.commit()?;
        Ok(changed > 0)
    }

    pub fn get_global_entry(&self, entry_id: &str) -> Result<Option<GlobalEntry>, DbError> {
        let conn = self.conn()?;
        conn.query_row(
            r#"
            SELECT id, volume_id, platform_file_id, parent_platform_file_id,
                   name, name_normalized, path, path_normalized, extension,
                   is_directory, size, created_at_fs, modified_at_fs,
                   file_attributes, is_hidden, is_system, is_stale,
                   source_provider, last_seen_at
            FROM global_entries WHERE id = ?1
            "#,
            params![entry_id],
            map_global_entry,
        )
        .optional()
        .map_err(DbError::from)
    }

    pub fn global_index_status(&self) -> Result<GlobalIndexStatus, DbError> {
        let conn = self.conn()?;
        global_index_status_from_connection(&conn)
    }

    pub fn search_global_entries(
        &self,
        query: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<GlobalSearchResult>, DbError> {
        super::search::search_global_entries(self, query, limit, offset)
    }

    pub(crate) fn search_global_entries_snapshot(
        &self,
        query: &str,
        limit: u32,
        offset: u32,
    ) -> Result<GlobalSearchSnapshot, DbError> {
        let mut conn = self.conn()?;
        let transaction = conn.transaction()?;
        let results =
            super::search::search_global_entries_on_connection(&transaction, query, limit, offset)?;
        let (source_health, source_revision) = load_snapshot_source_health(&transaction)?;
        let index_status = global_index_status_from_connection(&transaction)?;
        transaction.commit()?;
        Ok(GlobalSearchSnapshot {
            results,
            source_health,
            source_revision,
            index_status,
        })
    }

    #[cfg(test)]
    pub(crate) fn search_global_entries_snapshot_with_source_health_candidate(
        &self,
        query: &str,
        limit: u32,
        offset: u32,
        candidate: GlobalSearchSourceHealthQueryCandidate,
    ) -> Result<GlobalSearchSnapshot, DbError> {
        with_source_health_candidate_override(candidate, || {
            self.search_global_entries_snapshot(query, limit, offset)
        })
    }

    pub fn global_entry_for_path(&self, path: &Path) -> Result<Option<GlobalEntry>, DbError> {
        let normalized = normalize_path(&path.to_string_lossy());
        let conn = self.conn()?;
        conn.query_row(
            r#"
            SELECT id, volume_id, platform_file_id, parent_platform_file_id,
                   name, name_normalized, path, path_normalized, extension,
                   is_directory, size, created_at_fs, modified_at_fs,
                   file_attributes, is_hidden, is_system, is_stale,
                   source_provider, last_seen_at
            FROM global_entries
            WHERE path_normalized = ?1 AND is_stale = 0
            LIMIT 1
            "#,
            params![normalized],
            map_global_entry,
        )
        .optional()
        .map_err(DbError::from)
    }

    pub fn global_path_by_platform_identity(
        &self,
        volume_id: &str,
        platform_file_id: &str,
    ) -> Result<Option<String>, DbError> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT path FROM global_entries WHERE volume_id = ?1 AND platform_file_id = ?2 AND is_stale = 0 ORDER BY length(path) LIMIT 1",
            params![volume_id, platform_file_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(DbError::from)
    }

    pub fn global_entry_by_identity(
        &self,
        volume_id: &str,
        platform_file_id: &str,
        parent_platform_file_id: &str,
        name: &str,
    ) -> Result<Option<GlobalEntry>, DbError> {
        let conn = self.conn()?;
        conn.query_row(
            r#"
            SELECT id, volume_id, platform_file_id, parent_platform_file_id,
                   name, name_normalized, path, path_normalized, extension,
                   is_directory, size, created_at_fs, modified_at_fs,
                   file_attributes, is_hidden, is_system, is_stale,
                   source_provider, last_seen_at
            FROM global_entries
            WHERE volume_id = ?1 AND platform_file_id = ?2
              AND parent_platform_file_id = ?3 AND name = ?4
            LIMIT 1
            "#,
            params![volume_id, platform_file_id, parent_platform_file_id, name],
            map_global_entry,
        )
        .optional()
        .map_err(DbError::from)
    }
}

#[cfg(test)]
fn with_source_health_candidate_override<T>(
    candidate: GlobalSearchSourceHealthQueryCandidate,
    operation: impl FnOnce() -> T,
) -> T {
    struct ResetCandidate(Option<GlobalSearchSourceHealthQueryCandidate>);

    impl Drop for ResetCandidate {
        fn drop(&mut self) {
            GLOBAL_SEARCH_SOURCE_HEALTH_CANDIDATE_OVERRIDE
                .with(|override_slot| override_slot.set(self.0));
        }
    }

    GLOBAL_SEARCH_SOURCE_HEALTH_CANDIDATE_OVERRIDE.with(|override_slot| {
        let previous = override_slot.replace(Some(candidate));
        let _reset = ResetCandidate(previous);
        operation()
    })
}

fn global_index_status_from_connection(
    conn: &rusqlite::Connection,
) -> Result<GlobalIndexStatus, DbError> {
    let GlobalIndexStatusCounts {
        total_entries,
        indexed_volumes,
        ready_volumes,
        pending_volumes,
        spotlight_not_indexed_volumes,
        permission_required_volumes,
        spotlight_unavailable_volumes,
        spotlight_external_not_indexed_volumes,
        fsevents_unavailable_volumes,
        unavailable_volumes,
        error_volumes,
        paused_volumes,
        last_sync_at,
        last_error,
    } = conn.query_row(
        r#"
        SELECT
            (SELECT COUNT(*)
             FROM global_entries entry
             JOIN global_volumes volume ON volume.id = entry.volume_id
             WHERE entry.is_stale = 0 AND volume.enabled = 1),
            (SELECT COUNT(*) FROM global_volumes WHERE enabled = 1),
            (SELECT COUNT(*) FROM global_volumes WHERE enabled = 1 AND index_status = 'ready'),
            (SELECT COUNT(*) FROM global_volumes WHERE enabled = 1 AND index_status IN ('discovered', 'indexing', 'syncing', 'rebuild_required')),
            (SELECT COUNT(*) FROM global_volumes WHERE enabled = 1 AND index_status = 'spotlight_not_indexed'),
            (SELECT COUNT(*) FROM global_volumes WHERE enabled = 1 AND index_status = 'permission_required'),
            (SELECT COUNT(*) FROM global_volumes WHERE enabled = 1 AND index_status = 'spotlight_unavailable'),
            (SELECT COUNT(*) FROM global_volumes WHERE enabled = 1 AND index_status = 'spotlight_external_not_indexed'),
            (SELECT COUNT(*) FROM global_volumes WHERE enabled = 1 AND index_status = 'fsevents_unavailable'),
            (SELECT COUNT(*) FROM global_volumes WHERE enabled = 1 AND index_status = 'unavailable'),
            (SELECT COUNT(*) FROM global_volumes WHERE enabled = 1 AND index_status = 'error'),
            (SELECT COUNT(*) FROM global_volumes WHERE enabled = 1 AND index_status = 'paused'),
            (SELECT MAX(last_incremental_sync_at) FROM global_volumes WHERE enabled = 1),
            (SELECT last_error FROM global_volumes WHERE enabled = 1 AND last_error IS NOT NULL ORDER BY updated_at DESC LIMIT 1)
        "#,
        [],
        |row| {
            Ok(GlobalIndexStatusCounts {
                total_entries: row.get(0)?,
                indexed_volumes: row.get(1)?,
                ready_volumes: row.get(2)?,
                pending_volumes: row.get(3)?,
                spotlight_not_indexed_volumes: row.get(4)?,
                permission_required_volumes: row.get(5)?,
                spotlight_unavailable_volumes: row.get(6)?,
                spotlight_external_not_indexed_volumes: row.get(7)?,
                fsevents_unavailable_volumes: row.get(8)?,
                unavailable_volumes: row.get(9)?,
                error_volumes: row.get(10)?,
                paused_volumes: row.get(11)?,
                last_sync_at: row.get(12)?,
                last_error: row.get(13)?,
            })
        },
    )?;
    let status = if indexed_volumes == 0 {
        INDEX_STATUS_UNAVAILABLE
    } else if ready_volumes == indexed_volumes {
        INDEX_STATUS_READY
    } else if ready_volumes > 0 {
        "partial"
    } else if unavailable_volumes > 0 {
        INDEX_STATUS_UNAVAILABLE
    } else if error_volumes > 0 {
        INDEX_STATUS_ERROR
    } else if spotlight_not_indexed_volumes > 0 {
        INDEX_STATUS_SPOTLIGHT_NOT_INDEXED
    } else if permission_required_volumes > 0 {
        INDEX_STATUS_PERMISSION_REQUIRED
    } else if spotlight_unavailable_volumes > 0 {
        INDEX_STATUS_SPOTLIGHT_UNAVAILABLE
    } else if spotlight_external_not_indexed_volumes > 0 {
        INDEX_STATUS_SPOTLIGHT_EXTERNAL_NOT_INDEXED
    } else if fsevents_unavailable_volumes > 0 {
        INDEX_STATUS_FSEVENTS_UNAVAILABLE
    } else if paused_volumes > 0 {
        INDEX_STATUS_PAUSED
    } else {
        INDEX_STATUS_INDEXING
    };
    Ok(GlobalIndexStatus {
        platform: std::env::consts::OS.to_string(),
        enabled: indexed_volumes > 0,
        status: status.to_string(),
        provider_status: None,
        processed_entries: total_entries,
        collection_complete: indexed_volumes > 0
            && pending_volumes == 0
            && unavailable_volumes == 0
            && error_volumes == 0
            && paused_volumes == 0,
        total_entries,
        indexed_volumes,
        ready_volumes,
        pending_volumes,
        last_sync_at,
        last_error,
    })
}

fn load_global_search_sources_from_connection(
    conn: &rusqlite::Connection,
) -> Result<(Vec<GlobalSearchSourceHealth>, String), DbError> {
    let mut statement = conn.prepare(GLOBAL_SEARCH_SOURCE_HEALTH_SQL)?;
    let mut facts = Vec::new();
    let mut source_health = Vec::new();
    let rows = statement.query_map([], |row| {
        let source_id = row.get::<_, String>(0)?;
        let enabled = row.get::<_, i64>(1)? != 0;
        let provider = row.get::<_, String>(2)?;
        let status = row.get::<_, String>(3)?;
        let last_error = row.get::<_, Option<String>>(4)?;
        let updated_at = row.get::<_, i64>(5)?;
        let active_entry_count = row.get::<_, i64>(6)?;
        let max_last_seen_at = row.get::<_, Option<i64>>(7)?;
        Ok((
            GlobalSearchSourceHealth {
                source_id: source_id.clone(),
                enabled,
                provider: provider.clone(),
                status: status.clone(),
                last_error: last_error.clone(),
                updated_at,
            },
            GlobalSearchRevisionFact {
                source_id,
                enabled,
                provider,
                status,
                last_error,
                updated_at,
                active_entry_count,
                max_last_seen_at,
            },
        ))
    })?;
    for row in rows {
        let (health, fact) = row?;
        source_health.push(health);
        facts.push(fact);
    }
    let serialized = serde_json::to_vec(&facts).unwrap_or_default();
    let revision = blake3::hash(&serialized).to_hex().to_string();
    Ok((source_health, revision))
}

fn load_snapshot_source_health(
    conn: &rusqlite::Connection,
) -> Result<(Vec<GlobalSearchSourceHealth>, String), DbError> {
    #[cfg(test)]
    if let Some(candidate) =
        GLOBAL_SEARCH_SOURCE_HEALTH_CANDIDATE_OVERRIDE.with(std::cell::Cell::get)
    {
        let health = load_global_search_source_health_candidate(conn, candidate)?;
        return Ok((health.source_health, health.source_revision));
    }

    load_global_search_sources_from_connection(conn)
}

#[cfg(test)]
pub(crate) fn load_global_search_source_health_candidate(
    conn: &rusqlite::Connection,
    candidate: GlobalSearchSourceHealthQueryCandidate,
) -> Result<DiagnosticSourceHealth, DbError> {
    let mut statement = conn.prepare(candidate.sql())?;
    let mut facts = Vec::new();
    let mut source_health = Vec::new();
    let rows = statement.query_map([], |row| {
        let source_id = row.get::<_, String>(0)?;
        let enabled = row.get::<_, i64>(1)? != 0;
        let provider = row.get::<_, String>(2)?;
        let status = row.get::<_, String>(3)?;
        let last_error = row.get::<_, Option<String>>(4)?;
        let updated_at = row.get::<_, i64>(5)?;
        let active_entry_count = row.get::<_, i64>(6)?;
        let max_last_seen_at = row.get::<_, Option<i64>>(7)?;
        Ok((
            GlobalSearchSourceHealth {
                source_id: source_id.clone(),
                enabled,
                provider: provider.clone(),
                status: status.clone(),
                last_error: last_error.clone(),
                updated_at,
            },
            GlobalSearchRevisionFact {
                source_id,
                enabled,
                provider,
                status,
                last_error,
                updated_at,
                active_entry_count,
                max_last_seen_at,
            },
        ))
    })?;
    for row in rows {
        let (health, fact) = row?;
        source_health.push(health);
        facts.push(fact);
    }
    let revision_facts_json = serde_json::to_vec(&facts).unwrap_or_default();
    let source_revision = blake3::hash(&revision_facts_json).to_hex().to_string();
    Ok(DiagnosticSourceHealth {
        source_health,
        source_revision,
        revision_facts_json,
    })
}

#[cfg(test)]
pub(crate) fn diagnostic_compare_snapshot_queries_on_connection(
    conn: &rusqlite::Connection,
    query: &str,
    limit: u32,
    offset: u32,
    candidate: GlobalSearchSourceHealthQueryCandidate,
) -> Result<(GlobalSearchSnapshot, GlobalSearchSnapshot), DbError> {
    let results = super::search::search_global_entries_on_connection(conn, query, limit, offset)?;
    let original_source_health = load_global_search_source_health_candidate(
        conn,
        GlobalSearchSourceHealthQueryCandidate::Original,
    )?;
    let candidate_source_health = load_global_search_source_health_candidate(conn, candidate)?;
    let index_status = global_index_status_from_connection(conn)?;
    Ok((
        GlobalSearchSnapshot {
            results: results.clone(),
            source_health: original_source_health.source_health,
            source_revision: original_source_health.source_revision,
            index_status: index_status.clone(),
        },
        GlobalSearchSnapshot {
            results,
            source_health: candidate_source_health.source_health,
            source_revision: candidate_source_health.source_revision,
            index_status,
        },
    ))
}

#[derive(Debug, Clone)]
pub(crate) struct ManagedScopePolicy {
    id: String,
    allow_local_ai: bool,
    allow_cloud_ai: bool,
    path: String,
}

pub(crate) fn load_enabled_scope_policies(
    transaction: &Transaction<'_>,
) -> Result<Vec<ManagedScopePolicy>, DbError> {
    let mut statement = transaction.prepare(
        r#"
        SELECT id, allow_local_ai, allow_cloud_ai, path
        FROM managed_scopes
        WHERE enabled = 1
        ORDER BY length(path) DESC, id ASC
        "#,
    )?;
    let rows = statement.query_map([], |row| {
        Ok(ManagedScopePolicy {
            id: row.get(0)?,
            allow_local_ai: row.get::<_, i64>(1)? != 0,
            allow_cloud_ai: row.get::<_, i64>(2)? != 0,
            path: normalize_path(&row.get::<_, String>(3)?),
        })
    })?;
    let policies = rows.collect::<Result<Vec<_>, _>>().map_err(DbError::from)?;
    Ok(policies)
}

pub(crate) fn enqueue_ai_jobs_for_entry(
    transaction: &Transaction<'_>,
    entry_id: &str,
    entry: &GlobalEntryInput,
) -> Result<bool, DbError> {
    let scopes = load_enabled_scope_policies(transaction)?;
    enqueue_ai_jobs_for_entry_with_scopes(transaction, entry_id, entry, &scopes)
}

pub(crate) fn enqueue_managed_ai_for_library_files(
    transaction: &Transaction<'_>,
    file_ids: &[String],
) -> Result<(usize, bool), DbError> {
    let mut queued = 0usize;
    let mut has_eligible_pending_work = false;
    for file_id in file_ids {
        let path = transaction
            .query_row(
                "SELECT path FROM files WHERE id = ?1 AND is_stale = 0",
                params![file_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(path) = path else {
            continue;
        };
        let normalized = normalize_path(&path);
        let entry = transaction
            .query_row(
                "SELECT id, volume_id, platform_file_id, parent_platform_file_id, name,
                        path, extension, is_directory, size, created_at_fs, modified_at_fs,
                        file_attributes, is_hidden, is_system, source_provider, last_seen_at
                 FROM global_entries WHERE path_normalized = ?1 AND is_stale = 0
                 ORDER BY last_seen_at DESC, id LIMIT 1",
                params![normalized],
                |row| {
                    let id = row.get::<_, String>(0)?;
                    let input = GlobalEntryInput {
                        volume_id: row.get(1)?,
                        platform_file_id: row.get(2)?,
                        parent_platform_file_id: row.get(3)?,
                        name: row.get(4)?,
                        path: row.get(5)?,
                        extension: row.get(6)?,
                        is_directory: row.get::<_, i64>(7)? != 0,
                        size: row.get(8)?,
                        created_at_fs: row.get(9)?,
                        modified_at_fs: row.get(10)?,
                        file_attributes: row.get(11)?,
                        is_hidden: row.get::<_, i64>(12)? != 0,
                        is_system: row.get::<_, i64>(13)? != 0,
                        source_provider: row.get(14)?,
                        last_seen_at: row.get(15)?,
                    };
                    Ok((id, input))
                },
            )
            .optional()?;
        let Some((entry_id, input)) = entry else {
            continue;
        };
        has_eligible_pending_work |= enqueue_ai_jobs_for_entry(transaction, &entry_id, &input)?;
        queued += 1;
    }
    Ok((queued, has_eligible_pending_work))
}

/// Returns true only when this entry creates or reactivates scope-eligible pending work.
pub(crate) fn enqueue_ai_jobs_for_entry_with_scopes(
    transaction: &Transaction<'_>,
    entry_id: &str,
    entry: &GlobalEntryInput,
    scopes: &[ManagedScopePolicy],
) -> Result<bool, DbError> {
    let fingerprint = metadata_fingerprint(entry);
    let path_normalized = normalize_path(&entry.path);
    let Some(scope) = scopes
        .iter()
        .find(|scope| path_is_within(&path_normalized, &scope.path))
    else {
        return Ok(false);
    };
    let scope_id = &scope.id;
    let allow_local_ai = scope.allow_local_ai;
    let allow_cloud_ai = scope.allow_cloud_ai;
    let managed_entry_id = format!(
        "me_{}",
        blake3::hash(format!("{scope_id}\0{entry_id}").as_bytes()).to_hex()
    );
    transaction.execute(
            r#"
            INSERT INTO managed_entries (id, global_entry_id, managed_scope_id, enabled, created_at, updated_at)
            VALUES (?1, ?2, ?3, 1, ?4, ?4)
            ON CONFLICT(global_entry_id, managed_scope_id) DO UPDATE SET enabled = 1, updated_at = excluded.updated_at
            "#,
            params![managed_entry_id, entry_id, scope_id, unix_now()],
        )?;

    if entry.is_directory {
        let now = unix_now();
        transaction.execute(
                "UPDATE ai_jobs SET status = 'stale', completed_at = ?2, last_error = 'global_entry_is_directory' WHERE global_entry_id = ?1 AND status IN ('pending', 'running', 'completed')",
                params![entry_id, now],
            )?;
        transaction.execute(
                "UPDATE ai_job_items SET status = 'stale', updated_at = ?2, last_error = 'global_entry_is_directory' WHERE global_entry_id = ?1 AND status IN ('pending', 'running', 'completed')",
                params![entry_id, now],
            )?;
        transaction.execute(
                "UPDATE ai_analysis_state SET status = 'stale', last_error = 'global_entry_is_directory', updated_at = ?2 WHERE global_entry_id = ?1",
                params![entry_id, now],
            )?;
        return Ok(false);
    }
    let now = unix_now();
    transaction.execute(
            "UPDATE ai_jobs SET status = 'stale', completed_at = ?2, last_error = 'input_fingerprint_changed' WHERE global_entry_id = ?1 AND input_fingerprint <> ?3 AND status IN ('pending', 'running', 'completed')",
            params![entry_id, now, fingerprint],
        )?;
    transaction.execute(
            "UPDATE ai_job_items SET status = 'stale', updated_at = ?2, last_error = 'input_fingerprint_changed' WHERE global_entry_id = ?1 AND job_id IN (SELECT id FROM ai_jobs WHERE input_fingerprint <> ?3)",
            params![entry_id, now, fingerprint],
        )?;
    let existing: Option<(String, String)> = transaction
            .query_row(
                "SELECT id, status FROM ai_jobs WHERE global_entry_id = ?1 AND managed_scope_id = ?2 AND input_fingerprint = ?3 ORDER BY created_at DESC LIMIT 1",
                params![entry_id, scope_id, fingerprint],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
    if let Some((job_id, status)) = existing {
        if status == AI_JOB_STALE {
            let (provider, next_status) = if allow_local_ai {
                ("local", AI_JOB_PENDING)
            } else if allow_cloud_ai {
                ("cloud", AI_JOB_PENDING)
            } else {
                ("none", AI_JOB_BLOCKED_BY_POLICY)
            };
            let reactivated_eligible_work = next_status == AI_JOB_PENDING;
            transaction.execute(
                    "UPDATE ai_jobs SET provider = ?2, status = ?3, attempt_count = 0, last_error = NULL, started_at = NULL, completed_at = NULL WHERE id = ?1",
                    params![job_id, provider, next_status],
                )?;
            transaction.execute(
                    "UPDATE ai_job_items SET status = ?2, last_error = NULL, updated_at = ?3 WHERE job_id = ?1",
                    params![job_id, next_status, unix_now()],
                )?;
            transaction.execute(
                    "UPDATE ai_analysis_state SET status = ?2, provider = ?3, last_error = NULL, updated_at = ?4 WHERE global_entry_id = ?1",
                    params![entry_id, next_status, provider, unix_now()],
                )?;
            return Ok(reactivated_eligible_work);
        }
        return Ok(false);
    }
    let (provider, status) = if allow_local_ai {
        ("local", AI_JOB_PENDING)
    } else if allow_cloud_ai {
        ("cloud", AI_JOB_PENDING)
    } else {
        ("none", AI_JOB_BLOCKED_BY_POLICY)
    };
    let job_id = format!(
        "aij_{}",
        blake3::hash(format!("{entry_id}\0{scope_id}\0{fingerprint}").as_bytes()).to_hex()
    );
    transaction.execute(
        r#"
            INSERT INTO ai_jobs (
                id, global_entry_id, managed_scope_id, input_fingerprint, provider,
                model, processing_mode, status, attempt_count, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, '', 'metadata', ?6, 0, ?7)
            "#,
        params![
            job_id,
            entry_id,
            scope_id,
            fingerprint,
            provider,
            status,
            unix_now()
        ],
    )?;
    transaction.execute(
        r#"
            INSERT INTO ai_job_items (id, job_id, global_entry_id, status, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?5)
            "#,
        params![
            format!("aiji_{job_id}"),
            job_id,
            entry_id,
            status,
            unix_now()
        ],
    )?;
    transaction.execute(
        r#"
        INSERT INTO ai_analysis_state (global_entry_id, status, input_fingerprint, provider, updated_at)
        VALUES (?1, ?2, ?3, ?4, ?5)
        ON CONFLICT(global_entry_id) DO UPDATE SET
            status = excluded.status,
            input_fingerprint = excluded.input_fingerprint,
            provider = excluded.provider,
            updated_at = excluded.updated_at
        "#,
        params![entry_id, status, fingerprint, provider, unix_now()],
    )?;
    Ok(status == AI_JOB_PENDING)
}

fn metadata_fingerprint(entry: &GlobalEntryInput) -> String {
    format!(
        "mf_{}",
        blake3::hash(
            format!(
                "{}\0{}\0{}\0{}\0{}\0{}",
                entry.volume_id,
                entry.platform_file_id,
                entry.name,
                entry.size,
                entry.modified_at_fs.unwrap_or_default(),
                entry.is_directory
            )
            .as_bytes(),
        )
        .to_hex()
    )
}

fn path_is_within(path: &str, scope: &str) -> bool {
    let path = path.trim_end_matches('/');
    let scope = scope.trim_end_matches('/');
    path == scope || path.starts_with(&format!("{scope}/"))
}

pub(crate) fn global_entry_input_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<GlobalEntryInput> {
    Ok(GlobalEntryInput {
        volume_id: row.get(0)?,
        platform_file_id: row.get(1)?,
        parent_platform_file_id: row.get(2)?,
        name: row.get(3)?,
        path: row.get(4)?,
        extension: row.get(5)?,
        is_directory: row.get::<_, i64>(6)? != 0,
        size: row.get(7)?,
        created_at_fs: row.get(8)?,
        modified_at_fs: row.get(9)?,
        file_attributes: row.get(10)?,
        is_hidden: row.get::<_, i64>(11)? != 0,
        is_system: row.get::<_, i64>(12)? != 0,
        source_provider: row.get(13)?,
        last_seen_at: row.get(14)?,
    })
}

fn map_global_volume(row: &rusqlite::Row<'_>) -> rusqlite::Result<GlobalVolume> {
    Ok(GlobalVolume {
        id: row.get(0)?,
        platform: row.get(1)?,
        stable_volume_id: row.get(2)?,
        display_name: row.get(3)?,
        mount_path: row.get(4)?,
        filesystem_type: row.get(5)?,
        drive_kind: row.get(6)?,
        enabled: row.get::<_, i64>(7)? != 0,
        provider: row.get(8)?,
        index_status: row.get(9)?,
        last_error: row.get(10)?,
        journal_id: row.get(11)?,
        journal_cursor: row.get(12)?,
        last_full_index_at: row.get(13)?,
        last_incremental_sync_at: row.get(14)?,
        entry_count: row.get(15)?,
        created_at: row.get(16)?,
        updated_at: row.get(17)?,
    })
}

pub(super) fn ensure_source_operation_owner_tx(
    transaction: &Transaction<'_>,
    owner: &SourceOperationOwner,
) -> Result<(), DbError> {
    let owns_source = transaction.query_row(
        r#"
        SELECT EXISTS(
            SELECT 1 FROM global_volumes
            WHERE id = ?1 AND operation_revision = ?2
              AND operation_owner_id = ?3 AND operation_state = 'running'
              AND enabled = 1
        )
        "#,
        params![owner.volume_id, owner.revision, owner.operation_id],
        |row| row.get::<_, bool>(0),
    )?;
    if owns_source {
        Ok(())
    } else {
        Err(DbError::Validation(
            "global_index_operation_owner_lost".into(),
        ))
    }
}

fn current_process_identity() -> Result<(i64, i64), DbError> {
    let process_id = std::process::id();
    let started_at = process_start_time(process_id)
        .ok_or_else(|| DbError::Validation("global_index_process_identity_unavailable".into()))?;
    Ok((i64::from(process_id), started_at))
}

fn query_global_index_operation_claim_snapshot(
    conn: &Connection,
    volume: &GlobalVolume,
) -> Result<Option<GlobalIndexOperationClaimSnapshot>, DbError> {
    conn.query_row(
        r#"
        SELECT operation_revision, operation_owner_id, operation_owner_pid,
               operation_owner_started_at, operation_owner_route, operation_kind,
               operation_state
        FROM global_volumes
        WHERE id = ?1 AND stable_volume_id = ?2 AND provider = ?3 AND enabled = 1
        "#,
        params![volume.id, volume.stable_volume_id, volume.provider],
        |row| {
            Ok(GlobalIndexOperationClaimSnapshot {
                revision: row.get(0)?,
                owner_id: row.get(1)?,
                owner_pid: row.get(2)?,
                owner_started_at: row.get(3)?,
                owner_route: row.get(4)?,
                operation_kind: row.get(5)?,
                operation_state: row.get(6)?,
            })
        },
    )
    .optional()
    .map_err(DbError::from)
}

fn global_index_operation_snapshot_allows_claim(
    snapshot: &GlobalIndexOperationClaimSnapshot,
    current_pid: i64,
    current_started_at: i64,
    existing_owner_locally_active: bool,
    route: SourceOperationRoute,
    process_is_alive: &mut impl FnMut(i64, i64) -> Option<bool>,
) -> bool {
    if snapshot.owner_id.is_none() {
        // An ownerless row should not retain fragments of a previous owner.
        // Treat inconsistent durable state as ambiguous instead of reclaiming it.
        return snapshot.owner_pid.is_none()
            && snapshot.owner_started_at.is_none()
            && snapshot.owner_route.is_none()
            && snapshot.operation_kind.is_none()
            && snapshot.operation_state.is_none();
    }

    if existing_owner_locally_active {
        return false;
    }

    let (
        Some(owner_pid),
        Some(owner_started_at),
        Some(owner_route),
        Some(operation_kind),
        Some(state),
    ) = (
        snapshot.owner_pid,
        snapshot.owner_started_at,
        snapshot.owner_route.as_deref(),
        snapshot.operation_kind.as_deref(),
        snapshot.operation_state.as_deref(),
    )
    else {
        return false;
    };
    if !matches!(owner_route, "coordinator" | "service" | "direct")
        || !matches!(operation_kind, "initial" | "incremental" | "rebuild")
        || !matches!(state, "running" | "unknown")
    {
        return false;
    }

    let unresolved_service_owner = owner_route == "service";
    // A Direct request cannot prove that a previously admitted Service
    // operation stopped. Only a new validated Service admission may reconcile
    // the durable Service owner.
    if unresolved_service_owner && route != SourceOperationRoute::Service {
        return false;
    }

    let same_process = owner_pid == current_pid && owner_started_at == current_started_at;
    let owner_alive = if same_process {
        false
    } else {
        // `None` means the OS could not verify the process identity. Fail closed;
        // only an explicit false result proves a foreign Direct owner is gone.
        let Some(owner_alive) = process_is_alive(owner_pid, owner_started_at) else {
            return false;
        };
        owner_alive
    };

    let admitted_service_recovery =
        route == SourceOperationRoute::Service && unresolved_service_owner;
    if admitted_service_recovery {
        // The caller reached this point only after the Service's real admission
        // reservation. Preserve its existing ability to reconcile a previous
        // Service operation, while still requiring a complete, verifiable owner
        // identity and no matching process-local active operation above.
        true
    } else {
        !owner_alive
    }
}

fn process_instance_is_alive(process_id: i64, started_at: i64) -> Option<bool> {
    let process_id = u32::try_from(process_id).ok()?;
    let pid = sysinfo::Pid::from_u32(process_id);
    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    Some(
        system
            .process(pid)
            .is_some_and(|process| process.start_time() as i64 == started_at),
    )
}

fn process_start_time(process_id: u32) -> Option<i64> {
    let pid = sysinfo::Pid::from_u32(process_id);
    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
    system
        .process(pid)
        .map(|process| process.start_time() as i64)
}

fn unregister_local_global_index_operation(operation_id: &str) {
    if let Some(active) = LOCAL_GLOBAL_INDEX_OPERATIONS.get() {
        active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(operation_id);
    }
}

fn register_local_global_index_operation(operation_id: &str) -> bool {
    LOCAL_GLOBAL_INDEX_OPERATIONS
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(operation_id.to_string())
}

fn map_global_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<GlobalEntry> {
    Ok(GlobalEntry {
        id: row.get(0)?,
        volume_id: row.get(1)?,
        platform_file_id: row.get(2)?,
        parent_platform_file_id: row.get(3)?,
        name: row.get(4)?,
        name_normalized: row.get(5)?,
        path: row.get(6)?,
        path_normalized: row.get(7)?,
        extension: row.get(8)?,
        is_directory: row.get::<_, i64>(9)? != 0,
        size: row.get(10)?,
        created_at_fs: row.get(11)?,
        modified_at_fs: row.get(12)?,
        file_attributes: row.get(13)?,
        is_hidden: row.get::<_, i64>(14)? != 0,
        is_system: row.get::<_, i64>(15)? != 0,
        is_stale: row.get::<_, i64>(16)? != 0,
        source_provider: row.get(17)?,
        last_seen_at: row.get(18)?,
    })
}

fn bool_to_i64(value: bool) -> i64 {
    i64::from(value)
}
