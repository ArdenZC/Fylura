//! Bounded, writer-only native qualification for Issue #494.
//!
//! This deliberately separates the production SQLite WAL autocheckpoint path
//! from the historical forced-FULL-checkpoint stress measurement. Each arm
//! owns an identical on-disk clone; the candidate arm installs the same
//! connection-local filename mutation hooks used by the production pool.

use super::super::{
    benchmark_context, emit_record, populate_synthetic_database, sidecar_path, test_db_path,
    test_volume, BenchmarkCleanup,
};
use crate::db::Database;
use blake3::Hasher;
use rusqlite::Connection;
use serde_json::{json, Value as JsonValue};
use std::env;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::time::Instant;

const WRITER_FIXTURE_ENTRIES: u64 = 200_000;
const WRITER_WARMUPS: usize = 5;
const WRITER_SAMPLES: usize = 100;
const WRITER_BATCHES: [usize; 2] = [512, 5_000];
const WRITER_P95_OVERHEAD_LIMIT_PERCENT: f64 = 10.0;
const BOOTSTRAP_REPLICATES: usize = 2_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct WalState {
    busy: i64,
    log_frames: i64,
    checkpointed_frames: i64,
}

#[derive(Clone, Copy, Debug)]
struct CheckpointResult {
    busy: i64,
    log_frames: i64,
    checkpointed_frames: i64,
}

#[derive(Clone, Copy)]
struct WriterTiming {
    total_ms: f64,
    update_ms: f64,
    commit_ms: f64,
    full_checkpoint_ms: Option<f64>,
    wal_before: WalState,
    wal_after: WalState,
    full_checkpoint: Option<CheckpointResult>,
}

struct WriterCase {
    baseline: Vec<WriterTiming>,
    candidate: Vec<WriterTiming>,
    raw_pairs: Vec<JsonValue>,
    pair_orders: Vec<&'static str>,
}

#[test]
#[ignore = "Issue #494 controlled native writer-only qualification; exact 200k Release fixture"]
fn issue494_writer_only_native_qualification() {
    assert!(
        !std::hint::black_box(cfg!(debug_assertions)),
        "writer qualification must run in Release mode"
    );
    if cfg!(target_os = "windows") {
        assert_eq!(
            std::hint::black_box(std::env::consts::ARCH),
            "x86_64",
            "Windows qualification requires native x86_64"
        );
    } else if cfg!(target_os = "macos") {
        assert_eq!(
            std::hint::black_box(std::env::consts::ARCH),
            "aarch64",
            "macOS qualification requires native Apple Silicon"
        );
    } else {
        panic!("Issue #494 writer qualification requires native Windows or macOS");
    }

    let output = env::var("ZC_GLOBAL_SEARCH_BENCHMARK_OUTPUT").ok();
    if let Some(path) = &output {
        let output_path = std::path::PathBuf::from(path);
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).expect("create writer evidence directory");
        }
        fs::write(output_path, "").expect("start fresh writer JSONL evidence");
    }

    let master_path = test_db_path();
    let baseline_path = sidecar_path(&master_path, ".writer-baseline.sqlite3");
    let candidate_path = sidecar_path(&master_path, ".writer-candidate.sqlite3");
    let _master_cleanup = BenchmarkCleanup(master_path.clone());
    let _baseline_cleanup = BenchmarkCleanup(baseline_path.clone());
    let _candidate_cleanup = BenchmarkCleanup(candidate_path.clone());

    let context = benchmark_context(WRITER_FIXTURE_ENTRIES);
    let fixture_started = Instant::now();
    let db = Database::open(&master_path).expect("open production-schema writer fixture");
    db.upsert_global_volume(&test_volume())
        .expect("insert authorized synthetic source");
    let (expectations, generation_ms, population_ms, population_batch_ms) =
        populate_synthetic_database(&db, WRITER_FIXTURE_ENTRIES);
    drop(expectations);
    let fixture_connection = db.conn().expect("get fixture checkpoint connection");
    let source_rows: i64 = fixture_connection
        .query_row("SELECT COUNT(*) FROM global_entries", [], |row| row.get(0))
        .expect("verify production fixture row count");
    assert_eq!(source_rows, WRITER_FIXTURE_ENTRIES as i64);
    let production_trigger_count: i64 = fixture_connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type='trigger' AND tbl_name='global_entries'",
            [],
            |row| row.get(0),
        )
        .expect("verify production Global Entries triggers");
    assert!(
        production_trigger_count > 0,
        "fixture must retain production triggers"
    );
    let source_checkpoint = checkpoint(&fixture_connection, "TRUNCATE");
    assert_eq!(source_checkpoint.busy, 0, "source fixture WAL must be idle");
    drop(fixture_connection);
    drop(db);

    let source_digest = blake3_file(&master_path);
    fs::copy(&master_path, &baseline_path).expect("clone baseline writer database");
    fs::copy(&master_path, &candidate_path).expect("clone candidate writer database");
    let baseline_digest = blake3_file(&baseline_path);
    let candidate_digest = blake3_file(&candidate_path);
    assert_eq!(
        source_digest, baseline_digest,
        "baseline clone must be byte-identical"
    );
    assert_eq!(
        source_digest, candidate_digest,
        "candidate clone must be byte-identical"
    );

    // Open both arms through the same production r2d2_sqlite pool builder and
    // configure_connection hook used by the installed application. The test
    // seam disables mutation hooks only for the baseline pool.
    let baseline_db = Database::open_for_writer_qualification(&baseline_path, false)
        .expect("open baseline through production SQLite pool");
    let candidate_db = Database::open_for_writer_qualification(&candidate_path, true)
        .expect("open candidate through production SQLite pool with hooks");
    let process_before_index = super::native_process_sample();
    candidate_db
        .filename_search
        .rebuild_now_for_test()
        .expect("build candidate projection on its cloned fixture");
    let process_after_index = super::native_process_sample();
    let index_diagnostics = candidate_db.filename_search.diagnostics();
    assert!(
        index_diagnostics.ready,
        "candidate projection must be ready"
    );
    assert_eq!(
        index_diagnostics.index_entries,
        WRITER_FIXTURE_ENTRIES as usize
    );

    let mut baseline = baseline_db.conn().expect("get baseline pool connection");
    let mut candidate = candidate_db.conn().expect("get candidate pool connection");

    let baseline_contract = sqlite_writer_contract(&baseline);
    let candidate_contract = sqlite_writer_contract(&candidate);
    assert_eq!(baseline_contract, candidate_contract);
    assert_eq!(baseline_contract.0, "wal", "fixture must use WAL mode");
    assert_eq!(
        baseline_contract.1, 1,
        "production policy uses synchronous=NORMAL"
    );
    assert_eq!(
        baseline_contract.2, 1_000,
        "SQLite production default autocheckpoint must be 1,000 pages"
    );
    assert_database_pair_equivalent(&baseline, &candidate, &baseline_path, &candidate_path);
    let fixture_setup_ms = fixture_started.elapsed().as_secs_f64() * 1000.0;

    let mut cases = Vec::with_capacity(4);
    for (batch_index, batch_size) in WRITER_BATCHES.into_iter().enumerate() {
        assert_database_pair_equivalent(&baseline, &candidate, &baseline_path, &candidate_path);
        let measured = measure_writer_case(
            &mut baseline,
            &mut candidate,
            batch_size,
            batch_index,
            false,
        );
        assert_database_pair_equivalent(&baseline, &candidate, &baseline_path, &candidate_path);
        cases.push(writer_case_record(
            "production_wal_autocheckpoint",
            batch_size,
            measured,
        ));
    }

    // This reset is harness setup between two independently labeled policies.
    // It is reported and excluded from both measured latency distributions.
    let transition_started = Instant::now();
    let baseline_reset = checkpoint(&baseline, "TRUNCATE");
    let candidate_reset = checkpoint(&candidate, "TRUNCATE");
    assert_eq!(baseline_reset.busy, 0);
    assert_eq!(candidate_reset.busy, 0);
    baseline
        .pragma_update(None, "wal_autocheckpoint", 0)
        .expect("disable automatic checkpoints for stress arm");
    candidate
        .pragma_update(None, "wal_autocheckpoint", 0)
        .expect("disable automatic checkpoints for candidate stress arm");
    assert_eq!(
        baseline
            .query_row("PRAGMA wal_autocheckpoint", [], |row| row.get::<_, i64>(0))
            .expect("read baseline stress checkpoint policy"),
        0
    );
    assert_eq!(
        candidate
            .query_row("PRAGMA wal_autocheckpoint", [], |row| row.get::<_, i64>(0))
            .expect("read candidate stress checkpoint policy"),
        0
    );
    assert_database_pair_equivalent(&baseline, &candidate, &baseline_path, &candidate_path);
    let policy_transition_ms = transition_started.elapsed().as_secs_f64() * 1000.0;

    for (batch_index, batch_size) in WRITER_BATCHES.into_iter().enumerate() {
        assert_database_pair_equivalent(&baseline, &candidate, &baseline_path, &candidate_path);
        let measured =
            measure_writer_case(&mut baseline, &mut candidate, batch_size, batch_index, true);
        assert_database_pair_equivalent(&baseline, &candidate, &baseline_path, &candidate_path);
        cases.push(writer_case_record(
            "forced_full_checkpoint_stress",
            batch_size,
            measured,
        ));
    }

    let production_cases = cases
        .iter()
        .filter(|case| case["policy"] == "production_wal_autocheckpoint")
        .collect::<Vec<_>>();
    let production_no_go = production_cases
        .iter()
        .any(|case| case["production_writer_classification"].as_str() == Some("WRITER_NO_GO"));
    let production_inconclusive = production_cases.iter().any(|case| {
        case["production_writer_classification"].as_str() == Some("POINT_PASS_INCONCLUSIVE")
    });
    let production_pass_with_margin = !production_cases.is_empty()
        && production_cases.iter().all(|case| {
            case["production_writer_classification"].as_str()
                == Some("PRODUCTION_WRITER_PASS_WITH_MARGIN")
        });
    let qualification = if production_no_go {
        "WRITER_NO_GO"
    } else if production_inconclusive || !production_pass_with_margin {
        "POINT_PASS_INCONCLUSIVE"
    } else {
        "PRODUCTION_WRITER_PASS_WITH_MARGIN"
    };
    let forced_full_over_limit_count = cases
        .iter()
        .filter(|case| case["policy"] == "forced_full_checkpoint_stress")
        .filter(|case| {
            case["write_budget"]["p95_overhead_percent"]
                .as_f64()
                .is_some_and(|value| value > WRITER_P95_OVERHEAD_LIMIT_PERCENT)
        });
    let forced_full_over_limit_count = forced_full_over_limit_count.count();

    emit_record(&json!({
        "schema_version": 1,
        "record_type": "issue494_writer_only_native_qualification",
        "context": context,
        "qualification_status": qualification,
        "million_row_qualification_authorized": false,
        "fixture": {
            "entry_count": WRITER_FIXTURE_ENTRIES,
            "origin": "existing production Global Index synthetic fixture generator",
            "source_database_blake3": source_digest,
            "baseline_clone_blake3": baseline_digest,
            "candidate_clone_blake3": candidate_digest,
            "byte_identical_clones_verified": true,
            "source_rows": source_rows,
            "production_global_entries_trigger_count": production_trigger_count,
            "dataset_generation_ms": generation_ms,
            "sqlite_population_ms": population_ms,
            "population_transaction_ms": super::super::summarize_samples(&population_batch_ms),
            "fixture_and_index_setup_ms": fixture_setup_ms,
            "baseline_and_candidate_page_count_and_row_aggregates_verified_equal": true
        },
        "connection_contract": {
            "journal_mode": baseline_contract.0,
            "synchronous": "NORMAL",
            "wal_autocheckpoint_pages_production": baseline_contract.2,
            "foreign_keys_enabled": true,
            "pool": "production Database pool: r2d2_sqlite, max_size=8, min_idle=1, production configure_connection initialization",
            "candidate_only_difference": "candidate pool installs the same connection-local update, commit, and rollback hooks used by the production pool; baseline pool omits only those candidate hooks"
        },
        "candidate_memory": {
            "index_build_ms": index_diagnostics.last_build_micros as f64 / 1_000.0,
            "estimated_retained_index_bytes": index_diagnostics.index_private_bytes,
            "estimated_index_build_peak_bytes": index_diagnostics.index_build_peak_bytes,
            "process_before_index_build": super::native_sample_json(process_before_index),
            "process_after_index_build": super::native_sample_json(process_after_index),
            "private_bytes_delta": super::sample_private_delta(process_before_index, process_after_index),
            "working_set_delta": super::sample_rss_delta(process_before_index, process_after_index)
        },
        "warmups_per_arm_and_case": WRITER_WARMUPS,
        "paired_samples_per_arm_and_case": WRITER_SAMPLES,
        "counterbalanced_order": "writer order alternates each pair and is shifted for the second batch size; no outlier is dropped",
        "batch_sizes": WRITER_BATCHES,
        "update": "deterministic identical updates to the same rowids in both clones: modified_at_fs += 1 and last_seen_at += 1 in one transaction",
        "p95_write_budget_percent": WRITER_P95_OVERHEAD_LIMIT_PERCENT,
        "bootstrap": {
            "replicates": BOOTSTRAP_REPLICATES,
            "confidence": 0.95,
            "method": "paired sample-index bootstrap percentile interval for each candidate/baseline metric p95 ratio",
            "fixed_seed_strategy": "xorshift64*; batch_size XOR 0x49494a11, reported with each interval"
        },
        "policies": cases,
        "forced_full_stress_diagnostic": {
            "status": "REPORTED_ONLY_NOT_A_PRODUCTION_WRITE_GATE",
            "groups_over_10_percent": forced_full_over_limit_count,
            "historical_1m_macos_5000_row_plus_12_638_percent_failure_remains_valid": true
        },
        "policy_transition_setup": {
            "wal_autocheckpoint_disabled_only_for_forced_full_stress": true,
            "baseline_truncate_status": checkpoint_json(baseline_reset),
            "candidate_truncate_status": checkpoint_json(candidate_reset),
            "elapsed_ms_excluded_from_sample_distributions": policy_transition_ms
        },
        "interpretation": {
            "production_complete_write_latency": "update and commit; SQLite's normal synchronous autocheckpoint work, whenever it occurs, remains included in commit and total latency",
            "automatic_checkpoint_ms": "not isolated because the benchmark preserves SQLite's production autocheckpoint callback; before/after WAL frame counts are recorded per pair",
            "forced_full_complete_write_latency": "update + commit + explicit FULL checkpoint, preserving the historical end-to-end write metric separately",
            "no_threshold_waiver": true,
            "one_requalification_only": true
        }
    }));
}

fn measure_writer_case(
    baseline: &mut Connection,
    candidate: &mut Connection,
    batch_size: usize,
    batch_index: usize,
    force_full_checkpoint: bool,
) -> WriterCase {
    let mut baseline_samples = Vec::with_capacity(WRITER_SAMPLES);
    let mut candidate_samples = Vec::with_capacity(WRITER_SAMPLES);
    let mut raw_pairs = Vec::with_capacity(WRITER_SAMPLES);
    let mut pair_orders = Vec::with_capacity(WRITER_SAMPLES);

    for iteration in 0..(WRITER_WARMUPS + WRITER_SAMPLES) {
        assert_wal_states_equal(
            read_wal_state(baseline),
            read_wal_state(candidate),
            "before paired batch",
        );
        let candidate_first = (iteration + batch_index).is_multiple_of(2);
        let first = (iteration * batch_size) % (WRITER_FIXTURE_ENTRIES as usize - batch_size);
        let mut baseline_timing = None;
        let mut candidate_timing = None;
        for position in 0..2 {
            let is_candidate = (position == 0) == candidate_first;
            let timing = if is_candidate {
                timed_writer_batch(candidate, first, batch_size, force_full_checkpoint)
            } else {
                timed_writer_batch(baseline, first, batch_size, force_full_checkpoint)
            };
            if is_candidate {
                candidate_timing = Some(timing);
            } else {
                baseline_timing = Some(timing);
            }
        }
        let baseline_timing = baseline_timing.expect("baseline pair timing");
        let candidate_timing = candidate_timing.expect("candidate pair timing");
        assert_eq!(
            baseline_timing.wal_before, candidate_timing.wal_before,
            "paired clones must start each write with equivalent WAL state"
        );
        assert_eq!(
            baseline_timing.wal_after, candidate_timing.wal_after,
            "paired clones must finish each write with equivalent WAL/checkpoint work"
        );
        if force_full_checkpoint {
            for timing in [baseline_timing, candidate_timing] {
                let full = timing
                    .full_checkpoint
                    .expect("FULL stress checkpoint result");
                assert_eq!(full.busy, 0, "FULL checkpoint must not be blocked");
                assert_eq!(
                    full.log_frames, full.checkpointed_frames,
                    "FULL checkpoint must drain the WAL"
                );
            }
        } else {
            assert!(
                baseline_timing.full_checkpoint.is_none()
                    && candidate_timing.full_checkpoint.is_none(),
                "production WAL policy must not add a forced checkpoint"
            );
        }

        if iteration >= WRITER_WARMUPS {
            let sample_index = iteration - WRITER_WARMUPS;
            let order = if candidate_first {
                "candidate_first"
            } else {
                "baseline_first"
            };
            raw_pairs.push(json!({
                "pair_index": sample_index,
                "rowid_start": first,
                "writer_order": order,
                "baseline": writer_timing_json(baseline_timing, !force_full_checkpoint),
                "candidate": writer_timing_json(candidate_timing, !force_full_checkpoint),
                "candidate_minus_baseline_ms": {
                    "total": candidate_timing.total_ms - baseline_timing.total_ms,
                    "update": candidate_timing.update_ms - baseline_timing.update_ms,
                    "commit": candidate_timing.commit_ms - baseline_timing.commit_ms,
                    "full_checkpoint": match (candidate_timing.full_checkpoint_ms, baseline_timing.full_checkpoint_ms) {
                        (Some(candidate), Some(baseline)) => Some(candidate - baseline),
                        _ => None
                    }
                }
            }));
            baseline_samples.push(baseline_timing);
            candidate_samples.push(candidate_timing);
            pair_orders.push(order);
        }
    }

    WriterCase {
        baseline: baseline_samples,
        candidate: candidate_samples,
        raw_pairs,
        pair_orders,
    }
}

fn timed_writer_batch(
    connection: &mut Connection,
    rowid_start: usize,
    batch_size: usize,
    force_full_checkpoint: bool,
) -> WriterTiming {
    assert!(rowid_start + batch_size <= WRITER_FIXTURE_ENTRIES as usize);
    let wal_before = read_wal_state(connection);
    let started = Instant::now();
    let transaction = connection.transaction().expect("begin writer batch");
    let update_started = Instant::now();
    {
        let mut statement = transaction
            .prepare(
                "UPDATE global_entries
                 SET modified_at_fs=COALESCE(modified_at_fs,0)+1,
                     last_seen_at=last_seen_at+1
                 WHERE id=?1",
            )
            .expect("prepare deterministic writer update");
        for index in rowid_start..(rowid_start + batch_size) {
            let id = format!("ge_bench_{index:07}");
            assert_eq!(
                statement.execute([id]).expect("update one fixture row"),
                1,
                "every paired ID must exist"
            );
        }
    }
    let update_ms = update_started.elapsed().as_secs_f64() * 1000.0;
    let commit_started = Instant::now();
    transaction
        .commit()
        .expect("commit deterministic writer batch");
    let commit_ms = commit_started.elapsed().as_secs_f64() * 1000.0;

    let (full_checkpoint_ms, full_checkpoint) = if force_full_checkpoint {
        let checkpoint_started = Instant::now();
        let result = checkpoint(connection, "FULL");
        let elapsed = checkpoint_started.elapsed().as_secs_f64() * 1000.0;
        (Some(elapsed), Some(result))
    } else {
        (None, None)
    };
    let total_ms = started.elapsed().as_secs_f64() * 1000.0;
    let wal_after = read_wal_state(connection);
    WriterTiming {
        total_ms,
        update_ms,
        commit_ms,
        full_checkpoint_ms,
        wal_before,
        wal_after,
        full_checkpoint,
    }
}

fn writer_case_record(policy: &'static str, batch_size: usize, case: WriterCase) -> JsonValue {
    let metric = |pick: fn(WriterTiming) -> f64| {
        let baseline = case.baseline.iter().copied().map(pick).collect::<Vec<_>>();
        let candidate = case.candidate.iter().copied().map(pick).collect::<Vec<_>>();
        let baseline_summary = super::super::summarize_samples(&baseline);
        let candidate_summary = super::super::summarize_samples(&candidate);
        let baseline_p95 = baseline_summary["p95"].as_f64().expect("baseline p95");
        let candidate_p95 = candidate_summary["p95"].as_f64().expect("candidate p95");
        let overhead = overhead_percent(baseline_p95, candidate_p95);
        json!({
            "baseline_ms": baseline_summary,
            "candidate_ms": candidate_summary,
            "candidate_p95_over_baseline_percent": overhead,
            "paired_metric_p95_ratio_bootstrap_ci95_percent": paired_p95_ratio_ci95(&baseline, &candidate, batch_size as u64 ^ 0x4949_4a11),
            "paired_delta_ms": super::super::summarize_samples(
                &case.baseline.iter().zip(&case.candidate).map(|(base, candidate)| pick(*candidate) - pick(*base)).collect::<Vec<_>>()
            )
        })
    };

    let total = metric(|sample| sample.total_ms);
    let update = metric(|sample| sample.update_ms);
    let commit = metric(|sample| sample.commit_ms);
    let checkpoint = case
        .baseline
        .first()
        .and_then(|sample| sample.full_checkpoint_ms)
        .map(|_| metric(|sample| sample.full_checkpoint_ms.expect("stress checkpoint timing")));
    let total_overhead = total["candidate_p95_over_baseline_percent"]
        .as_f64()
        .expect("total p95 overhead");
    let bootstrap_upper = total["paired_metric_p95_ratio_bootstrap_ci95_percent"]["high_percent"]
        .as_f64()
        .expect("total p95 bootstrap upper bound");
    let production_policy = policy == "production_wal_autocheckpoint";
    let baseline_checkpoint_observations = case
        .baseline
        .iter()
        .filter(|sample| automatic_checkpoint_frame_work_observed(**sample))
        .count();
    let candidate_checkpoint_observations = case
        .candidate
        .iter()
        .filter(|sample| automatic_checkpoint_frame_work_observed(**sample))
        .count();
    let checkpoint_coverage =
        baseline_checkpoint_observations > 0 && candidate_checkpoint_observations > 0;
    let checkpoint_coverage_status = if checkpoint_coverage {
        "AUTOCHECKPOINT_OBSERVED_IN_BOTH_ARMS"
    } else {
        "INSUFFICIENT_AUTOCHECKPOINT_COVERAGE"
    };
    let (production_writer_classification, classification_reason) = production_policy
        .then(|| classify_production_writer(total_overhead, bootstrap_upper, checkpoint_coverage))
        .unzip();
    let candidate_slower_pairs = case
        .baseline
        .iter()
        .zip(&case.candidate)
        .filter(|(baseline, candidate)| candidate.total_ms > baseline.total_ms)
        .count();

    json!({
        "policy": policy,
        "batch_size": batch_size,
        "sample_count": case.baseline.len(),
        "counterbalanced_pair_order": case.pair_orders,
        "candidate_slower_pair_count": candidate_slower_pairs,
        "total": total,
        "update": update,
        "commit": commit,
        "full_checkpoint": checkpoint,
        "write_budget": {
            "limit_percent": WRITER_P95_OVERHEAD_LIMIT_PERCENT,
            "p95_overhead_percent": total_overhead,
            "point_estimate_passed": total_overhead <= WRITER_P95_OVERHEAD_LIMIT_PERCENT,
            "bootstrap_ci95_upper_percent": bootstrap_upper,
            "bootstrap_ci95_upper_passed": bootstrap_upper <= WRITER_P95_OVERHEAD_LIMIT_PERCENT,
            "gate_applies": production_policy,
            "rule": if production_policy {
                "production gate: p95 point estimate <=10% and paired-bootstrap 95% CI upper bound <=10%; real autocheckpoint work must be observed in both arms"
            } else {
                "diagnostic only: forced FULL checkpoint is reported separately and does not gate production WAL eligibility"
            }
        },
        "production_writer_classification": production_writer_classification,
        "classification_reason": classification_reason,
        "automatic_checkpoint_observation": if production_policy {
            json!({
                "coverage_status": checkpoint_coverage_status,
                "baseline_transactions_with_frame_work": baseline_checkpoint_observations,
                "candidate_transactions_with_frame_work": candidate_checkpoint_observations,
                "sample_count_per_arm": case.baseline.len(),
                "wal_frame_counts_recorded_before_and_after_every_transaction": true,
                "timing_accounting": "checkpoint time remains inside SQLite commit_ms and complete total_ms; not moved outside the production callback"
            })
        } else {
            json!({
                "coverage_status": "NOT_APPLICABLE_AUTOMATIC_CHECKPOINT_DISABLED_FOR_STRESS",
                "timing_accounting": "automatic checkpoint disabled; each transaction is followed by an explicit timed FULL checkpoint included in total_ms"
            })
        },
        "raw_paired_samples": case.raw_pairs
    })
}

fn paired_p95_ratio_ci95(baseline: &[f64], candidate: &[f64], seed: u64) -> JsonValue {
    assert_eq!(baseline.len(), candidate.len());
    assert!(!baseline.is_empty());
    let mut state = seed.max(1);
    let mut ratios = Vec::with_capacity(BOOTSTRAP_REPLICATES);
    let mut sampled_baseline = Vec::with_capacity(baseline.len());
    let mut sampled_candidate = Vec::with_capacity(candidate.len());
    for _ in 0..BOOTSTRAP_REPLICATES {
        sampled_baseline.clear();
        sampled_candidate.clear();
        for _ in 0..baseline.len() {
            let index = next_random(&mut state) as usize % baseline.len();
            sampled_baseline.push(baseline[index]);
            sampled_candidate.push(candidate[index]);
        }
        sampled_baseline.sort_by(f64::total_cmp);
        sampled_candidate.sort_by(f64::total_cmp);
        let base_p95 = super::super::quantile(&sampled_baseline, 0.95);
        let candidate_p95 = super::super::quantile(&sampled_candidate, 0.95);
        ratios.push(overhead_percent(base_p95, candidate_p95));
    }
    ratios.sort_by(f64::total_cmp);
    json!({
        "low_percent": super::super::quantile(&ratios, 0.025),
        "high_percent": super::super::quantile(&ratios, 0.975),
        "replicates": BOOTSTRAP_REPLICATES,
        "seed": seed,
        "paired_resampling_unit": "one baseline/candidate sample pair"
    })
}

fn next_random(state: &mut u64) -> u64 {
    // Deterministic xorshift64*; no new dependency or nondeterministic seed.
    *state ^= *state >> 12;
    *state ^= *state << 25;
    *state ^= *state >> 27;
    state.wrapping_mul(0x2545_f491_4f6c_dd1d)
}

fn overhead_percent(baseline: f64, candidate: f64) -> f64 {
    if baseline <= 0.0 {
        0.0
    } else {
        ((candidate / baseline) - 1.0) * 100.0
    }
}

fn automatic_checkpoint_frame_work_observed(timing: WriterTiming) -> bool {
    timing.wal_after.checkpointed_frames > timing.wal_before.checkpointed_frames
        || timing.wal_after.log_frames < timing.wal_before.log_frames
}

fn classify_production_writer(
    p95_overhead_percent: f64,
    bootstrap_upper_percent: f64,
    checkpoint_coverage: bool,
) -> (&'static str, &'static str) {
    if p95_overhead_percent > WRITER_P95_OVERHEAD_LIMIT_PERCENT {
        ("WRITER_NO_GO", "POINT_ESTIMATE_EXCEEDS_10_PERCENT")
    } else if bootstrap_upper_percent > WRITER_P95_OVERHEAD_LIMIT_PERCENT {
        (
            "POINT_PASS_INCONCLUSIVE",
            "BOOTSTRAP_95_PERCENT_UPPER_BOUND_EXCEEDS_10_PERCENT",
        )
    } else if !checkpoint_coverage {
        (
            "POINT_PASS_INCONCLUSIVE",
            "REPRESENTATIVE_AUTOCHECKPOINT_WORK_NOT_OBSERVED_IN_BOTH_ARMS",
        )
    } else {
        (
            "PRODUCTION_WRITER_PASS_WITH_MARGIN",
            "POINT_ESTIMATE_AND_BOOTSTRAP_UPPER_BOUND_WITHIN_10_PERCENT",
        )
    }
}

#[test]
fn writer_gate_classification_requires_statistical_and_checkpoint_margin() {
    assert_eq!(
        classify_production_writer(10.0, 10.0, true).0,
        "PRODUCTION_WRITER_PASS_WITH_MARGIN"
    );
    assert_eq!(
        classify_production_writer(9.0, 10.01, true).0,
        "POINT_PASS_INCONCLUSIVE"
    );
    assert_eq!(
        classify_production_writer(9.0, 9.0, false).0,
        "POINT_PASS_INCONCLUSIVE"
    );
    assert_eq!(
        classify_production_writer(10.01, 9.0, true).0,
        "WRITER_NO_GO"
    );
}

fn writer_timing_json(timing: WriterTiming, production_policy: bool) -> JsonValue {
    let checkpoint_observed = automatic_checkpoint_frame_work_observed(timing);
    json!({
        "total_ms": timing.total_ms,
        "update_ms": timing.update_ms,
        "commit_ms": timing.commit_ms,
        "full_checkpoint_ms": timing.full_checkpoint_ms,
        "automatic_checkpoint_frame_work_indicated": production_policy.then_some(checkpoint_observed),
        "automatic_checkpoint_ms_separately_measured": None::<f64>,
        "automatic_checkpoint_timing_accounting": if production_policy {
            "not isolated to avoid replacing SQLite's production callback; any synchronous checkpoint is included in commit_ms and total_ms"
        } else {
            "disabled for this policy; explicit FULL duration is reported separately"
        },
        "wal_before": wal_json(timing.wal_before),
        "wal_after": wal_json(timing.wal_after),
        "explicit_full_checkpoint": timing.full_checkpoint.map(checkpoint_json)
    })
}

fn sqlite_writer_contract(connection: &Connection) -> (String, i64, i64) {
    let journal_mode = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .expect("read writer journal mode");
    let synchronous = connection
        .query_row("PRAGMA synchronous", [], |row| row.get(0))
        .expect("read writer synchronous mode");
    let wal_autocheckpoint = connection
        .query_row("PRAGMA wal_autocheckpoint", [], |row| row.get(0))
        .expect("read writer automatic checkpoint policy");
    let foreign_keys: i64 = connection
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .expect("read writer foreign key policy");
    assert_eq!(foreign_keys, 1);
    (journal_mode, synchronous, wal_autocheckpoint)
}

fn assert_database_pair_equivalent(
    baseline: &Connection,
    candidate: &Connection,
    baseline_path: &Path,
    candidate_path: &Path,
) {
    assert_eq!(database_signature(baseline), database_signature(candidate));
    assert_eq!(
        pragma_i64(baseline, "page_count"),
        pragma_i64(candidate, "page_count"),
        "paired databases must have equal SQLite page counts"
    );
    assert_eq!(file_size(baseline_path), file_size(candidate_path));
    assert_wal_states_equal(
        read_wal_state(baseline),
        read_wal_state(candidate),
        "database pair equivalence check",
    );
}

fn database_signature(connection: &Connection) -> (i64, i64, i64, i64, i64, i64, i64) {
    connection
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(rowid),0),
                    COALESCE(SUM(modified_at_fs),0), COALESCE(SUM(last_seen_at),0),
                    COALESCE(SUM(length(name_normalized)),0), COALESCE(SUM(is_stale),0),
                    (SELECT COUNT(*) FROM global_entries_fts)
             FROM global_entries",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .expect("read paired database content signature")
}

fn assert_wal_states_equal(left: WalState, right: WalState, context: &str) {
    assert_eq!(
        left, right,
        "independent writer clones must have equivalent WAL state {context}"
    );
}

fn read_wal_state(connection: &Connection) -> WalState {
    let (busy, log_frames, checkpointed_frames) = connection
        .query_row("PRAGMA wal_checkpoint(NOOP)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .expect("read WAL state without checkpointing");
    WalState {
        busy,
        log_frames,
        checkpointed_frames,
    }
}

fn checkpoint(connection: &Connection, mode: &str) -> CheckpointResult {
    assert!(matches!(mode, "TRUNCATE" | "FULL"));
    let (busy, log_frames, checkpointed_frames) = connection
        .query_row(&format!("PRAGMA wal_checkpoint({mode})"), [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .expect("run requested SQLite checkpoint");
    CheckpointResult {
        busy,
        log_frames,
        checkpointed_frames,
    }
}

fn checkpoint_json(result: CheckpointResult) -> JsonValue {
    json!({
        "busy": result.busy,
        "log_frames": result.log_frames,
        "checkpointed_frames": result.checkpointed_frames
    })
}

fn wal_json(state: WalState) -> JsonValue {
    json!({
        "busy": state.busy,
        "log_frames": state.log_frames,
        "checkpointed_frames": state.checkpointed_frames
    })
}

fn pragma_i64(connection: &Connection, name: &str) -> i64 {
    assert_eq!(name, "page_count");
    connection
        .query_row("PRAGMA page_count", [], |row| row.get(0))
        .expect("read SQLite page count")
}

fn file_size(path: &Path) -> u64 {
    fs::metadata(path).expect("read paired database size").len()
}

fn blake3_file(path: &Path) -> String {
    let mut input = File::open(path).expect("open cloned database for streaming digest");
    let mut hasher = Hasher::new();
    let mut buffer = [0_u8; 128 * 1024];
    loop {
        let count = input.read(&mut buffer).expect("read cloned database bytes");
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    hasher.finalize().to_hex().to_string()
}
