use super::{test_db_path, test_entry, test_volume};
use crate::db::Database;
use crate::global_index::repository::snapshot_stage_probe::{
    capture, snapshot_on_connection, with_exact_count,
};
use crate::global_index::repository::{
    diagnostic_compare_snapshot_queries_on_connection, load_global_search_source_health_candidate,
    GlobalSearchSourceHealthQueryCandidate as Query,
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

fn assert_candidate_matches_production(db: &Database, query: &str) {
    let original = db
        .search_global_entries_snapshot(query, 80, 0)
        .expect("read production source snapshot");

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

    let (count_candidate, count_probe) = capture(false, || {
        with_exact_count(|| db.search_global_entries_snapshot(query, 80, 0))
    });
    assert_snapshot_equal(&original, &count_candidate.unwrap());
    assert_eq!(
        with_exact_count(|| db.global_index_status()).unwrap(),
        original.index_status
    );

    let mut conn = db.conn().expect("borrow database connection");
    let transaction = conn.transaction().expect("begin fact comparison snapshot");
    let baseline = load_global_search_source_health_candidate(&transaction, Query::Original)
        .expect("load baseline revision facts");
    assert_eq!(
        count_probe.revision_facts_json,
        baseline.revision_facts_json
    );
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
         INSERT INTO global_entries VALUES (NULL, 'gv_null_count', 0, 777);",
    )
    .expect("shadow production table with nullable isolated fixture");
    let transaction = conn.transaction().expect("begin nullable fact snapshot");
    let original = load_global_search_source_health_candidate(&transaction, Query::Original)
        .expect("load original nullable aggregate");
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
        assert_eq!(serialized[0]["active_entry_count"], 1);
        assert!(serialized[0]["max_last_seen_at"].is_null());
        assert_eq!(serialized[1]["active_entry_count"], 0);
        assert_eq!(serialized[1]["max_last_seen_at"], 777);
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

    let mut null_entry = entry(
        "gv_writer",
        r"C:\Global\writer\nullable.bin",
        "nullable.bin",
        50,
    );
    null_entry.extension = "bin".to_string();
    db.upsert_global_entries_batch(std::slice::from_ref(&null_entry))
        .unwrap();
    db.conn()
        .unwrap()
        .execute(
            "UPDATE global_entries SET id = NULL WHERE id = ?1",
            [null_entry.entry_id()],
        )
        .unwrap();

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
            worker_db
                .conn()
                .unwrap()
                .execute(
                    "UPDATE global_entries SET is_stale = ?1, last_seen_at = ?2 WHERE id IS NULL",
                    params![i64::from(index % 2 == 0), 300 + index],
                )
                .unwrap();
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
        let pinned_before = snapshot_on_connection(&transaction, "txt").unwrap();
        writer_start.wait();
        writer_commit.wait();
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
        let (candidate, probe) = capture(false, || {
            with_exact_count(|| snapshot_on_connection(&transaction, "txt"))
        });
        assert_snapshot_equal(&pinned_before, &candidate.unwrap());
        let facts =
            load_global_search_source_health_candidate(&transaction, Query::Original).unwrap();
        assert_eq!(probe.revision_facts_json, facts.revision_facts_json);
        assert_eq!(
            pinned_before.index_status,
            snapshot_on_connection(&transaction, "txt")
                .unwrap()
                .index_status
        );
        transaction
            .commit()
            .expect("commit concurrent reader snapshot");
        assert_ne!(
            db.search_global_entries_snapshot("txt", 80, 0)
                .unwrap()
                .source_revision,
            pinned_before.source_revision
        );
    }
    worker.join().expect("join source-health writer");

    drop(db);
    let _ = std::fs::remove_file(path);
}

#[test]
fn exact_count_snapshot_preserves_nullable_max_orphans_and_all_source_states() {
    let path = test_db_path();
    let db = Database::open(&path).unwrap();
    for source in [
        volume("a", true),
        volume("b", false),
        volume("c", true),
        volume("d", true),
    ] {
        db.upsert_global_volume(&source).unwrap();
    }
    let template = entry("a", r"C:\Global\template.txt", "template.txt", 1);
    db.upsert_global_entries_batch(&[template]).unwrap();
    let mut conn = db.conn().unwrap();
    // Isolated temporary shadow retains the full search row shape, while
    // allowing hostile NULL/MAX/orphan data that writers normally avoid.
    conn.execute_batch(
        "CREATE TEMP TABLE global_entries AS SELECT * FROM main.global_entries;
        DELETE FROM temp.global_entries;
        CREATE UNIQUE INDEX temp.idx_nullable_id ON global_entries(id);
        CREATE INDEX temp.idx_global_entries_volume ON global_entries(volume_id,is_stale);
        CREATE INDEX temp.idx_global_entries_active_name_order ON global_entries(name_normalized,modified_at_fs DESC,id ASC) WHERE is_stale=0;
        CREATE INDEX temp.idx_global_entries_active_extension_order ON global_entries(extension,modified_at_fs DESC,id ASC) WHERE is_stale=0;",
    )
    .unwrap();
    // Every row inherits a valid full search shape, then only facts are varied.
    for (id, volume_id, stale, seen) in [
        (Some("a1"), "a", 0, Some(10)),
        (None, "a", 0, Some(30)),
        (None, "a", 1, Some(999)),
        (Some("b1"), "b", 0, None),
        (None, "b", 0, Some(77)),
        (Some("d1"), "d", 1, Some(88)),
        (None, "orphan", 0, Some(100)),
        (Some("orphan"), "orphan", 0, Some(200)),
    ] {
        conn.execute(
            "INSERT INTO temp.global_entries SELECT * FROM main.global_entries",
            [],
        )
        .unwrap();
        conn.execute("UPDATE temp.global_entries SET id=?1, volume_id=?2, is_stale=?3, last_seen_at=?4 WHERE rowid = last_insert_rowid()",params![id,volume_id,stale,seen]).unwrap();
    }
    let tx = conn.transaction().unwrap();
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
        tx.execute("UPDATE global_volumes SET index_status=?1, last_error='old error', updated_at=10, last_incremental_sync_at=123, entry_count=9999",[status]).unwrap();
        tx.execute(
            "UPDATE global_volumes SET last_error='latest error', updated_at=20 WHERE id='c'",
            [],
        )
        .unwrap();
        for enabled in [true, false] {
            tx.execute(
                "UPDATE global_volumes SET enabled=?1 WHERE id='b'",
                [i64::from(enabled)],
            )
            .unwrap();
            let (baseline, base_probe) =
                capture(false, || snapshot_on_connection(&tx, "zzznomatchtoken"));
            let baseline = baseline.unwrap();
            let (candidate, probe) = capture(false, || {
                with_exact_count(|| snapshot_on_connection(&tx, "zzznomatchtoken"))
            });
            assert_snapshot_equal(&baseline, &candidate.unwrap());
            assert_eq!(probe.revision_facts_json, base_probe.revision_facts_json);
            let reference =
                load_global_search_source_health_candidate(&tx, Query::Original).unwrap();
            assert_eq!(probe.revision_facts_json, reference.revision_facts_json);
            assert_eq!(
                probe.exact_count_parts,
                Some((if enabled { 2 } else { 1 }, if enabled { 2 } else { 1 }))
            );
            assert_eq!(
                baseline.index_status.last_error.as_deref(),
                Some("latest error")
            );
        }
    }
    // Empty, all-disabled and all-stale cases must supply a real zero instead
    // of accidentally invoking the original COUNT(*) branch.
    for sql in [
        "UPDATE global_volumes SET enabled=0",
        "UPDATE global_volumes SET enabled=1; UPDATE global_entries SET is_stale=1",
        "DELETE FROM global_entries",
        "DELETE FROM global_volumes",
    ] {
        tx.execute_batch(sql).unwrap();
        let baseline = snapshot_on_connection(&tx, "zzznomatchtoken").unwrap();
        let (candidate, probe) = capture(false, || {
            with_exact_count(|| snapshot_on_connection(&tx, "zzznomatchtoken"))
        });
        assert_snapshot_equal(&baseline, &candidate.unwrap());
        assert_eq!(probe.exact_count_parts, Some((0, 0)));
        assert_eq!(baseline.index_status.total_entries, 0);
    }
    tx.rollback().unwrap();
    drop(conn);
    drop(db);
    let _ = std::fs::remove_file(path);
}
