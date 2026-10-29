use super::*;
use rusqlite::{params_from_iter, OpenFlags};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(super) struct Table {
    pub name: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Cell>>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(super) struct Snapshot {
    pub tables: Vec<Table>,
    pub effective_rows: u64,
    pub query_revision: i64,
}

const ROOTS: &str = "SELECT id FROM scan_roots WHERE source_kind='file_library'";
const ENABLED: &str = "SELECT id FROM scan_roots WHERE source_kind='file_library' AND enabled=1";
const UIDS: &str = "SELECT source_uid FROM library_source_root_claims WHERE root_id IN (SELECT id FROM scan_roots WHERE source_kind='file_library' AND enabled=1)";

pub(super) fn read_only(path: &Path) -> Result<Connection, DbError> {
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(unavailable("source_symlink"));
    }
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.execute_batch(
        "PRAGMA query_only=ON; PRAGMA temp_store=MEMORY; PRAGMA cache_size=-256;
                        PRAGMA mmap_size=0; PRAGMA trusted_schema=OFF;",
    )?;
    conn.busy_timeout(Duration::ZERO)?;
    // Per-connection bounds; never change SQLite global heap limits.
    unsafe {
        rusqlite::ffi::sqlite3_limit(
            conn.handle(),
            rusqlite::ffi::SQLITE_LIMIT_LENGTH,
            256 * 1024,
        );
    }

    Ok(conn)
}

/// A separate READ_ONLY connection cannot accidentally initialize/migrate the main DB.
pub(super) fn capture(path: &Path, guard: &Guard) -> Result<Snapshot, DbError> {
    let conn = read_only(path)?;
    let mut reading = guard.clone();
    reading.read_start = Some(Instant::now());
    reading.check()?;
    let _progress = Progress::install(&conn, &reading);
    conn.execute_batch("BEGIN DEFERRED")?;
    let result = capture_tx(&conn, &mut reading);
    conn.execute_batch("ROLLBACK")?;
    result
}

fn capture_tx(conn: &Connection, guard: &mut Guard) -> Result<Snapshot, DbError> {
    match conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? {
        41 => {}
        // Schema42 adds dormant Global Index objects, Schema43 search indexes,
        // Schema44 source facts, and Schema45 opt-in state. File Library authority is unchanged.
        // Validate the full dormant signature without migrating/writing source.
        42..=45 => crate::db::schema_global_generations::validate(conn)?,
        _ => return Err(unavailable("source_schema_unsupported")),
    }
    let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(unavailable("source_journal_unsupported"));
    }
    let rows = validate_coverage(conn)?;
    let revision = super::super::library::current_library_revision(conn)?;
    let scope = super::super::library::resolve_scope(
        conn,
        &super::super::library::FileLibraryScopeV2::AllEnabledRoots,
    )?;
    let visible = format!(
        "SELECT f.id FROM files f WHERE f.is_stale=0 AND ({})",
        scope.clause
    );
    let mut specs = vec![
        ("scan_roots", format!("SELECT * FROM scan_roots WHERE id IN ({ROOTS}) ORDER BY id"), vec![]),
        ("scan_runs", format!("SELECT * FROM scan_runs WHERE scan_root_id IN ({ROOTS}) ORDER BY id"), vec![]),
        ("scan_seen", format!("SELECT * FROM scan_seen WHERE run_id IN (SELECT r.id FROM scan_runs r JOIN scan_roots s ON s.id=r.scan_root_id AND s.last_successful_generation=r.generation WHERE s.id IN ({ENABLED})) ORDER BY run_id,file_id"), vec![]),
        ("scan_sessions", "SELECT * FROM scan_sessions ORDER BY id".into(), vec![]),
        ("scan_session_roots", "SELECT * FROM scan_session_roots ORDER BY session_id,requested_index".into(), vec![]),
        ("library_root_epoch_state", format!("SELECT * FROM library_root_epoch_state WHERE root_id IN ({ENABLED}) ORDER BY root_id"), vec![]),
        ("library_source_root_claims", format!("SELECT * FROM library_source_root_claims WHERE root_id IN ({ENABLED}) ORDER BY root_id,source_uid,observed_path"), vec![]),
        ("library_source_incarnations", format!("SELECT * FROM library_source_incarnations WHERE source_uid IN ({UIDS}) ORDER BY source_uid"), vec![]),
        ("library_source_observations", format!("SELECT * FROM library_source_observations WHERE source_uid IN ({UIDS}) ORDER BY source_uid,observation_revision"), vec![]),
        ("library_legacy_attributions", format!("SELECT * FROM library_legacy_attributions WHERE legacy_file_id IN ({visible}) ORDER BY legacy_file_id,decision_revision,claim_id"), scope.params.clone()),
        ("files", format!("SELECT f.rowid AS rowid,f.* FROM files f WHERE f.id IN ({visible}) ORDER BY f.id"), scope.params.clone()),
        ("user_tags", "SELECT * FROM user_tags ORDER BY id".into(), vec![]),
        ("file_user_tags", format!("SELECT * FROM file_user_tags WHERE file_id IN ({visible}) ORDER BY file_id,tag_id"), scope.params.clone()),
        ("active_duplicate_membership", format!("SELECT DISTINCT file_id FROM active_duplicate_membership WHERE file_id IN ({visible}) ORDER BY file_id"), scope.params),
        ("library_query_state", "SELECT * FROM library_query_state ORDER BY singleton_id".into(), vec![]),
        ("library_generation_authority", "SELECT * FROM library_generation_authority ORDER BY singleton_id".into(), vec![]),
    ];
    let mut tables = Vec::new();
    for (name, sql, params) in specs.drain(..) {
        guard.check()?;
        let mut statement = conn.prepare(&sql)?;
        let columns = statement
            .column_names()
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        let mut table = Table {
            name: name.into(),
            columns,
            rows: vec![],
        };
        let mut result = statement.query(params_from_iter(params.iter()))?;
        while let Some(row) = result.next()? {
            let cells = (0..table.columns.len())
                .map(|i| cell(row, i))
                .collect::<Result<Vec<_>, _>>()?;
            // Include Vec/String capacities, serialization and query result headroom.
            let bytes = serde_json::to_vec(&cells)?.len() as u64;
            guard.charged = guard
                .charged
                .saturating_add(bytes.saturating_mul(8).saturating_add(512));
            if guard.charged > guard.budget.staging_bytes {
                return Err(unavailable("staging_budget"));
            }
            guard.check()?;
            table.rows.push(cells);
        }
        tables.push(table);
    }
    guard.check()?;
    Ok(Snapshot {
        tables,
        effective_rows: rows,
        query_revision: revision,
    })
}

fn count(conn: &Connection, sql: &str) -> Result<i64, DbError> {
    Ok(conn.query_row(sql, [], |r| r.get(0))?)
}

/// Narrow, source-backed proof: every enabled root has a completed full scan,
/// with no later writer history. Epochs may differ across batches of that scan.
/// Batch claim.owner_revision is the fenced root revision, NOT the scan
/// generation. Completed reconciliation's root-epoch owner revision IS the
/// generation. Bind claims to the completed run's same-transaction scan_seen
/// observations, time window and root revision ceiling, plus exact source proof.
/// Other legitimate histories are unavailable, never inferred complete.
pub(super) fn validate_coverage(conn: &Connection) -> Result<u64, DbError> {
    let sentinel: (String,i64,Option<String>,i64) = conn.query_row(
        "SELECT read_mode,active_catalog_epoch,active_fts_slot_id,revision FROM library_generation_authority WHERE singleton_id=1", [],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
    if sentinel != ("legacy".into(), 0, None, 1) {
        return Err(unavailable("authority_changed"));
    }
    if count(conn, &format!("SELECT COUNT(*) FROM scan_roots s WHERE s.id IN ({ENABLED}) AND (
        s.health_status!='healthy' OR s.needs_reconciliation!=0 OR s.watcher_rule_recovery_required!=0
        OR s.watcher_revision!=s.watcher_applied_revision OR s.active_run_id IS NOT NULL
        OR s.active_generation IS NOT NULL OR s.last_successful_generation IS NULL OR s.last_full_scan_at IS NULL
        OR NOT EXISTS (SELECT 1 FROM scan_runs r WHERE r.scan_root_id=s.id AND r.generation=s.last_successful_generation
            AND r.status='completed' AND r.phase='completed' AND r.coverage_complete=1
            AND r.stale_reconciliation_allowed=1 AND r.cancel_requested=0 AND r.coverage_error_count=0
            AND r.watcher_revision_at_start=s.watcher_revision AND r.finished_at IS NOT NULL)
        OR EXISTS (SELECT 1 FROM scan_runs r WHERE r.scan_root_id=s.id AND (r.status IN ('queued','running','cancelling') OR r.generation>s.last_successful_generation))
        OR NOT EXISTS (SELECT 1 FROM library_root_epoch_state e WHERE e.root_id=s.id AND e.epoch>0
            AND e.last_owner_kind='scan' AND e.last_owner_revision=s.last_successful_generation))"))? != 0 {
        return Err(unavailable("coverage_unproven: root_state"));
    }
    let claims = format!(
        "FROM library_source_root_claims c JOIN scan_roots s ON s.id=c.root_id
        WHERE s.id IN ({ENABLED}) AND c.claim_state!='absent'"
    );
    if count(conn, &format!("SELECT COUNT(*) {claims} AND (
        c.claim_state!='present' OR c.owner_kind!='scan' OR c.owner_revision<1 OR c.owner_revision>s.revision
        OR c.root_epoch<1 OR c.root_epoch>(SELECT epoch FROM library_root_epoch_state WHERE root_id=c.root_id)
        OR NOT EXISTS (SELECT 1 FROM scan_runs r JOIN scan_seen seen ON seen.run_id=r.id
            WHERE r.scan_root_id=c.root_id AND r.generation=s.last_successful_generation AND seen.observed_path=c.observed_path AND seen.observed_at=c.observed_at
              AND r.started_at IS NOT NULL AND c.observed_at>=r.started_at AND c.observed_at<=r.finished_at)
        OR NOT EXISTS (SELECT 1 FROM library_source_incarnations i JOIN library_source_observations o
            ON o.source_uid=i.source_uid AND o.observation_revision=c.observation_revision
            WHERE i.source_uid=c.source_uid AND i.verification_state='verified'
              AND i.physical_platform IS NOT NULL AND i.physical_platform!=''
              AND i.physical_volume_id IS NOT NULL AND i.physical_volume_id!=''
              AND i.physical_object_id IS NOT NULL AND i.physical_object_id!=''
              AND i.verification_digest IS NOT NULL AND i.verification_digest!=''
              AND o.physical_claim_digest=i.verification_digest AND o.observed_path=c.observed_path))"))? != 0 {
        return Err(unavailable("coverage_unproven: claim_binding"));
    }
    if count(conn, &format!("SELECT COUNT(*) FROM (SELECT c.source_uid {claims}
        GROUP BY c.source_uid HAVING COUNT(DISTINCT c.observed_path)>1 OR COUNT(DISTINCT c.observation_revision)>1)"))? != 0
        || count(conn, &format!("SELECT COUNT(*) FROM (SELECT c.observed_path {claims} GROUP BY c.observed_path HAVING COUNT(DISTINCT c.source_uid)>1)"))? != 0 {
        return Err(unavailable("ambiguous_projection"));
    }
    let scope = super::super::library::resolve_scope(
        conn,
        &super::super::library::FileLibraryScopeV2::AllEnabledRoots,
    )?;
    if !matches!(scope.health.state.as_str(), "healthy" | "empty") {
        return Err(unavailable("coverage_unproven: scope_health"));
    }
    let mut stmt = conn.prepare(&format!(
        "SELECT f.id,f.path,f.name,f.extension,f.size,f.mtime,f.ctime,f.is_dir FROM files f
        WHERE f.is_stale=0 AND ({}) ORDER BY f.id",
        scope.clause
    ))?;
    let mut visible = stmt.query(params_from_iter(scope.params.iter()))?;
    let mut effective = 0u64;
    while let Some(row) = visible.next()? {
        let id: String = row.get(0)?;
        let path: String = row.get(1)?;
        let matching: i64 = conn.query_row(&format!("SELECT COUNT(DISTINCT c.source_uid) FROM library_source_root_claims c
            JOIN library_source_observations o ON o.source_uid=c.source_uid AND o.observation_revision=c.observation_revision
            JOIN library_legacy_attributions a ON a.legacy_file_id=?1 AND a.source_uid=c.source_uid
            JOIN library_source_incarnations i ON i.source_uid=c.source_uid
            WHERE c.root_id IN ({ENABLED}) AND c.claim_state='present' AND c.observed_path=?2
              AND o.name=?3 AND o.extension=?4 AND o.size=?5 AND o.mtime=?6 AND o.ctime=?7 AND o.is_dir=?8
              AND a.claim_state='verified' AND a.evidence_digest IS NOT NULL AND a.evidence_digest!=''
              AND a.decision_revision=(SELECT MAX(a2.decision_revision) FROM library_legacy_attributions a2 WHERE a2.legacy_file_id=a.legacy_file_id)
              AND (SELECT COUNT(*) FROM library_legacy_attributions a2 WHERE a2.legacy_file_id=a.legacy_file_id AND a2.decision_revision=a.decision_revision)=1"),
            rusqlite::params![id,path,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,i64>(4)?,row.get::<_,i64>(5)?,row.get::<_,i64>(6)?,row.get::<_,i64>(7)?], |r|r.get(0))?;
        if matching != 1 {
            return Err(unavailable("coverage_unproven: visible_projection"));
        }
        let (uid,prior_path,proof,evidence):(String,String,String,String)=conn.query_row(
            "SELECT a.source_uid,a.legacy_path_snapshot,i.verification_digest,a.evidence_digest FROM library_legacy_attributions a JOIN library_source_incarnations i ON i.source_uid=a.source_uid WHERE a.legacy_file_id=?1 ORDER BY a.decision_revision DESC LIMIT 1",[&id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
        let expected = blake3::hash(format!("{uid}\0{prior_path}\0{proof}").as_bytes())
            .to_hex()
            .to_string();
        if evidence != expected {
            return Err(unavailable("coverage_unproven: attribution_digest"));
        }
        let observed_digest:String=conn.query_row(
            "SELECT o.observation_digest FROM library_source_root_claims c JOIN library_source_observations o ON o.source_uid=c.source_uid AND o.observation_revision=c.observation_revision WHERE c.source_uid=?1 AND c.observed_path=?2 AND c.claim_state='present' LIMIT 1",rusqlite::params![uid,path],|r|r.get(0))?;
        let expected_observation = digest(&(
            &path,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, i64>(5)?,
            row.get::<_, i64>(6)?,
            row.get::<_, i64>(7)? != 0,
            "verified",
            Some(proof.as_str()),
        ))?;
        if expected_observation != observed_digest {
            return Err(unavailable("coverage_unproven: observation_digest"));
        }
        effective += 1;
    }
    // Reverse coverage checks the exact projection, not a count of a filtered subset.
    if count(
        conn,
        &format!(
            "SELECT COUNT(*) {claims} AND NOT EXISTS (
        SELECT 1 FROM files f WHERE f.path=c.observed_path AND f.is_stale=0)"
        ),
    )? != 0
    {
        return Err(unavailable("coverage_unproven: reverse_projection"));
    }
    // Every latest-scan observation must still be accounted for by a present claim.
    if count(
        conn,
        &format!(
            "SELECT COUNT(*) FROM scan_seen seen JOIN scan_runs r ON r.id=seen.run_id
        JOIN scan_roots s ON s.id=r.scan_root_id AND s.last_successful_generation=r.generation
        WHERE s.id IN ({ENABLED}) AND NOT EXISTS (SELECT 1 FROM library_source_root_claims c
          WHERE c.root_id=s.id AND c.observed_path=seen.observed_path AND c.claim_state='present')"
        ),
    )? != 0
    {
        return Err(unavailable("coverage_unproven: scan_seen"));
    }
    Ok(effective)
}

/// Full re-capture also re-proves coverage. A query revision alone is insufficient.
pub(crate) fn qualify_current(
    candidate: &Candidate,
    reader: SnapshotReader<'_>,
    budget: BuildBudget,
    cancellation: CancellationToken,
) -> Result<(), DbError> {
    let _single = admit_single()?;
    let _lease = reader
        .scheduler
        .try_acquire(
            WorkRequest::new(
                format!("shadow-current-{}", candidate.id),
                WorkClass::Background,
                ResourceHints::cpu_io(1, 1),
            )
            .with_cancellation(cancellation.clone()),
        )
        .map_err(|_| unavailable("resource_deferred"))?;
    let _qos = crate::resource_governor::scope_thread_qos(WorkClass::Background);
    let guard = Guard::new(
        candidate
            .directory
            .parent()
            .ok_or_else(|| unavailable("workspace"))?,
        reader.database,
        budget,
        cancellation,
    );
    let current = capture(reader.database, &guard)?;
    if digest(&current)? != candidate.source_digest {
        return Err(unavailable("stale"));
    }
    Ok(())
}
