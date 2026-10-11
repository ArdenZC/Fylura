//! #380-D0 executable, test-only contract.
//!
//! This file intentionally owns its SQLite schema and transition code. It does
//! not call or change the production Global Index coordinator, service,
//! repository, Schema 37, or IPC v3 implementation.

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const IPC_V3: u16 = 3;
const MANAGED_SCOPE: &str = "scope-managed";
const FAILPOINTS: [PublishStep; 6] = [
    PublishStep::AfterMissingEntries,
    PublishStep::AfterEntryMerge,
    PublishStep::AfterManagedReconcile,
    PublishStep::AfterAiReconcile,
    PublishStep::AfterSourceStateUpdate,
    PublishStep::BeforeOperationComplete,
];

#[derive(Clone, Debug, PartialEq, Eq)]
struct SourceIdentity {
    id: String,
    stable_volume_id: String,
    mount_path: String,
    provider: String,
    enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    id: String,
    volume_id: String,
    name: String,
    path: String,
    extension: String,
    fingerprint: String,
    stale: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ManagedLink {
    entry_id: String,
    enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AnalysisState {
    entry_id: String,
    status: String,
    fingerprint: String,
    classification_json: Option<String>,
    user_corrected: bool,
    last_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AiJob {
    id: String,
    entry_id: String,
    status: String,
    fingerprint: String,
    last_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AiJobItem {
    id: String,
    job_id: String,
    entry_id: String,
    status: String,
    last_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SourceHealth {
    index_status: String,
    last_error: Option<String>,
    journal_id: Option<String>,
    journal_cursor: Option<String>,
    last_full_index_at: Option<i64>,
    last_incremental_sync_at: Option<i64>,
    entry_count: i64,
    updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Projection {
    entries: BTreeMap<String, Entry>,
    managed: BTreeMap<String, ManagedLink>,
    analysis: BTreeMap<String, AnalysisState>,
    jobs: BTreeMap<String, AiJob>,
    items: BTreeMap<String, AiJobItem>,
    health: SourceHealth,
    old_needle_hits: BTreeSet<String>,
    new_needle_hits: BTreeSet<String>,
}

impl Projection {
    fn baseline() -> Self {
        let entries = [
            Entry {
                id: "entry-alpha".into(),
                volume_id: "source-a".into(),
                name: "oldneedle-alpha.txt".into(),
                path: "C:/managed/oldneedle-alpha.txt".into(),
                extension: "txt".into(),
                fingerprint: "fp-alpha-v1".into(),
                stale: false,
            },
            Entry {
                id: "entry-beta".into(),
                volume_id: "source-a".into(),
                name: "gone-beta.txt".into(),
                path: "C:/managed/gone-beta.txt".into(),
                extension: "txt".into(),
                fingerprint: "fp-beta-v1".into(),
                stale: false,
            },
            Entry {
                id: "entry-gamma".into(),
                volume_id: "source-a".into(),
                name: "stable-gamma.txt".into(),
                path: "C:/outside/stable-gamma.txt".into(),
                extension: "txt".into(),
                fingerprint: "fp-gamma-v1".into(),
                stale: false,
            },
        ]
        .into_iter()
        .map(|entry| (entry.id.clone(), entry))
        .collect();
        let managed = ["entry-alpha", "entry-beta"]
            .into_iter()
            .map(|entry_id| {
                let link = ManagedLink {
                    entry_id: entry_id.into(),
                    enabled: true,
                };
                (format!("managed-{entry_id}"), link)
            })
            .collect();
        let analysis = [
            AnalysisState {
                entry_id: "entry-alpha".into(),
                status: "completed".into(),
                fingerprint: "fp-alpha-v1".into(),
                classification_json: Some(r#"{"class":"owner-corrected"}"#.into()),
                user_corrected: true,
                last_error: None,
            },
            AnalysisState {
                entry_id: "entry-beta".into(),
                status: "pending".into(),
                fingerprint: "fp-beta-v1".into(),
                classification_json: None,
                user_corrected: false,
                last_error: None,
            },
        ]
        .into_iter()
        .map(|state| (state.entry_id.clone(), state))
        .collect();
        let jobs = [
            AiJob {
                id: "job-alpha".into(),
                entry_id: "entry-alpha".into(),
                status: "completed".into(),
                fingerprint: "fp-alpha-v1".into(),
                last_error: None,
            },
            AiJob {
                id: "job-beta".into(),
                entry_id: "entry-beta".into(),
                status: "pending".into(),
                fingerprint: "fp-beta-v1".into(),
                last_error: None,
            },
        ]
        .into_iter()
        .map(|job| (job.id.clone(), job))
        .collect();
        let items = [
            AiJobItem {
                id: "item-alpha".into(),
                job_id: "job-alpha".into(),
                entry_id: "entry-alpha".into(),
                status: "completed".into(),
                last_error: None,
            },
            AiJobItem {
                id: "item-beta".into(),
                job_id: "job-beta".into(),
                entry_id: "entry-beta".into(),
                status: "pending".into(),
                last_error: None,
            },
        ]
        .into_iter()
        .map(|item| (item.id.clone(), item))
        .collect();
        let mut projection = Self {
            entries,
            managed,
            analysis,
            jobs,
            items,
            health: SourceHealth {
                index_status: "ready".into(),
                last_error: None,
                journal_id: Some("journal-old".into()),
                journal_cursor: Some("cursor-old".into()),
                last_full_index_at: Some(700),
                last_incremental_sync_at: Some(900),
                entry_count: 3,
                updated_at: 1000,
            },
            old_needle_hits: BTreeSet::new(),
            new_needle_hits: BTreeSet::new(),
        };
        projection.rebuild_fts_projection();
        projection
    }

    fn rebuild_fts_projection(&mut self) {
        self.old_needle_hits = self
            .entries
            .values()
            .filter(|entry| !entry.stale && entry_matches(entry, "oldneedle"))
            .map(|entry| entry.id.clone())
            .collect();
        self.new_needle_hits = self
            .entries
            .values()
            .filter(|entry| !entry.stale && entry_matches(entry, "newneedle"))
            .map(|entry| entry.id.clone())
            .collect();
    }

    fn publish_independently(
        &mut self,
        staged: &BTreeMap<String, Entry>,
        checkpoint: (&str, &str),
    ) {
        for link in self.managed.values_mut() {
            link.enabled = false;
        }
        for (id, entry) in &mut self.entries {
            if !staged.contains_key(id) {
                entry.stale = true;
                if let Some(link) = self.managed.get_mut(&format!("managed-{id}")) {
                    link.enabled = false;
                }
                if let Some(state) = self.analysis.get_mut(id) {
                    state.status = "stale".into();
                    state.last_error = Some("global_entry_stale".into());
                }
                for job in self.jobs.values_mut().filter(|job| job.entry_id == *id) {
                    if matches!(job.status.as_str(), "pending" | "running") {
                        job.status = "stale".into();
                        job.last_error = Some("global_entry_stale".into());
                    }
                }
                for item in self.items.values_mut().filter(|item| item.entry_id == *id) {
                    if matches!(item.status.as_str(), "pending" | "running") {
                        item.status = "stale".into();
                        item.last_error = Some("global_entry_stale".into());
                    }
                }
            }
        }

        for (id, incoming) in staged {
            let mut active = incoming.clone();
            active.stale = false;
            self.entries.insert(id.clone(), active.clone());
            if active.path.starts_with("C:/managed/") {
                self.managed.insert(
                    format!("managed-{id}"),
                    ManagedLink {
                        entry_id: id.clone(),
                        enabled: true,
                    },
                );
            }
            if let Some(state) = self.analysis.get_mut(id) {
                if state.fingerprint != active.fingerprint {
                    state.status = "stale".into();
                    state.fingerprint = active.fingerprint.clone();
                    state.last_error = Some("input_fingerprint_changed".into());
                }
            }
            for job in self.jobs.values_mut().filter(|job| job.entry_id == *id) {
                if job.fingerprint != active.fingerprint
                    && matches!(job.status.as_str(), "pending" | "running" | "completed")
                {
                    job.status = "stale".into();
                    job.last_error = Some("input_fingerprint_changed".into());
                }
            }
            for item in self.items.values_mut().filter(|item| item.entry_id == *id) {
                let fingerprint_changed = self
                    .jobs
                    .values()
                    .any(|job| job.entry_id == *id && job.fingerprint != active.fingerprint);
                if fingerprint_changed
                    && matches!(item.status.as_str(), "pending" | "running" | "completed")
                {
                    item.status = "stale".into();
                    item.last_error = Some("input_fingerprint_changed".into());
                }
            }
        }

        self.health.index_status = "ready".into();
        self.health.last_error = None;
        self.health.journal_id = Some(checkpoint.0.into());
        self.health.journal_cursor = Some(checkpoint.1.into());
        self.health.last_full_index_at = Some(3000);
        self.health.entry_count = self.entries.values().filter(|entry| !entry.stale).count() as i64;
        self.health.updated_at = 3000;
        self.rebuild_fts_projection();
    }
}

fn entry_matches(entry: &Entry, needle: &str) -> bool {
    entry.name.contains(needle) || entry.path.contains(needle) || entry.extension.contains(needle)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Phase {
    Queued,
    Dispatching,
    Staging,
    Unknown,
    Cancelled,
    Completed,
}

impl Phase {
    fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Queued | Self::Dispatching | Self::Staging | Self::Unknown
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OracleOperation {
    id: String,
    source_id: String,
    revision: i64,
    request_id: String,
    phase: Phase,
    staged: BTreeMap<String, Entry>,
    checkpoint: Option<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum IssueResult {
    Created { operation_id: String, revision: i64 },
    Coalesced { operation_id: String, revision: i64 },
    RejectedDisabled,
    RejectedMissingSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum AdmissionResult {
    Accepted,
    RejectedBusy,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AdmissionAck {
    protocol_version: u16,
    request_id: String,
    source_id: String,
    stable_volume_id: String,
    mount_path: String,
    provider: String,
    event_kind: String,
    event_status: String,
    event_ordinal: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct BusyResponse {
    protocol_version: u16,
    request_id: String,
    ok: bool,
    error_code: Option<String>,
}

/// Independent in-memory oracle. It does not call the SQLite fixture helpers.
#[derive(Clone, Debug)]
struct CorrectnessOracle {
    source: SourceIdentity,
    projection: Projection,
    revision: i64,
    operations: BTreeMap<String, OracleOperation>,
}

impl CorrectnessOracle {
    fn new(source: SourceIdentity, projection: Projection) -> Self {
        Self {
            source,
            projection,
            revision: 0,
            operations: BTreeMap::new(),
        }
    }

    fn issue_rebuild(&mut self, operation_id: &str, request_id: &str) -> IssueResult {
        if !self.source.enabled {
            return IssueResult::RejectedDisabled;
        }
        if let Some(existing) = self
            .operations
            .values()
            .find(|operation| operation.source_id == self.source.id && operation.phase.is_active())
        {
            return IssueResult::Coalesced {
                operation_id: existing.id.clone(),
                revision: existing.revision,
            };
        }
        self.revision += 1;
        self.operations.insert(
            operation_id.into(),
            OracleOperation {
                id: operation_id.into(),
                source_id: self.source.id.clone(),
                revision: self.revision,
                request_id: request_id.into(),
                phase: Phase::Queued,
                staged: BTreeMap::new(),
                checkpoint: None,
            },
        );
        IssueResult::Created {
            operation_id: operation_id.into(),
            revision: self.revision,
        }
    }

    fn claim(&mut self, operation_id: &str, revision: i64) -> bool {
        let Some(operation) = self.operations.get_mut(operation_id) else {
            return false;
        };
        if operation.revision != revision || operation.phase != Phase::Queued {
            return false;
        }
        operation.phase = Phase::Dispatching;
        true
    }

    fn service_busy(&mut self, operation_id: &str, response: &BusyResponse) -> AdmissionResult {
        let Some(operation) = self.operations.get_mut(operation_id) else {
            return AdmissionResult::Unknown;
        };
        if response.request_id != operation.request_id
            || response.protocol_version != IPC_V3
            || response.ok
            || response.error_code.as_deref() != Some("index_service_busy")
            || operation.phase != Phase::Dispatching
        {
            operation.phase = Phase::Unknown;
            return AdmissionResult::Unknown;
        }
        operation.phase = Phase::Queued;
        AdmissionResult::RejectedBusy
    }

    fn acknowledge(&mut self, operation_id: &str, ack: &AdmissionAck) -> AdmissionResult {
        let Some(operation) = self.operations.get_mut(operation_id) else {
            return AdmissionResult::Unknown;
        };
        let source = &self.source;
        if operation.request_id != ack.request_id
            || operation.phase != Phase::Dispatching
            || ack.protocol_version != IPC_V3
            || ack.source_id != source.id
            || ack.stable_volume_id != source.stable_volume_id
            || ack.mount_path != source.mount_path
            || ack.provider != source.provider
            || ack.event_kind != "source_state"
            || ack.event_status != "indexing"
            || ack.event_ordinal != 0
        {
            operation.phase = Phase::Unknown;
            return AdmissionResult::Unknown;
        }
        operation.phase = Phase::Staging;
        AdmissionResult::Accepted
    }

    fn transport_unknown(&mut self, operation_id: &str) {
        if let Some(operation) = self.operations.get_mut(operation_id) {
            if matches!(operation.phase, Phase::Dispatching | Phase::Staging) {
                operation.phase = Phase::Unknown;
            }
        }
    }

    fn stage(
        &mut self,
        operation_id: &str,
        revision: i64,
        entries: &[Entry],
        checkpoint: (&str, &str),
    ) -> bool {
        let Some(operation) = self.operations.get_mut(operation_id) else {
            return false;
        };
        if operation.revision != revision || operation.phase != Phase::Staging {
            return false;
        }
        for entry in entries {
            operation.staged.insert(entry.id.clone(), entry.clone());
        }
        operation.checkpoint = Some((checkpoint.0.into(), checkpoint.1.into()));
        true
    }

    fn cancel(&mut self, operation_id: &str) -> bool {
        let Some(operation) = self.operations.get_mut(operation_id) else {
            return false;
        };
        if !operation.phase.is_active() {
            return false;
        }
        operation.phase = Phase::Cancelled;
        operation.staged.clear();
        operation.checkpoint = None;
        true
    }

    fn recover_after_process_restart(&mut self) {
        for operation in self.operations.values_mut() {
            if matches!(operation.phase, Phase::Dispatching | Phase::Staging) {
                operation.phase = Phase::Unknown;
            }
        }
    }

    fn publish(&mut self, operation_id: &str, revision: i64) -> bool {
        let Some(operation) = self.operations.get_mut(operation_id) else {
            return false;
        };
        if operation.revision != revision
            || operation.phase != Phase::Staging
            || !self.source.enabled
        {
            return false;
        }
        let Some(checkpoint) = operation.checkpoint.clone() else {
            return false;
        };
        self.projection
            .publish_independently(&operation.staged, (&checkpoint.0, &checkpoint.1));
        operation.phase = Phase::Completed;
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TransportKnowledge {
    ProvenNotSent,
    ServiceBusy,
    UnknownAfterSend,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirectPolicy {
    Disabled,
    AllowOnlyProvenNotSent,
}

fn direct_fallback_allowed(transport: TransportKnowledge, policy: DirectPolicy) -> bool {
    transport == TransportKnowledge::ProvenNotSent && policy == DirectPolicy::AllowOnlyProvenNotSent
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PublishStep {
    AfterMissingEntries,
    AfterEntryMerge,
    AfterManagedReconcile,
    AfterAiReconcile,
    AfterSourceStateUpdate,
    BeforeOperationComplete,
}

#[derive(Debug)]
struct PublishFailure {
    message: String,
    transaction_duration: Duration,
}

static TEMP_DB_COUNTER: AtomicUsize = AtomicUsize::new(0);

struct DatabaseFixture {
    path: PathBuf,
}

impl DatabaseFixture {
    fn new() -> Self {
        let unique = TEMP_DB_COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "zen-canvas-issue380-d0-{}-{unique}-{nanos}.sqlite3",
            std::process::id()
        ));
        let fixture = Self { path };
        fixture.initialize();
        fixture
    }

    fn connect(&self) -> Connection {
        let connection = Connection::open(&self.path).expect("open isolated D0 database");
        connection
            .execute_batch(
                "PRAGMA foreign_keys=ON;
                 PRAGMA journal_mode=WAL;
                 PRAGMA synchronous=NORMAL;
                 PRAGMA wal_autocheckpoint=0;
                 PRAGMA busy_timeout=10000;",
            )
            .expect("configure isolated D0 database");
        connection
    }

    fn initialize(&self) {
        let mut connection = self.connect();
        connection
            .execute_batch(
                r#"
                CREATE TABLE global_volumes (
                    id TEXT PRIMARY KEY,
                    stable_volume_id TEXT NOT NULL,
                    mount_path TEXT NOT NULL,
                    provider TEXT NOT NULL,
                    enabled INTEGER NOT NULL,
                    index_status TEXT NOT NULL,
                    last_error TEXT,
                    journal_id TEXT,
                    journal_cursor TEXT,
                    last_full_index_at INTEGER,
                    last_incremental_sync_at INTEGER,
                    entry_count INTEGER NOT NULL DEFAULT 0,
                    updated_at INTEGER NOT NULL,
                    operation_revision INTEGER NOT NULL DEFAULT 0
                );
                CREATE TABLE global_entries (
                    id TEXT PRIMARY KEY,
                    volume_id TEXT NOT NULL REFERENCES global_volumes(id),
                    name TEXT NOT NULL,
                    path TEXT NOT NULL,
                    extension TEXT NOT NULL,
                    input_fingerprint TEXT NOT NULL,
                    is_stale INTEGER NOT NULL DEFAULT 0
                );
                CREATE INDEX idx_d0_global_entries_volume ON global_entries(volume_id, is_stale);
                CREATE VIRTUAL TABLE global_entries_fts USING fts5(
                    name, path, extension,
                    content='global_entries', content_rowid='rowid', tokenize='trigram'
                );
                CREATE TRIGGER global_entries_ai AFTER INSERT ON global_entries BEGIN
                    INSERT INTO global_entries_fts(rowid, name, path, extension)
                    VALUES (new.rowid, new.name, new.path, new.extension);
                END;
                CREATE TRIGGER global_entries_ad AFTER DELETE ON global_entries BEGIN
                    INSERT INTO global_entries_fts(global_entries_fts, rowid, name, path, extension)
                    VALUES ('delete', old.rowid, old.name, old.path, old.extension);
                END;
                CREATE TRIGGER global_entries_au AFTER UPDATE OF name, path, extension ON global_entries
                WHEN old.name IS NOT new.name OR old.path IS NOT new.path OR old.extension IS NOT new.extension BEGIN
                    INSERT INTO global_entries_fts(global_entries_fts, rowid, name, path, extension)
                    VALUES ('delete', old.rowid, old.name, old.path, old.extension);
                    INSERT INTO global_entries_fts(rowid, name, path, extension)
                    VALUES (new.rowid, new.name, new.path, new.extension);
                END;
                CREATE TABLE managed_entries (
                    id TEXT PRIMARY KEY,
                    global_entry_id TEXT NOT NULL REFERENCES global_entries(id) ON DELETE CASCADE,
                    managed_scope_id TEXT NOT NULL,
                    enabled INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    UNIQUE(global_entry_id, managed_scope_id)
                );
                CREATE TABLE ai_analysis_state (
                    global_entry_id TEXT PRIMARY KEY REFERENCES global_entries(id) ON DELETE CASCADE,
                    status TEXT NOT NULL,
                    input_fingerprint TEXT NOT NULL,
                    classification_json TEXT,
                    user_corrected INTEGER NOT NULL,
                    last_error TEXT
                );
                CREATE TABLE ai_jobs (
                    id TEXT PRIMARY KEY,
                    global_entry_id TEXT NOT NULL REFERENCES global_entries(id) ON DELETE CASCADE,
                    input_fingerprint TEXT NOT NULL,
                    status TEXT NOT NULL,
                    last_error TEXT
                );
                CREATE TABLE ai_job_items (
                    id TEXT PRIMARY KEY,
                    job_id TEXT NOT NULL REFERENCES ai_jobs(id) ON DELETE CASCADE,
                    global_entry_id TEXT NOT NULL REFERENCES global_entries(id) ON DELETE CASCADE,
                    status TEXT NOT NULL,
                    last_error TEXT
                );
                CREATE TABLE d0_operations (
                    operation_id TEXT PRIMARY KEY,
                    source_id TEXT NOT NULL REFERENCES global_volumes(id),
                    revision INTEGER NOT NULL CHECK (revision > 0),
                    request_id TEXT NOT NULL UNIQUE,
                    protocol_version INTEGER NOT NULL,
                    expected_stable_volume_id TEXT NOT NULL,
                    expected_mount_path TEXT NOT NULL,
                    expected_provider TEXT NOT NULL,
                    phase TEXT NOT NULL CHECK (
                        phase IN ('queued','dispatching','staging','unknown','cancelled','completed')
                    ),
                    last_error TEXT,
                    UNIQUE(source_id, revision)
                );
                CREATE UNIQUE INDEX d0_one_active_operation_per_source
                    ON d0_operations(source_id)
                    WHERE phase IN ('queued', 'dispatching', 'staging', 'unknown');
                CREATE TABLE d0_stage_entries (
                    operation_id TEXT NOT NULL REFERENCES d0_operations(operation_id) ON DELETE CASCADE,
                    entry_id TEXT NOT NULL,
                    source_id TEXT NOT NULL REFERENCES global_volumes(id),
                    name TEXT NOT NULL,
                    path TEXT NOT NULL,
                    extension TEXT NOT NULL,
                    input_fingerprint TEXT NOT NULL,
                    PRIMARY KEY(operation_id, entry_id)
                );
                CREATE TABLE d0_stage_checkpoint (
                    operation_id TEXT PRIMARY KEY REFERENCES d0_operations(operation_id) ON DELETE CASCADE,
                    journal_id TEXT NOT NULL,
                    journal_cursor TEXT NOT NULL
                );
                "#,
            )
            .expect("create test-only Schema 38-shaped fixture");
        seed_baseline(&mut connection);
    }

    fn path(&self) -> PathBuf {
        self.path.clone()
    }

    fn seed_source(&self, identity: &SourceIdentity) {
        let connection = self.connect();
        connection
            .execute(
                "INSERT INTO global_volumes
                 (id, stable_volume_id, mount_path, provider, enabled, index_status,
                  last_error, journal_id, journal_cursor, last_full_index_at,
                  last_incremental_sync_at, entry_count, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'ready', NULL,
                         'journal-old', 'cursor-old', 700, 900, 0, 1000)",
                params![
                    identity.id,
                    identity.stable_volume_id,
                    identity.mount_path,
                    identity.provider,
                    i64::from(identity.enabled)
                ],
            )
            .expect("seed secondary source");
    }

    fn enabled_source_ids(&self) -> Vec<String> {
        let connection = self.connect();
        let mut statement = connection
            .prepare("SELECT id FROM global_volumes WHERE enabled=1 ORDER BY id")
            .expect("prepare enabled-source query");
        statement
            .query_map([], |row| row.get(0))
            .expect("query enabled sources")
            .collect::<Result<Vec<String>, _>>()
            .expect("read enabled sources")
    }

    fn create_intent(
        &self,
        source_id: &str,
        operation_id: &str,
        request_id: &str,
    ) -> Result<IssueResult, String> {
        let mut connection = self.connect();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let source = transaction
            .query_row(
                "SELECT stable_volume_id, mount_path, provider, enabled, operation_revision
                 FROM global_volumes WHERE id=?1",
                [source_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)? != 0,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some((stable_id, mount_path, provider, enabled, revision)) = source else {
            transaction.commit().map_err(|error| error.to_string())?;
            return Ok(IssueResult::RejectedMissingSource);
        };
        if !enabled {
            transaction.commit().map_err(|error| error.to_string())?;
            return Ok(IssueResult::RejectedDisabled);
        }
        if let Some((existing_id, existing_revision)) = transaction
            .query_row(
                "SELECT operation_id, revision FROM d0_operations
                 WHERE source_id=?1 AND phase IN ('queued','dispatching','staging','unknown')",
                [source_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?
        {
            transaction.commit().map_err(|error| error.to_string())?;
            return Ok(IssueResult::Coalesced {
                operation_id: existing_id,
                revision: existing_revision,
            });
        }
        let next_revision = revision + 1;
        let changed = transaction
            .execute(
                "UPDATE global_volumes SET operation_revision=?2
                 WHERE id=?1 AND enabled=1 AND operation_revision=?3
                   AND stable_volume_id=?4 AND mount_path=?5 AND provider=?6",
                params![
                    source_id,
                    next_revision,
                    revision,
                    stable_id,
                    mount_path,
                    provider
                ],
            )
            .map_err(|error| error.to_string())?;
        if changed != 1 {
            transaction.rollback().map_err(|error| error.to_string())?;
            return Ok(IssueResult::RejectedMissingSource);
        }
        transaction
            .execute(
                "INSERT INTO d0_operations
                 (operation_id, source_id, revision, request_id, protocol_version,
                  expected_stable_volume_id, expected_mount_path, expected_provider, phase)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'queued')",
                params![
                    operation_id,
                    source_id,
                    next_revision,
                    request_id,
                    IPC_V3,
                    stable_id,
                    mount_path,
                    provider
                ],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(IssueResult::Created {
            operation_id: operation_id.into(),
            revision: next_revision,
        })
    }

    fn operation(&self, operation_id: &str) -> (i64, String, String, String) {
        let connection = self.connect();
        connection
            .query_row(
                "SELECT revision, phase, request_id, source_id FROM d0_operations WHERE operation_id=?1",
                [operation_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("read D0 operation")
    }

    fn claim(&self, operation_id: &str, revision: i64) -> Result<bool, String> {
        let connection = self.connect();
        let changed = connection
            .execute(
                "UPDATE d0_operations SET phase='dispatching'
                 WHERE operation_id=?1 AND revision=?2 AND phase='queued'
                   AND EXISTS (
                     SELECT 1 FROM global_volumes gv
                     WHERE gv.id=d0_operations.source_id AND gv.enabled=1
                       AND gv.operation_revision=d0_operations.revision
                       AND gv.stable_volume_id=d0_operations.expected_stable_volume_id
                       AND gv.mount_path=d0_operations.expected_mount_path
                       AND gv.provider=d0_operations.expected_provider
                   )",
                params![operation_id, revision],
            )
            .map_err(|error| error.to_string())?;
        Ok(changed == 1)
    }

    fn busy_response(
        &self,
        operation_id: &str,
        response: &BusyResponse,
    ) -> Result<AdmissionResult, String> {
        let mut connection = self.connect();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let row = transaction
            .query_row(
                "SELECT phase,request_id FROM d0_operations WHERE operation_id=?1",
                [operation_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some((phase, request_id)) = row else {
            transaction.commit().map_err(|error| error.to_string())?;
            return Ok(AdmissionResult::Unknown);
        };
        if response.request_id != request_id
            || response.protocol_version != IPC_V3
            || response.ok
            || response.error_code.as_deref() != Some("index_service_busy")
            || phase != "dispatching"
        {
            transaction
                .execute(
                    "UPDATE d0_operations SET phase='unknown', last_error='untrusted_service_frame'
                     WHERE operation_id=?1 AND phase IN ('dispatching','staging')",
                    [operation_id],
                )
                .map_err(|error| error.to_string())?;
            transaction.commit().map_err(|error| error.to_string())?;
            return Ok(AdmissionResult::Unknown);
        }
        let changed = transaction
            .execute(
                "UPDATE d0_operations SET phase='queued', last_error='service_busy'
                 WHERE operation_id=?1 AND request_id=?2 AND phase='dispatching'
                   AND revision=(SELECT operation_revision FROM global_volumes
                                 WHERE id=d0_operations.source_id)",
                params![operation_id, response.request_id],
            )
            .map_err(|error| error.to_string())?;
        if changed != 1 {
            transaction
                .execute(
                    "UPDATE d0_operations SET phase='unknown',last_error='busy_reply_lost_claim'
                     WHERE operation_id=?1 AND phase='dispatching'",
                    [&operation_id],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(if changed == 1 {
            AdmissionResult::RejectedBusy
        } else {
            AdmissionResult::Unknown
        })
    }

    fn acknowledge(
        &self,
        operation_id: &str,
        ack: &AdmissionAck,
    ) -> Result<AdmissionResult, String> {
        let mut connection = self.connect();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let record = transaction
            .query_row(
                "SELECT operation_id, source_id, phase, request_id, protocol_version,
                        expected_stable_volume_id,
                        expected_mount_path, expected_provider
                 FROM d0_operations WHERE operation_id=?1",
                [operation_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some((
            operation_id,
            source_id,
            phase,
            request_id,
            expected_protocol,
            stable_id,
            mount_path,
            provider,
        )) = record
        else {
            transaction.commit().map_err(|error| error.to_string())?;
            return Ok(AdmissionResult::Unknown);
        };
        let source_now = transaction
            .query_row(
                "SELECT enabled, stable_volume_id, mount_path, provider
                 FROM global_volumes WHERE id=?1",
                [&source_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)? != 0,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let valid = phase == "dispatching"
            && ack.request_id == request_id
            && expected_protocol == i64::from(IPC_V3)
            && ack.protocol_version == IPC_V3
            && ack.source_id == source_id
            && ack.stable_volume_id == stable_id
            && ack.mount_path == mount_path
            && ack.provider == provider
            && ack.event_kind == "source_state"
            && ack.event_status == "indexing"
            && ack.event_ordinal == 0
            && source_now.is_some_and(
                |(enabled, current_stable, current_mount, current_provider)| {
                    enabled
                        && current_stable == stable_id
                        && current_mount == mount_path
                        && current_provider == provider
                },
            );
        if !valid {
            transaction
                .execute(
                    "UPDATE d0_operations SET phase='unknown', last_error='untrusted_service_frame'
                     WHERE operation_id=?1 AND phase='dispatching'",
                    [&operation_id],
                )
                .map_err(|error| error.to_string())?;
            transaction.commit().map_err(|error| error.to_string())?;
            return Ok(AdmissionResult::Unknown);
        }
        let changed = transaction
            .execute(
                "UPDATE d0_operations SET phase='staging', last_error=NULL
                 WHERE operation_id=?1 AND phase='dispatching'
                   AND revision=(SELECT operation_revision FROM global_volumes WHERE id=?2)",
                params![operation_id, source_id],
            )
            .map_err(|error| error.to_string())?;
        if changed != 1 {
            transaction
                .execute(
                    "UPDATE d0_operations SET phase='unknown',last_error='admission_lost_claim'
                     WHERE operation_id=?1 AND phase='dispatching'",
                    [&operation_id],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(if changed == 1 {
            AdmissionResult::Accepted
        } else {
            AdmissionResult::Unknown
        })
    }

    fn stage_batch(
        &self,
        operation_id: &str,
        revision: i64,
        entries: &[Entry],
        checkpoint: (&str, &str),
    ) -> Result<bool, String> {
        let mut connection = self.connect();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let owner = transaction
            .query_row(
                "SELECT o.source_id, o.phase, gv.enabled, gv.operation_revision,
                        gv.stable_volume_id, gv.mount_path, gv.provider,
                        o.expected_stable_volume_id, o.expected_mount_path, o.expected_provider
                 FROM d0_operations o JOIN global_volumes gv ON gv.id=o.source_id
                 WHERE o.operation_id=?1 AND o.revision=?2",
                params![operation_id, revision],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)? != 0,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, String>(9)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some((
            source_id,
            phase,
            enabled,
            current_revision,
            current_stable,
            current_mount,
            current_provider,
            expected_stable,
            expected_mount,
            expected_provider,
        )) = owner
        else {
            transaction.commit().map_err(|error| error.to_string())?;
            return Ok(false);
        };
        if phase != "staging"
            || !enabled
            || current_revision != revision
            || current_stable != expected_stable
            || current_mount != expected_mount
            || current_provider != expected_provider
            || entries.iter().any(|entry| entry.volume_id != source_id)
        {
            transaction.commit().map_err(|error| error.to_string())?;
            return Ok(false);
        }
        for entry in entries {
            transaction
                .execute(
                    "INSERT INTO d0_stage_entries
                     (operation_id, entry_id, source_id, name, path, extension, input_fingerprint)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(operation_id, entry_id) DO UPDATE SET
                       source_id=excluded.source_id, name=excluded.name, path=excluded.path,
                       extension=excluded.extension, input_fingerprint=excluded.input_fingerprint",
                    params![
                        operation_id,
                        entry.id,
                        entry.volume_id,
                        entry.name,
                        entry.path,
                        entry.extension,
                        entry.fingerprint
                    ],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction
            .execute(
                "INSERT INTO d0_stage_checkpoint(operation_id,journal_id,journal_cursor)
                 VALUES (?1,?2,?3)
                 ON CONFLICT(operation_id) DO UPDATE SET
                   journal_id=excluded.journal_id,journal_cursor=excluded.journal_cursor",
                params![operation_id, checkpoint.0, checkpoint.1],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(true)
    }

    fn cancel(&self, operation_id: &str, revision: i64) -> Result<bool, String> {
        let mut connection = self.connect();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;
        let changed = transaction
            .execute(
                "UPDATE d0_operations SET phase='cancelled',last_error='cancelled'
                 WHERE operation_id=?1 AND revision=?2
                   AND phase IN ('queued','dispatching','staging','unknown')",
                params![operation_id, revision],
            )
            .map_err(|error| error.to_string())?;
        if changed == 1 {
            transaction
                .execute(
                    "DELETE FROM d0_stage_entries WHERE operation_id=?1",
                    [operation_id],
                )
                .map_err(|error| error.to_string())?;
            transaction
                .execute(
                    "DELETE FROM d0_stage_checkpoint WHERE operation_id=?1",
                    [operation_id],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(changed == 1)
    }

    fn mark_unknown_after_restart(&self) -> Result<(), String> {
        let connection = self.connect();
        connection
            .execute(
                "UPDATE d0_operations SET phase='unknown',last_error='process_restart_outcome_unknown'
                 WHERE phase IN ('dispatching','staging')",
                [],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn mark_unknown_after_transport(&self, operation_id: &str) -> Result<(), String> {
        let connection = self.connect();
        connection
            .execute(
                "UPDATE d0_operations SET phase='unknown',last_error='service_transport_outcome_unknown'
                 WHERE operation_id=?1 AND phase IN ('dispatching','staging')",
                [operation_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn publish(
        &self,
        operation_id: &str,
        revision: i64,
        failpoint: Option<PublishStep>,
    ) -> Result<Duration, PublishFailure> {
        let started = Instant::now();
        let mut connection = self.connect();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| PublishFailure {
                message: error.to_string(),
                transaction_duration: started.elapsed(),
            })?;
        let result = publish_transaction(&transaction, operation_id, revision, failpoint);
        match result {
            Ok(()) => {
                if let Err(error) = transaction.commit() {
                    return Err(PublishFailure {
                        message: error.to_string(),
                        transaction_duration: started.elapsed(),
                    });
                }
                Ok(started.elapsed())
            }
            Err(message) => {
                drop(transaction);
                Err(PublishFailure {
                    message,
                    transaction_duration: started.elapsed(),
                })
            }
        }
    }

    fn stage_count(&self, operation_id: &str) -> i64 {
        let connection = self.connect();
        connection
            .query_row(
                "SELECT COUNT(*) FROM d0_stage_entries WHERE operation_id=?1",
                [operation_id],
                |row| row.get(0),
            )
            .expect("read stage row count")
    }

    fn snapshot(&self, source_id: &str) -> Projection {
        let connection = self.connect();
        snapshot_projection(&connection, source_id)
    }

    fn wal_bytes(&self) -> u64 {
        let wal = PathBuf::from(format!("{}-wal", self.path.display()));
        fs::metadata(wal)
            .map(|metadata| metadata.len())
            .unwrap_or(0)
    }

    fn checkpoint_wal(&self) {
        let connection = self.connect();
        let _: (i64, i64, i64) = connection
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .expect("checkpoint isolated D0 WAL");
    }
}

impl Drop for DatabaseFixture {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let mut path = self.path.as_os_str().to_os_string();
            path.push(suffix);
            let _ = fs::remove_file(PathBuf::from(path));
        }
    }
}

fn seed_baseline(connection: &mut Connection) {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .expect("begin seed transaction");
    transaction
        .execute(
            "INSERT INTO global_volumes
             (id,stable_volume_id,mount_path,provider,enabled,index_status,last_error,
              journal_id,journal_cursor,last_full_index_at,last_incremental_sync_at,entry_count,updated_at)
             VALUES ('source-a','volume-a','C:/','windows_mft_usn',1,'ready',NULL,
                     'journal-old','cursor-old',700,900,0,1000)",
            [],
        )
        .expect("seed baseline source");
    for entry in Projection::baseline().entries.values() {
        transaction
            .execute(
                "INSERT INTO global_entries
                 (id,volume_id,name,path,extension,input_fingerprint,is_stale)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    entry.id,
                    entry.volume_id,
                    entry.name,
                    entry.path,
                    entry.extension,
                    entry.fingerprint,
                    i64::from(entry.stale)
                ],
            )
            .expect("seed global entry");
    }
    transaction
        .execute(
            "UPDATE global_volumes SET entry_count=3 WHERE id='source-a'",
            [],
        )
        .expect("seed source count");
    for entry_id in ["entry-alpha", "entry-beta"] {
        transaction
            .execute(
                "INSERT INTO managed_entries(id,global_entry_id,managed_scope_id,enabled,updated_at)
                 VALUES (?1,?2,?3,1,1000)",
                params![format!("managed-{entry_id}"), entry_id, MANAGED_SCOPE],
            )
            .expect("seed managed association");
    }
    transaction
        .execute(
            "INSERT INTO ai_analysis_state
             (global_entry_id,status,input_fingerprint,classification_json,user_corrected,last_error)
             VALUES ('entry-alpha','completed','fp-alpha-v1','{\"class\":\"owner-corrected\"}',1,NULL),
                    ('entry-beta','pending','fp-beta-v1',NULL,0,NULL)",
            [],
        )
        .expect("seed analysis state");
    transaction
        .execute(
            "INSERT INTO ai_jobs(id,global_entry_id,input_fingerprint,status,last_error)
             VALUES ('job-alpha','entry-alpha','fp-alpha-v1','completed',NULL),
                    ('job-beta','entry-beta','fp-beta-v1','pending',NULL)",
            [],
        )
        .expect("seed AI jobs");
    transaction
        .execute(
            "INSERT INTO ai_job_items(id,job_id,global_entry_id,status,last_error)
             VALUES ('item-alpha','job-alpha','entry-alpha','completed',NULL),
                    ('item-beta','job-beta','entry-beta','pending',NULL)",
            [],
        )
        .expect("seed AI job items");
    transaction.commit().expect("commit baseline seed");
}

fn snapshot_projection(connection: &Connection, source_id: &str) -> Projection {
    let health = connection
        .query_row(
            "SELECT index_status,last_error,journal_id,journal_cursor,last_full_index_at,
                    last_incremental_sync_at,entry_count,updated_at
             FROM global_volumes WHERE id=?1",
            [source_id],
            |row| {
                Ok(SourceHealth {
                    index_status: row.get(0)?,
                    last_error: row.get(1)?,
                    journal_id: row.get(2)?,
                    journal_cursor: row.get(3)?,
                    last_full_index_at: row.get(4)?,
                    last_incremental_sync_at: row.get(5)?,
                    entry_count: row.get(6)?,
                    updated_at: row.get(7)?,
                })
            },
        )
        .expect("read source health projection");
    let entries = read_map(
        connection,
        source_id,
        "SELECT id,volume_id,name,path,extension,input_fingerprint,is_stale
         FROM global_entries WHERE volume_id=?1 ORDER BY id",
        |row| {
            let entry = Entry {
                id: row.get(0)?,
                volume_id: row.get(1)?,
                name: row.get(2)?,
                path: row.get(3)?,
                extension: row.get(4)?,
                fingerprint: row.get(5)?,
                stale: row.get::<_, i64>(6)? != 0,
            };
            Ok((entry.id.clone(), entry))
        },
    );
    let managed = read_map(
        connection,
        source_id,
        "SELECT me.id,me.global_entry_id,me.enabled FROM managed_entries me
         JOIN global_entries ge ON ge.id=me.global_entry_id WHERE ge.volume_id=?1 ORDER BY me.id",
        |row| {
            let id: String = row.get(0)?;
            let link = ManagedLink {
                entry_id: row.get(1)?,
                enabled: row.get::<_, i64>(2)? != 0,
            };
            Ok((id, link))
        },
    );
    let analysis = read_map(connection, source_id,
        "SELECT s.global_entry_id,s.status,s.input_fingerprint,s.classification_json,s.user_corrected,s.last_error
         FROM ai_analysis_state s JOIN global_entries ge ON ge.id=s.global_entry_id
         WHERE ge.volume_id=?1 ORDER BY s.global_entry_id",
        |row| {
            let state = AnalysisState {
                entry_id: row.get(0)?, status: row.get(1)?, fingerprint: row.get(2)?,
                classification_json: row.get(3)?, user_corrected: row.get::<_, i64>(4)? != 0,
                last_error: row.get(5)?,
            };
            Ok((state.entry_id.clone(), state))
        });
    let jobs = read_map(
        connection,
        source_id,
        "SELECT j.id,j.global_entry_id,j.status,j.input_fingerprint,j.last_error
         FROM ai_jobs j JOIN global_entries ge ON ge.id=j.global_entry_id
         WHERE ge.volume_id=?1 ORDER BY j.id",
        |row| {
            let job = AiJob {
                id: row.get(0)?,
                entry_id: row.get(1)?,
                status: row.get(2)?,
                fingerprint: row.get(3)?,
                last_error: row.get(4)?,
            };
            Ok((job.id.clone(), job))
        },
    );
    let items = read_map(
        connection,
        source_id,
        "SELECT i.id,i.job_id,i.global_entry_id,i.status,i.last_error
         FROM ai_job_items i JOIN global_entries ge ON ge.id=i.global_entry_id
         WHERE ge.volume_id=?1 ORDER BY i.id",
        |row| {
            let item = AiJobItem {
                id: row.get(0)?,
                job_id: row.get(1)?,
                entry_id: row.get(2)?,
                status: row.get(3)?,
                last_error: row.get(4)?,
            };
            Ok((item.id.clone(), item))
        },
    );
    let old_needle_hits = fts_hits(connection, source_id, "oldneedle");
    let new_needle_hits = fts_hits(connection, source_id, "newneedle");
    Projection {
        entries,
        managed,
        analysis,
        jobs,
        items,
        health,
        old_needle_hits,
        new_needle_hits,
    }
}

fn read_map<T>(
    connection: &Connection,
    source_id: &str,
    sql: &str,
    mut map_row: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<(String, T)>,
) -> BTreeMap<String, T> {
    let mut statement = connection.prepare(sql).expect("prepare projection query");
    statement
        .query_map([source_id], |row| map_row(row))
        .expect("query projection")
        .collect::<Result<BTreeMap<_, _>, _>>()
        .expect("collect projection")
}

fn fts_hits(connection: &Connection, source_id: &str, needle: &str) -> BTreeSet<String> {
    let mut statement = connection
        .prepare(
            "SELECT ge.id FROM global_entries_fts
             JOIN global_entries ge ON ge.rowid=global_entries_fts.rowid
             WHERE global_entries_fts MATCH ?1 AND ge.volume_id=?2 AND ge.is_stale=0
             ORDER BY ge.id",
        )
        .expect("prepare FTS projection query");
    statement
        .query_map(params![needle, source_id], |row| row.get(0))
        .expect("query FTS projection")
        .collect::<Result<BTreeSet<String>, _>>()
        .expect("collect FTS hits")
}

fn publish_transaction(
    transaction: &Transaction<'_>,
    operation_id: &str,
    revision: i64,
    failpoint: Option<PublishStep>,
) -> Result<(), String> {
    let owner = transaction
        .query_row(
            "SELECT o.source_id,o.phase,gv.enabled,gv.operation_revision,
                    gv.stable_volume_id,gv.mount_path,gv.provider,
                    o.expected_stable_volume_id,o.expected_mount_path,o.expected_provider
             FROM d0_operations o JOIN global_volumes gv ON gv.id=o.source_id
             WHERE o.operation_id=?1 AND o.revision=?2",
            params![operation_id, revision],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? != 0,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((
        source_id,
        phase,
        enabled,
        current_revision,
        stable,
        mount,
        provider,
        expected_stable,
        expected_mount,
        expected_provider,
    )) = owner
    else {
        return Err("operation owner is absent".into());
    };
    if phase != "staging"
        || !enabled
        || current_revision != revision
        || stable != expected_stable
        || mount != expected_mount
        || provider != expected_provider
    {
        return Err("operation lost source/revision ownership".into());
    }
    let checkpoint = transaction
        .query_row(
            "SELECT journal_id,journal_cursor FROM d0_stage_checkpoint WHERE operation_id=?1",
            [operation_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "staged checkpoint is absent".to_string())?;
    let staged = {
        let mut statement = transaction
            .prepare(
                "SELECT entry_id,source_id,name,path,extension,input_fingerprint
                 FROM d0_stage_entries WHERE operation_id=?1 ORDER BY entry_id",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([operation_id], |row| {
                Ok(Entry {
                    id: row.get(0)?,
                    volume_id: row.get(1)?,
                    name: row.get(2)?,
                    path: row.get(3)?,
                    extension: row.get(4)?,
                    fingerprint: row.get(5)?,
                    stale: false,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    if staged.iter().any(|entry| entry.volume_id != source_id) {
        return Err("stage contains a different source identity".into());
    }

    transaction
        .execute(
            "UPDATE global_entries SET is_stale=1
             WHERE volume_id=?1 AND id NOT IN
               (SELECT entry_id FROM d0_stage_entries WHERE operation_id=?2)",
            params![source_id, operation_id],
        )
        .map_err(|error| error.to_string())?;
    trip(failpoint, PublishStep::AfterMissingEntries)?;

    for entry in &staged {
        transaction
            .execute(
                "INSERT INTO global_entries
                 (id,volume_id,name,path,extension,input_fingerprint,is_stale)
                 VALUES (?1,?2,?3,?4,?5,?6,0)
                 ON CONFLICT(id) DO UPDATE SET
                   name=excluded.name,path=excluded.path,extension=excluded.extension,
                   input_fingerprint=excluded.input_fingerprint,is_stale=0
                 WHERE global_entries.volume_id=excluded.volume_id",
                params![
                    entry.id,
                    entry.volume_id,
                    entry.name,
                    entry.path,
                    entry.extension,
                    entry.fingerprint
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    trip(failpoint, PublishStep::AfterEntryMerge)?;

    transaction
        .execute(
            "UPDATE managed_entries SET enabled=0,updated_at=3000
             WHERE global_entry_id IN (SELECT id FROM global_entries WHERE volume_id=?1)",
            [&source_id],
        )
        .map_err(|error| error.to_string())?;
    for entry in &staged {
        if entry.path.starts_with("C:/managed/") {
            transaction
                .execute(
                    "INSERT INTO managed_entries(id,global_entry_id,managed_scope_id,enabled,updated_at)
                     VALUES (?1,?2,?3,1,3000)
                     ON CONFLICT(global_entry_id,managed_scope_id) DO UPDATE SET
                       enabled=1,updated_at=excluded.updated_at",
                    params![format!("managed-{}", entry.id), entry.id, MANAGED_SCOPE],
                )
                .map_err(|error| error.to_string())?;
        }
    }
    trip(failpoint, PublishStep::AfterManagedReconcile)?;

    transaction
        .execute(
            "UPDATE ai_jobs SET status='stale',last_error='global_entry_stale'
             WHERE global_entry_id IN (
               SELECT id FROM global_entries WHERE volume_id=?1 AND is_stale=1
             ) AND status IN ('pending','running')",
            [&source_id],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE ai_job_items SET status='stale',last_error='global_entry_stale'
             WHERE global_entry_id IN (
               SELECT id FROM global_entries WHERE volume_id=?1 AND is_stale=1
             ) AND status IN ('pending','running')",
            [&source_id],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE ai_analysis_state SET status='stale',last_error='global_entry_stale'
             WHERE global_entry_id IN (
               SELECT id FROM global_entries WHERE volume_id=?1 AND is_stale=1
             )",
            [&source_id],
        )
        .map_err(|error| error.to_string())?;
    for entry in &staged {
        transaction
            .execute(
                "UPDATE ai_jobs SET status='stale',last_error='input_fingerprint_changed'
                 WHERE global_entry_id=?1 AND input_fingerprint<>?2
                   AND status IN ('pending','running','completed')",
                params![entry.id, entry.fingerprint],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "UPDATE ai_job_items SET status='stale',last_error='input_fingerprint_changed'
                 WHERE global_entry_id=?1 AND job_id IN (
                   SELECT id FROM ai_jobs WHERE global_entry_id=?1 AND input_fingerprint<>?2
                 )",
                params![entry.id, entry.fingerprint],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "UPDATE ai_analysis_state SET status='stale',input_fingerprint=?2,
                    last_error='input_fingerprint_changed'
                 WHERE global_entry_id=?1 AND input_fingerprint<>?2",
                params![entry.id, entry.fingerprint],
            )
            .map_err(|error| error.to_string())?;
    }
    trip(failpoint, PublishStep::AfterAiReconcile)?;

    transaction
        .execute(
            "UPDATE global_volumes SET index_status='ready',last_error=NULL,
                 journal_id=?2,journal_cursor=?3,last_full_index_at=3000,
                 entry_count=(SELECT COUNT(*) FROM global_entries WHERE volume_id=?1 AND is_stale=0),
                 updated_at=3000
             WHERE id=?1 AND operation_revision=?4 AND enabled=1",
            params![source_id, checkpoint.0, checkpoint.1, revision],
        )
        .map_err(|error| error.to_string())?;
    trip(failpoint, PublishStep::AfterSourceStateUpdate)?;
    trip(failpoint, PublishStep::BeforeOperationComplete)?;

    let changed = transaction
        .execute(
            "UPDATE d0_operations SET phase='completed',last_error=NULL
             WHERE operation_id=?1 AND revision=?2 AND phase='staging'",
            params![operation_id, revision],
        )
        .map_err(|error| error.to_string())?;
    if changed != 1 {
        return Err("operation completion lost CAS".into());
    }
    transaction
        .execute(
            "DELETE FROM d0_stage_entries WHERE operation_id=?1",
            [operation_id],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "DELETE FROM d0_stage_checkpoint WHERE operation_id=?1",
            [operation_id],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn trip(failpoint: Option<PublishStep>, step: PublishStep) -> Result<(), String> {
    if failpoint == Some(step) {
        Err(format!("test injected failure at {step:?}"))
    } else {
        Ok(())
    }
}

fn source_a() -> SourceIdentity {
    SourceIdentity {
        id: "source-a".into(),
        stable_volume_id: "volume-a".into(),
        mount_path: "C:/".into(),
        provider: "windows_mft_usn".into(),
        enabled: true,
    }
}

fn source_b() -> SourceIdentity {
    SourceIdentity {
        id: "source-b".into(),
        stable_volume_id: "volume-b".into(),
        mount_path: "D:/".into(),
        provider: "windows_mft_usn".into(),
        enabled: true,
    }
}

fn replacement_entries() -> Vec<Entry> {
    vec![
        Entry {
            id: "entry-alpha".into(),
            volume_id: "source-a".into(),
            name: "newneedle-alpha.txt".into(),
            path: "C:/managed/newneedle-alpha.txt".into(),
            extension: "txt".into(),
            fingerprint: "fp-alpha-v2".into(),
            stale: false,
        },
        Entry {
            id: "entry-gamma".into(),
            volume_id: "source-a".into(),
            name: "stable-gamma.txt".into(),
            path: "C:/outside/stable-gamma.txt".into(),
            extension: "txt".into(),
            fingerprint: "fp-gamma-v1".into(),
            stale: false,
        },
        Entry {
            id: "entry-delta".into(),
            volume_id: "source-a".into(),
            name: "added-delta.txt".into(),
            path: "C:/managed/added-delta.txt".into(),
            extension: "txt".into(),
            fingerprint: "fp-delta-v1".into(),
            stale: false,
        },
    ]
}

fn valid_ack_for(source: &SourceIdentity, request_id: &str) -> AdmissionAck {
    AdmissionAck {
        protocol_version: IPC_V3,
        request_id: request_id.into(),
        source_id: source.id.clone(),
        stable_volume_id: source.stable_volume_id.clone(),
        mount_path: source.mount_path.clone(),
        provider: source.provider.clone(),
        event_kind: "source_state".into(),
        event_status: "indexing".into(),
        event_ordinal: 0,
    }
}

fn create_claimed_operation(
    database: &DatabaseFixture,
    operation_id: &str,
    request_id: &str,
) -> i64 {
    let issued = database
        .create_intent("source-a", operation_id, request_id)
        .expect("create operation intent");
    let IssueResult::Created { revision, .. } = issued else {
        panic!("expected newly created intent, got {issued:?}");
    };
    assert!(database
        .claim(operation_id, revision)
        .expect("claim intent"));
    revision
}

fn admit_operation(database: &DatabaseFixture, operation_id: &str, request_id: &str) -> i64 {
    let revision = create_claimed_operation(database, operation_id, request_id);
    assert_eq!(
        database
            .acknowledge(operation_id, &valid_ack_for(&source_a(), request_id))
            .unwrap(),
        AdmissionResult::Accepted
    );
    revision
}

fn stage_replacement(database: &DatabaseFixture, operation_id: &str, revision: i64) {
    database
        .stage_batch(
            operation_id,
            revision,
            &replacement_entries(),
            ("journal-new", "cursor-new"),
        )
        .expect("stage replacement entries")
        .then_some(())
        .expect("operation still owns source revision");
}

fn assert_health_is_original(database: &DatabaseFixture, source_id: &str) {
    let health = database.snapshot(source_id).health;
    assert_eq!(health.index_status, "ready");
    assert_eq!(health.last_error, None);
    assert_eq!(health.journal_id.as_deref(), Some("journal-old"));
    assert_eq!(health.journal_cursor.as_deref(), Some("cursor-old"));
    assert_eq!(health.last_full_index_at, Some(700));
    assert_eq!(health.last_incremental_sync_at, Some(900));
    assert_eq!(
        health.entry_count,
        if source_id == "source-a" { 3 } else { 0 }
    );
    assert_eq!(health.updated_at, 1000);
}

fn operation_count(database: &DatabaseFixture) -> i64 {
    database
        .connect()
        .query_row("SELECT COUNT(*) FROM d0_operations", [], |row| row.get(0))
        .expect("count D0 operations")
}

fn source_revision(database: &DatabaseFixture, source_id: &str) -> i64 {
    database
        .connect()
        .query_row(
            "SELECT operation_revision FROM global_volumes WHERE id=?1",
            [source_id],
            |row| row.get(0),
        )
        .expect("read source operation revision")
}

fn has_stage_checkpoint(database: &DatabaseFixture, operation_id: &str) -> bool {
    database
        .connect()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM d0_stage_checkpoint WHERE operation_id=?1)",
            [operation_id],
            |row| row.get::<_, i64>(0),
        )
        .expect("read staged checkpoint")
        != 0
}

#[test]
fn independent_oracle_matches_sqlite_and_active_projection_changes_only_at_publish() {
    let source = source_a();
    let baseline = Projection::baseline();
    let mut oracle = CorrectnessOracle::new(source.clone(), baseline.clone());
    let database = DatabaseFixture::new();

    let oracle_issue = oracle.issue_rebuild("op-a1", "req-a1");
    let db_issue = database
        .create_intent("source-a", "op-a1", "req-a1")
        .expect("create durable intent");
    assert_eq!(db_issue, oracle_issue);
    assert_eq!(database.snapshot("source-a"), baseline);

    let revision = match oracle_issue {
        IssueResult::Created { revision, .. } => revision,
        other => panic!("expected new operation, got {other:?}"),
    };
    assert!(oracle.claim("op-a1", revision));
    assert!(database.claim("op-a1", revision).unwrap());
    assert_eq!(
        oracle.acknowledge("op-a1", &valid_ack_for(&source, "req-a1")),
        AdmissionResult::Accepted
    );
    assert_eq!(
        database
            .acknowledge("op-a1", &valid_ack_for(&source, "req-a1"))
            .unwrap(),
        AdmissionResult::Accepted
    );
    let replacement = replacement_entries();
    assert!(oracle.stage(
        "op-a1",
        revision,
        &replacement,
        ("journal-new", "cursor-new")
    ));
    assert!(database
        .stage_batch(
            "op-a1",
            revision,
            &replacement,
            ("journal-new", "cursor-new")
        )
        .unwrap());
    assert_eq!(oracle.projection, baseline);
    assert_eq!(database.snapshot("source-a"), baseline);
    assert_health_is_original(&database, "source-a");

    assert!(oracle.publish("op-a1", revision));
    database.publish("op-a1", revision, None).unwrap();
    assert_eq!(database.snapshot("source-a"), oracle.projection);
    let published = database.snapshot("source-a");
    assert_eq!(published.health.entry_count, 3);
    assert_eq!(
        published.health.journal_cursor.as_deref(),
        Some("cursor-new")
    );
    assert_eq!(published.old_needle_hits, BTreeSet::new());
    assert_eq!(
        published.new_needle_hits,
        BTreeSet::from(["entry-alpha".into()])
    );
    assert!(!published.entries["entry-alpha"].stale);
    assert!(published.entries["entry-beta"].stale);
    assert!(published.managed["managed-entry-alpha"].enabled);
    assert!(!published.managed["managed-entry-beta"].enabled);
    assert_eq!(
        published.analysis["entry-alpha"]
            .classification_json
            .as_deref(),
        Some(r#"{"class":"owner-corrected"}"#)
    );
    assert!(published.analysis["entry-alpha"].user_corrected);
    assert_eq!(database.operation("op-a1").1, "completed");
}

#[test]
fn service_busy_is_typed_rejection_without_health_or_index_side_effects() {
    let source = source_a();
    let baseline = Projection::baseline();
    let mut oracle = CorrectnessOracle::new(source, baseline.clone());
    let database = DatabaseFixture::new();
    let issued = oracle.issue_rebuild("op-busy", "req-busy");
    let revision = match issued {
        IssueResult::Created { revision, .. } => revision,
        other => panic!("expected new operation, got {other:?}"),
    };
    assert!(
        database
            .create_intent("source-a", "op-busy", "req-busy")
            .unwrap()
            == IssueResult::Created {
                operation_id: "op-busy".into(),
                revision
            }
    );
    assert!(oracle.claim("op-busy", revision));
    assert!(database.claim("op-busy", revision).unwrap());
    let response = BusyResponse {
        protocol_version: IPC_V3,
        request_id: "req-busy".into(),
        ok: false,
        error_code: Some("index_service_busy".into()),
    };
    assert_eq!(
        oracle.service_busy("op-busy", &response),
        AdmissionResult::RejectedBusy
    );
    assert_eq!(
        database.busy_response("op-busy", &response).unwrap(),
        AdmissionResult::RejectedBusy
    );
    assert_eq!(database.operation("op-busy").1, "queued");
    assert_eq!(source_revision(&database, "source-a"), revision);
    assert_eq!(database.snapshot("source-a"), baseline);
    assert_health_is_original(&database, "source-a");
    assert!(!direct_fallback_allowed(
        TransportKnowledge::ServiceBusy,
        DirectPolicy::AllowOnlyProvenNotSent
    ));
    assert_eq!(oracle.projection, baseline);
}

#[test]
fn malformed_or_mismatched_admission_frames_are_unknown_and_side_effect_free() {
    for mismatch in 0..9 {
        let source = source_a();
        let baseline = Projection::baseline();
        let mut oracle = CorrectnessOracle::new(source.clone(), baseline.clone());
        let database = DatabaseFixture::new();
        let revision = create_claimed_operation(&database, "op-invalid", "req-valid");
        assert!(matches!(
            oracle.issue_rebuild("op-invalid", "req-valid"),
            IssueResult::Created { revision: oracle_revision, .. } if oracle_revision == revision
        ));
        assert!(oracle.claim("op-invalid", revision));
        let mut ack = valid_ack_for(&source, "req-valid");
        match mismatch {
            0 => ack.request_id = "req-other".into(),
            1 => ack.protocol_version = IPC_V3 + 1,
            2 => ack.source_id = "source-other".into(),
            3 => ack.stable_volume_id = "volume-other".into(),
            4 => ack.mount_path = "E:/".into(),
            5 => ack.provider = "other-provider".into(),
            6 => ack.event_kind = "progress".into(),
            7 => ack.event_status = "ready".into(),
            8 => ack.event_ordinal = 1,
            _ => unreachable!(),
        }
        assert_eq!(
            oracle.acknowledge("op-invalid", &ack),
            AdmissionResult::Unknown,
            "oracle mismatch case {mismatch}"
        );
        assert_eq!(
            database.acknowledge("op-invalid", &ack).unwrap(),
            AdmissionResult::Unknown,
            "SQLite mismatch case {mismatch}"
        );
        assert_eq!(database.operation("op-invalid").1, "unknown");
        assert_eq!(database.snapshot("source-a"), baseline);
        assert_health_is_original(&database, "source-a");
        assert!(!direct_fallback_allowed(
            TransportKnowledge::UnknownAfterSend,
            DirectPolicy::AllowOnlyProvenNotSent
        ));
    }
}

#[test]
fn busy_frame_mismatch_is_unknown_and_direct_fallback_is_limited_to_proven_not_sent() {
    for mismatch in 0..4 {
        let database = DatabaseFixture::new();
        let baseline = database.snapshot("source-a");
        let revision = create_claimed_operation(&database, "op-busy-invalid", "req-busy-valid");
        let mut response = BusyResponse {
            protocol_version: IPC_V3,
            request_id: "req-busy-valid".into(),
            ok: false,
            error_code: Some("index_service_busy".into()),
        };
        match mismatch {
            0 => response.request_id = "req-wrong".into(),
            1 => response.protocol_version = IPC_V3 + 1,
            2 => response.error_code = Some("other_error".into()),
            3 => response.ok = true,
            _ => unreachable!(),
        }
        assert_eq!(
            database
                .busy_response("op-busy-invalid", &response)
                .unwrap(),
            AdmissionResult::Unknown
        );
        assert_eq!(database.operation("op-busy-invalid").1, "unknown");
        assert_eq!(database.operation("op-busy-invalid").0, revision);
        assert_eq!(database.snapshot("source-a"), baseline);
        assert_health_is_original(&database, "source-a");
        assert!(!direct_fallback_allowed(
            TransportKnowledge::UnknownAfterSend,
            DirectPolicy::AllowOnlyProvenNotSent
        ));
    }
    assert!(direct_fallback_allowed(
        TransportKnowledge::ProvenNotSent,
        DirectPolicy::AllowOnlyProvenNotSent
    ));
    assert!(!direct_fallback_allowed(
        TransportKnowledge::ServiceBusy,
        DirectPolicy::AllowOnlyProvenNotSent
    ));
    assert!(!direct_fallback_allowed(
        TransportKnowledge::UnknownAfterSend,
        DirectPolicy::AllowOnlyProvenNotSent
    ));
}

#[test]
fn duplicate_same_source_intents_coalesce_and_revision_replay_is_fenced() {
    let database = DatabaseFixture::new();
    let baseline = database.snapshot("source-a");
    let first = database
        .create_intent("source-a", "op-first", "req-first")
        .unwrap();
    assert_eq!(
        first,
        IssueResult::Created {
            operation_id: "op-first".into(),
            revision: 1
        }
    );
    assert_eq!(
        database
            .create_intent("source-a", "op-duplicate", "req-duplicate")
            .unwrap(),
        IssueResult::Coalesced {
            operation_id: "op-first".into(),
            revision: 1
        }
    );
    assert_eq!(operation_count(&database), 1);
    assert_eq!(source_revision(&database, "source-a"), 1);
    assert!(database.cancel("op-first", 1).unwrap());
    assert_eq!(database.stage_count("op-first"), 0);
    assert_eq!(
        database
            .create_intent("source-a", "op-second", "req-second")
            .unwrap(),
        IssueResult::Created {
            operation_id: "op-second".into(),
            revision: 2
        }
    );
    assert!(!database.claim("op-first", 1).unwrap());
    assert!(!database
        .stage_batch(
            "op-first",
            1,
            &replacement_entries(),
            ("old-journal", "old-cursor")
        )
        .unwrap());
    assert!(database.publish("op-first", 1, None).is_err());
    assert_eq!(database.snapshot("source-a"), baseline);
    assert_eq!(source_revision(&database, "source-a"), 2);
}

#[test]
fn full_rebuild_enumeration_skips_disabled_sources_and_targeted_rebuild_is_a_noop() {
    let database = DatabaseFixture::new();
    let disabled = SourceIdentity {
        enabled: false,
        ..source_b()
    };
    database.seed_source(&disabled);
    assert_eq!(database.enabled_source_ids(), vec!["source-a".to_string()]);

    let before = database.snapshot("source-a");
    let result = database
        .create_intent("source-b", "op-disabled", "req-disabled")
        .unwrap();
    assert_eq!(result, IssueResult::RejectedDisabled);
    assert_eq!(operation_count(&database), 0);
    assert_eq!(source_revision(&database, "source-b"), 0);
    assert_eq!(database.snapshot("source-a"), before);
    assert_health_is_original(&database, "source-a");
    assert_health_is_original(&database, "source-b");
}

#[test]
fn racing_same_source_requests_coalesce_while_different_sources_claim_independently() {
    let database = Arc::new(DatabaseFixture::new());
    database.seed_source(&source_b());
    let barrier = Arc::new(Barrier::new(3));
    let mut threads = Vec::new();
    for (source_id, operation_id, request_id) in [
        ("source-a", "op-race-a", "req-race-a"),
        ("source-b", "op-race-b", "req-race-b"),
    ] {
        let database = Arc::clone(&database);
        let barrier = Arc::clone(&barrier);
        threads.push(thread::spawn(move || {
            barrier.wait();
            database
                .create_intent(source_id, operation_id, request_id)
                .expect("concurrent independent-source intent")
        }));
    }
    barrier.wait();
    let outcomes: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert_eq!(outcomes.len(), 2);
    assert!(outcomes
        .iter()
        .all(|outcome| matches!(outcome, IssueResult::Created { revision: 1, .. })));
    assert!(database.claim("op-race-a", 1).unwrap());
    assert!(database.claim("op-race-b", 1).unwrap());
    assert_eq!(
        database
            .acknowledge("op-race-a", &valid_ack_for(&source_a(), "req-race-a"))
            .unwrap(),
        AdmissionResult::Accepted
    );
    assert_eq!(
        database
            .acknowledge("op-race-b", &valid_ack_for(&source_b(), "req-race-b"))
            .unwrap(),
        AdmissionResult::Accepted
    );
    assert_eq!(database.operation("op-race-a").1, "staging");
    assert_eq!(database.operation("op-race-b").1, "staging");
    assert_eq!(database.snapshot("source-a"), Projection::baseline());
    assert_health_is_original(&database, "source-a");
    assert_health_is_original(&database, "source-b");
}

#[test]
fn racing_duplicate_intents_for_one_source_create_only_one_revision() {
    let database = Arc::new(DatabaseFixture::new());
    let barrier = Arc::new(Barrier::new(3));
    let mut threads = Vec::new();
    for (operation_id, request_id) in [("op-race-left", "req-left"), ("op-race-right", "req-right")]
    {
        let database = Arc::clone(&database);
        let barrier = Arc::clone(&barrier);
        threads.push(thread::spawn(move || {
            barrier.wait();
            database
                .create_intent("source-a", operation_id, request_id)
                .expect("concurrent same-source intent")
        }));
    }
    barrier.wait();
    let outcomes: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, IssueResult::Created { .. }))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, IssueResult::Coalesced { .. }))
            .count(),
        1
    );
    assert_eq!(operation_count(&database), 1);
    assert_eq!(source_revision(&database, "source-a"), 1);
}

#[test]
fn source_disable_and_identity_change_fence_staged_publish() {
    for change in 0..2 {
        let database = DatabaseFixture::new();
        let revision = admit_operation(&database, "op-source-fence", "req-source-fence");
        stage_replacement(&database, "op-source-fence", revision);
        let before = database.snapshot("source-a");
        let connection = database.connect();
        if change == 0 {
            connection
                .execute(
                    "UPDATE global_volumes SET enabled=0 WHERE id='source-a'",
                    [],
                )
                .unwrap();
        } else {
            connection
                .execute(
                    "UPDATE global_volumes SET mount_path='Z:/' WHERE id='source-a'",
                    [],
                )
                .unwrap();
        }
        assert!(database.publish("op-source-fence", revision, None).is_err());
        assert_eq!(database.snapshot("source-a"), before);
        assert_eq!(database.operation("op-source-fence").1, "staging");
        assert!(database.stage_count("op-source-fence") > 0);
        assert_eq!(before.health.index_status, "ready");
        assert_eq!(before.health.entry_count, 3);
    }
}

#[test]
fn restart_marks_accepted_incomplete_operation_unknown_but_keeps_old_projection() {
    let source = source_a();
    let baseline = Projection::baseline();
    let mut oracle = CorrectnessOracle::new(source.clone(), baseline.clone());
    let database = DatabaseFixture::new();
    let revision = admit_operation(&database, "op-restart", "req-restart");
    assert_eq!(
        oracle.issue_rebuild("op-restart", "req-restart"),
        IssueResult::Created {
            operation_id: "op-restart".into(),
            revision
        }
    );
    assert!(oracle.claim("op-restart", revision));
    assert_eq!(
        oracle.acknowledge("op-restart", &valid_ack_for(&source, "req-restart")),
        AdmissionResult::Accepted
    );
    stage_replacement(&database, "op-restart", revision);
    assert!(oracle.stage(
        "op-restart",
        revision,
        &replacement_entries(),
        ("journal-new", "cursor-new")
    ));
    let before = database.snapshot("source-a");
    let staged_count = database.stage_count("op-restart");
    assert!(staged_count > 0);
    assert!(has_stage_checkpoint(&database, "op-restart"));

    database.mark_unknown_after_restart().unwrap();
    oracle.recover_after_process_restart();
    assert_eq!(database.operation("op-restart").1, "unknown");
    assert_eq!(database.snapshot("source-a"), before);
    assert_eq!(oracle.projection, baseline);
    assert_eq!(oracle.operations["op-restart"].phase, Phase::Unknown);
    assert_eq!(database.stage_count("op-restart"), staged_count);
    assert!(has_stage_checkpoint(&database, "op-restart"));
    assert!(database.publish("op-restart", revision, None).is_err());
    assert_eq!(
        database
            .create_intent("source-a", "op-after-restart", "req-after-restart")
            .unwrap(),
        IssueResult::Coalesced {
            operation_id: "op-restart".into(),
            revision
        }
    );
    assert!(!direct_fallback_allowed(
        TransportKnowledge::UnknownAfterSend,
        DirectPolicy::AllowOnlyProvenNotSent
    ));
}

#[test]
fn explicit_cancel_removes_stage_and_fences_publish_without_touching_active_data() {
    let source = source_a();
    let baseline = Projection::baseline();
    let mut oracle = CorrectnessOracle::new(source.clone(), baseline.clone());
    let database = DatabaseFixture::new();
    let revision = admit_operation(&database, "op-cancel", "req-cancel");
    assert_eq!(
        oracle.issue_rebuild("op-cancel", "req-cancel"),
        IssueResult::Created {
            operation_id: "op-cancel".into(),
            revision
        }
    );
    assert!(oracle.claim("op-cancel", revision));
    assert_eq!(
        oracle.acknowledge("op-cancel", &valid_ack_for(&source, "req-cancel")),
        AdmissionResult::Accepted
    );
    stage_replacement(&database, "op-cancel", revision);
    assert!(oracle.stage(
        "op-cancel",
        revision,
        &replacement_entries(),
        ("journal-new", "cursor-new")
    ));
    assert!(oracle.cancel("op-cancel"));
    assert!(database.cancel("op-cancel", revision).unwrap());
    assert_eq!(database.operation("op-cancel").1, "cancelled");
    assert_eq!(database.stage_count("op-cancel"), 0);
    assert!(!has_stage_checkpoint(&database, "op-cancel"));
    assert!(database.publish("op-cancel", revision, None).is_err());
    assert_eq!(database.snapshot("source-a"), baseline);
    assert_eq!(oracle.projection, baseline);
    assert_eq!(oracle.operations["op-cancel"].phase, Phase::Cancelled);
}

#[test]
fn transport_interruption_after_send_is_unknown_without_fallback_or_projection_changes() {
    let source = source_a();
    let baseline = Projection::baseline();
    let mut oracle = CorrectnessOracle::new(source, baseline.clone());
    let database = DatabaseFixture::new();
    let revision = create_claimed_operation(&database, "op-transport", "req-transport");
    assert_eq!(
        oracle.issue_rebuild("op-transport", "req-transport"),
        IssueResult::Created {
            operation_id: "op-transport".into(),
            revision
        }
    );
    assert!(oracle.claim("op-transport", revision));
    oracle.transport_unknown("op-transport");
    database
        .mark_unknown_after_transport("op-transport")
        .unwrap();
    assert_eq!(database.operation("op-transport").1, "unknown");
    assert_eq!(database.snapshot("source-a"), baseline);
    assert_eq!(oracle.projection, baseline);
    assert_health_is_original(&database, "source-a");
    assert!(!direct_fallback_allowed(
        TransportKnowledge::UnknownAfterSend,
        DirectPolicy::AllowOnlyProvenNotSent
    ));
    assert!(!direct_fallback_allowed(
        TransportKnowledge::ProvenNotSent,
        DirectPolicy::Disabled
    ));
}

#[test]
fn staging_write_failure_rolls_back_the_batch_without_touching_active_projection() {
    let database = DatabaseFixture::new();
    let baseline = database.snapshot("source-a");
    let revision = admit_operation(&database, "op-stage-error", "req-stage-error");
    database
        .connect()
        .execute_batch(
            "CREATE TRIGGER fail_stage_insert BEFORE INSERT ON d0_stage_entries
             WHEN new.entry_id='entry-gamma'
             BEGIN SELECT RAISE(ABORT, 'injected stage write failure'); END;",
        )
        .unwrap();
    let error = database
        .stage_batch(
            "op-stage-error",
            revision,
            &replacement_entries(),
            ("journal-new", "cursor-new"),
        )
        .expect_err("stage batch failure should abort transaction");
    assert!(error.contains("injected stage write failure"));
    assert_eq!(database.stage_count("op-stage-error"), 0);
    assert!(!has_stage_checkpoint(&database, "op-stage-error"));
    assert_eq!(database.operation("op-stage-error").1, "staging");
    assert_eq!(database.snapshot("source-a"), baseline);
    assert_health_is_original(&database, "source-a");
}

#[test]
fn every_publish_failpoint_rolls_back_entries_managed_ai_fts_count_and_checkpoint() {
    for failpoint in FAILPOINTS {
        let database = DatabaseFixture::new();
        let baseline = database.snapshot("source-a");
        let revision = admit_operation(&database, "op-failpoint", "req-failpoint");
        stage_replacement(&database, "op-failpoint", revision);
        let error = database
            .publish("op-failpoint", revision, Some(failpoint))
            .expect_err("injected publish failure must abort transaction");
        assert!(error.message.contains("test injected failure"));
        assert_eq!(database.snapshot("source-a"), baseline, "at {failpoint:?}");
        assert_eq!(database.operation("op-failpoint").1, "staging");
        assert_eq!(database.stage_count("op-failpoint"), 3);
        assert!(has_stage_checkpoint(&database, "op-failpoint"));
    }
}

#[test]
fn sqlite_write_error_rolls_back_partial_publish_and_retains_stage_for_recovery() {
    let database = DatabaseFixture::new();
    let baseline = database.snapshot("source-a");
    let revision = admit_operation(&database, "op-write-error", "req-write-error");
    stage_replacement(&database, "op-write-error", revision);
    database
        .connect()
        .execute_batch(
            "CREATE TRIGGER fail_analysis_publish BEFORE UPDATE ON ai_analysis_state
             BEGIN SELECT RAISE(ABORT, 'injected ai state write failure'); END;",
        )
        .unwrap();
    let error = database
        .publish("op-write-error", revision, None)
        .expect_err("real SQLite write failure should abort publish");
    assert!(error.message.contains("injected ai state write failure"));
    assert_eq!(database.snapshot("source-a"), baseline);
    assert_eq!(database.operation("op-write-error").1, "staging");
    assert_eq!(database.stage_count("op-write-error"), 3);
    assert!(has_stage_checkpoint(&database, "op-write-error"));
}

#[test]
fn process_exit_during_publish_transaction_recovers_the_old_active_projection() {
    const CHILD_DB_PATH: &str = "ZEN_CANVAS_380_D0_CRASH_DB";
    let database = DatabaseFixture::new();
    let revision = admit_operation(&database, "op-process-crash", "req-process-crash");
    stage_replacement(&database, "op-process-crash", revision);
    let before = database.snapshot("source-a");
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("child_exit_during_publish_transaction")
        .arg("--nocapture")
        .env(CHILD_DB_PATH, database.path())
        .status()
        .expect("launch test child for crash recovery");
    assert_eq!(child.code(), Some(73));
    database.mark_unknown_after_restart().unwrap();
    assert_eq!(database.snapshot("source-a"), before);
    assert_eq!(database.operation("op-process-crash").1, "unknown");
    assert_eq!(database.stage_count("op-process-crash"), 3);
    assert!(has_stage_checkpoint(&database, "op-process-crash"));
}

#[test]
fn child_exit_during_publish_transaction() {
    let Ok(path) = std::env::var("ZEN_CANVAS_380_D0_CRASH_DB") else {
        return;
    };
    let mut connection = Connection::open(path).expect("child opens isolated database");
    connection
        .execute_batch(
            "PRAGMA foreign_keys=ON;
             PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA busy_timeout=10000;",
        )
        .unwrap();
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    transaction
        .execute(
            "UPDATE global_entries SET name='newneedle-alpha.txt',
                 path='C:/managed/newneedle-alpha.txt',is_stale=1 WHERE id='entry-alpha'",
            [],
        )
        .unwrap();
    transaction
        .execute("UPDATE managed_entries SET enabled=0", [])
        .unwrap();
    transaction
        .execute("UPDATE ai_analysis_state SET status='stale'", [])
        .unwrap();
    transaction
        .execute(
            "UPDATE global_volumes SET index_status='error',entry_count=0 WHERE id='source-a'",
            [],
        )
        .unwrap();
    std::process::exit(73);
}

#[test]
fn bounded_1k_and_10k_publish_reports_transaction_time_wal_growth_and_rollback() {
    for count in [1_000_usize, 10_000] {
        let database = DatabaseFixture::new();
        let sqlite_version = database
            .connect()
            .query_row("SELECT sqlite_version()", [], |row| row.get::<_, String>(0))
            .expect("read bundled SQLite version");
        let operation_id = format!("op-bench-{count}");
        let request_id = format!("req-bench-{count}");
        let revision = admit_operation(&database, &operation_id, &request_id);
        let entries: Vec<_> = (0..count)
            .map(|index| Entry {
                id: format!("synthetic-{index:05}"),
                volume_id: "source-a".into(),
                name: format!("synthetic-name-{index:05}.txt"),
                path: format!("C:/synthetic/bounded/{index:05}.txt"),
                extension: "txt".into(),
                fingerprint: format!("synthetic-fingerprint-{index:05}"),
                stale: false,
            })
            .collect();
        database.checkpoint_wal();
        let _wal_keeper = database.connect();
        let wal_before_stage = database.wal_bytes();
        let stage_started = Instant::now();
        assert!(database
            .stage_batch(
                &operation_id,
                revision,
                &entries,
                ("journal-synthetic", "cursor-synthetic")
            )
            .unwrap());
        let stage_duration = stage_started.elapsed();
        let wal_after_stage = database.wal_bytes();
        assert!(
            wal_after_stage > wal_before_stage,
            "{count} staged rows should grow the WAL"
        );
        let before_rollback = database.snapshot("source-a");
        let rollback = database
            .publish(&operation_id, revision, Some(PublishStep::AfterEntryMerge))
            .expect_err("bounded publish rollback failpoint");
        let wal_after_rollback = database.wal_bytes();
        assert_eq!(database.snapshot("source-a"), before_rollback);
        assert_eq!(database.stage_count(&operation_id), count as i64);
        let publish_duration = database
            .publish(&operation_id, revision, None)
            .expect("bounded publish success");
        let wal_after_publish = database.wal_bytes();
        let published = database.snapshot("source-a");
        assert_eq!(published.health.entry_count, count as i64);
        assert_eq!(published.entries.len(), count + 3);
        assert_eq!(published.new_needle_hits, BTreeSet::new());
        assert_eq!(published.old_needle_hits, BTreeSet::new());

        const MAX_BOUNDED_TRANSACTION: Duration = Duration::from_secs(30);
        const MAX_WAL_BYTES: u64 = 128 * 1024 * 1024;
        assert!(
            stage_duration < MAX_BOUNDED_TRANSACTION,
            "{count} stage took {stage_duration:?}"
        );
        assert!(
            rollback.transaction_duration < MAX_BOUNDED_TRANSACTION,
            "{count} rollback took {:?}",
            rollback.transaction_duration
        );
        assert!(
            publish_duration < MAX_BOUNDED_TRANSACTION,
            "{count} publish took {publish_duration:?}"
        );
        let wal_growth = wal_after_publish.saturating_sub(wal_before_stage);
        assert!(
            wal_growth <= MAX_WAL_BYTES,
            "{count} WAL grew to {wal_growth} bytes"
        );
        println!(
            "D0_SQLITE_METRIC sqlite_version={sqlite_version} rows={count} stage_ms={} rollback_ms={} publish_ms={} wal_before={} wal_after_stage={} wal_after_rollback={} wal_after_publish={} wal_growth={} rollback_error={}",
            stage_duration.as_millis(),
            rollback.transaction_duration.as_millis(),
            publish_duration.as_millis(),
            wal_before_stage,
            wal_after_stage,
            wal_after_rollback,
            wal_after_publish,
            wal_growth,
            rollback.message.replace(' ', "_")
        );
    }
}
