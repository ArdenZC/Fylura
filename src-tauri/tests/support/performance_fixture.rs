use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

use rusqlite::{params, Connection};
use zen_canvas_tauri::db::{Database, InsertFileRequest};

pub const ROOT_ID: &str = "task05-benchmark-root";
pub const ROOT_PATH: &str = "/task05/benchmark-library";
pub const TAG_A: &str = "task05-benchmark-tag-a";
pub const TAG_B: &str = "task05-benchmark-tag-b";
pub const FIXTURE_FORMAT_VERSION: i64 = 1;
pub const FIXTURE_SCHEMA_VERSION: i64 = 42;

pub fn fixture_root() -> Option<PathBuf> {
    std::env::var_os("ZC_PERF_FIXTURE_ROOT").map(PathBuf::from)
}

pub fn library_fixture_path(root: &Path, row_count: usize) -> PathBuf {
    root.join(format!("file-library-{row_count}.sqlite3"))
}

pub fn library_working_copy_path(root: &Path, row_count: usize, purpose: &str) -> PathBuf {
    assert!(
        matches!(purpose, "query" | "library-migration" | "content-migration"),
        "unsupported performance fixture purpose: {purpose}"
    );
    root.join(format!("file-library-{row_count}-{purpose}.sqlite3"))
}

pub fn seed_library(path: &Path, row_count: usize) -> Database {
    if std::env::var("ZC_PERF_FIXTURE_BUILD").as_deref() != Ok("1") {
        if let Some(root) = fixture_root() {
            let source = library_fixture_path(&root, row_count);
            if source.exists() {
                copy_and_validate_fixture(&source, path, row_count);
                return Database::open(path).expect("open reusable file-library fixture");
            }
            if std::env::var("ZC_PERF_FIXTURE_REQUIRED").as_deref() == Ok("1") {
                panic!(
                    "required reusable file-library fixture is missing: {}",
                    source.display()
                );
            }
        }
    }
    seed_library_fresh(path, row_count)
}

pub fn seed_library_for_benchmark(path: &Path, row_count: usize, purpose: &str) -> Database {
    let started = Instant::now();
    if std::env::var("ZC_PERF_PREPARED_WORKING_COPY").as_deref() == Ok("1") {
        let root = std::env::var_os("ZC_PERF_FIXTURE_ROOT")
            .map(PathBuf::from)
            .expect("ZC_PERF_FIXTURE_ROOT for prepared performance fixture");
        let source = library_working_copy_path(&root, row_count, purpose);
        if !source.exists() {
            panic!(
                "required prepared working fixture is missing: {}",
                source.display()
            );
        }
        validate_fixture_shape(&source, row_count);
        if path.exists() {
            fs::remove_file(path).expect("remove benchmark destination");
        }
        copy_prepared_fixture(&source, path);
        validate_fixture_shape(path, row_count);
        println!(
            "[perf-phase] suite=library-content phase=fixture-restore purpose={purpose} rows={row_count} ms={}",
            started.elapsed().as_millis()
        );
        return Database::open(path).expect("open prepared file-library fixture");
    }
    let db = seed_library(path, row_count);
    println!(
        "[perf-phase] suite=library-content phase=fixture-restore purpose={purpose} rows={row_count} ms={} fallback=fresh-or-base",
        started.elapsed().as_millis()
    );
    db
}

pub fn prepare_library_fixture(root: &Path, row_count: usize) {
    fs::create_dir_all(root).expect("create performance fixture root");
    let path = library_fixture_path(root, row_count);
    if path.exists() {
        validate_fixture(&path, row_count);
        return;
    }
    let db = seed_library_fresh(&path, row_count);
    checkpoint_and_validate(&path, row_count, db);
}

pub fn prepare_library_working_copy(
    source_root: &Path,
    destination_root: &Path,
    row_count: usize,
    purpose: &str,
) {
    let source = library_fixture_path(source_root, row_count);
    let destination = library_working_copy_path(destination_root, row_count, purpose);
    if destination.exists() {
        validate_fixture_shape(&destination, row_count);
        return;
    }
    validate_fixture_shape(&source, row_count);
    fs::create_dir_all(destination_root).expect("create working fixture root");
    let copied_bytes = fs::copy(&source, &destination).expect("copy disposable working fixture");
    let source_bytes = fs::metadata(&source)
        .expect("read source fixture metadata")
        .len();
    assert_eq!(
        copied_bytes, source_bytes,
        "prepared working fixture copy is incomplete"
    );
    validate_fixture_shape(&destination, row_count);
}

pub fn validate_fixture(path: &Path, expected_rows: usize) {
    assert_no_wal_sidecars(path);
    let connection = Connection::open(path).expect("open reusable fixture for validation");
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .expect("run fixture integrity check");
    assert_eq!(
        integrity,
        "ok",
        "fixture integrity check failed: {}",
        path.display()
    );
    let rows: i64 = connection
        .query_row("SELECT COUNT(*) FROM files WHERE is_stale = 0", [], |row| {
            row.get(0)
        })
        .expect("read fixture row count");
    assert_eq!(rows, expected_rows as i64, "fixture row count mismatch");
    validate_fixture_structure(&connection, path);
    drop(connection);
    assert_no_wal_sidecars(path);
}

fn validate_fixture_shape(path: &Path, expected_rows: usize) {
    assert_no_wal_sidecars(path);
    let connection = Connection::open(path).expect("open prepared fixture for shape validation");
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .expect("run prepared fixture integrity check");
    assert_eq!(
        integrity,
        "ok",
        "prepared fixture integrity check failed: {}",
        path.display()
    );
    let rows: i64 = connection
        .query_row("SELECT COUNT(*) FROM files WHERE is_stale = 0", [], |row| {
            row.get(0)
        })
        .expect("read prepared fixture row count");
    assert_eq!(
        rows, expected_rows as i64,
        "prepared fixture row count mismatch"
    );
    validate_fixture_structure(&connection, path);
    drop(connection);
    assert_no_wal_sidecars(path);
}

fn validate_fixture_structure(connection: &Connection, path: &Path) {
    let schema_version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("read prepared fixture schema version");
    assert_eq!(
        schema_version,
        FIXTURE_SCHEMA_VERSION,
        "prepared fixture schema version mismatch: {}",
        path.display()
    );
    let generation_tables: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name IN
         ('global_index_generations', 'global_index_generation_heads',
          'global_index_generation_entries', 'global_index_generation_fts')",
            [],
            |row| row.get(0),
        )
        .expect("read Schema42 fixture foundation");
    assert_eq!(
        generation_tables,
        4,
        "prepared fixture Schema42 foundation is incomplete: {}",
        path.display()
    );
    for table in [
        "global_index_generations",
        "global_index_generation_entries",
        "global_index_generation_fts",
    ] {
        let count: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("read dormant fixture generation rows");
        assert_eq!(
            count,
            0,
            "prepared fixture has staged/published generation rows: {}",
            path.display()
        );
    }
    let active_heads: i64 = connection.query_row(
        "SELECT COUNT(*) FROM global_index_generation_heads WHERE legacy_state <> 'LEGACY_UNVERIFIED'
         OR published_generation_id IS NOT NULL OR candidate_generation_id IS NOT NULL", [], |row| row.get(0)
    ).expect("read dormant fixture generation heads");
    assert_eq!(
        active_heads,
        0,
        "prepared fixture activates generation authority: {}",
        path.display()
    );
    let library_index_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'index' AND name IN ('idx_library_files_modified', 'idx_library_files_created', 'idx_library_files_name', 'idx_library_files_size', 'idx_library_files_confidence')",
            [],
            |row| row.get(0),
        )
        .expect("read prepared fixture library indexes");
    assert_eq!(
        library_index_count,
        5,
        "prepared fixture library index set is incomplete: {}",
        path.display()
    );
}

fn copy_prepared_fixture(source: &Path, destination: &Path) {
    if destination.exists() {
        fs::remove_file(destination).expect("remove benchmark working fixture");
    }
    let source_bytes = fs::metadata(source)
        .expect("read prepared fixture metadata")
        .len();
    let copied_bytes = fs::copy(source, destination).expect("copy prepared working fixture");
    assert_eq!(
        copied_bytes, source_bytes,
        "prepared working fixture copy is incomplete"
    );
}

fn assert_no_wal_sidecars(path: &Path) {
    let wal_path = PathBuf::from(format!("{}-wal", path.display()));
    let shm_path = PathBuf::from(format!("{}-shm", path.display()));
    assert!(
        !wal_path.exists(),
        "fixture has an active WAL: {}",
        wal_path.display()
    );
    assert!(
        !shm_path.exists(),
        "fixture has an active SHM file: {}",
        shm_path.display()
    );
}

fn copy_and_validate_fixture(source: &Path, destination: &Path, expected_rows: usize) {
    validate_fixture(source, expected_rows);
    if destination.exists() {
        fs::remove_file(destination).expect("remove benchmark working database");
    }
    fs::copy(source, destination).expect("copy reusable fixture to working database");
    validate_fixture(destination, expected_rows);
}

fn checkpoint_and_validate(path: &Path, expected_rows: usize, db: Database) {
    drop(db);
    let connection = Connection::open(path).expect("open fixture for checkpoint");
    connection
        .execute_batch(
            "PRAGMA wal_checkpoint(TRUNCATE); PRAGMA optimize; PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;",
        )
        .expect("checkpoint reusable fixture");
    drop(connection);
    validate_fixture(path, expected_rows);
}

fn seed_library_fresh(path: &Path, row_count: usize) -> Database {
    let db = Database::open(path).expect("open benchmark database");
    let conn = Connection::open(path).expect("open benchmark seed connection");
    let rebuild_fts_after_seed = row_count >= 1_000_000;
    conn.execute(
        "INSERT INTO scan_roots (id, normalized_path, display_name, source_kind, enabled, health_status, current_generation, needs_reconciliation, created_at, updated_at) VALUES (?1, ?2, 'Task 05 benchmark', 'file_library', 1, 'healthy', 1, 0, 1, 1)",
        params![ROOT_ID, ROOT_PATH],
    )
    .expect("seed benchmark root");
    conn.execute(
        "INSERT INTO user_tags (id, display_name, normalized_name, color_token, created_at, updated_at) VALUES (?1, 'Benchmark A', 'benchmark a', 'blue', 1, 1), (?2, 'Benchmark B', 'benchmark b', 'green', 1, 1)",
        params![TAG_A, TAG_B],
    )
    .expect("seed benchmark tags");
    if rebuild_fts_after_seed {
        conn.execute_batch(
            "DROP TRIGGER IF EXISTS files_ai; DROP TRIGGER IF EXISTS files_ad; DROP TRIGGER IF EXISTS files_au;",
        )
        .expect("suspend FTS triggers for the 1M fixture");
    }
    let tx = conn
        .unchecked_transaction()
        .expect("start benchmark transaction");
    for index in 0..row_count {
        let file = benchmark_insert_request(index);
        tx.execute(
            "INSERT INTO files (id, path, name, extension, size, mtime, ctime, is_dir, state_code, file_type, purpose, lifecycle, context, risk_level, confidence, classification_status, requires_confirmation, suggested_action, is_stale, last_seen_at) VALUES (?1, ?2, ?3, 'pdf', ?4, ?5, ?6, 0, 0, 'Document', 'Work', 'Active', '', 'Normal', ?7, 'classified', ?8, ?9, 0, ?5)",
            params![
                file.id,
                file.path,
                file.name,
                file.size,
                file.mtime,
                file.ctime,
                (index % 100) as f64 / 100.0,
                i64::from(index % 10 == 0),
                if index % 20 == 0 { "Review" } else { "Keep" },
            ],
        )
        .expect("seed benchmark file");
        if index % 10 < 8 {
            tx.execute(
                "INSERT INTO file_user_tags(file_id, tag_id, created_at) VALUES (?1, ?2, 1)",
                params![file.id, if index % 2 == 0 { TAG_A } else { TAG_B }],
            )
            .expect("seed benchmark tag assignment");
        }
    }
    tx.commit().expect("commit benchmark files");
    if rebuild_fts_after_seed {
        conn.execute_batch(
            r#"
            INSERT INTO files_fts(files_fts) VALUES('rebuild');
            CREATE TRIGGER files_ai AFTER INSERT ON files BEGIN
                INSERT INTO files_fts(rowid, name, path) VALUES (new.rowid, new.name, new.path);
            END;
            CREATE TRIGGER files_ad AFTER DELETE ON files BEGIN
                INSERT INTO files_fts(files_fts, rowid, name, path)
                VALUES('delete', old.rowid, old.name, old.path);
            END;
            CREATE TRIGGER files_au AFTER UPDATE ON files BEGIN
                INSERT INTO files_fts(files_fts, rowid, name, path)
                VALUES('delete', old.rowid, old.name, old.path);
                INSERT INTO files_fts(rowid, name, path) VALUES (new.rowid, new.name, new.path);
            END;
            INSERT INTO files_fts(files_fts) VALUES('optimize');
            PRAGMA optimize;
            "#,
        )
        .expect("rebuild and restore FTS triggers for the 1M fixture");
    }
    drop(conn);
    db
}

pub fn benchmark_insert_request(index: usize) -> InsertFileRequest {
    let name = if index.is_multiple_of(100) {
        format!("report-{index:07}.pdf")
    } else {
        format!("asset-{index:07}.pdf")
    };
    InsertFileRequest {
        id: format!("task05-file-{index:07}"),
        path: format!("{ROOT_PATH}/{name}"),
        name,
        size: 4_096 + index as i64,
        mtime: 1_700_000_000 + index as i64,
        ctime: 1_600_000_000 + index as i64,
        extension: "pdf".into(),
        is_dir: false,
        state_code: 0,
    }
}

// Test/benchmark fixture teardown only: historical input schemas must not
// retain current-generation objects before exercising the real migration.
pub fn drop_schema42_for_historical_fixture(conn: &Connection) {
    conn.execute_batch(
        "DROP TRIGGER IF EXISTS global_index_generation_entries_ai;
         DROP TRIGGER IF EXISTS global_index_generation_entries_insert_guard;
         DROP TRIGGER IF EXISTS global_index_generation_entries_no_update;
         DROP TRIGGER IF EXISTS global_index_generation_entries_no_delete;
         DROP TABLE IF EXISTS global_index_generation_fts;
         DROP TABLE IF EXISTS global_index_generation_entries;
         DROP TABLE IF EXISTS global_index_generation_heads;
         DROP TABLE IF EXISTS global_index_generations;",
    )
    .expect("remove only Schema42 objects from historical performance fixture");
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixtureDirectory(PathBuf);
    impl FixtureDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("zc434-fixture-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for FixtureDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn restore_schema41(path: &Path) {
        let conn = Connection::open(path).unwrap();
        drop_schema42_for_historical_fixture(&conn);
        conn.execute_batch("PRAGMA user_version = 41;").unwrap();
    }

    fn version(path: &Path) -> i64 {
        Connection::open(path)
            .unwrap()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn schema42_fixture_rejects_schema41_cache_and_working_copy_without_relabeling() {
        let root = FixtureDirectory::new();
        prepare_library_fixture(&root.0, 8);
        let source = library_fixture_path(&root.0, 8);
        validate_fixture(&source, 8);
        prepare_library_working_copy(&root.0, &root.0, 8, "query");
        let working = library_working_copy_path(&root.0, 8, "query");
        validate_fixture_shape(&working, 8);
        assert_eq!(version(&source), 42);
        assert_eq!(version(&working), 42);
        restore_schema41(&source);
        assert!(std::panic::catch_unwind(|| prepare_library_fixture(&root.0, 8)).is_err());
        let untouched = fs::read(&working).unwrap();
        assert!(
            std::panic::catch_unwind(|| copy_and_validate_fixture(&source, &working, 8)).is_err()
        );
        assert_eq!(
            fs::read(&working).unwrap(),
            untouched,
            "bad cache cannot replace a good working copy"
        );
        assert!(std::panic::catch_unwind(|| prepare_library_working_copy(
            &root.0,
            &root.0,
            8,
            "library-migration"
        ))
        .is_err());
        assert!(!library_working_copy_path(&root.0, 8, "library-migration").exists());
        restore_schema41(&working);
        assert!(std::panic::catch_unwind(|| prepare_library_working_copy(
            &root.0, &root.0, 8, "query"
        ))
        .is_err());
        assert!(std::panic::catch_unwind(|| validate_fixture_shape(&working, 8)).is_err());
        assert_eq!(
            version(&source),
            41,
            "validation must not migrate stale cache data"
        );
        assert_eq!(
            version(&working),
            41,
            "validation must not relabel stale working data"
        );
    }

    #[test]
    fn schema42_fixture_rejects_a_forged_version_with_missing_foundation() {
        let root = FixtureDirectory::new();
        prepare_library_fixture(&root.0, 8);
        let path = library_fixture_path(&root.0, 8);
        restore_schema41(&path);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA user_version = 42").unwrap();
        drop(conn);
        assert!(std::panic::catch_unwind(|| validate_fixture(&path, 8)).is_err());
        assert!(std::panic::catch_unwind(|| validate_fixture_shape(&path, 8)).is_err());
        assert_eq!(version(&path), 42);
    }
}
