//! Test-only ISG-00E ingestion-cost experiment.
//!
//! This harness models the Owner's proposed classifier as an experimental
//! test fixture. It never touches the production index scope, walks the file
//! system, or reads native file metadata. Group D's metadata accessor is a
//! deterministic in-memory stand-in; its counters are not native stat savings.

use super::{test_volume, BenchmarkCleanup};
use crate::db::Database;
use crate::global_index::models::GlobalEntryInput;
use crate::global_index::search::search_global_entries_on_connection;
use rusqlite::Connection;
use serde_json::{json, Value as JsonValue};
use std::collections::{BTreeMap, HashSet};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const FIXTURE_SEED: u64 = 0x4953_4700_45;
const SMOKE_ENTRIES: u64 = 1_000;
const ALLOWED_ENTRIES: [u64; 3] = [1_000, 10_000, 100_000];
const REPEAT_SAMPLES: usize = 3;
const INSERT_BATCH_SIZE: usize = 512;
const FTS_SENTINEL: &str = "isgbench";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum CorpusClass {
    PersonalDocumentsMedia,
    ProjectReadmeDocsSrcManifests,
    UserNamedLookalikes,
    RecognizedDependencies,
    GeneratedBuildOutputs,
    GeneratedCaches,
    ProtectedSystemInternals,
    UnknownUnicodePaths,
    ProjectMinimalAncestors,
}

impl CorpusClass {
    const ALL: [Self; 9] = [
        Self::PersonalDocumentsMedia,
        Self::ProjectReadmeDocsSrcManifests,
        Self::UserNamedLookalikes,
        Self::RecognizedDependencies,
        Self::GeneratedBuildOutputs,
        Self::GeneratedCaches,
        Self::ProtectedSystemInternals,
        Self::UnknownUnicodePaths,
        Self::ProjectMinimalAncestors,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::PersonalDocumentsMedia => "personal_documents_media",
            Self::ProjectReadmeDocsSrcManifests => "project_readme_docs_src_manifests",
            Self::UserNamedLookalikes => "user_named_lookalikes",
            Self::RecognizedDependencies => "recognized_dependencies",
            Self::GeneratedBuildOutputs => "generated_build_outputs",
            Self::GeneratedCaches => "generated_caches",
            Self::ProtectedSystemInternals => "protected_system_internals",
            Self::UnknownUnicodePaths => "unknown_unicode_paths",
            Self::ProjectMinimalAncestors => "project_minimal_ancestors",
        }
    }
}

#[derive(Clone, Debug)]
struct PolicyEvidence {
    project_root: bool,
    manifest: Option<&'static str>,
    generated_output_signature: bool,
    known_ephemeral_cache_identity: bool,
    protected_system_identity: bool,
}

#[derive(Clone, Debug)]
struct CorpusRow {
    class: CorpusClass,
    input: GlobalEntryInput,
    evidence: PolicyEvidence,
    search_token: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExperimentalDecision {
    Include,
    MinimalAncestor,
    Exclude(&'static str),
    Undetermined,
}

impl ExperimentalDecision {
    fn label(self) -> &'static str {
        match self {
            Self::Include => "included",
            Self::MinimalAncestor => "minimal_ancestor",
            Self::Exclude(_) => "excluded",
            Self::Undetermined => "undetermined",
        }
    }

    fn admitted(self) -> bool {
        !matches!(self, Self::Exclude(_))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExperimentGroup {
    A,
    B,
    C,
    D,
}

impl ExperimentGroup {
    const ALL: [Self; 4] = [Self::A, Self::B, Self::C, Self::D];

    fn label(self) -> &'static str {
        match self {
            Self::A => "A_broad_baseline",
            Self::B => "B_dry_run_all_rows_written",
            Self::C => "C_experimental_sink_gate",
            Self::D => "D_synthetic_pre_metadata_gate",
        }
    }
}

#[derive(Default)]
struct PipelineCounters {
    policy_evaluated: u64,
    included: u64,
    minimal_ancestor: u64,
    excluded: u64,
    undetermined: u64,
    metadata_read_attempted: u64,
    metadata_read_succeeded: u64,
    metadata_accessor_ns: u128,
    classification_ns: u128,
    upsert_batches: u64,
    upsert_returned_rows: u64,
    admitted_rows: u64,
    excluded_by_reason: BTreeMap<&'static str, u64>,
    decisions_by_class: BTreeMap<&'static str, BTreeMap<&'static str, u64>>,
}

struct SampleResult {
    group: ExperimentGroup,
    repeat: usize,
    execution_order_index: usize,
    wall_ms: f64,
    classification_ms: f64,
    synthetic_metadata_accessor_ms: f64,
    synthetic_metadata_read_attempted: u64,
    synthetic_metadata_read_succeeded: u64,
    input_candidates: u64,
    policy_evaluated: u64,
    included: u64,
    minimal_ancestor: u64,
    excluded: u64,
    undetermined: u64,
    excluded_by_reason: BTreeMap<&'static str, u64>,
    decisions_by_class: BTreeMap<&'static str, BTreeMap<&'static str, u64>>,
    upsert_batches: u64,
    upsert_returned_rows: u64,
    admitted_rows: u64,
    database_rows: u64,
    volume_entry_count: u64,
    fts_sentinel_matches: u64,
    sqlite_metrics: JsonValue,
    ai_job_rows: u64,
    ai_job_item_rows: u64,
    identity_set_correct: bool,
    search_cases_checked: usize,
    search_cases_correct: bool,
}

fn configured_entries() -> u64 {
    let entries = env::var("ZC_GLOBAL_INDEX_COST_BENCHMARK_ENTRIES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(SMOKE_ENTRIES);
    assert!(
        ALLOWED_ENTRIES.contains(&entries),
        "ZC_GLOBAL_INDEX_COST_BENCHMARK_ENTRIES must be one of {ALLOWED_ENTRIES:?}, got {entries}"
    );
    entries
}

fn deterministic_word(index: u64) -> u64 {
    let mut value = FIXTURE_SEED ^ index.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

fn scheduled_class(index: u64) -> CorpusClass {
    let slot = index % 100;
    match slot {
        0..=29 => CorpusClass::PersonalDocumentsMedia,
        30..=44 => CorpusClass::ProjectReadmeDocsSrcManifests,
        45..=54 => CorpusClass::UserNamedLookalikes,
        55..=69 => CorpusClass::RecognizedDependencies,
        70..=79 => CorpusClass::GeneratedBuildOutputs,
        80..=89 => CorpusClass::GeneratedCaches,
        90..=94 => CorpusClass::ProtectedSystemInternals,
        95..=97 => CorpusClass::UnknownUnicodePaths,
        _ => CorpusClass::ProjectMinimalAncestors,
    }
}

fn fixture_row(index: u64) -> CorpusRow {
    let class = scheduled_class(index);
    let bucket = index / 100;
    let nonce = deterministic_word(index);
    let search_token = format!("isgfixture{index:07}");
    let root = format!(r"D:\Projects\IsgRepo{bucket:06}");
    let (path, basename, extension, is_directory, evidence) = match class {
        CorpusClass::PersonalDocumentsMedia => {
            let (directory, stem, extension) = match index % 3 {
                0 => (
                    r"C:\Users\Benchmark\Documents\Personal",
                    "FamilyNotes",
                    "docx",
                ),
                1 => (
                    r"C:\Users\Benchmark\Pictures\Camera Roll",
                    "VacationPhoto",
                    "jpg",
                ),
                _ => (r"C:\Users\Benchmark\Videos\Personal", "HomeVideo", "mp4"),
            };
            (
                format!(r"{directory}\{search_token}_{stem}_{nonce:016x}.{extension}"),
                format!("{search_token}_{stem}_{nonce:016x}.{extension}"),
                extension.to_string(),
                false,
                PolicyEvidence {
                    project_root: false,
                    manifest: None,
                    generated_output_signature: false,
                    known_ephemeral_cache_identity: false,
                    protected_system_identity: false,
                },
            )
        }
        CorpusClass::ProjectReadmeDocsSrcManifests => {
            let (relative, manifest) = match index % 6 {
                0 => ("README.md", None),
                1 => ("docs/usage-guide.md", None),
                2 => ("src/main.rs", Some("Cargo.toml")),
                3 => ("Cargo.toml", Some("Cargo.toml")),
                4 => ("package.json", Some("package.json")),
                _ => (".vscode/settings.json", Some("package.json")),
            };
            let relative = relative.replace('/', r"\");
            let basename = relative
                .rsplit('\\')
                .next()
                .expect("synthetic project basename");
            let relative_parent = relative
                .rsplit_once('\\')
                .map(|(parent, _)| parent)
                .unwrap_or("");
            let project_module = format!(r"isgbench_module-{}", index % 2);
            let directory = if relative_parent.is_empty() {
                format!(r"{root}\{project_module}")
            } else {
                format!(r"{root}\{project_module}\{relative_parent}")
            };
            (
                format!(r"{directory}\{search_token}"),
                format!("{search_token}_{basename}"),
                basename
                    .rsplit_once('.')
                    .map(|(_, ext)| ext)
                    .unwrap_or("")
                    .to_string(),
                false,
                PolicyEvidence {
                    project_root: true,
                    manifest,
                    generated_output_signature: false,
                    known_ephemeral_cache_identity: false,
                    protected_system_identity: false,
                },
            )
        }
        CorpusClass::UserNamedLookalikes => {
            let (directory, basename, extension) = match index % 4 {
                0 => (r"D:\Projects\Windows", "README", "md"),
                1 => (r"D:\Design\build", "logo", "ai"),
                2 => (r"D:\Configs\AppData", "notes", "json"),
                _ => (r"D:\Tools\Program Files2", "README", "txt"),
            };
            (
                format!(r"{directory}\{search_token}_{nonce:016x}.{extension}"),
                format!("{search_token}_{basename}_{nonce:016x}.{extension}"),
                extension.to_string(),
                false,
                PolicyEvidence {
                    project_root: false,
                    manifest: None,
                    generated_output_signature: false,
                    known_ephemeral_cache_identity: false,
                    protected_system_identity: false,
                },
            )
        }
        CorpusClass::RecognizedDependencies => {
            let (directory, manifest, extension) = match index % 4 {
                0 => (
                    format!(r"{root}\node_modules\package-{bucket:06}"),
                    "package-lock.json",
                    "js",
                ),
                1 => (
                    format!(r"{root}\.venv\Lib\site-packages\package-{bucket:06}"),
                    "pyproject.toml",
                    "py",
                ),
                2 => (
                    format!(r"{root}\vendor\python\site-packages\package-{bucket:06}"),
                    "requirements.txt",
                    "py",
                ),
                _ => (format!(r"{root}\target\debug\deps"), "Cargo.toml", "rlib"),
            };
            (
                format!(r"{directory}\{search_token}_{nonce:016x}.{extension}"),
                format!("{search_token}_dependency_{nonce:016x}.{extension}"),
                extension.to_string(),
                false,
                PolicyEvidence {
                    project_root: true,
                    manifest: Some(manifest),
                    generated_output_signature: false,
                    known_ephemeral_cache_identity: false,
                    protected_system_identity: false,
                },
            )
        }
        CorpusClass::GeneratedBuildOutputs => {
            let (directory, manifest, extension) = match index % 3 {
                0 => (
                    format!(r"{root}\build\release"),
                    "build-manifest.json",
                    "bin",
                ),
                1 => (format!(r"{root}\dist\assets"), "package.json", "js"),
                _ => (format!(r"{root}\target\release"), "Cargo.toml", "exe"),
            };
            (
                format!(r"{directory}\{search_token}_{nonce:016x}.{extension}"),
                format!("{search_token}_generated_{nonce:016x}.{extension}"),
                extension.to_string(),
                false,
                PolicyEvidence {
                    project_root: true,
                    manifest: Some(manifest),
                    generated_output_signature: true,
                    known_ephemeral_cache_identity: false,
                    protected_system_identity: false,
                },
            )
        }
        CorpusClass::GeneratedCaches => {
            let (directory, extension) = match index % 3 {
                0 => (r"C:\Users\Benchmark\AppData\Local\Temp", "tmp"),
                1 => (
                    r"C:\Users\Benchmark\AppData\Local\Packages\Example\Cache",
                    "bin",
                ),
                _ => (r"D:\Tools\ZenCache\generated-cache", "dat"),
            };
            (
                format!(r"{directory}\{search_token}_{nonce:016x}.{extension}"),
                format!("{search_token}_cache_{nonce:016x}.{extension}"),
                extension.to_string(),
                false,
                PolicyEvidence {
                    project_root: false,
                    manifest: None,
                    generated_output_signature: false,
                    known_ephemeral_cache_identity: true,
                    protected_system_identity: false,
                },
            )
        }
        CorpusClass::ProtectedSystemInternals => {
            let directory = if index % 2 == 0 {
                r"C:\Windows\System32\drivers"
            } else {
                r"C:\Program Files\WindowsApps\Synthetic.Package"
            };
            (
                format!(r"{directory}\{search_token}_{nonce:016x}.sys"),
                format!("{search_token}_protected_{nonce:016x}.sys"),
                "sys".to_string(),
                false,
                PolicyEvidence {
                    project_root: false,
                    manifest: None,
                    generated_output_signature: false,
                    known_ephemeral_cache_identity: false,
                    protected_system_identity: true,
                },
            )
        }
        CorpusClass::UnknownUnicodePaths => {
            let directory = if index % 2 == 0 {
                "D:\\資料\\Résumé\\Cafe\u{301}"
            } else {
                "D:\\資料\\履歷\\Café"
            };
            (
                format!(r"{directory}\{search_token}_未分類_{nonce:016x}.txt"),
                format!("{search_token}_未知_{nonce:016x}.txt"),
                "txt".to_string(),
                false,
                PolicyEvidence {
                    project_root: false,
                    manifest: None,
                    generated_output_signature: false,
                    known_ephemeral_cache_identity: false,
                    protected_system_identity: false,
                },
            )
        }
        CorpusClass::ProjectMinimalAncestors => {
            let project_module = format!(r"isgbench_module-{}", index % 2);
            (
                format!(r"{root}\{project_module}"),
                project_module,
                String::new(),
                true,
                PolicyEvidence {
                    project_root: true,
                    manifest: None,
                    generated_output_signature: false,
                    known_ephemeral_cache_identity: false,
                    protected_system_identity: false,
                },
            )
        }
    };

    let bucket_parent_id = format!("synthetic-project-root-{bucket:06}-{}", index % 2);
    let platform_file_id = if class == CorpusClass::ProjectMinimalAncestors {
        bucket_parent_id.clone()
    } else {
        format!("synthetic-file-{index:07}-{nonce:016x}")
    };
    let parent_platform_file_id = match class {
        CorpusClass::ProjectReadmeDocsSrcManifests => bucket_parent_id,
        CorpusClass::ProjectMinimalAncestors => format!("synthetic-project-parent-{bucket:06}"),
        _ => format!("synthetic-parent-{index:07}"),
    };
    let name = if is_directory {
        basename
    } else {
        format!("isgbench_{basename}")
    };
    let full_path = if is_directory {
        path
    } else {
        let directory = path
            .rsplit_once('\\')
            .map(|(directory, _)| directory)
            .unwrap_or(path.as_str());
        format!(r"{directory}\{name}")
    };
    let input = GlobalEntryInput {
        volume_id: "gv_test".to_string(),
        platform_file_id,
        parent_platform_file_id,
        name,
        path: full_path,
        extension,
        is_directory,
        size: if is_directory {
            0
        } else {
            ((nonce % 1_000_000_000) + 1) as i64
        },
        created_at_fs: Some(1_700_000_000 + (index as i64 / 2)),
        modified_at_fs: Some(1_700_000_000 + index as i64),
        file_attributes: (nonce % 16) as i64,
        is_hidden: index.is_multiple_of(97),
        is_system: evidence.protected_system_identity,
        source_provider: "synthetic_isg00e_benchmark".to_string(),
        last_seen_at: 1_700_000_000 + index as i64,
    };

    CorpusRow {
        class,
        input,
        evidence,
        search_token,
    }
}

fn corpus(entries: u64) -> Vec<CorpusRow> {
    assert!(
        entries.is_multiple_of(100),
        "fixture scale must preserve the exact 100-slot distribution"
    );
    (0..entries).map(fixture_row).collect()
}

fn path_has_component(path: &str, wanted: &str) -> bool {
    path.split(['\\', '/'])
        .any(|component| component.eq_ignore_ascii_case(wanted))
}

fn experimental_decision(
    row: &CorpusRow,
    needed_ancestors: &HashSet<String>,
) -> ExperimentalDecision {
    if row.evidence.protected_system_identity {
        return ExperimentalDecision::Exclude("protected_system");
    }
    if row.evidence.known_ephemeral_cache_identity {
        return ExperimentalDecision::Exclude("ephemeral_cache");
    }
    if row.evidence.project_root
        && row.evidence.manifest.is_some()
        && ["node_modules", ".venv", "site-packages", "target"]
            .iter()
            .any(|component| path_has_component(&row.input.path, component))
    {
        return ExperimentalDecision::Exclude("dependency_tree");
    }
    if row.evidence.project_root
        && row.evidence.manifest.is_some()
        && row.evidence.generated_output_signature
        && ["build", "dist", "target"]
            .iter()
            .any(|component| path_has_component(&row.input.path, component))
    {
        return ExperimentalDecision::Exclude("generated_build");
    }
    if row.input.is_directory && needed_ancestors.contains(&row.input.platform_file_id) {
        return ExperimentalDecision::MinimalAncestor;
    }
    if row.class == CorpusClass::UnknownUnicodePaths {
        return ExperimentalDecision::Undetermined;
    }
    ExperimentalDecision::Include
}

fn expected_decisions(rows: &[CorpusRow]) -> Vec<ExperimentalDecision> {
    rows.iter()
        .map(|row| match row.class {
            CorpusClass::PersonalDocumentsMedia
            | CorpusClass::ProjectReadmeDocsSrcManifests
            | CorpusClass::UserNamedLookalikes => ExperimentalDecision::Include,
            CorpusClass::RecognizedDependencies => ExperimentalDecision::Exclude("dependency_tree"),
            CorpusClass::GeneratedBuildOutputs => {
                ExperimentalDecision::Exclude(if path_has_component(&row.input.path, "target") {
                    "dependency_tree"
                } else {
                    "generated_build"
                })
            }
            CorpusClass::GeneratedCaches => ExperimentalDecision::Exclude("ephemeral_cache"),
            CorpusClass::ProtectedSystemInternals => {
                ExperimentalDecision::Exclude("protected_system")
            }
            CorpusClass::UnknownUnicodePaths => ExperimentalDecision::Undetermined,
            CorpusClass::ProjectMinimalAncestors => ExperimentalDecision::MinimalAncestor,
        })
        .collect()
}

fn decision_record(
    counters: &mut PipelineCounters,
    class: CorpusClass,
    decision: ExperimentalDecision,
) {
    counters.policy_evaluated += 1;
    *counters
        .decisions_by_class
        .entry(class.label())
        .or_default()
        .entry(decision.label())
        .or_default() += 1;
    match decision {
        ExperimentalDecision::Include => counters.included += 1,
        ExperimentalDecision::MinimalAncestor => counters.minimal_ancestor += 1,
        ExperimentalDecision::Exclude(reason) => {
            counters.excluded += 1;
            *counters.excluded_by_reason.entry(reason).or_default() += 1;
        }
        ExperimentalDecision::Undetermined => counters.undetermined += 1,
    }
}

fn synthetic_metadata_read(row: &CorpusRow, counters: &mut PipelineCounters) -> GlobalEntryInput {
    let started = Instant::now();
    counters.metadata_read_attempted += 1;
    // This returns deterministic in-memory fixture fields. It does not call
    // std::fs, Metadata::from_path, MFT, Spotlight, or a native stat API.
    let value = row.input.clone();
    counters.metadata_read_succeeded += 1;
    counters.metadata_accessor_ns += started.elapsed().as_nanos();
    value
}

fn account_decision(
    counters: &mut PipelineCounters,
    class: CorpusClass,
    decision: ExperimentalDecision,
) {
    decision_record(counters, class, decision);
}

fn run_group(
    rows: &[CorpusRow],
    expected: &[ExperimentalDecision],
    group: ExperimentGroup,
    repeat: usize,
    execution_order_index: usize,
) -> SampleResult {
    let path = env::temp_dir().join(format!(
        "zen-canvas-isg00e-{}-{}.db",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let _cleanup = BenchmarkCleanup(path.clone());
    let db = Database::open(&path).expect("open isolated synthetic Global Index database");
    db.upsert_global_volume(&test_volume())
        .expect("insert isolated synthetic volume");

    let started = Instant::now();
    let mut counters = PipelineCounters::default();
    let mut batch = Vec::with_capacity(INSERT_BATCH_SIZE);
    let needed_ancestors = needed_ancestor_ids(rows);
    for (row_index, row) in rows.iter().enumerate() {
        let decision = if group == ExperimentGroup::A {
            None
        } else if group == ExperimentGroup::D {
            let classify_started = Instant::now();
            let decision = experimental_decision(row, &needed_ancestors);
            counters.classification_ns += classify_started.elapsed().as_nanos();
            assert_eq!(
                decision, expected[row_index],
                "pre-metadata experimental classifier disagrees with the fixture oracle for row {row_index}"
            );
            account_decision(&mut counters, row.class, decision);
            Some(decision)
        } else {
            None
        };

        let metadata = if group == ExperimentGroup::D {
            if decision.expect("D decision evaluated").admitted() {
                Some(synthetic_metadata_read(row, &mut counters))
            } else {
                None
            }
        } else {
            let metadata = synthetic_metadata_read(row, &mut counters);
            if group == ExperimentGroup::B || group == ExperimentGroup::C {
                let classify_started = Instant::now();
                let actual_decision = experimental_decision(row, &needed_ancestors);
                counters.classification_ns += classify_started.elapsed().as_nanos();
                account_decision(&mut counters, row.class, actual_decision);
                assert_eq!(
                    actual_decision, expected[row_index],
                    "experimental classifier disagrees with its isolated oracle for row {row_index}"
                );
                if group == ExperimentGroup::C && !actual_decision.admitted() {
                    None
                } else {
                    Some(metadata)
                }
            } else {
                Some(metadata)
            }
        };

        if let Some(metadata) = metadata {
            counters.admitted_rows += 1;
            batch.push(metadata);
            if batch.len() == INSERT_BATCH_SIZE {
                flush_batch(&db, &mut batch, &mut counters);
            }
        }
    }
    if !batch.is_empty() {
        flush_batch(&db, &mut batch, &mut counters);
    }
    let wall_ms = started.elapsed().as_secs_f64() * 1_000.0;

    let conn = db.conn().expect("inspect isolated benchmark database");
    let database_rows = count_rows(
        &conn,
        "SELECT COUNT(*) FROM global_entries WHERE volume_id = 'gv_test'",
    );
    let volume_entry_count = count_rows(
        &conn,
        "SELECT entry_count FROM global_volumes WHERE id = 'gv_test'",
    );
    let fts_sentinel_matches = count_rows(
        &conn,
        "SELECT COUNT(DISTINCT rowid) FROM global_entries_fts WHERE global_entries_fts MATCH '\"isgbench\"'",
    );
    let ai_job_rows = count_rows(&conn, "SELECT COUNT(*) FROM ai_jobs");
    let ai_job_item_rows = count_rows(&conn, "SELECT COUNT(*) FROM ai_job_items");
    let sqlite_metrics = sqlite_metrics(&conn, &path);
    let expected_ids = expected_ids(rows, expected, group);
    let actual_ids = query_ids(&conn);
    let identity_set_correct = actual_ids == expected_ids;
    assert!(
        identity_set_correct,
        "stored Global Index identity set differs for group {}",
        group.label()
    );
    assert_eq!(database_rows, expected_ids.len() as u64);
    assert_eq!(volume_entry_count, database_rows);
    assert_eq!(counters.upsert_returned_rows, database_rows);
    assert_eq!(fts_sentinel_matches, database_rows, "the synthetic sentinel query must find every admitted fixture row; this is a fixture-specific FTS correctness check, not a general trigger-to-row ratio");
    assert_eq!(
        ai_job_rows, 0,
        "isolated benchmark must not enqueue AI jobs"
    );
    assert_eq!(
        ai_job_item_rows, 0,
        "isolated benchmark must not create AI job items"
    );

    let search_cases_checked = assert_search_correctness(&conn, rows, expected, group);
    let search_cases_correct = true;
    drop(conn);
    drop(db);

    let expected_admitted_count = expected
        .iter()
        .filter(|decision| decision.admitted())
        .count() as u64;
    if group == ExperimentGroup::A {
        assert_eq!(counters.policy_evaluated, 0);
        assert_eq!(counters.admitted_rows, rows.len() as u64);
    } else {
        assert_eq!(counters.policy_evaluated, rows.len() as u64);
        assert_eq!(
            counters.included
                + counters.minimal_ancestor
                + counters.excluded
                + counters.undetermined,
            counters.policy_evaluated,
            "classifier decisions must conserve every evaluated candidate"
        );
        if group == ExperimentGroup::B {
            assert_eq!(counters.admitted_rows, rows.len() as u64);
        } else {
            assert_eq!(counters.admitted_rows, expected_admitted_count);
        }
    }

    SampleResult {
        group,
        repeat,
        execution_order_index,
        wall_ms,
        classification_ms: counters.classification_ns as f64 / 1_000_000.0,
        synthetic_metadata_accessor_ms: counters.metadata_accessor_ns as f64 / 1_000_000.0,
        synthetic_metadata_read_attempted: counters.metadata_read_attempted,
        synthetic_metadata_read_succeeded: counters.metadata_read_succeeded,
        input_candidates: rows.len() as u64,
        policy_evaluated: counters.policy_evaluated,
        included: counters.included,
        minimal_ancestor: counters.minimal_ancestor,
        excluded: counters.excluded,
        undetermined: counters.undetermined,
        excluded_by_reason: counters.excluded_by_reason,
        decisions_by_class: counters.decisions_by_class,
        upsert_batches: counters.upsert_batches,
        upsert_returned_rows: counters.upsert_returned_rows,
        admitted_rows: counters.admitted_rows,
        database_rows,
        volume_entry_count,
        fts_sentinel_matches,
        sqlite_metrics,
        ai_job_rows,
        ai_job_item_rows,
        identity_set_correct,
        search_cases_checked,
        search_cases_correct,
    }
}

fn needed_ancestor_ids(rows: &[CorpusRow]) -> HashSet<String> {
    rows.iter()
        .filter(|row| row.class == CorpusClass::ProjectReadmeDocsSrcManifests)
        .map(|row| row.input.parent_platform_file_id.clone())
        .collect()
}

fn flush_batch(db: &Database, batch: &mut Vec<GlobalEntryInput>, counters: &mut PipelineCounters) {
    counters.upsert_returned_rows += db
        .upsert_global_entries_batch(batch)
        .expect("write isolated benchmark batch through Global Index repository")
        as u64;
    counters.upsert_batches += 1;
    batch.clear();
}

fn count_rows(conn: &Connection, sql: &str) -> u64 {
    conn.query_row(sql, [], |row| row.get::<_, i64>(0))
        .expect("read benchmark counter") as u64
}

fn query_ids(conn: &Connection) -> Vec<String> {
    let mut statement = conn
        .prepare("SELECT id FROM global_entries WHERE volume_id = 'gv_test' ORDER BY id")
        .expect("prepare benchmark identity query");
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query benchmark identities")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect benchmark identities")
}

fn expected_ids(
    rows: &[CorpusRow],
    decisions: &[ExperimentalDecision],
    group: ExperimentGroup,
) -> Vec<String> {
    let mut ids = rows
        .iter()
        .enumerate()
        .filter(|(index, _)| {
            group == ExperimentGroup::A
                || group == ExperimentGroup::B
                || decisions[*index].admitted()
        })
        .map(|(_, row)| row.input.entry_id())
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

fn assert_search_correctness(
    conn: &Connection,
    rows: &[CorpusRow],
    decisions: &[ExperimentalDecision],
    group: ExperimentGroup,
) -> usize {
    let mut samples = Vec::new();
    for class in CorpusClass::ALL {
        if class == CorpusClass::ProjectMinimalAncestors {
            continue;
        }
        if class == CorpusClass::UserNamedLookalikes {
            for variant in 0..4 {
                if let Some((index, row)) = rows.iter().enumerate().find(|(_, row)| {
                    row.class == class && index_from_token(&row.search_token) % 4 == variant
                }) {
                    let _ = index;
                    samples.push(row);
                }
            }
        } else if let Some((_, row)) = rows.iter().enumerate().find(|(_, row)| row.class == class) {
            samples.push(row);
        }
    }

    // The lookalike set above must include each exact name collision case.
    let mut lookalike_tokens = HashSet::new();
    for row in rows
        .iter()
        .filter(|row| row.class == CorpusClass::UserNamedLookalikes)
    {
        let variant = index_from_token(&row.search_token) % 4;
        lookalike_tokens.insert(variant);
        if lookalike_tokens.len() == 4 {
            break;
        }
    }
    assert_eq!(
        lookalike_tokens.len(),
        4,
        "fixture must include all lookalike path variants"
    );
    let mut checked = 0;
    let mut verified = HashSet::new();
    for row in samples {
        if !verified.insert(row.search_token.clone()) {
            continue;
        }
        let index = index_from_token(&row.search_token) as usize;
        let decision = decisions[index];
        let should_be_searchable = match group {
            ExperimentGroup::A | ExperimentGroup::B => true,
            ExperimentGroup::C | ExperimentGroup::D => decision.admitted(),
        };
        let results = search_global_entries_on_connection(conn, &row.search_token, 5, 0)
            .unwrap_or_else(|error| panic!("synthetic search correctness query failed: {error}"));
        if should_be_searchable {
            assert!(
                results
                    .iter()
                    .any(|result| result.id == row.input.entry_id()),
                "included fixture row {} was not searchable in group {}",
                row.search_token,
                group.label()
            );
        } else {
            assert!(
                !results
                    .iter()
                    .any(|result| result.id == row.input.entry_id()),
                "excluded fixture row {} remained searchable in group {}",
                row.search_token,
                group.label()
            );
        }
        checked += 1;
    }
    assert!(
        checked >= 11,
        "search correctness must cover every key corpus class and all user lookalikes"
    );
    checked
}

fn index_from_token(token: &str) -> u64 {
    token
        .strip_prefix("isgfixture")
        .expect("fixture search token prefix")
        .parse()
        .expect("fixture index in search token")
}

fn sidecar_bytes(path: &Path, suffix: &str) -> u64 {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    fs::metadata(PathBuf::from(name))
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

fn sqlite_metrics(conn: &Connection, path: &Path) -> JsonValue {
    let page_count = count_rows(conn, "PRAGMA page_count");
    let page_size = count_rows(conn, "PRAGMA page_size");
    let main_db_bytes = fs::metadata(path)
        .expect("read isolated benchmark SQLite file size")
        .len();
    let wal_bytes = sidecar_bytes(path, "-wal");
    let shm_bytes = sidecar_bytes(path, "-shm");
    let fts_table_page_bytes = conn
        .query_row(
            "SELECT SUM(pgsize) FROM dbstat WHERE name LIKE 'global_entries_fts%'",
            [],
            |row| row.get::<_, Option<i64>>(0),
        )
        .ok()
        .flatten()
        .map(|value| value as u64);
    json!({
        "main_db_bytes": main_db_bytes,
        "wal_bytes_at_capture": wal_bytes,
        "shm_bytes_at_capture": shm_bytes,
        "main_db_plus_wal_bytes": main_db_bytes + wal_bytes,
        "page_count": page_count,
        "page_size": page_size,
        "journal_mode": conn.query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0)).ok(),
        "fts_shadow_table_page_bytes_dbstat": fts_table_page_bytes,
        "fts_shadow_table_page_bytes_status": if fts_table_page_bytes.is_some() { "measured" } else { "UNVERIFIED" },
        "database_is_fresh_throwaway_synthetic_only": true
    })
}

fn class_distribution(rows: &[CorpusRow]) -> BTreeMap<&'static str, u64> {
    let mut counts = BTreeMap::new();
    for row in rows {
        *counts.entry(row.class.label()).or_default() += 1;
    }
    counts
}

fn summarize(samples: &[&SampleResult]) -> JsonValue {
    assert!(!samples.is_empty());
    let values = samples
        .iter()
        .map(|sample| sample.wall_ms)
        .collect::<Vec<_>>();
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = if values.len() > 1 {
        values
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f64>()
            / (values.len() - 1) as f64
    } else {
        0.0
    };
    let mut sorted = values.clone();
    sorted.sort_by(f64::total_cmp);
    json!({
        "samples_in_repeat_order_ms": values,
        "sample_count": values.len(),
        "min_ms": round_ms(sorted[0]),
        "median_ms": round_ms(sorted[sorted.len() / 2]),
        "max_ms": round_ms(*sorted.last().expect("sample exists")),
        "mean_ms": round_ms(mean),
        "sample_standard_deviation_ms": round_ms(variance.sqrt()),
        "coefficient_of_variation_pct": if mean == 0.0 { 0.0 } else { round_ms(variance.sqrt() / mean * 100.0) }
    })
}

fn round_ms(value: f64) -> f64 {
    (value * 1_000.0).round() / 1_000.0
}

fn context(entries: u64, repeats: usize) -> JsonValue {
    json!({
        "dataset_type": "synthetic_global_index_ingestion_cost",
        "source_sha": env::var("ZEN_CANVAS_BENCHMARK_SOURCE_SHA").ok(),
        "source_tree": env::var("ZEN_CANVAS_BENCHMARK_SOURCE_TREE").ok(),
        "runner_os": env::var("RUNNER_OS").unwrap_or_else(|_| env::consts::OS.to_string()),
        "runner_arch": env::var("RUNNER_ARCH").unwrap_or_else(|_| env::consts::ARCH.to_string()),
        "runner_name": env::var("RUNNER_NAME").ok(),
        "logical_cpu_count": std::thread::available_parallelism().map(|count| count.get()).ok(),
        "cpu_model_memory_native_stat_and_rss": "UNVERIFIED",
        "github_run_id": env::var("GITHUB_RUN_ID").ok(),
        "github_job": env::var("GITHUB_JOB").ok(),
        "timestamp_unix_seconds": SystemTime::now().duration_since(UNIX_EPOCH).expect("system time after epoch").as_secs(),
        "seed": format!("0x{FIXTURE_SEED:x}"),
        "entry_count": entries,
        "repeat_count": repeats,
        "database_schema": "current production schema in a fresh isolated test database",
        "timing_scope": "in-memory synthetic metadata accessor + experimental classifier where applicable + Global Index repository batch upserts; excludes corpus generation, database initialization, volume setup, and post-run verification",
        "filesystem_enumeration": "NOT PERFORMED",
        "native_mft_spotlight_fsevents_and_real_metadata_stat": "UNVERIFIED"
    })
}

fn emit_record(record: &JsonValue) {
    let line = serde_json::to_string(record).expect("serialize ISG-00E JSONL record");
    println!("{line}");
    if let Ok(path) = env::var("ZC_GLOBAL_INDEX_COST_BENCHMARK_OUTPUT") {
        let output_path = PathBuf::from(path);
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).expect("create ISG-00E evidence directory");
        }
        let mut output = OpenOptions::new()
            .create(true)
            .append(true)
            .open(output_path)
            .expect("open ISG-00E JSONL evidence file");
        writeln!(output, "{line}").expect("append ISG-00E JSONL evidence");
    }
}

fn sample_record(context: &JsonValue, sample: &SampleResult) -> JsonValue {
    json!({
        "schema_version": 1,
        "record_type": "ingestion_sample",
        "context": context,
        "group": sample.group.label(),
        "repeat": sample.repeat,
        "execution_order_index": sample.execution_order_index,
        "wall_time_ms": round_ms(sample.wall_ms),
        "experimental_classifier_time_ms": round_ms(sample.classification_ms),
        "synthetic_metadata_accessor_time_ms": round_ms(sample.synthetic_metadata_accessor_ms),
        "synthetic_metadata_reads_attempted": sample.synthetic_metadata_read_attempted,
        "synthetic_metadata_reads_succeeded": sample.synthetic_metadata_read_succeeded,
        "input_candidates": sample.input_candidates,
        "native_metadata_stat_attempted": "UNVERIFIED",
        "native_metadata_stat_succeeded": "UNVERIFIED",
        "policy_evaluated": sample.policy_evaluated,
        "included": sample.included,
        "minimal_ancestor": sample.minimal_ancestor,
        "excluded": sample.excluded,
        "undetermined": sample.undetermined,
        "excluded_by_reason": sample.excluded_by_reason,
        "decisions_by_class": sample.decisions_by_class,
        "admitted_to_sink": sample.admitted_rows,
        "db_upsert_returned_rows": sample.upsert_returned_rows,
        "db_upsert_batch_count": sample.upsert_batches,
        "global_entries_rows_after_run": sample.database_rows,
        "global_volumes_entry_count_after_run": sample.volume_entry_count,
        "fts_sentinel_query": FTS_SENTINEL,
        "fts_sentinel_match_rows": sample.fts_sentinel_matches,
        "fts_match_count_is_fixture_specific_not_general_trigger_ratio": true,
        "sqlite": sample.sqlite_metrics,
        "ai_jobs_rows": sample.ai_job_rows,
        "ai_job_items_rows": sample.ai_job_item_rows,
        "identity_set_correct": sample.identity_set_correct,
        "search_cases_checked": sample.search_cases_checked,
        "search_cases_correct": sample.search_cases_correct,
        "native_stat_or_platform_indexing_claimed": false
    })
}

fn run_benchmark(entries: u64, repeats: usize) {
    assert!(ALLOWED_ENTRIES.contains(&entries));
    assert!(repeats > 0);
    let rows = corpus(entries);
    let decisions = expected_decisions(&rows);
    let classifier_oracle = decision_counts(&rows, &decisions);
    let context = context(entries, repeats);
    if let Ok(path) = env::var("ZC_GLOBAL_INDEX_COST_BENCHMARK_OUTPUT") {
        if let Some(parent) = PathBuf::from(&path).parent() {
            fs::create_dir_all(parent).expect("create ISG-00E evidence directory");
        }
        fs::write(path, "").expect("truncate ISG-00E JSONL evidence file");
    }
    emit_record(&json!({
        "schema_version": 1,
        "record_type": "fixture",
        "context": context,
        "distribution_and_classifier_oracle": classifier_oracle,
        "distribution_actual_counts": class_distribution(&rows),
        "all_paths_are_synthetic_and_unopened": true,
        "distribution_schedule": "fixed 100-slot cycle; supports exactly divisible 1k/10k/100k sizes",
        "groups": ["A_broad_baseline", "B_dry_run_all_rows_written", "C_experimental_sink_gate", "D_synthetic_pre_metadata_gate"],
        "classifier_status": "experimental model of proposed ADR-0014 rules; not approved product policy",
        "D_metadata_accessor": "deterministic in-memory field projection; no filesystem stat",
        "seed": format!("0x{FIXTURE_SEED:x}"),
        "repeat_order": "four group order rotates by repeat to spread within-run order effects"
    }));

    let mut results = Vec::with_capacity(repeats * ExperimentGroup::ALL.len());
    for repeat in 0..repeats {
        for order in 0..ExperimentGroup::ALL.len() {
            let group = ExperimentGroup::ALL[(order + repeat) % ExperimentGroup::ALL.len()];
            let sample = run_group(&rows, &decisions, group, repeat + 1, order + 1);
            emit_record(&sample_record(&context, &sample));
            results.push(sample);
        }
    }

    for group in ExperimentGroup::ALL {
        let samples = results
            .iter()
            .filter(|sample| sample.group == group)
            .collect::<Vec<_>>();
        emit_record(&json!({
            "schema_version": 1,
            "record_type": "group_summary",
            "context": context,
            "group": group.label(),
            "wall_time": summarize(&samples),
            "row_counts_by_repeat": samples.iter().map(|sample| sample.database_rows).collect::<Vec<_>>(),
            "sqlite_main_plus_wal_bytes_by_repeat": samples.iter().map(|sample| sample.sqlite_metrics["main_db_plus_wal_bytes"].as_u64().unwrap_or(0)).collect::<Vec<_>>(),
            "fts_sentinel_match_rows_by_repeat": samples.iter().map(|sample| sample.fts_sentinel_matches).collect::<Vec<_>>(),
            "metadata_reads_by_repeat": samples.iter().map(|sample| sample.synthetic_metadata_read_attempted).collect::<Vec<_>>(),
            "classification_counts_by_repeat": samples.iter().map(|sample| json!({
                "policy_evaluated": sample.policy_evaluated,
                "included": sample.included,
                "minimal_ancestor": sample.minimal_ancestor,
                "excluded": sample.excluded,
                "undetermined": sample.undetermined,
                "excluded_by_reason": sample.excluded_by_reason,
            })).collect::<Vec<_>>(),
            "all_identity_and_search_checks_passed": samples.iter().all(|sample| sample.identity_set_correct && sample.search_cases_correct)
        }));
    }

    for repeat in 0..repeats {
        let sample = |group| {
            results
                .iter()
                .find(|result| result.repeat == repeat + 1 && result.group == group)
                .expect("paired group sample")
        };
        let baseline = sample(ExperimentGroup::A);
        let c = sample(ExperimentGroup::C);
        let d = sample(ExperimentGroup::D);
        emit_record(&json!({
            "schema_version": 1,
            "record_type": "paired_comparison",
            "context": context,
            "repeat": repeat + 1,
            "A_rows": baseline.database_rows,
            "C_rows": c.database_rows,
            "D_rows": d.database_rows,
            "C_rows_avoided_vs_A": baseline.database_rows - c.database_rows,
            "D_rows_avoided_vs_A": baseline.database_rows - d.database_rows,
            "A_main_plus_wal_bytes": baseline.sqlite_metrics["main_db_plus_wal_bytes"],
            "C_main_plus_wal_bytes": c.sqlite_metrics["main_db_plus_wal_bytes"],
            "D_main_plus_wal_bytes": d.sqlite_metrics["main_db_plus_wal_bytes"],
            "A_wall_ms": round_ms(baseline.wall_ms),
            "C_wall_ms": round_ms(c.wall_ms),
            "D_wall_ms": round_ms(d.wall_ms),
            "C_vs_A_wall_time_ratio": if baseline.wall_ms == 0.0 { JsonValue::Null } else { json!(round_ms(c.wall_ms / baseline.wall_ms)) },
            "D_vs_A_wall_time_ratio": if baseline.wall_ms == 0.0 { JsonValue::Null } else { json!(round_ms(d.wall_ms / baseline.wall_ms)) },
            "C_and_D_stored_identity_sets_equal": c.database_rows == d.database_rows && c.identity_set_correct && d.identity_set_correct
        }));
    }

    emit_record(&json!({
        "schema_version": 1,
        "record_type": "limitations",
        "context": context,
        "windows_mft_native_enumeration_and_staging_savings": "UNVERIFIED",
        "macos_spotlight_gather_savings": "UNVERIFIED",
        "real_filesystem_stat_cpu_rss_io_and_wall_savings": "UNVERIFIED",
        "real_user_database_fk_history_migration_and_ai_side_effects": "NOT TESTED; every sample used a new throwaway schema database with no user state",
        "search_correctness_scope": "exact synthetic identity set plus representative production Global Search queries for each corpus class and every user path-name lookalike",
        "historical_12_query_p95_gate_changed": false
    }));
}

fn decision_counts(rows: &[CorpusRow], decisions: &[ExperimentalDecision]) -> JsonValue {
    let mut counts = BTreeMap::<&'static str, u64>::new();
    let mut excluded_by_reason = BTreeMap::<&'static str, u64>::new();
    let mut decisions_by_class = BTreeMap::<String, BTreeMap<&'static str, u64>>::new();
    let mut included = 0;
    let mut minimal_ancestor = 0;
    let mut excluded = 0;
    let mut undetermined = 0;
    for (row, decision) in rows.iter().zip(decisions) {
        *counts.entry(decision.label()).or_default() += 1;
        *decisions_by_class
            .entry(row.class.label().to_string())
            .or_default()
            .entry(decision.label())
            .or_default() += 1;
        match decision {
            ExperimentalDecision::Include => included += 1,
            ExperimentalDecision::MinimalAncestor => minimal_ancestor += 1,
            ExperimentalDecision::Exclude(reason) => {
                excluded += 1;
                *excluded_by_reason.entry(reason).or_default() += 1;
            }
            ExperimentalDecision::Undetermined => undetermined += 1,
        }
    }
    json!({
        "class_counts": class_distribution(rows),
        "experimental_decisions_by_class": decisions_by_class,
        "experimental_decision_totals": {
            "policy_evaluated": rows.len(),
            "included": included,
            "minimal_ancestor": minimal_ancestor,
            "excluded": excluded,
            "undetermined": undetermined,
            "conservation_holds": included + minimal_ancestor + excluded + undetermined == rows.len(),
            "excluded_by_reason": excluded_by_reason
        },
        "decision_label_counts": counts
    })
}

#[test]
fn global_search_index_cost_benchmark_smoke() {
    run_benchmark(SMOKE_ENTRIES, 1);
}

#[test]
#[ignore = "synthetic Global Index ingestion-cost benchmark; set ZC_GLOBAL_INDEX_COST_BENCHMARK_ENTRIES to 1k, 10k, or 100k"]
fn global_search_index_cost_synthetic_benchmark() {
    run_benchmark(configured_entries(), REPEAT_SAMPLES);
}
