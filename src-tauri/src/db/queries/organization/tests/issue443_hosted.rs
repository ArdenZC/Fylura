//! One fixed-sample, Windows-only research probe; not a production optimization.
//! Invoked only by the existing ignored Intelligence benchmark, before its
//! unchanged 100/1k/10k workload and 500 ms assertion.

use super::*;
use serde::Serialize;
use std::collections::BTreeMap;

const ITEMS: usize = 10_000;
const SAMPLES: usize = 5;
const ARMS: [&str; 3] = ["product", "sidecar_per_row", "sidecar_reuse"];

#[derive(Clone, Serialize)]
struct Span {
    wall_ms: f64,
    cpu_ms: Option<f64>,
}

struct Timer {
    wall: Instant,
    cpu: Option<OrganizationProcessSample>,
}

impl Timer {
    fn start() -> Self {
        let cpu = organization_process_sample();
        Self {
            wall: Instant::now(),
            cpu,
        }
    }

    fn finish(self) -> Span {
        let wall_ms = self.wall.elapsed().as_secs_f64() * 1_000.0;
        Span {
            wall_ms,
            cpu_ms: self
                .cpu
                .zip(organization_process_sample())
                .map(|(a, b)| b.cpu_time_100ns.saturating_sub(a.cpu_time_100ns) as f64 / 10_000.0),
        }
    }
}

#[derive(Default, Serialize)]
struct Metrics {
    phases: BTreeMap<&'static str, Span>,
    prepare_count: usize,
    prepare_wall_ms: f64,
    execute_count: usize,
    execute_wall_ms: f64,
    successful_cas: usize,
}

impl Metrics {
    fn checkpoint(&mut self, name: &'static str, timer: &mut Timer) {
        let previous = std::mem::replace(timer, Timer::start());
        self.phases.insert(name, previous.finish());
    }
}

#[derive(Serialize)]
struct Record {
    sample: usize,
    arm: &'static str,
    fixture: String,
    rows: usize,
    total: Span,
    exceeds_500ms: bool,
    metrics: Option<Metrics>,
    checksum: String,
}

fn seed(db: &Database) {
    let mut conn = db.conn().expect("research seed connection");
    let tx = conn.transaction().expect("research seed transaction");
    tx.execute(
        "INSERT INTO organization_plans (
            id, title, status, source_kind, source_snapshot_revision,
            requested_count, materialized_count, planner_version, revision,
            created_at, updated_at, ready_at
         ) VALUES ('plan-bench', 'Benchmark', 'ready', 'explicit', 1, 10000, 10000, 1, 1, 1, 1, 1)",
        [],
    )
    .expect("seed original 10k plan");
    tx.execute(
        "INSERT INTO scan_roots (
            id, normalized_path, display_name, source_kind, enabled,
            health_status, current_generation, needs_reconciliation, created_at, updated_at
         ) VALUES ('organization-benchmark-root', '/missing', 'Organization benchmark',
                   'file_library', 1, 'healthy', 1, 0, 1, 1)",
        [],
    )
    .expect("seed original managed root");
    {
        let mut insert = tx
            .prepare(
                "INSERT INTO organization_plan_items (
                id, plan_id, ordinal, file_id_snapshot, source_path_snapshot,
                source_name_snapshot, source_size_snapshot, source_mtime_snapshot,
                source_is_dir_snapshot, proposal_fingerprint, proposal_kind,
                proposed_target_directory, proposed_name, proposed_target_path,
                decision, validity, confidence, risk_level, requires_confirmation,
                authoritative_preview_id, revision, created_at, updated_at
             ) VALUES (?1, 'plan-bench', ?2, ?3, ?4, ?5, 1, 1, 0, ?6,
                       'rename', '/missing', ?7, ?8, 'undecided', 'ready',
                       0.95, 'Normal', 0, ?9, 1, 1, 1)",
            )
            .expect("prepare original fixture insert");
        for ordinal in 0..ITEMS {
            insert
                .execute(params![
                    format!("bench-item-{ordinal:05}"),
                    ordinal as i64,
                    format!("bench-file-{ordinal:05}"),
                    format!("/missing/source-{ordinal:05}.txt"),
                    format!("source-{ordinal:05}.txt"),
                    format!("fingerprint-{ordinal:05}"),
                    format!("renamed-{ordinal:05}.txt"),
                    format!("/missing/renamed-{ordinal:05}.txt"),
                    format!("preview-{ordinal:05}"),
                ])
                .expect("seed each of 10000 original ledger rows");
        }
    }
    tx.commit().expect("publish research fixture");
}

fn mutation_rows(db: &Database) -> Vec<(String, i64)> {
    let conn = db.conn().expect("research mutation rows");
    let mut statement = conn.prepare(
        "SELECT id, revision FROM organization_plan_items WHERE plan_id = 'plan-bench' ORDER BY ordinal",
    ).expect("prepare original mutation rows");
    let rows = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("read mutation revisions")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect exactly 10000 mutations");
    assert_eq!(rows.len(), ITEMS);
    rows
}

fn digest(conn: &rusqlite::Connection, immutable_only: bool) -> String {
    let mut statement = conn.prepare(
        "SELECT * FROM organization_plan_items WHERE plan_id = 'plan-bench' ORDER BY ordinal, id",
    ).expect("prepare full-ledger correctness snapshot");
    let indices = statement
        .column_names()
        .iter()
        .enumerate()
        .filter_map(|(i, name)| {
            (!(*name == "updated_at"
                || (immutable_only && matches!(*name, "decision" | "revision"))))
            .then_some(i)
        })
        .collect::<Vec<_>>();
    let mut rows = statement.query([]).expect("snapshot rows");
    let mut hash = blake3::Hasher::new();
    let mut count = 0;
    while let Some(row) = rows.next().expect("snapshot row") {
        for &index in &indices {
            let encoded = format!("{:?}", row.get_ref(index).expect("snapshot value"));
            hash.update(&(encoded.len() as u64).to_le_bytes());
            hash.update(encoded.as_bytes());
        }
        count += 1;
    }
    assert_eq!(count, ITEMS);
    hash.finalize().to_hex().to_string()
}

fn schema_digest(conn: &rusqlite::Connection) -> Vec<(String, String, String, Option<String>)> {
    conn.prepare("SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY type,name")
        .expect("schema snapshot")
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .expect("schema rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("complete schema snapshot")
}

fn invoke(
    db: &Database,
    arm: &str,
    request: UpdateOrganizationPlanDecisionsRequest,
    metrics: &mut Metrics,
) -> Result<OrganizationPlanDto, DbError> {
    match arm {
        "product" => db.update_organization_plan_decisions(request),
        "sidecar_per_row" => sidecar(db, request, false, metrics),
        "sidecar_reuse" => sidecar(db, request, true, metrics),
        _ => unreachable!("fixed research arm"),
    }
}

fn observation(
    sample: usize,
    arm: &'static str,
    timed: bool,
    fixtures: &mut HashSet<std::path::PathBuf>,
) -> Option<Record> {
    let (db, path) = test_database();
    assert!(
        fixtures.insert(path.clone()),
        "fresh fixture required, including warmup"
    );
    seed(&db);
    let before = digest(&db.conn().expect("initial snapshot"), true);
    let schema = schema_digest(&db.conn().expect("initial schema"));
    let rows = mutation_rows(&db);
    let sqlite = {
        let conn = db.conn().expect("research SQLite context");
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            42
        );
        assert_eq!(
            conn.query_row("PRAGMA busy_timeout", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            5000
        );
        serde_json::json!({"version": rusqlite::version(), "journal_mode": conn.query_row("PRAGMA journal_mode", [], |r| r.get::<_,String>(0)).unwrap(), "synchronous": conn.query_row("PRAGMA synchronous", [], |r| r.get::<_,i64>(0)).unwrap(), "files": conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get::<_,i64>(0)).unwrap()})
    };
    assert_eq!(sqlite["files"], 0);
    let started_at = current_unix_seconds();
    let mut metrics = Metrics::default();
    let timer = timed.then(Timer::start);
    // Match the original benchmark: row loading is outside, request/decision
    // String construction and the complete product API call are inside timing.
    let request = UpdateOrganizationPlanDecisionsRequest {
        plan_id: "plan-bench".into(),
        expected_plan_revision: 1,
        safe_batch: false,
        mutations: rows
            .into_iter()
            .map(|(item_id, revision)| OrganizationDecisionMutation {
                item_id,
                expected_item_revision: revision,
                decision: "kept".into(),
                edited_filename: None,
            })
            .collect(),
    };
    let result = invoke(&db, arm, request, &mut metrics);
    let total = timer.map(Timer::finish);
    let finished_at = current_unix_seconds();
    if let Err(error) = &result {
        println!(
            "[issue443-error] {}",
            serde_json::json!({"sample":sample,"arm":arm,"fixture":path,"error":error.to_string(),"metrics":metrics})
        );
    }
    let plan = result.expect("all business guards must succeed for original fixture");
    assert_eq!(
        (plan.revision, plan.summary.kept, plan.summary.undecided),
        (2, 10000, 0)
    );
    assert_eq!(
        (
            plan.requested_count,
            plan.materialized_count,
            plan.status.as_str()
        ),
        (10000, 10000, "ready")
    );
    let checksum = {
        let conn = db.conn().expect("result verification");
        assert!(conn.is_autocommit());
        assert_eq!(schema_digest(&conn), schema);
        assert_eq!(digest(&conn, true), before);
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            42
        );
        assert_eq!(
            conn.query_row("PRAGMA busy_timeout", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            5000
        );
        let verified: i64 = conn.query_row(
            "SELECT COUNT(*) FROM organization_plan_items WHERE decision='kept' AND revision=2 AND validity='ready' AND updated_at BETWEEN ?1 AND ?2",
            params![started_at,finished_at], |r| r.get(0),
        ).expect("verify all 10000 CAS results and timestamps");
        assert_eq!(verified, 10000);
        digest(&conn, false)
    };
    if arm != "product" {
        assert_eq!(metrics.successful_cas, ITEMS);
        assert_eq!(metrics.execute_count, ITEMS);
        assert_eq!(
            metrics.prepare_count,
            if arm == "sidecar_reuse" { 1 } else { ITEMS }
        );
    }
    // Outside timing: preserve plan-version rejection and partial-loop rollback.
    let mut bad = UpdateOrganizationPlanDecisionsRequest {
        plan_id: "plan-bench".into(),
        expected_plan_revision: 1,
        safe_batch: false,
        mutations: vec![OrganizationDecisionMutation {
            item_id: "bench-item-00000".into(),
            expected_item_revision: 2,
            decision: "undecided".into(),
            edited_filename: None,
        }],
    };
    let mut ignored_metrics = Metrics::default();
    let error = invoke(&db, arm, bad.clone(), &mut ignored_metrics).unwrap_err();
    assert!(error
        .to_string()
        .contains("organization_plan_revision_conflict"));
    bad.expected_plan_revision = 2;
    bad.mutations.push(OrganizationDecisionMutation {
        item_id: "bench-item-00001".into(),
        expected_item_revision: 999,
        decision: "undecided".into(),
        edited_filename: None,
    });
    let error = invoke(&db, arm, bad, &mut ignored_metrics).unwrap_err();
    assert!(error
        .to_string()
        .contains("organization_item_revision_conflict"));
    {
        let conn = db.conn().expect("rollback verification");
        assert!(conn.is_autocommit());
        assert_eq!(digest(&conn, false), checksum);
        assert_eq!(load_plan(&conn, "plan-bench").unwrap().revision, 2);
        assert_eq!(schema_digest(&conn), schema);
    }
    let record = total.map(|total| Record {
        sample,
        arm,
        fixture: path.display().to_string(),
        rows: ITEMS,
        exceeds_500ms: total.wall_ms > 500.0,
        total,
        metrics: (arm != "product").then_some(metrics),
        checksum: checksum.clone(),
    });
    println!(
        "[issue443-correctness] {}",
        serde_json::json!({"sample":sample,"arm":arm,"fixture":path,"sqlite":sqlite,"kept":10000,"plan_revision":2,"all_item_revisions":2,"immutable_ledger_unchanged":true,"schema_unchanged":true,"plan_cas_rejected":true,"partial_cas_rollback":true,"checksum":checksum,"untimed_warmup":!timed})
    );
    if let Some(record) = &record {
        println!(
            "[issue443-sample] {}",
            serde_json::to_string(record).unwrap()
        );
    }
    drop(db);
    let _ = std::fs::remove_file(path);
    record
}

fn distribution(values: &[f64]) -> serde_json::Value {
    assert!(!values.is_empty());
    serde_json::json!({"n":values.len(),"min":values.iter().copied().reduce(f64::min).unwrap(),"p50":organization_profile_quantile(values,0.5),"p95":organization_profile_quantile(values,0.95),"max":values.iter().copied().reduce(f64::max).unwrap(),"quantile":"linear interpolation at (n-1)*q"})
}

pub(super) fn run() {
    let source = include_str!("../mod.rs");
    let method = source
        .split("pub fn update_organization_plan_decisions(")
        .nth(1)
        .unwrap()
        .split("pub fn refresh_organization_plan(")
        .next()
        .unwrap();
    assert!(
        method.replace("\r\n", "\n").contains(ITEM_CAS_SQL),
        "sidecar must retain exact product SQL"
    );
    let system = sysinfo::System::new_all();
    println!(
        "[issue443-plan] {}",
        serde_json::json!({"warmups":1,"formal_groups":SAMPLES,"arms":ARMS,"fixtures_per_group":3,"rows":ITEMS,"original_gate_ms":500,"original_gate_runs_after_probe":true,"pid":std::process::id(),"runner":std::env::var("RUNNER_NAME").ok(),"runner_os":std::env::var("RUNNER_OS").ok(),"runner_arch":std::env::var("RUNNER_ARCH").ok(),"run_id":std::env::var("GITHUB_RUN_ID").ok(),"attempt":std::env::var("GITHUB_RUN_ATTEMPT").ok(),"source":std::env::var("PERF_CHECKOUT_SHA").ok(),"image":std::env::var("ImageVersion").ok(),"cpu":system.cpus().first().map(|c|c.brand()),"cpu_mhz":system.cpus().first().map(|c|c.frequency()),"logical_cpus":system.cpus().len(),"available_parallelism":std::thread::available_parallelism().ok().map(usize::from),"total_memory":system.total_memory(),"available_memory":system.available_memory(),"cpu_clock":"GetProcessTimes; process-wide, coarse OS resolution","primary_internal_phases":null,"busy_callbacks":null,"lock_wait_ms":null,"new_connections_during_decision":null,"unmeasurable_reason":"production untouched; Cargo trace/hooks disabled; original busy_timeout=5000 preserved; execute wall includes unseparated wait","source_derived_opens_during_decision":0,"source_derived_migrations_during_decision":0,"sidecar":"full product path; prepare/execute clock overhead retained; no SQL microbenchmark"})
    );
    let mut fixtures = HashSet::new();
    assert!(observation(0, "product", false, &mut fixtures).is_none());
    let mut records = Vec::with_capacity(SAMPLES * ARMS.len());
    let mut expected_checksum = None;
    for sample in 1..=SAMPLES {
        for offset in 0..ARMS.len() {
            let arm = ARMS[(offset + sample - 1) % ARMS.len()];
            let record = observation(sample, arm, true, &mut fixtures).unwrap();
            if let Some(expected) = &expected_checksum {
                assert_eq!(&record.checksum, expected);
            } else {
                expected_checksum = Some(record.checksum.clone());
            }
            records.push(record);
        }
    }
    assert_eq!(fixtures.len(), 1 + SAMPLES * ARMS.len());
    for arm in ARMS {
        let rows = records.iter().filter(|r| r.arm == arm).collect::<Vec<_>>();
        assert_eq!(rows.len(), SAMPLES);
        let wall = rows.iter().map(|r| r.total.wall_ms).collect::<Vec<_>>();
        let cpu = rows
            .iter()
            .filter_map(|r| r.total.cpu_ms)
            .collect::<Vec<_>>();
        let mut phases = BTreeMap::new();
        if let Some(metrics) = &rows[0].metrics {
            for name in metrics.phases.keys() {
                let times = rows
                    .iter()
                    .map(|r| r.metrics.as_ref().unwrap().phases[name].wall_ms)
                    .collect::<Vec<_>>();
                phases.insert(*name, distribution(&times));
            }
            for (name, values) in [
                (
                    "sql_prepare",
                    rows.iter()
                        .map(|r| r.metrics.as_ref().unwrap().prepare_wall_ms)
                        .collect::<Vec<_>>(),
                ),
                (
                    "sql_execute",
                    rows.iter()
                        .map(|r| r.metrics.as_ref().unwrap().execute_wall_ms)
                        .collect::<Vec<_>>(),
                ),
            ] {
                phases.insert(name, distribution(&values));
            }
        }
        println!(
            "[issue443-summary] {}",
            serde_json::json!({"arm":arm,"wall_ms":distribution(&wall),"cpu_ms":(!cpu.is_empty()).then(||distribution(&cpu)),"over_500ms":rows.iter().filter(|r|r.exceeds_500ms).count(),"phases_wall_ms":phases,"all_checksums_equal":true})
        );
    }
}

// The complete product method follows, with test-only timing and the equivalent
// explicit prepare+execute split. No business branch, SQL, parameter or CAS
// condition is simplified. The optional reusable statement is test-only.

const ITEM_CAS_SQL: &str = "UPDATE organization_plan_items SET decision = ?1, edited_name = ?2,
                        validity = CASE WHEN ?3 THEN 'needs_review' ELSE validity END,
                        blocking_code = CASE WHEN ?3 THEN NULL ELSE blocking_code END,
                        revision = revision + 1, updated_at = ?4
                 WHERE id = ?5 AND plan_id = ?6 AND revision = ?7
                   AND validity NOT IN ('executing', 'executed')";

fn sidecar(
    db: &Database,
    request: UpdateOrganizationPlanDecisionsRequest,
    reuse: bool,
    metrics: &mut Metrics,
) -> Result<OrganizationPlanDto, DbError> {
    let mut phase = Timer::start();
    if request.mutations.is_empty() || request.mutations.len() > ORGANIZATION_PLAN_MAX_ITEMS {
        return Err(DbError::Validation(
            "organization_decision_batch_invalid".to_string(),
        ));
    }
    let plan_id = validate_id(&request.plan_id, "organization_plan_id_invalid")?;
    let mut unique = HashSet::new();
    if request
        .mutations
        .iter()
        .any(|mutation| !unique.insert(mutation.item_id.as_str()))
    {
        return Err(DbError::Validation(
            "organization_decision_duplicate_item".to_string(),
        ));
    }
    metrics.checkpoint("request_validation", &mut phase);
    let mut conn = db.conn()?;
    metrics.checkpoint("pool_checkout", &mut phase);
    let tx = conn.transaction()?;
    metrics.checkpoint("transaction_begin", &mut phase);
    require_plan_revision_and_status(
        &tx,
        &plan_id,
        request.expected_plan_revision,
        &["ready", "stale", "partially_completed"],
    )?;
    metrics.checkpoint("plan_guard", &mut phase);
    let projected_items = if request.mutations.len() >= 32 {
        Some(
            load_organization_plan_items_for_projection(&tx, &plan_id)?
                .into_iter()
                .map(|item| (item.id.clone(), item))
                .collect::<HashMap<_, _>>(),
        )
    } else {
        None
    };
    metrics.checkpoint("projection", &mut phase);
    let now = current_unix_seconds();
    let mut reusable = if reuse {
        let started = Instant::now();
        let statement = tx.prepare(ITEM_CAS_SQL)?;
        metrics.prepare_wall_ms += started.elapsed().as_secs_f64() * 1_000.0;
        metrics.prepare_count += 1;
        Some(statement)
    } else {
        None
    };
    metrics.checkpoint("prepare_once", &mut phase);
    for mutation in request.mutations {
        let item_id = validate_id(&mutation.item_id, "organization_item_id_invalid")?;
        let decision = normalize_decision(&mutation.decision)?;
        let current_item = match projected_items.as_ref() {
            Some(items) => items
                .get(&item_id)
                .cloned()
                .ok_or_else(|| DbError::Validation("organization_item_not_found".to_string()))?,
            None => load_organization_plan_item_for_projection(&tx, &plan_id, &item_id)?,
        };
        if current_item.revision != mutation.expected_item_revision {
            return Err(DbError::Validation(
                "organization_item_revision_conflict".to_string(),
            ));
        }
        match decision {
            "accepted" => {
                if request.safe_batch {
                    require_safe_batch_item_projection(&current_item)?;
                } else if !current_item
                    .available_actions
                    .iter()
                    .any(|action| action == "accept_suggestion")
                {
                    return Err(DbError::Validation(
                        organization_item_action_error("accept_suggestion").to_string(),
                    ));
                }
            }
            "edited" => {
                if !current_item
                    .available_actions
                    .iter()
                    .any(|action| action == "edit_name")
                {
                    return Err(DbError::Validation(
                        organization_item_action_error("edit_name").to_string(),
                    ));
                }
            }
            "kept" => {
                if !current_item
                    .available_actions
                    .iter()
                    .any(|action| action == "keep")
                {
                    return Err(DbError::Validation(
                        organization_item_action_error("keep").to_string(),
                    ));
                }
            }
            "undecided" if current_item.decision != "undecided" => {
                if !current_item
                    .available_actions
                    .iter()
                    .any(|action| action == "clear_decision")
                {
                    return Err(DbError::Validation(
                        organization_item_action_error("clear_decision").to_string(),
                    ));
                }
            }
            "undecided" => {}
            _ => unreachable!("normalize_decision returns only durable decisions"),
        }
        if request.safe_batch && decision != "accepted" {
            return Err(DbError::Validation(
                "organization_safe_batch_accept_only".to_string(),
            ));
        }
        let edited_name = if decision == "edited" {
            Some(validate_edited_filename(
                &tx,
                &plan_id,
                &item_id,
                mutation.edited_filename.as_deref(),
            )?)
        } else {
            None
        };
        let resolves_target_collision = decision == "edited"
            && matches!(
                current_item.blocking_code.as_deref(),
                Some("target_collision" | "organization_target_collision")
            );
        let updated = if let Some(statement) = reusable.as_mut() {
            let started = Instant::now();
            let result = statement.execute(params![
                decision,
                edited_name,
                bool_to_i64(resolves_target_collision),
                now,
                item_id,
                plan_id,
                mutation.expected_item_revision
            ]);
            metrics.execute_wall_ms += started.elapsed().as_secs_f64() * 1_000.0;
            metrics.execute_count += 1;
            result?
        } else {
            let started = Instant::now();
            let mut statement = tx.prepare(ITEM_CAS_SQL)?;
            metrics.prepare_wall_ms += started.elapsed().as_secs_f64() * 1_000.0;
            metrics.prepare_count += 1;
            let started = Instant::now();
            let result = statement.execute(params![
                decision,
                edited_name,
                bool_to_i64(resolves_target_collision),
                now,
                item_id,
                plan_id,
                mutation.expected_item_revision
            ]);
            metrics.execute_wall_ms += started.elapsed().as_secs_f64() * 1_000.0;
            metrics.execute_count += 1;
            result?
        };
        if updated != 1 {
            return Err(DbError::Validation(
                "organization_item_revision_conflict".to_string(),
            ));
        }
        metrics.successful_cas += 1;
    }
    metrics.checkpoint("mutation_loop", &mut phase);
    drop(reusable);
    metrics.checkpoint("statement_finalize", &mut phase);
    tx.execute(
        "UPDATE organization_plans SET
                    status = CASE
                        WHEN status = 'stale'
                         AND NOT EXISTS (
                            SELECT 1 FROM organization_plan_items
                            WHERE plan_id = ?1 AND validity = 'stale'
                         )
                        THEN 'ready' ELSE status END,
                    revision = revision + 1, updated_at = ?3
             WHERE id = ?1 AND revision = ?2",
        params![plan_id, request.expected_plan_revision, now],
    )?;
    metrics.checkpoint("plan_cas", &mut phase);
    let plan = load_plan(&tx, &plan_id)?;
    metrics.checkpoint("plan_and_summary", &mut phase);
    tx.commit()?;
    metrics.checkpoint("commit", &mut phase);
    Ok(plan)
}
