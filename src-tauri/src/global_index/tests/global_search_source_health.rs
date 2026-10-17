use super::{test_db_path, test_entry, test_volume};
use crate::db::Database;
use crate::global_index::repository::{
    diagnostic_compare_snapshot_queries_on_connection, diagnostic_snapshot_source_health,
    diagnostic_snapshot_status, diagnostic_snapshot_status_sql,
    load_global_search_source_health_candidate, GlobalSearchSourceHealthQueryCandidate as Query,
};
use crate::global_index::search::with_legacy_name_prefix_search_candidate;
use crate::global_index::{GlobalEntryInput, GlobalVolume};
use rusqlite::params;
use std::sync::{Arc, Barrier};

fn volume(id: &str, enabled: bool) -> GlobalVolume {
    let mut volume = test_volume();
    volume.id = id.to_string();
    volume.stable_volume_id = format!("stable-{id}");
    volume.mount_path = format!(r"C:\Global\{id}\");
    volume.enabled = enabled;
    volume
}

fn entry(volume_id: &str, path: &str, name: &str, last_seen_at: i64) -> GlobalEntryInput {
    let mut entry = test_entry(path, name, false);
    entry.volume_id = volume_id.to_string();
    entry.last_seen_at = last_seen_at;
    entry
}

fn assert_snapshot_equal(
    original: &crate::global_index::repository::GlobalSearchSnapshot,
    candidate: &crate::global_index::repository::GlobalSearchSnapshot,
) {
    assert_eq!(original.results, candidate.results);
    assert_eq!(original.source_health, candidate.source_health);
    assert_eq!(
        original.source_revision.as_bytes(),
        candidate.source_revision.as_bytes()
    );
    assert_eq!(original.index_status, candidate.index_status);
}

// Every comparison below uses one established SQLite snapshot. The extra
// COUNT(*) must not change canonical COUNT(id) facts, their bytes, or status.
fn assert_snapshot_count_matches(conn: &rusqlite::Connection) -> i64 {
    let legacy = load_global_search_source_health_candidate(conn, Query::LegacyNarrowAggregate)
        .expect("read pre-optimization source facts");
    let (source, count) = diagnostic_snapshot_source_health(conn)
        .expect("read production source facts and private count");
    assert_eq!(legacy, source);
    let original_status = diagnostic_snapshot_status(conn, None).expect("old COUNT(*) status");
    let reused_status = diagnostic_snapshot_status(conn, Some(count)).expect("reused count status");
    assert_eq!(original_status, reused_status);
    assert_eq!(reused_status.total_entries, count);
    assert_eq!(reused_status.processed_entries, count);
    count
}

fn assert_candidate_matches_production(db: &Database, query: &str) {
    let standalone = db
        .global_index_status()
        .expect("standalone status remains truthful");
    let original = db
        .search_global_entries_snapshot(query, 80, 0)
        .expect("read production source snapshot");
    assert_eq!(standalone, original.index_status);

    for candidate in [
        Query::CorrelatedAggregates,
        Query::NarrowAggregate,
        Query::GroupByVolumeId,
    ] {
        let optimized = db
            .search_global_entries_snapshot_with_source_health_candidate(query, 80, 0, candidate)
            .expect("read diagnostic candidate snapshot");
        assert_snapshot_equal(&original, &optimized);
    }

    let mut conn = db.conn().expect("borrow database connection");
    let transaction = conn.transaction().expect("begin fact comparison snapshot");
    assert_snapshot_count_matches(&transaction);
    let baseline = load_global_search_source_health_candidate(&transaction, Query::Original)
        .expect("load baseline revision facts");
    for candidate in [
        Query::CorrelatedAggregates,
        Query::NarrowAggregate,
        Query::GroupByVolumeId,
    ] {
        let optimized = load_global_search_source_health_candidate(&transaction, candidate)
            .expect("load candidate revision facts");
        assert_eq!(baseline.source_health, optimized.source_health);
        assert_eq!(baseline.revision_facts_json, optimized.revision_facts_json);
        assert_eq!(
            baseline.source_revision.as_bytes(),
            optimized.source_revision.as_bytes()
        );
    }
    transaction
        .commit()
        .expect("commit fact comparison snapshot");
}

#[test]
fn global_search_source_health_candidates_match_zero_and_empty_volumes() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open source-health candidate database");

    assert_candidate_matches_production(&db, "nothing");
    let no_sources = db
        .search_global_entries_snapshot("nothing", 80, 0)
        .expect("read zero-volume snapshot");
    assert!(no_sources.source_health.is_empty());

    db.upsert_global_volume(&volume("gv_empty", true))
        .expect("insert empty volume");
    assert_candidate_matches_production(&db, "nothing");
    let empty = db
        .search_global_entries_snapshot("nothing", 80, 0)
        .expect("read empty-volume snapshot");
    assert_eq!(empty.source_health.len(), 1);

    let conn = db.conn().expect("borrow database connection");
    let facts = load_global_search_source_health_candidate(&conn, Query::NarrowAggregate)
        .expect("read empty-volume facts");
    let serialized: serde_json::Value =
        serde_json::from_slice(&facts.revision_facts_json).expect("parse revision facts");
    assert_eq!(serialized[0]["active_entry_count"], 0);
    assert!(serialized[0]["max_last_seen_at"].is_null());

    drop(conn);
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn global_search_source_health_candidates_match_adversarial_volume_and_entry_states() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open source-health candidate database");
    for source in [
        volume("gv_z_empty", true),
        volume("gv_b_disabled", false),
        volume("gv_a_enabled", true),
    ] {
        db.upsert_global_volume(&source)
            .expect("insert adversarial volume");
    }
    db.update_global_volume_state(
        "gv_b_disabled",
        "paused",
        Some("offline"),
        None,
        None,
        None,
        None,
    )
    .expect("set disabled source status");
    db.update_global_volume_state("gv_a_enabled", "ready", None, None, None, None, Some(80))
        .expect("set enabled source status");

    let a_old = entry("gv_a_enabled", r"C:\Global\a\old.txt", "old.txt", 10);
    let a_tie = entry("gv_a_enabled", r"C:\Global\a\tie.txt", "tie.txt", 10);
    let a_stale = entry("gv_a_enabled", r"C:\Global\a\stale.txt", "stale.txt", 20);
    let b_stale = entry("gv_b_disabled", r"C:\Global\b\stale.txt", "stale.txt", 50);
    let z_max = entry("gv_z_empty", r"C:\Global\z\max.txt", "max.txt", 99);
    let z_tie = entry("gv_z_empty", r"C:\Global\z\tie.txt", "tie.txt", 99);
    db.upsert_global_entries_batch(&[
        a_old.clone(),
        a_tie.clone(),
        a_stale.clone(),
        b_stale.clone(),
        z_max.clone(),
        z_tie.clone(),
    ])
    .expect("insert mixed active and stale candidates");
    db.mark_global_entry_stale(&a_stale.entry_id())
        .expect("mark one enabled-volume row stale");
    db.mark_global_entry_stale(&b_stale.entry_id())
        .expect("make disabled volume entirely stale");

    assert_candidate_matches_production(&db, "tie");
    let conn = db.conn().expect("borrow database connection");
    conn.execute(
        "UPDATE global_volumes SET entry_count = 12345 WHERE id = 'gv_a_enabled'",
        [],
    )
    .expect("deliberately desynchronize the convenience entry_count");
    let facts = load_global_search_source_health_candidate(&conn, Query::NarrowAggregate)
        .expect("load facts with inconsistent entry_count");
    assert_eq!(
        facts
            .source_health
            .iter()
            .map(|source| source.source_id.as_str())
            .collect::<Vec<_>>(),
        ["gv_a_enabled", "gv_b_disabled", "gv_z_empty"]
    );
    let serialized: serde_json::Value =
        serde_json::from_slice(&facts.revision_facts_json).expect("parse revision facts");
    assert_eq!(serialized[0]["active_entry_count"], 2);
    assert_eq!(serialized[0]["max_last_seen_at"], 10);
    assert_eq!(serialized[1]["active_entry_count"], 0);
    assert!(serialized[1]["max_last_seen_at"].is_null());
    assert_eq!(serialized[2]["active_entry_count"], 2);
    assert_eq!(serialized[2]["max_last_seen_at"], 99);

    let baseline_revision = facts.source_revision.clone();
    conn.execute(
        "UPDATE global_entries SET last_seen_at = 9 WHERE id = ?1",
        params![a_old.entry_id()],
    )
    .expect("change a non-maximum last_seen_at without touching volume metadata");
    let non_max_facts = load_global_search_source_health_candidate(&conn, Query::NarrowAggregate)
        .expect("load facts after non-maximum change");
    assert_eq!(non_max_facts.source_revision, baseline_revision);
    conn.execute(
        "UPDATE global_entries SET last_seen_at = 11 WHERE id = ?1",
        params![a_tie.entry_id()],
    )
    .expect("change an entry that raises the maximum");
    let raised_max = load_global_search_source_health_candidate(&conn, Query::NarrowAggregate)
        .expect("load facts after maximum change");
    assert_ne!(raised_max.source_revision, baseline_revision);
    drop(conn);
    assert_candidate_matches_production(&db, "tie");

    let added = entry("gv_a_enabled", r"C:\Global\a\added.txt", "added.txt", 12);
    db.upsert_global_entries_batch(std::slice::from_ref(&added))
        .expect("insert active row");
    assert_candidate_matches_production(&db, "added");
    let conn = db.conn().expect("borrow database connection");
    conn.execute(
        "DELETE FROM global_entries WHERE id = ?1",
        params![added.entry_id()],
    )
    .expect("delete active row");
    drop(conn);
    assert_candidate_matches_production(&db, "added");

    db.upsert_global_entries_batch(std::slice::from_ref(&a_stale))
        .expect("restore stale entry through production upsert");
    db.set_global_volume_enabled("gv_b_disabled", true)
        .expect("enable formerly disabled source");
    db.update_global_volume_provider("gv_z_empty", "alternate-provider")
        .expect("change source provider");
    db.update_global_volume_state(
        "gv_z_empty",
        "error",
        Some("injected failure"),
        None,
        None,
        None,
        None,
    )
    .expect("change source index status and error");
    let conn = db.conn().expect("borrow database connection");
    conn.execute(
        "UPDATE global_volumes SET updated_at = updated_at + 1 WHERE id = 'gv_z_empty'",
        [],
    )
    .expect("change source updated_at directly");
    drop(conn);
    assert_candidate_matches_production(&db, "stale");

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn global_search_name_prefix_cte_matches_legacy_product_path() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open name-prefix equivalence database");
    let enabled = volume("gv_prefix_enabled", true);
    let disabled = volume("gv_prefix_disabled", false);
    db.upsert_global_volume(&enabled)
        .expect("insert enabled name-prefix source");
    db.upsert_global_volume(&disabled)
        .expect("insert disabled name-prefix source");

    let prefix_entry = |volume_id: &str, name: &str, modified_at: i64| {
        let mut row = entry(
            volume_id,
            &format!(r"C:\Global\{volume_id}\{name}"),
            name,
            modified_at,
        );
        row.modified_at_fs = Some(modified_at);
        row
    };

    let exact = prefix_entry("gv_prefix_enabled", "quarterly", 1_200);
    let tied_a = prefix_entry("gv_prefix_enabled", "Quarterly Review tie-a.txt", 1_000);
    let tied_b = prefix_entry("gv_prefix_enabled", "Quarterly Review tie-b.txt", 1_000);
    let older = prefix_entry("gv_prefix_enabled", "Quarterly Review older.txt", 900);
    let stale = prefix_entry("gv_prefix_enabled", "Quarterly Review stale.txt", 2_000);
    let disabled_hit = prefix_entry("gv_prefix_disabled", "Quarterly Review disabled.txt", 3_000);
    let chinese = prefix_entry("gv_prefix_enabled", "数据库-报告.txt", 800);
    let accented = prefix_entry("gv_prefix_enabled", "Résumé-final.txt", 799);
    let punctuation = prefix_entry("gv_prefix_enabled", "build[one]-artifact?.txt", 798);
    db.upsert_global_entries_batch(&[
        exact.clone(),
        tied_a.clone(),
        tied_b.clone(),
        older,
        stale.clone(),
        disabled_hit,
        chinese.clone(),
        accented.clone(),
        punctuation.clone(),
    ])
    .expect("insert name-prefix equivalence entries");
    db.mark_global_entry_stale(&stale.entry_id())
        .expect("exclude stale high-ranked name-prefix candidate");

    let conn = db
        .conn()
        .expect("borrow name-prefix equivalence connection");
    conn.execute(
        "INSERT INTO managed_scopes (id, path, global_entry_id, enabled, allow_local_ai, allow_cloud_ai, created_at, updated_at) VALUES ('scope-prefix-tie-a', 'C:\\Global\\managed', ?1, 1, 1, 0, 1700000000, 1700000000)",
        params![tied_a.entry_id()],
    )
    .expect("mark a tied result as managed");
    conn.execute(
        "INSERT INTO managed_entries (id, global_entry_id, managed_scope_id, enabled, created_at, updated_at) VALUES ('entry-prefix-tie-a', ?1, 'scope-prefix-tie-a', 1, 1700000000, 1700000000)",
        params![tied_a.entry_id()],
    )
    .expect("insert managed marker for tied result");
    drop(conn);

    let assert_old_and_new_equal = |query: &str, limit: u32, offset: u32| {
        let legacy = with_legacy_name_prefix_search_candidate(|| {
            db.search_global_entries_snapshot(query, limit, offset)
        })
        .expect("read legacy name-prefix product snapshot");
        let production = db
            .search_global_entries_snapshot(query, limit, offset)
            .expect("read key-only name-prefix product snapshot");
        assert_snapshot_equal(&legacy, &production);
        production
    };

    let first_page = assert_old_and_new_equal("quarterly", 3, 0);
    let mut tied_ids = [tied_a.entry_id(), tied_b.entry_id()];
    tied_ids.sort();
    let expected_ids = vec![exact.entry_id(), tied_ids[0].clone(), tied_ids[1].clone()];
    assert_eq!(
        first_page
            .results
            .iter()
            .map(|row| row.id.clone())
            .collect::<Vec<_>>(),
        expected_ids
    );
    assert!(first_page
        .results
        .iter()
        .any(|row| row.id == tied_a.entry_id() && row.managed));
    assert!(first_page
        .results
        .iter()
        .all(|row| row.id != stale.entry_id() && row.volume_id != disabled.id));

    let second_page = assert_old_and_new_equal("quarterly", 1, 1);
    assert_eq!(second_page.results[0].id, tied_ids[0]);
    let third_page = assert_old_and_new_equal("quarterly", 1, 2);
    assert_eq!(third_page.results[0].id, tied_ids[1]);

    let chinese_result = assert_old_and_new_equal("数据库", 20, 0);
    assert_eq!(chinese_result.results[0].id, chinese.entry_id());
    let accented_result = assert_old_and_new_equal("RÉSUMÉ", 20, 0);
    assert_eq!(accented_result.results[0].id, accented.entry_id());
    let punctuation_result = assert_old_and_new_equal("build[", 20, 0);
    assert_eq!(punctuation_result.results[0].id, punctuation.entry_id());
    assert!(assert_old_and_new_equal("missing-prefix", 20, 0)
        .results
        .is_empty());

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn global_search_source_health_candidates_preserve_null_max_semantics() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open source-health null fixture");
    db.upsert_global_volume(&volume("gv_null", true))
        .expect("insert source for nullable fixture");
    db.upsert_global_volume(&volume("gv_null_count", true))
        .expect("insert source for COUNT(column) fixture");
    db.upsert_global_volume(&volume("gv_disabled", false))
        .expect("insert disabled nullable source");
    let mut conn = db.conn().expect("borrow database connection");
    conn.execute_batch(
        "CREATE TEMP TABLE global_entries (
             id TEXT PRIMARY KEY,
             volume_id TEXT NOT NULL,
             is_stale INTEGER NOT NULL,
             last_seen_at INTEGER
         );
         INSERT INTO global_entries VALUES ('active-null', 'gv_null', 0, NULL);
         INSERT INTO global_entries VALUES ('stale-value', 'gv_null', 1, 123);
         INSERT INTO global_entries VALUES (NULL, 'gv_null_count', 0, 777);
         INSERT INTO global_entries VALUES ('disabled-active', 'gv_disabled', 0, 888);
         INSERT INTO global_entries VALUES ('disabled-stale', 'gv_disabled', 1, 999);
         INSERT INTO global_entries VALUES ('orphan', 'missing-source', 0, 111);",
    )
    .expect("shadow production table with nullable isolated fixture");
    let transaction = conn.transaction().expect("begin nullable fact snapshot");
    let original = load_global_search_source_health_candidate(&transaction, Query::Original)
        .expect("load original nullable aggregate");
    assert_eq!(assert_snapshot_count_matches(&transaction), 2);
    for candidate in [
        Query::CorrelatedAggregates,
        Query::NarrowAggregate,
        Query::GroupByVolumeId,
    ] {
        let optimized = load_global_search_source_health_candidate(&transaction, candidate)
            .expect("load candidate nullable aggregate");
        assert_eq!(original, optimized);
        let serialized: serde_json::Value =
            serde_json::from_slice(&optimized.revision_facts_json).expect("parse facts");
        let facts = serialized.as_array().expect("fact array");
        let null_max = facts
            .iter()
            .find(|fact| fact["source_id"] == "gv_null")
            .unwrap();
        let null_id = facts
            .iter()
            .find(|fact| fact["source_id"] == "gv_null_count")
            .unwrap();
        assert_eq!(null_max["active_entry_count"], 1);
        assert!(null_max["max_last_seen_at"].is_null());
        assert_eq!(null_id["active_entry_count"], 0);
        assert_eq!(null_id["max_last_seen_at"], 777);
    }
    transaction.commit().expect("commit nullable fact snapshot");

    drop(conn);
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn global_search_source_health_candidates_match_during_concurrent_writes() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open concurrent source-health fixture");
    db.upsert_global_volume(&volume("gv_writer", true))
        .expect("insert writer volume");
    db.upsert_global_volume(&volume("gv_reader", true))
        .expect("insert reader volume");
    let changing = entry(
        "gv_writer",
        r"C:\Global\writer\changing.txt",
        "changing.txt",
        30,
    );
    let stable = entry(
        "gv_reader",
        r"C:\Global\reader\stable.txt",
        "stable.txt",
        40,
    );
    db.upsert_global_entries_batch(&[changing.clone(), stable])
        .expect("insert concurrent fixture rows");

    let writer_start = Arc::new(Barrier::new(2));
    let writer_commit = Arc::new(Barrier::new(2));
    let worker_db = db.clone();
    let worker_start = Arc::clone(&writer_start);
    let worker_commit = Arc::clone(&writer_commit);
    let changing_for_worker = changing.clone();
    let worker = std::thread::spawn(move || {
        for index in 0..32 {
            worker_start.wait();
            let enabled = index % 2 == 0;
            worker_db
                .set_global_volume_enabled("gv_writer", enabled)
                .expect("toggle volume while reader snapshots");
            worker_db
                .update_global_volume_state(
                    "gv_writer",
                    if enabled { "ready" } else { "syncing" },
                    if enabled { None } else { Some("writer active") },
                    None,
                    None,
                    None,
                    Some(100 + index),
                )
                .expect("change status while reader snapshots");
            if index % 2 == 0 {
                worker_db
                    .mark_global_entry_stale(&changing_for_worker.entry_id())
                    .expect("mark row stale while reader snapshots");
            } else {
                let mut restored = changing_for_worker.clone();
                restored.last_seen_at = 100 + index;
                worker_db
                    .upsert_global_entries_batch(std::slice::from_ref(&restored))
                    .expect("restore row while reader snapshots");
            }
            worker_commit.wait();
        }
    });

    for _ in 0..32 {
        let mut conn = db.conn().expect("borrow concurrent reader connection");
        let transaction = conn
            .transaction()
            .expect("begin consistent reader snapshot");
        super::super::search::search_global_entries_on_connection(&transaction, "txt", 80, 0)
            .expect("establish reader snapshot before concurrent writer commit");
        writer_start.wait();
        writer_commit.wait();
        assert_snapshot_count_matches(&transaction);
        for candidate in [
            Query::CorrelatedAggregates,
            Query::NarrowAggregate,
            Query::GroupByVolumeId,
        ] {
            let (original, optimized) = diagnostic_compare_snapshot_queries_on_connection(
                &transaction,
                "txt",
                80,
                0,
                candidate,
            )
            .expect("compare both snapshots within one read transaction");
            assert_snapshot_equal(&original, &optimized);
        }
        transaction
            .commit()
            .expect("commit concurrent reader snapshot");
    }
    worker.join().expect("join source-health writer");

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn snapshot_reused_count_preserves_status_transitions_and_exact_enabled_filter() {
    let path = test_db_path();
    let db = Database::open(&path).expect("open status transition fixture");
    db.upsert_global_volume(&volume("gv_count", true)).unwrap();
    db.upsert_global_entries_batch(&[entry("gv_count", r"C:\Global\count.txt", "count.txt", 77)])
        .unwrap();
    let mut conn = db.conn().unwrap();
    for status in [
        "ready",
        "discovered",
        "indexing",
        "syncing",
        "rebuild_required",
        "spotlight_not_indexed",
        "permission_required",
        "spotlight_unavailable",
        "spotlight_external_not_indexed",
        "fsevents_unavailable",
        "unavailable",
        "error",
        "paused",
    ] {
        for enabled in [0, 1] {
            conn.execute("UPDATE global_volumes SET enabled=?1, index_status=?2,
                entry_count=999, last_incremental_sync_at=123, last_error='kept error', updated_at=456",
                params![enabled, status]).unwrap();
            for stale in [0, 1] {
                conn.execute("UPDATE global_entries SET is_stale=?1", params![stale])
                    .unwrap();
                let transaction = conn.transaction().unwrap();
                let count = assert_snapshot_count_matches(&transaction);
                assert_eq!(count, i64::from(enabled == 1 && stale == 0));
                transaction.commit().unwrap();
                assert_candidate_matches_production(&db, "count");
            }
        }
    }
    drop(conn);
    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn snapshot_precomputed_status_skips_the_entries_count_branch() {
    let path = test_db_path();
    let db = Database::open(&path).unwrap();
    db.upsert_global_volume(&volume("gv_steps", true)).unwrap();
    let mut conn = db.conn().unwrap();
    conn.execute_batch("CREATE TEMP TABLE global_entries (id TEXT, volume_id TEXT, is_stale INTEGER, last_seen_at INTEGER);
        WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<256)
        INSERT INTO global_entries SELECT CAST(x AS TEXT), 'gv_steps', 0, x FROM n;").unwrap();
    let transaction = conn.transaction().unwrap();
    let count = assert_snapshot_count_matches(&transaction);
    assert_eq!(count, 256);
    let mut statement = transaction
        .prepare(diagnostic_snapshot_status_sql())
        .unwrap();
    let original: i64 = statement
        .query_row(params![None::<i64>], |row| row.get(0))
        .unwrap();
    let original_steps = statement.get_status(rusqlite::StatementStatus::VmStep);
    statement.reset_status(rusqlite::StatementStatus::VmStep);
    let reused: i64 = statement
        .query_row(params![Some(count)], |row| row.get(0))
        .unwrap();
    let reused_steps = statement.get_status(rusqlite::StatementStatus::VmStep);
    assert_eq!(original, reused);
    assert!(
        reused_steps < original_steps,
        "the bound count must bypass the entries scan"
    );
    // A supplied zero is a real count, not a request to execute the fallback.
    let zero: i64 = statement
        .query_row(params![Some(0_i64)], |row| row.get(0))
        .unwrap();
    assert_eq!(zero, 0);
    drop(statement);
    transaction.commit().unwrap();
    drop(conn);
    drop(db);
    let _ = std::fs::remove_file(path);
}
