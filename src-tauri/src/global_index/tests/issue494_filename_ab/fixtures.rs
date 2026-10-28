//! Fixtures for the #494 experiment.
//!
//! Two fixtures are used, and every reported number names the fixture it came
//! from:
//!
//! * `official` – the existing `populate_synthetic_database` generator from
//!   `global_search_benchmark`, unchanged. It is what the historical 100 ms gate
//!   and the 331 ms / 347 ms 1M FTS numbers were measured on, so it stays the
//!   only source for scale comparisons.
//! * `density` – a controlled fixture that guarantees exact hit counts (1, 40,
//!   5 000, 50 000), a path-only class, an extension class, Chinese, punctuation,
//!   mixed case, high-tie groups, stale rows and a disabled volume. The official
//!   generator cannot produce those counts on demand.

use super::super::{configured_connection, populate_synthetic_database, remove_database_files};
use crate::db::Database;
use crate::global_index::models::GlobalEntryInput;
use crate::global_index::tests::{test_db_path, test_volume};
use rusqlite::Connection;
use std::path::PathBuf;

/// A disposable fixture database plus its pooled handle.
pub(super) struct Fixture {
    pub path: PathBuf,
    db: Option<Database>,
}

impl Fixture {
    pub(super) fn db(&self) -> &Database {
        self.db.as_ref().expect("fixture database")
    }

    pub(super) fn connection(&self) -> Connection {
        configured_connection(&self.path).expect("open fixture read connection")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.db.take();
        remove_database_files(&self.path);
    }
}

fn open_fixture() -> Fixture {
    let path = test_db_path();
    let db = Database::open(&path).expect("open migrated fixture");
    db.upsert_global_volume(&test_volume())
        .expect("authorize fixture volume");
    Fixture { path, db: Some(db) }
}

/// The official 12-class synthetic fixture. `entries` must be one of the values
/// the official generator accepts.
pub(super) fn official(entries: u64) -> Fixture {
    let fixture = open_fixture();
    let _ = populate_synthetic_database(fixture.db(), entries);
    fixture
}

/// Query → guaranteed hit count for the density fixture. `None` means the count
/// is asserted by the correctness tests rather than fixed here.
pub(super) const DENSITY_QUERIES: [(&str, u64); 9] = [
    ("density-singleton", 1),
    ("density-dozen", 40),
    ("density-five-thousand", 5_000),
    ("density-fifty-thousand", 50_000),
    ("数据库", 1_000),
    ("density-punctuation-v2", 1_000),
    ("density-mixed-case", 1_000),
    ("density-path-only-marker", 500),
    ("pdf", 2_000),
];

const INSERT_BATCH: usize = 512;
const FILLER_MODIFIED_BASE: i64 = 1_700_000_000;

fn entry(
    volume_id: &str,
    path: String,
    name: String,
    extension: &str,
    modified: Option<i64>,
) -> GlobalEntryInput {
    GlobalEntryInput {
        volume_id: volume_id.to_string(),
        platform_file_id: format!("frn:{path}"),
        parent_platform_file_id: "frn:parent".to_string(),
        name,
        path,
        extension: extension.to_string(),
        is_directory: false,
        size: 1024,
        created_at_fs: Some(1_600_000_000),
        modified_at_fs: modified,
        file_attributes: 0,
        is_hidden: false,
        is_system: false,
        source_provider: "synthetic_issue494_density".to_string(),
        last_seen_at: 1_700_000_000,
    }
}

fn push_row(
    rows: &mut Vec<GlobalEntryInput>,
    path: String,
    name: String,
    extension: &str,
    modified: Option<i64>,
) {
    rows.push(entry("gv_test", path, name, extension, modified));
}

/// Controlled-density fixture with exactly `total` authorized rows.
pub(super) fn density(total: u64) -> Fixture {
    let fixture = open_fixture();
    let mut disabled = test_volume();
    disabled.id = "gv_disabled_494".to_string();
    disabled.stable_volume_id = "disabled-volume-494".to_string();
    disabled.display_name = "Disabled volume".to_string();
    disabled.mount_path = r"D:\Revoked\".to_string();
    disabled.enabled = false;
    fixture
        .db()
        .upsert_global_volume(&disabled)
        .expect("seed disabled volume");

    let mut rows = Vec::<GlobalEntryInput>::new();

    // 1 hit.
    push_row(
        &mut rows,
        r"C:\Density\Single\density-singleton.pdf".to_string(),
        "density-singleton.pdf".to_string(),
        "pdf",
        Some(FILLER_MODIFIED_BASE + 9_000),
    );

    // 40 hits.
    for index in 0..40u64 {
        push_row(
            &mut rows,
            format!(r"C:\Density\Dozens\density-dozen-{index:03}.txt"),
            format!("density-dozen-{index:03}.txt"),
            "txt",
            Some(FILLER_MODIFIED_BASE + 8_000 + index as i64),
        );
    }

    // 5 000 hits, all with the same mtime to force a high-tie group that only
    // the stable id can break.
    for index in 0..5_000u64 {
        push_row(
            &mut rows,
            format!(r"C:\Density\Five\density-five-thousand-{index:05}.md"),
            format!("density-five-thousand-{index:05}.md"),
            "md",
            Some(FILLER_MODIFIED_BASE),
        );
    }

    // 50 000 hits, spread over three mtimes so the class order is stable.
    for index in 0..50_000u64 {
        push_row(
            &mut rows,
            format!(
                r"C:\Density\Fifty\Bucket{}\density-fifty-thousand-{index:05}.txt",
                index % 3
            ),
            format!("density-fifty-thousand-{index:05}.txt"),
            "txt",
            Some(FILLER_MODIFIED_BASE + (index % 3) as i64),
        );
    }

    // 1 000 Chinese hits.
    for index in 0..1_000u64 {
        push_row(
            &mut rows,
            format!(r"C:\Density\Chinese\数据库技术教案-{index:04}.docx"),
            format!("数据库技术教案-{index:04}.docx"),
            "docx",
            Some(FILLER_MODIFIED_BASE + 7_000 + index as i64),
        );
    }

    // 1 000 punctuation hits.
    for index in 0..1_000u64 {
        push_row(
            &mut rows,
            format!(r"C:\Density\Punctuation\density-punctuation-v2-{index:04}.md"),
            format!("density-punctuation-v2-{index:04}.md"),
            "md",
            Some(FILLER_MODIFIED_BASE + 6_000 + index as i64),
        );
    }

    // 1 000 mixed-case hits. The stored name is normalized, the query is not.
    for index in 0..1_000u64 {
        push_row(
            &mut rows,
            format!(r"C:\Density\Case\DeNsItY-MiXeD-CaSe-{index:04}.txt"),
            format!("DeNsItY-MiXeD-CaSe-{index:04}.txt"),
            "txt",
            Some(FILLER_MODIFIED_BASE + 5_000 + index as i64),
        );
    }

    // 500 path-only hits: the marker lives in the directory, not the filename.
    for index in 0..500u64 {
        push_row(
            &mut rows,
            format!(r"C:\Archive\density-path-only-marker\{index:04}\attachment-{index:04}.bin"),
            format!("attachment-{index:04}.bin"),
            "bin",
            Some(FILLER_MODIFIED_BASE + 4_000 + index as i64),
        );
    }

    // 2 000 extension hits. Names deliberately do not contain "pdf".
    for index in 0..2_000u64 {
        push_row(
            &mut rows,
            format!(r"C:\Density\Extension\scanned-image-{index:04}.pdf"),
            format!("scanned-image-{index:04}.pdf"),
            "pdf",
            Some(FILLER_MODIFIED_BASE + 3_000 + index as i64),
        );
    }

    // 100 rows on a disabled volume: must never appear in any result.
    for index in 0..100u64 {
        rows.push(entry(
            "gv_disabled_494",
            format!(r"D:\Revoked\density-fifty-thousand-{index:04}.txt"),
            format!("density-fifty-thousand-{index:04}.txt"),
            "txt",
            Some(FILLER_MODIFIED_BASE + 100_000),
        ));
    }

    // 100 stale rows: must never appear in any result.
    let stale_start = rows.len();
    for index in 0..100u64 {
        push_row(
            &mut rows,
            format!(r"C:\Density\Stale\density-fifty-thousand-stale-{index:04}.txt"),
            format!("density-fifty-thousand-stale-{index:04}.txt"),
            "txt",
            Some(FILLER_MODIFIED_BASE + 200_000),
        );
    }
    let stale_end = rows.len();

    // Filler to reach the requested total.
    let used = rows.len() as u64;
    assert!(total > used, "density fixture needs room for filler rows");
    for index in 0..(total - used) {
        push_row(
            &mut rows,
            format!(
                r"C:\Density\Filler\{:03}\filler-{index:07}.bin",
                index % 500
            ),
            format!("filler-{index:07}.bin"),
            "bin",
            Some(FILLER_MODIFIED_BASE + (index % 97) as i64),
        );
    }

    let db = fixture.db();
    for chunk in rows.chunks(INSERT_BATCH) {
        db.upsert_global_entries_batch(chunk)
            .expect("seed density batch");
    }
    db.conn()
        .expect("writer")
        .execute(
            "UPDATE global_entries SET is_stale = 1 WHERE rowid > ?1 AND rowid <= ?2",
            rusqlite::params![stale_start as i64, stale_end as i64],
        )
        .expect("mark stale density rows");

    fixture
}

/// Small fixture with the awkward identities the ranking must not lose.
pub(super) fn edge_cases() -> Fixture {
    let fixture = open_fixture();
    let mut disabled = test_volume();
    disabled.id = "gv_disabled_494".to_string();
    disabled.stable_volume_id = "disabled-volume-494".to_string();
    disabled.enabled = false;
    fixture
        .db()
        .upsert_global_volume(&disabled)
        .expect("seed disabled volume");

    let rows = vec![
        entry(
            "gv_test",
            r"C:\Edge\annual-report.pdf".to_string(),
            "annual-report.pdf".to_string(),
            "pdf",
            Some(30),
        ),
        entry(
            "gv_test",
            r"C:\Edge\report.pdf".to_string(),
            "report.pdf".to_string(),
            "pdf",
            Some(20),
        ),
        entry(
            "gv_test",
            r"C:\Edge\board_report_q3.pdf".to_string(),
            "board_report_q3.pdf".to_string(),
            "pdf",
            Some(10),
        ),
        entry(
            "gv_test",
            r"C:\Edge\pre-report-notes.pdf".to_string(),
            "pre-report-notes.pdf".to_string(),
            "pdf",
            Some(40),
        ),
        entry(
            "gv_test",
            r"C:\Edge\Report.PDF".to_string(),
            "Report.PDF".to_string(),
            "pdf",
            Some(50),
        ),
        entry(
            "gv_test",
            r"C:\Edge\数据库教案.docx".to_string(),
            "数据库教案.docx".to_string(),
            "docx",
            Some(60),
        ),
        entry(
            "gv_test",
            r"C:\Edge\final-v2-notes.md".to_string(),
            "final-v2-notes.md".to_string(),
            "md",
            Some(70),
        ),
        entry(
            "gv_test",
            r"C:\Edge\Résumé Project.pdf".to_string(),
            "Résumé Project.pdf".to_string(),
            "pdf",
            Some(80),
        ),
        entry(
            "gv_test",
            r"C:\Edge\duplicate.txt".to_string(),
            "duplicate.txt".to_string(),
            "txt",
            Some(90),
        ),
        entry(
            "gv_test",
            r"C:\Other\duplicate.txt".to_string(),
            "duplicate.txt".to_string(),
            "txt",
            Some(95),
        ),
        entry(
            "gv_test",
            r"C:\Edge\no-extension".to_string(),
            "no-extension".to_string(),
            "",
            Some(100),
        ),
        entry(
            "gv_test",
            r"C:\PathMarker\quarterly-review\attachment.bin".to_string(),
            "attachment.bin".to_string(),
            "bin",
            Some(110),
        ),
        entry(
            "gv_test",
            r"C:\PathMarker\quarterly-review\notes.bin".to_string(),
            "notes.bin".to_string(),
            "bin",
            Some(120),
        ),
        entry(
            "gv_disabled_494",
            r"D:\Revoked\annual-report.pdf".to_string(),
            "annual-report.pdf".to_string(),
            "pdf",
            Some(130),
        ),
    ];
    fixture
        .db()
        .upsert_global_entries_batch(&rows)
        .expect("seed edge-case rows");
    fixture
        .db()
        .conn()
        .expect("writer")
        .execute(
            "UPDATE global_entries SET is_stale = 1 WHERE path = 'C:\\Edge\\pre-report-notes.pdf'",
            [],
        )
        .expect("mark stale edge-case row");
    fixture
}
