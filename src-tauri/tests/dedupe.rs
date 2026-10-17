use rusqlite::{params, Connection};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use zen_canvas_tauri::{
    db::{Database, InsertFileRequest},
    dedupe::{
        run_duplicate_detection, run_duplicate_detection_with_hasher, ContentHasher, DedupeError,
        NoopDedupeEventEmitter,
    },
};

#[test]
fn current_schema_retains_content_hash_and_dedupe_index() {
    let db = Database::open(test_db_path()).expect("open test database");
    let conn = Connection::open(db.path()).expect("open migrated database");

    let version: i64 = conn
        .query_row("SELECT user_version FROM pragma_user_version", [], |row| {
            row.get(0)
        })
        .expect("schema version");
    let (content_hash_type, content_hash_notnull): (String, i64) = conn
        .query_row(
            "SELECT type, \"notnull\" FROM pragma_table_info('files') WHERE name = 'content_hash'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("content_hash column");
    let index_sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = 'idx_files_dedupe'",
            [],
            |row| row.get(0),
        )
        .expect("dedupe index");

    let cleanup_table_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name IN ('cleanup_trash_batches', 'cleanup_trash_items')",
            [],
            |row| row.get(0),
        )
        .expect("cleanup trash tables");

    assert_eq!(version, 42);
    assert_eq!(cleanup_table_count, 2);
    assert_eq!(content_hash_type, "TEXT");
    assert_eq!(content_hash_notnull, 1);
    assert!(index_sql.contains("files(size, content_hash)"));
    assert!(index_sql.contains("WHERE is_dir = 0 AND size > 0"));

    insert_virtual_file(&db, "default-hash.bin", 42, 1);
    let default_content_hash: String = conn
        .query_row(
            "SELECT content_hash FROM files WHERE id = '/test/virtual/default-hash.bin'",
            [],
            |row| row.get(0),
        )
        .expect("default content_hash");
    assert_eq!(default_content_hash, "");
}

#[test]
fn durable_run_persists_fingerprints_groups_and_invalidates_on_metadata_change() {
    let dir = test_dir("durable-ledger");
    let db = Database::open(dir.join("db.sqlite3")).expect("open database");
    insert_managed_root(&db, &dir, "root-ledger");
    let first = write_indexed_file(&db, &dir, "first.txt", b"same-content", 1);
    let second = write_indexed_file(&db, &dir, "second.txt", b"same-content", 2);
    write_indexed_file(&db, &dir, "different.txt", b"other-value", 3);

    let summary = run_duplicate_detection(&db, &NoopDedupeEventEmitter).expect("durable run");
    assert_eq!(summary.candidate_files, 2);
    assert_eq!(summary.duplicate_files, 2);
    let progress: (i64, i64, i64, i64) = Connection::open(db.path())
        .expect("open run database")
        .query_row(
            "SELECT processed_files, candidate_files, processed_bytes, total_bytes FROM dedupe_runs ORDER BY created_at DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("completed dedupe run progress");
    assert!(progress.0 <= progress.1);
    assert!(progress.2 <= progress.3);

    let mut warm_cache_hasher = CountingHasher::default();
    let warm_summary =
        run_duplicate_detection_with_hasher(&db, &NoopDedupeEventEmitter, &mut warm_cache_hasher)
            .expect("warm-cache dedupe run");
    assert_eq!(warm_summary.duplicate_files, 2);
    assert_eq!(warm_cache_hasher.calls, 0);

    let conn = Connection::open(db.path()).expect("open migrated database");
    let fingerprint_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM file_fingerprints", [], |row| {
            row.get(0)
        })
        .expect("fingerprint rows");
    let active_group_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM duplicate_groups WHERE status = 'active'",
            [],
            |row| row.get(0),
        )
        .expect("active groups");
    let active_members: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM active_duplicate_membership",
            [],
            |row| row.get(0),
        )
        .expect("active memberships");
    assert_eq!(fingerprint_count, 2);
    assert_eq!(active_group_count, 1);
    assert_eq!(active_members, 2);
    let (group_id, group_revision): (String, i64) = conn
        .query_row(
            "SELECT id, revision FROM duplicate_groups WHERE status = 'active' LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("initial deterministic group");
    let memberships: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM active_duplicate_membership WHERE file_id = ?1",
            [second.to_string_lossy().to_string()],
            |row| row.get(0),
        )
        .expect("file duplicate membership");
    assert_eq!(memberships, 1);

    let page = db
        .get_paged_files(Some(20), Some(0), None)
        .expect("paged files");
    assert!(page
        .files
        .iter()
        .any(|file| file.path == first.to_string_lossy() && file.is_duplicate));
    assert!(page
        .files
        .iter()
        .any(|file| file.path == second.to_string_lossy() && file.is_duplicate));

    let changed_path = first.to_string_lossy().to_string();
    fs::write(&first, b"changed").expect("change file");
    let metadata = fs::metadata(&first).expect("changed metadata");
    db.insert_file(InsertFileRequest {
        id: changed_path.clone(),
        path: changed_path,
        name: "first.txt".to_string(),
        extension: "txt".to_string(),
        size: i64::try_from(metadata.len()).expect("size"),
        mtime: metadata
            .modified()
            .expect("mtime")
            .duration_since(UNIX_EPOCH)
            .expect("unix mtime")
            .as_secs() as i64,
        ctime: 0,
        is_dir: false,
        state_code: 0,
    })
    .expect("invalidate changed fingerprint");
    let stale_group_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM duplicate_groups WHERE status = 'active'",
            [],
            |row| row.get(0),
        )
        .expect("active groups after invalidation");
    assert_eq!(stale_group_count, 0);
    let stale_fingerprint: String = conn
        .query_row(
            "SELECT fingerprint_status FROM file_fingerprints WHERE file_id = ?1",
            params![first.to_string_lossy().to_string()],
            |row| row.get(0),
        )
        .expect("stale fingerprint");
    assert_eq!(stale_fingerprint, "stale");

    fs::write(&first, b"same-content").expect("restore original content");
    let restored_metadata = fs::metadata(&first).expect("restored metadata");
    db.insert_file(InsertFileRequest {
        id: first.to_string_lossy().to_string(),
        path: first.to_string_lossy().to_string(),
        name: "first.txt".to_string(),
        extension: "txt".to_string(),
        size: i64::try_from(restored_metadata.len()).expect("restored size"),
        mtime: restored_metadata
            .modified()
            .expect("restored mtime")
            .duration_since(UNIX_EPOCH)
            .expect("restored unix mtime")
            .as_secs() as i64,
        ctime: 0,
        is_dir: false,
        state_code: 0,
    })
    .expect("restore indexed row");
    run_duplicate_detection(&db, &NoopDedupeEventEmitter).expect("republish restored group");
    let (restored_group_id, restored_revision): (String, i64) = conn
        .query_row(
            "SELECT id, revision FROM duplicate_groups WHERE status = 'active' LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("republished deterministic group");
    assert_eq!(restored_group_id, group_id);
    assert!(restored_revision > group_revision);
}

#[test]
fn stale_rename_fingerprint_is_reused_for_the_same_physical_file() {
    let dir = test_dir("durable-rename-cache");
    let db = Database::open(dir.join("db.sqlite3")).expect("open database");
    insert_managed_root(&db, &dir, "root-rename-cache");
    let old_path = write_indexed_file(&db, &dir, "old-name.bin", b"rename-cache", 1);
    write_indexed_file(&db, &dir, "peer.bin", b"rename-cache", 2);

    run_duplicate_detection(&db, &NoopDedupeEventEmitter).expect("seed durable cache");
    let old_path_text = old_path.to_string_lossy().to_string();
    let renamed_path = dir.join("new-name.bin");
    fs::rename(&old_path, &renamed_path).expect("rename physical file");
    db.remove_files_by_paths(std::slice::from_ref(&old_path_text))
        .expect("invalidate old path");
    insert_path_only_indexed_file(&db, &renamed_path, "new-name.bin");

    let mut hasher = CountingHasher::default();
    let summary = run_duplicate_detection_with_hasher(&db, &NoopDedupeEventEmitter, &mut hasher)
        .expect("reuse renamed fingerprint");

    assert_eq!(hasher.calls, 0, "rename should reuse the retained cache");
    assert_eq!(summary.duplicate_files, 2);
    let conn = Connection::open(db.path()).expect("open database connection");
    let (status, hash, mirror_hash): (String, String, String) = conn
        .query_row(
            "SELECT fingerprint_status, COALESCE(full_hash, ''), COALESCE((SELECT content_hash FROM files WHERE id = file_fingerprints.file_id), '') FROM file_fingerprints WHERE file_id = ?1",
            [renamed_path.to_string_lossy().to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("renamed fingerprint");
    assert_eq!(status, "complete");
    assert!(!hash.is_empty());
    assert_eq!(
        mirror_hash, hash,
        "files.content_hash must mirror the full hash"
    );
}

#[test]
fn prehash_prunes_distinct_large_samples_and_full_hashes_collisions_without_false_groups() {
    let dir = test_dir("dedupe-prehash-stages");
    let db = Database::open(dir.join("db.sqlite3")).expect("open database");
    insert_managed_root(&db, &dir, "root-prehash-stages");
    let size = 1024 * 1024;

    let mut distinct_a = vec![0_u8; size];
    distinct_a[..4096].fill(1);
    distinct_a[size - 4096..].fill(9);
    let mut distinct_b = distinct_a.clone();
    distinct_b[..4096].fill(2);
    let mut collision_a = vec![0_u8; size];
    collision_a[..4096].fill(7);
    collision_a[size - 4096..].fill(8);
    collision_a[4096..size - 4096].fill(3);
    let mut collision_b = collision_a.clone();
    collision_b[4096..size - 4096].fill(4);

    write_indexed_file(&db, &dir, "distinct-a.bin", &distinct_a, 1);
    write_indexed_file(&db, &dir, "distinct-b.bin", &distinct_b, 2);
    write_indexed_file(&db, &dir, "collision-a.bin", &collision_a, 3);
    write_indexed_file(&db, &dir, "collision-b.bin", &collision_b, 4);

    let mut hasher = CountingHasher::default();
    let summary = run_duplicate_detection_with_hasher(&db, &NoopDedupeEventEmitter, &mut hasher)
        .expect("run staged prehash");

    assert_eq!(summary.candidate_files, 4);
    assert_eq!(summary.hashed_files, 2);
    assert_eq!(hasher.calls, 2);
    assert_eq!(summary.duplicate_files, 0);
    let conn = Connection::open(db.path()).expect("open database connection");
    let prehash_pruned: i64 = conn
        .query_row(
            "SELECT prehash_pruned_files FROM dedupe_runs ORDER BY created_at DESC, id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("prehash pruning count");
    assert_eq!(prehash_pruned, 2);
    let active_groups: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM duplicate_groups WHERE status = 'active'",
            [],
            |row| row.get(0),
        )
        .expect("active groups");
    assert_eq!(active_groups, 0);
}

#[test]
fn fingerprint_algorithm_version_change_forces_full_hash_again() {
    let dir = test_dir("dedupe-fingerprint-version");
    let db = Database::open(dir.join("db.sqlite3")).expect("open database");
    insert_managed_root(&db, &dir, "root-fingerprint-version");
    write_indexed_file(&db, &dir, "version-a.bin", b"versioned-content", 1);
    write_indexed_file(&db, &dir, "version-b.bin", b"versioned-content", 2);
    run_duplicate_detection(&db, &NoopDedupeEventEmitter).expect("seed fingerprint cache");

    let conn = Connection::open(db.path()).expect("open database connection");
    conn.execute(
        "UPDATE file_fingerprints SET prehash_algorithm = 'legacy', prehash_version = 99, full_hash_algorithm = 'legacy', full_hash_version = 99",
        [],
    )
    .expect("invalidate algorithm version");
    drop(conn);

    let mut hasher = CountingHasher::default();
    let summary = run_duplicate_detection_with_hasher(&db, &NoopDedupeEventEmitter, &mut hasher)
        .expect("recompute changed algorithm cache");
    assert_eq!(hasher.calls, 2);
    assert_eq!(summary.duplicate_files, 0);
}

#[test]
fn modified_ns_detects_same_second_same_size_content_change() {
    let dir = test_dir("dedupe-modified-ns");
    let db = Database::open(dir.join("db.sqlite3")).expect("open database");
    insert_managed_root(&db, &dir, "root-modified-ns");
    let size = 1024 * 1024;
    let mut original = vec![0_u8; size];
    original[..4096].fill(5);
    original[size - 4096..].fill(6);
    original[4096..size - 4096].fill(1);
    let mut changed = original.clone();
    changed[4096..size - 4096].fill(2);
    let changed_path = write_indexed_file(&db, &dir, "modified-ns-a.bin", &original, 1);
    write_indexed_file(&db, &dir, "modified-ns-b.bin", &original, 2);
    run_duplicate_detection(&db, &NoopDedupeEventEmitter).expect("seed same-content group");

    let old_mtime: i64 = Connection::open(db.path())
        .expect("open database connection")
        .query_row(
            "SELECT mtime FROM files WHERE path = ?1",
            [changed_path.to_string_lossy().to_string()],
            |row| row.get(0),
        )
        .expect("old mtime");
    std::thread::sleep(std::time::Duration::from_millis(5));
    fs::write(&changed_path, &changed).expect("change content");
    Connection::open(db.path())
        .expect("open database connection")
        .execute(
            "UPDATE files SET mtime = ?1 WHERE path = ?2",
            rusqlite::params![old_mtime, changed_path.to_string_lossy().to_string()],
        )
        .expect("retain same-second indexed mtime");

    let mut hasher = CountingHasher::default();
    let summary = run_duplicate_detection_with_hasher(&db, &NoopDedupeEventEmitter, &mut hasher)
        .expect("detect modified nanosecond identity");
    assert_eq!(hasher.calls, 1);
    assert_eq!(summary.duplicate_files, 0);
}

#[test]
fn hardlink_aliases_share_identity_without_becoming_duplicate_copies() {
    let dir = test_dir("durable-hardlink");
    let db = Database::open(dir.join("db.sqlite3")).expect("open database");
    insert_managed_root(&db, &dir, "root-hardlink");
    let first = write_indexed_file(&db, &dir, "hardlink-a.bin", b"hardlink-content", 1);
    let alias = dir.join("hardlink-b.bin");
    fs::hard_link(&first, &alias).expect("create hardlink");
    insert_path_only_indexed_file(&db, &alias, "hardlink-b.bin");

    let summary =
        run_duplicate_detection(&db, &NoopDedupeEventEmitter).expect("durable hardlink run");
    assert_eq!(summary.duplicate_files, 0);
    let conn = Connection::open(db.path()).expect("open database connection");
    let physical_keys: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT physical_key) FROM file_fingerprints WHERE physical_key IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .expect("physical keys");
    let aliases: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM file_fingerprints WHERE link_count > 1",
            [],
            |row| row.get(0),
        )
        .expect("hardlink count");
    assert_eq!(physical_keys, 1);
    assert_eq!(aliases, 2);
}

#[test]
fn hardlink_alias_and_true_copy_have_physical_reclaim_semantics() {
    let dir = test_dir("durable-hardlink-copy");
    let db = Database::open(dir.join("db.sqlite3")).expect("open database");
    insert_managed_root(&db, &dir, "root-hardlink-copy");
    let first = write_indexed_file(&db, &dir, "original.bin", b"same-physical-content", 1);
    let alias = dir.join("alias.bin");
    fs::hard_link(&first, &alias).expect("create hardlink");
    insert_path_only_indexed_file(&db, &alias, "alias.bin");
    write_indexed_file(&db, &dir, "true-copy.bin", b"same-physical-content", 2);

    let summary = run_duplicate_detection(&db, &NoopDedupeEventEmitter).expect("run dedupe");
    assert_eq!(summary.duplicate_files, 3);
    let conn = Connection::open(db.path()).expect("open database connection");
    let (member_count, physical_copy_count, hardlink_alias_count, exact, potential, confidence): (
        i64,
        i64,
        i64,
        Option<i64>,
        i64,
        String,
    ) = conn
        .query_row(
            "SELECT member_count, physical_copy_count, hardlink_alias_count, exact_reclaimable_bytes, potential_reclaimable_bytes, reclaimable_confidence FROM duplicate_groups WHERE status = 'active'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        )
        .expect("duplicate group");
    let size = i64::try_from(b"same-physical-content".len()).expect("size");
    assert_eq!(member_count, 3);
    assert_eq!(physical_copy_count, 2);
    assert_eq!(hardlink_alias_count, 1);
    assert_eq!(exact, Some(size));
    assert_eq!(potential, size);
    assert_eq!(confidence, "exact");
    let physical_keys: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT physical_key) FROM file_fingerprints WHERE physical_key IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .expect("distinct physical keys");
    assert_eq!(physical_keys, 2);
}

#[test]
fn duplicate_detection_marks_only_same_size_same_content_files_as_duplicates() {
    let dir = test_dir("dedupe-content");
    let db = Database::open(dir.join("db.sqlite3")).expect("open test database");
    insert_managed_root(&db, &dir, "root-content");
    let duplicate_a = write_indexed_file(&db, &dir, "duplicate-a.txt", b"abc123abc123", 10);
    let duplicate_b = write_indexed_file(&db, &dir, "duplicate-b.txt", b"abc123abc123", 11);
    let same_size_different = write_indexed_file(&db, &dir, "different.txt", b"xyz789xyz789", 12);
    Connection::open(db.path())
        .expect("open database connection")
        .execute("UPDATE files SET content_hash = 'legacy-shared-hash'", [])
        .expect("seed legacy compatibility hashes");

    let summary =
        run_duplicate_detection(&db, &NoopDedupeEventEmitter).expect("run duplicate detection");
    let page = db.get_paged_files(Some(10), Some(0), None).expect("page");
    let files = page
        .files
        .iter()
        .map(|file| (file.path.clone(), file))
        .collect::<HashMap<_, _>>();

    let duplicate_a = files
        .get(&duplicate_a.to_string_lossy().to_string())
        .expect("duplicate a");
    let duplicate_b = files
        .get(&duplicate_b.to_string_lossy().to_string())
        .expect("duplicate b");
    let same_size_different = files
        .get(&same_size_different.to_string_lossy().to_string())
        .expect("different content");

    assert_eq!(summary.candidate_files, 3);
    // Small files below PREHASH_MIN_SIZE are intentionally read once in the
    // full-hash path; both equal-size candidates and the same-size mismatch
    // therefore count as real hash work.
    assert_eq!(summary.hashed_files, 3);
    assert_eq!(summary.duplicate_files, 2);
    assert!(duplicate_a.is_duplicate);
    assert!(duplicate_b.is_duplicate);
    assert!(!same_size_different.is_duplicate);
    assert_eq!(duplicate_a.hash, duplicate_b.hash);
    assert_ne!(duplicate_a.hash, same_size_different.hash);
    assert!(duplicate_a
        .hash
        .as_deref()
        .is_some_and(|hash| !hash.is_empty()));

    let stats = db.get_stats_summary().expect("stats");
    assert_eq!(stats.duplicate_files, 2);
}

#[test]
fn unique_file_sizes_do_not_trigger_hash_calculation() {
    let dir = test_dir("dedupe-unique");
    let db = Database::open(dir.join("db.sqlite3")).expect("open test database");
    insert_managed_root(&db, &dir, "root-unique");
    for index in 0..128 {
        let bytes = vec![b'x'; index + 1];
        write_indexed_file(
            &db,
            &dir,
            &format!("unique-{index}.bin"),
            &bytes,
            index as i64,
        );
    }
    let mut hasher = CountingHasher::default();

    let summary = run_duplicate_detection_with_hasher(&db, &NoopDedupeEventEmitter, &mut hasher)
        .expect("run duplicate detection");

    assert_eq!(summary.candidate_files, 0);
    assert_eq!(summary.hashed_files, 0);
    assert_eq!(hasher.calls, 0);
}

#[test]
fn duplicate_detection_hashes_new_file_when_matching_size_already_has_hash() {
    let dir = test_dir("dedupe-incremental");
    let db = Database::open(dir.join("db.sqlite3")).expect("open test database");
    insert_managed_root(&db, &dir, "root-incremental");
    let bytes = b"already-hashed";
    let existing = write_indexed_file(&db, &dir, "existing.txt", bytes, 20);
    let new_file = write_indexed_file(&db, &dir, "new.txt", bytes, 21);
    let existing_hash = blake3::hash(bytes).to_hex().to_string();
    let conn = Connection::open(db.path()).expect("open database connection");
    conn.execute(
        "UPDATE files SET content_hash = ?1 WHERE path = ?2",
        (&existing_hash, existing.to_string_lossy().to_string()),
    )
    .expect("seed existing hash");

    let summary =
        run_duplicate_detection(&db, &NoopDedupeEventEmitter).expect("run duplicate detection");
    let page = db.get_paged_files(Some(10), Some(0), None).expect("page");
    let files = page
        .files
        .iter()
        .map(|file| (file.path.clone(), file))
        .collect::<HashMap<_, _>>();
    let existing = files
        .get(&existing.to_string_lossy().to_string())
        .expect("existing file");
    let new_file = files
        .get(&new_file.to_string_lossy().to_string())
        .expect("new file");

    assert_eq!(summary.candidate_files, 2);
    assert_eq!(summary.hashed_files, 2);
    assert_eq!(summary.duplicate_files, 2);
    assert_eq!(existing.hash.as_deref(), Some(existing_hash.as_str()));
    assert_eq!(new_file.hash.as_deref(), Some(existing_hash.as_str()));
    assert!(existing.is_duplicate);
    assert!(new_file.is_duplicate);
}

#[test]
fn duplicate_detection_does_not_persist_a_hash_when_file_changes_during_hashing() {
    let dir = test_dir("dedupe-race");
    let db = Database::open(dir.join("db.sqlite3")).expect("open test database");
    insert_managed_root(&db, &dir, "root-race");
    let changing = write_indexed_file(&db, &dir, "changing.txt", b"same-content", 30);
    write_indexed_file(&db, &dir, "peer.txt", b"same-content", 31);
    let mut hasher = MutatingHasher {
        target: changing.clone(),
    };

    let summary = run_duplicate_detection_with_hasher(&db, &NoopDedupeEventEmitter, &mut hasher)
        .expect("run duplicate detection");
    let conn = Connection::open(db.path()).expect("open database connection");
    let stored_hash: String = conn
        .query_row(
            "SELECT content_hash FROM files WHERE path = ?1",
            [changing.to_string_lossy().to_string()],
            |row| row.get(0),
        )
        .expect("stored hash");

    assert!(summary.error_files >= 1);
    assert_eq!(stored_hash, "");
}

#[derive(Default)]
struct CountingHasher {
    calls: usize,
}

struct MutatingHasher {
    target: PathBuf,
}

impl ContentHasher for MutatingHasher {
    fn hash_file(&mut self, path: &Path) -> Result<String, DedupeError> {
        if path == self.target {
            fs::write(path, b"changed-size").expect("mutate file while hashing");
        }
        Ok("synthetic-hash".to_string())
    }
}

impl ContentHasher for CountingHasher {
    fn hash_file(&mut self, path: &Path) -> Result<String, DedupeError> {
        self.calls += 1;
        Ok(format!("hash:{}", path.display()))
    }
}

fn write_indexed_file(db: &Database, dir: &Path, name: &str, bytes: &[u8], _mtime: i64) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, bytes).expect("write file");
    let metadata = fs::metadata(&path).expect("file metadata");
    let mtime = metadata
        .modified()
        .expect("modified time")
        .duration_since(UNIX_EPOCH)
        .expect("unix mtime")
        .as_secs() as i64;
    db.insert_file(InsertFileRequest {
        id: path.to_string_lossy().into_owned(),
        path: path.to_string_lossy().into_owned(),
        name: name.to_string(),
        extension: "txt".to_string(),
        size: i64::try_from(bytes.len()).expect("test size fits i64"),
        mtime,
        ctime: 0,
        is_dir: false,
        state_code: 0,
    })
    .expect("insert file");
    path
}

fn insert_path_only_indexed_file(db: &Database, path: &Path, name: &str) {
    let metadata = fs::metadata(path).expect("file metadata");
    db.insert_file(InsertFileRequest {
        id: path.to_string_lossy().into_owned(),
        path: path.to_string_lossy().into_owned(),
        name: name.to_string(),
        extension: "bin".to_string(),
        size: i64::try_from(metadata.len()).expect("size"),
        mtime: metadata
            .modified()
            .expect("modified")
            .duration_since(UNIX_EPOCH)
            .expect("unix mtime")
            .as_secs() as i64,
        ctime: 0,
        is_dir: false,
        state_code: 0,
    })
    .expect("insert hardlink file");
}

fn insert_managed_root(db: &Database, dir: &Path, id: &str) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("unix time")
        .as_secs() as i64;
    let conn = Connection::open(db.path()).expect("open database connection");
    conn.execute(
        "INSERT INTO scan_roots (id, normalized_path, display_name, source_kind, enabled, health_status, current_generation, needs_reconciliation, created_at, updated_at) VALUES (?1, ?2, ?3, 'file_library', 1, 'healthy', 0, 0, ?4, ?4)",
        params![id, dir.to_string_lossy().to_string(), id, now],
    )
    .expect("insert managed root");
}

fn insert_virtual_file(db: &Database, name: &str, size: usize, mtime: i64) {
    let path = format!("/test/virtual/{name}");
    db.insert_file(InsertFileRequest {
        id: path.clone(),
        path,
        name: name.to_string(),
        extension: "bin".to_string(),
        size: i64::try_from(size).expect("test size fits i64"),
        mtime,
        ctime: 0,
        is_dir: false,
        state_code: 0,
    })
    .expect("insert file");
}

fn test_db_path() -> PathBuf {
    test_dir("dedupe-db").join("zen-canvas-dedupe-test.sqlite3")
}

fn test_dir(prefix: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("zen-canvas-{prefix}-{nonce}"));
    fs::create_dir_all(&dir).expect("create test dir");
    dir
}
