use super::*;
use crate::{
    db::Database,
    global_index::{GlobalEntryInput, GlobalVolume, SourceOperationKind, SourceOperationRoute},
};
use std::{
    fs,
    path::{Path, PathBuf},
};

struct Fixture {
    // Drop database connections before the task-owned directory on Windows.
    db: Database,
    source: GlobalVolume,
    root: PathBuf,
    _cleanup: FixtureCleanup,
}
struct FixtureCleanup(PathBuf);
impl Drop for FixtureCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("zen-smart-lean-{}", uuid::Uuid::new_v4()));
        Self::new_with_source_root(root.clone(), root)
    }
    fn new_with_source_root(root: PathBuf, source_root: PathBuf) -> Self {
        fs::create_dir_all(&root).unwrap();
        let db = Database::open(root.join("fixture.sqlite")).unwrap();
        let mut source = crate::global_index::tests::test_volume();
        source.mount_path = source_root.to_string_lossy().into_owned();
        source.scope_authorization_state = "authorized".into();
        source.enabled = true;
        db.upsert_global_volume(&source).unwrap();
        Self {
            _cleanup: FixtureCleanup(root.clone()),
            root,
            db,
            source,
        }
    }
    fn file(&self, path: &str) -> GlobalEntryInput {
        let full = self.root.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(&full, b"fixture").unwrap();
        let mut entry = GlobalEntryInput::from_path(&self.source.id, &full, &self.source.provider);
        super::native::enrich_fixture_identity(&mut entry);
        entry
    }
    fn owner(&self) -> super::super::repository::SourceOperationOwner {
        self.db
            .claim_global_index_operation(
                &self.source,
                &uuid::Uuid::new_v4().to_string(),
                SourceOperationKind::Rebuild,
                SourceOperationRoute::Coordinator,
            )
            .unwrap()
            .unwrap()
    }
    fn dependencies(&self) -> GlobalEntryInput {
        self.file("project/package.json");
        self.file("project/node_modules/.package-lock.json");
        self.file("project/node_modules/lib/noise.js")
    }
}
#[test]
fn four_decisions_and_safety_precedence_are_distinct() {
    let mut policy = CoveragePolicy::default();
    assert_eq!(evaluate(&policy, &Evidence::default()), Decision::Include);
    assert_eq!(
        evaluate(
            &policy,
            &Evidence {
                performance_reason: Some("dependencies"),
                ..Evidence::default()
            }
        ),
        Decision::Exclude("dependencies")
    );
    assert_eq!(
        evaluate(
            &policy,
            &Evidence {
                performance_reason: Some("dependencies"),
                directory: true,
                ..Evidence::default()
            }
        ),
        Decision::MinimalAncestor("dependencies")
    );
    assert_eq!(
        evaluate(
            &policy,
            &Evidence {
                uncertain: Some("alias"),
                performance_reason: Some("dependencies"),
                ..Evidence::default()
            }
        ),
        Decision::Undetermined("alias")
    );
    policy.mode = CoverageMode::Custom;
    assert_eq!(
        evaluate(
            &policy,
            &Evidence {
                explicit_rule: Some(true),
                performance_reason: Some("dependencies"),
                ..Evidence::default()
            }
        ),
        Decision::Include
    );
    assert_eq!(
        evaluate(
            &policy,
            &Evidence {
                protected: true,
                explicit_rule: Some(true),
                ..Evidence::default()
            }
        ),
        Decision::Exclude("protected_boundary")
    );
}
#[test]
fn native_false_exclusion_oracle_preserves_user_files() {
    let f = Fixture::new();
    // Literal expected-include ledger, independent of native evaluator branches.
    let names = [
        "Projects/Windows/README.md",
        "Design/build/logo.ai",
        "AppData/preferences.json",
        "src/main.rs",
        "README",
        ".vscode/settings.json",
        "logs/important.log",
        "Pictures/first-light.jpg",
        "资料/设计稿.png",
        "Program Files2/notes.txt",
        "node_modules/notes.md",
        "build/report.pdf",
        "target/notes.md",
        ".venv/notes.txt",
        "tmp/meeting-notes.md",
        "cache/indexed-report.pdf",
    ];
    let entries = names.iter().map(|p| f.file(p)).collect::<Vec<_>>();
    assert_eq!(
        observe_batch(&CoveragePolicy::default(), &f.source, &entries),
        vec![Decision::Include; names.len()]
    );
}

#[test]
fn native_system_temporary_root_is_excluded_without_excluding_tmp_named_folders() {
    let source_root = std::env::temp_dir();
    let fixture_root = source_root.join(format!("zen-smart-lean-temp-{}", uuid::Uuid::new_v4()));
    let f = Fixture::new_with_source_root(fixture_root, source_root.clone());
    let temporary = f.file("task-owned-temp/volatile.txt");

    assert!(Path::new(&temporary.path).starts_with(&source_root));
    assert_eq!(
        observe_batch(
            &CoveragePolicy::default(),
            &f.source,
            std::slice::from_ref(&temporary)
        ),
        vec![Decision::Exclude("known_temporary_root")]
    );
}

#[test]
fn proven_dependency_and_build_trees_are_excluded_but_generic_names_are_not() {
    let f = Fixture::new();
    let npm = f.dependencies();
    let python = f.file("python/.venv/lib/site-packages/module.py");
    f.file("python/.venv/pyvenv.cfg");
    f.file("python/.venv/Scripts/activate");
    let rust = f.file("rust/target/debug/generated.o");
    f.file("rust/Cargo.toml");
    f.file("rust/target/.rustc_info.json");
    let cmake = f.file("cmake/build/CMakeFiles/generated.o");
    f.file("cmake/build/CMakeCache.txt");
    f.file("cmake/build/CMakeFiles/.fixture");
    let lookalikes = [
        f.file("cache/valuable.json"),
        f.file("tmp/valuable.txt"),
        f.file("build/valuable-design.pdf"),
        f.file("target/valuable-notes.md"),
        f.file(".venv/valuable-notes.txt"),
    ];

    let mut candidates = vec![npm, python, rust, cmake];
    candidates.extend(lookalikes);
    assert_eq!(
        observe_batch(&CoveragePolicy::default(), &f.source, &candidates),
        vec![
            Decision::Exclude("npm_dependencies"),
            Decision::Exclude("python_environment"),
            Decision::Exclude("rust_build_output"),
            Decision::Exclude("cmake_build_output"),
            Decision::Include,
            Decision::Include,
            Decision::Include,
            Decision::Include,
            Decision::Include,
        ]
    );
}

#[test]
fn ordinary_documents_photos_and_project_sources_remain_searchable() {
    let f = Fixture::new();
    let document = f.file("docs/meeting-notes.md");
    let photo = f.file("Pictures/first-light.jpg");
    let source = f.file("project/src/main.rs");
    let dependency = f.dependencies();
    let owner = f.owner();
    f.db.upsert_global_entries_batch_owned(
        &[
            document.clone(),
            photo.clone(),
            source.clone(),
            dependency.clone(),
        ],
        &owner,
    )
    .unwrap();

    for (query, expected) in [
        ("meeting-notes", document),
        ("first-light", photo),
        ("main", source),
    ] {
        let results = f.db.search_global_entries(query, 25, 0).unwrap();
        assert!(
            results.iter().any(|entry| entry.path == expected.path),
            "missing {query}: {results:?}"
        );
    }
    assert!(f
        .db
        .search_global_entries("noise", 25, 0)
        .unwrap()
        .is_empty());
}

#[test]
fn production_admission_excludes_only_proven_trees_and_keeps_graph_identity() {
    let f = Fixture::new();
    let dependency = f.dependencies();
    let source = f.file("project/src/main.js");
    let owner = f.owner();
    let dir = GlobalEntryInput::from_path(
        &f.source.id,
        &f.root.join("project/node_modules"),
        &f.source.provider,
    );
    assert_eq!(
        f.db.upsert_global_entries_batch_owned(
            &[dependency.clone(), source.clone(), dir.clone()],
            &owner
        )
        .unwrap(),
        3
    );
    assert!(f
        .db
        .get_global_entry(&dependency.entry_id())
        .unwrap()
        .is_none());
    assert!(
        !f.db
            .get_global_entry(&source.entry_id())
            .unwrap()
            .unwrap()
            .is_stale
    );
    assert!(
        f.db.get_global_entry(&dir.entry_id())
            .unwrap()
            .unwrap()
            .is_stale
    );
    assert_eq!(
        f.db.global_path_by_platform_identity(&f.source.id, &dir.platform_file_id)
            .unwrap(),
        Some(dir.path.clone())
    );
    assert!(f
        .db
        .search_global_entries("noise", 20, 0)
        .unwrap()
        .is_empty());
    let progress = f.db.global_index_coverage().unwrap().sources.remove(0);
    assert_eq!(
        (progress.included, progress.excluded, progress.undetermined),
        (1, 2, 0)
    );
    f.db.mark_global_entry_stale_owned(&dir.entry_id(), &owner)
        .unwrap();
    assert!(f
        .db
        .global_path_by_platform_identity(&f.source.id, &dir.platform_file_id)
        .unwrap()
        .is_none());
}

#[test]
fn merged_scope_write_preserves_rebuild_incremental_fts_and_managed_links() {
    use crate::global_index::AddManagedScopeRequest;

    let f = Fixture::new();
    let current_path = f.root.join("include/currenttoken.txt");
    fs::create_dir_all(current_path.parent().unwrap()).unwrap();
    fs::write(&current_path, b"managed input").unwrap();
    let mut current = GlobalEntryInput::from_path(&f.source.id, &current_path, &f.source.provider);
    // Keep the synthetic native identity stable across this fixture rename.
    current.platform_file_id = "mac:fixture:managed-current".into();
    let duplicate_included = f.file("include/duplicate.txt");
    let duplicate_elsewhere = f.file("archive/duplicate.txt");
    let excluded = f.file("exclude/hiddenneedle.txt");
    let include_path = f.root.join("include");
    let exclude_path = f.root.join("exclude");
    let scope =
        f.db.add_managed_scope(AddManagedScopeRequest {
            path: include_path.to_string_lossy().into_owned(),
            global_entry_id: None,
            enabled: true,
            allow_local_ai: true,
            allow_cloud_ai: false,
        })
        .unwrap();
    let entries = [
        current.clone(),
        duplicate_included.clone(),
        duplicate_elsewhere.clone(),
        excluded.clone(),
    ];

    // Seed existing rows and managed associations, then verify that a policy
    // rebuild can retire/reinsert rows without losing those relationships.
    f.db.upsert_global_entries_batch(&entries).unwrap();
    let include_rule = bind_rule(&f.source, &include_path.to_string_lossy(), true).unwrap();
    let exclude_rule = bind_rule(&f.source, &exclude_path.to_string_lossy(), false).unwrap();
    f.db.save_global_index_coverage(1, CoverageMode::Custom, vec![include_rule, exclude_rule])
        .unwrap();
    let owner = f.owner();
    f.db.mark_global_entries_stale_for_volume_owned(&owner)
        .unwrap();
    f.db.upsert_global_entries_batch_owned(&entries, &owner)
        .unwrap();
    f.db.finish_global_index_operation(&owner, "ready", None, Some(1), Some(1), true)
        .unwrap();

    let conn = f.db.conn().unwrap();
    let status: (i64, String) = conn
        .query_row(
            "SELECT is_stale,scope_decision FROM global_entries WHERE id=?1",
            [current.entry_id()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, (0, "include".into()));
    let excluded_status: (i64, String) = conn
        .query_row(
            "SELECT is_stale,scope_decision FROM global_entries WHERE id=?1",
            [excluded.entry_id()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(excluded_status, (1, "custom_exclude".into()));
    assert!(f
        .db
        .search_global_entries("hiddenneedle", 20, 0)
        .unwrap()
        .is_empty());
    assert_eq!(
        f.db.search_global_entries("duplicate", 20, 0)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        f.db.search_global_entries("currenttoken", 20, 0)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM managed_entries WHERE global_entry_id=?1 AND managed_scope_id=?2 AND enabled=1",
            rusqlite::params![current.entry_id(), scope.id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        1
    );
    conn.execute(
        "INSERT INTO global_entries_fts(global_entries_fts,rank) VALUES('integrity-check',1)",
        [],
    )
    .unwrap();
    drop(conn);

    let renamed_path = f.root.join("include/renamedtoken.txt");
    fs::rename(&current_path, &renamed_path).unwrap();
    current.path = renamed_path.to_string_lossy().into_owned();
    current.name = "renamedtoken.txt".into();
    current.last_seen_at = crate::global_index::unix_now();
    let owner =
        f.db.claim_global_index_operation(
            &f.source,
            &uuid::Uuid::new_v4().to_string(),
            SourceOperationKind::Incremental,
            SourceOperationRoute::Coordinator,
        )
        .unwrap()
        .unwrap();
    f.db.upsert_global_entries_batch_owned(std::slice::from_ref(&current), &owner)
        .unwrap();
    f.db.finish_global_index_operation(&owner, "ready", None, None, Some(2), false)
        .unwrap();

    assert!(f
        .db
        .search_global_entries("currenttoken", 20, 0)
        .unwrap()
        .is_empty());
    let renamed = f.db.search_global_entries("renamedtoken", 20, 0).unwrap();
    assert_eq!(renamed.len(), 1);
    assert_eq!(renamed[0].id, current.entry_id());
    let conn = f.db.conn().unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM managed_entries WHERE global_entry_id=?1 AND managed_scope_id=?2 AND enabled=1",
            rusqlite::params![current.entry_id(), scope.id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        1,
        "incremental rename keeps the managed entry association"
    );
    assert!(conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM ai_jobs WHERE global_entry_id=?1 AND managed_scope_id=?2 AND status='stale') AND EXISTS(SELECT 1 FROM ai_jobs WHERE global_entry_id=?1 AND managed_scope_id=?2 AND status='pending')",
            rusqlite::params![current.entry_id(), scope.id],
            |row| row.get::<_, bool>(0),
        )
        .unwrap());
    conn.execute(
        "INSERT INTO global_entries_fts(global_entries_fts,rank) VALUES('integrity-check',1)",
        [],
    )
    .unwrap();
}
#[test]
fn policy_switch_fences_old_writes_hides_old_results_and_survives_restart() {
    let f = Fixture::new();
    let dependency = f.dependencies();
    let doc = f.file("docs/keep.md");
    f.db.upsert_global_entries_batch(&[dependency.clone(), doc.clone()])
        .unwrap();
    let owner = f.owner();
    f.db.save_global_index_coverage(1, CoverageMode::SmartLean, vec![])
        .unwrap();
    assert!(f
        .db
        .upsert_global_entries_batch_owned(std::slice::from_ref(&dependency), &owner)
        .is_err());
    assert!(!f
        .db
        .finish_global_index_operation(&owner, "ready", None, Some(1), Some(1), true)
        .unwrap());
    assert!(f
        .db
        .search_global_entries("keep", 20, 0)
        .unwrap()
        .is_empty());
    assert!(f.db.get_global_entry(&doc.entry_id()).unwrap().is_some());
    let reopened = Database::open(f.root.join("fixture.sqlite")).unwrap();
    assert!(reopened.global_index_coverage().unwrap().sources[0].pending);
    assert!(!reopened.global_index_status().unwrap().collection_complete);
    let owner = f.owner();
    f.db.mark_global_entries_stale_for_volume_owned(&owner)
        .unwrap();
    f.db.upsert_global_entries_batch_owned(&[dependency.clone(), doc.clone()], &owner)
        .unwrap();
    f.db.finish_global_index_operation(&owner, "ready", None, Some(2), Some(2), true)
        .unwrap();
    assert!(f
        .db
        .search_global_entries("noise", 20, 0)
        .unwrap()
        .is_empty());
    assert_eq!(f.db.search_global_entries("keep", 20, 0).unwrap().len(), 1);
    assert!(
        f.db.get_global_entry(&dependency.entry_id())
            .unwrap()
            .unwrap()
            .is_stale
    );
    assert!(!f.db.global_index_coverage().unwrap().sources[0].pending);
    let conn = f.db.conn().unwrap();
    assert_eq!(
        conn.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
}

#[test]
fn stale_policy_writer_cannot_replace_existing_broad_coverage_after_restart() {
    let f = Fixture::new();
    assert_eq!(
        f.db.global_index_coverage().unwrap().policy.mode,
        CoverageMode::SmartLean
    );
    f.db.save_global_index_coverage(1, CoverageMode::BroadCoverage, vec![])
        .unwrap();

    let stale =
        f.db.save_global_index_coverage(1, CoverageMode::SmartLean, vec![]);
    assert!(
        matches!(stale, Err(crate::db::DbError::Validation(message)) if message == "coverage_revision_conflict")
    );

    let reopened = Database::open(f.root.join("fixture.sqlite")).unwrap();
    let snapshot = reopened.global_index_coverage().unwrap();
    assert_eq!(snapshot.policy.revision, 2);
    assert_eq!(snapshot.policy.mode, CoverageMode::BroadCoverage);
    assert!(snapshot.sources[0].pending);
}
#[test]
fn lawful_include_and_broad_restore_excluded_files_without_source_opt_in() {
    let f = Fixture::new();
    let dependency = f.dependencies();
    let rule = bind_rule(
        &f.source,
        &f.root.join("project/node_modules").to_string_lossy(),
        true,
    )
    .unwrap();
    let mut p = CoveragePolicy {
        mode: CoverageMode::Custom,
        rules: vec![rule],
        ..CoveragePolicy::default()
    };
    assert_eq!(
        observe_batch(&p, &f.source, std::slice::from_ref(&dependency)),
        vec![Decision::Include]
    );
    p.mode = CoverageMode::BroadCoverage;
    p.rules.clear();
    assert_eq!(
        observe_batch(&p, &f.source, &[dependency]),
        vec![Decision::Include]
    );
    let mut disabled = f.source.clone();
    disabled.enabled = false;
    assert!(bind_rule(&disabled, &f.root.to_string_lossy(), true).is_err());
    disabled.enabled = true;
    disabled.platform = "windows".into();
    disabled.scope_authorization_state = "not_authorized".into();
    assert!(bind_rule(&disabled, &f.root.to_string_lossy(), true).is_err());
}
#[test]
fn moves_and_context_loss_reclassify_without_cached_false_exclusions() {
    let f = Fixture::new();
    let dependency = f.dependencies();
    let p = CoveragePolicy::default();
    assert_eq!(
        observe_batch(&p, &f.source, std::slice::from_ref(&dependency)),
        vec![Decision::Exclude("npm_dependencies")]
    );
    let target = f.root.join("restored.js");
    fs::rename(&dependency.path, &target).unwrap();
    let moved = GlobalEntryInput::from_path(&f.source.id, &target, &f.source.provider);
    assert_eq!(
        observe_batch(&p, &f.source, &[moved]),
        vec![Decision::Include]
    );
    fs::remove_file(f.root.join("project/node_modules/.package-lock.json")).unwrap();
    let entry = f.file("project/node_modules/new.js");
    assert_eq!(
        observe_batch(&p, &f.source, &[entry]),
        vec![Decision::Include]
    );
}
#[cfg(unix)]
#[test]
fn aliases_hardlinks_and_missing_metadata_are_undetermined_not_hidden() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let dependency = f.dependencies();
    fs::hard_link(&dependency.path, f.root.join("valuable.js")).unwrap();
    assert!(matches!(
        observe_batch(&CoveragePolicy::default(), &f.source, &[dependency])[0],
        Decision::Undetermined(_)
    ));
    symlink(f.root.join("project/node_modules"), f.root.join("alias")).unwrap();
    let e = GlobalEntryInput::from_path(
        &f.source.id,
        &f.root.join("alias/lib/noise.js"),
        &f.source.provider,
    );
    assert!(matches!(
        observe_batch(&CoveragePolicy::default(), &f.source, &[e])[0],
        Decision::Undetermined(_)
    ));
}
#[test]
fn outside_authorized_root_is_fenced_without_probing_it() {
    let f = Fixture::new();
    let mut e = f.file("owned.txt");
    e.path = Path::new(&f.root)
        .join("../foreign.txt")
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        observe_batch(&CoveragePolicy::default(), &f.source, &[e]),
        vec![Decision::Exclude("outside_authorized_source")]
    );
}

/// Writer-only paired benchmark. The baseline branch retains the previous UPSERT,
/// per-row scope decision UPDATE, and empty-scope AI fingerprint work; the candidate
/// branch uses the optimized production path. Both use the same task-owned files.
#[test]
#[ignore = "explicit bounded performance run"]
fn global_index_writer_ingestion_100k_pair() {
    use std::time::Instant;

    let n = std::env::var("ZEN_GLOBAL_WRITER_ENTRIES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(100_000);
    assert_eq!(
        n, 100_000,
        "this paired workflow is intentionally fixed at 100k"
    );
    let f = Fixture::new();
    f.file("project/package.json");
    f.file("project/node_modules/.package-lock.json");
    f.file("rust/Cargo.toml");
    f.file("rust/target/.rustc_info.json");
    let entries = (0..n)
        .map(|index| {
            f.file(&match index % 10 {
                0..=2 => format!("project/node_modules/lib/dependency{index}.js"),
                3 => format!("rust/target/debug/generated{index}.o"),
                4 => format!("project/src/source{index}.rs"),
                5 => format!("project/docs/README{index}.md"),
                6 => format!("documents/report{index}.pdf"),
                7 => format!("Projects/Windows/README{index}.md"),
                8 => format!("Design/build/logo{index}.ai"),
                _ => format!("资料/作品{index}.png"),
            })
        })
        .collect::<Vec<_>>();
    let expected_rows = (0..n).filter(|index| index % 10 >= 4).count() as i64;
    let mut system = sysinfo::System::new();
    let pid = sysinfo::get_current_pid().unwrap();
    let mut records = Vec::new();

    for repetition in 0..4 {
        // Balance first/second position across four paired repetitions.
        let groups = if repetition % 2 == 0 {
            ["legacy_writer", "optimized_writer"]
        } else {
            ["optimized_writer", "legacy_writer"]
        };
        for group in groups {
            let canonical_writer = group == "legacy_writer";
            // The baseline models the previous Windows 64-row IPC writer.
            // The candidate models MFT's native 512-row batch, subject to the
            // production IPC frame-size bound. Spotlight already writes 512.
            let writer_batch_size = if cfg!(target_os = "windows") && canonical_writer {
                64
            } else {
                512
            };
            let writer_batches = n.div_ceil(writer_batch_size);
            let db_path = f.root.join(format!("writer-{repetition}-{group}.sqlite"));
            let db = Database::open(&db_path).unwrap();
            db.upsert_global_volume(&f.source).unwrap();
            db.save_global_index_coverage(1, CoverageMode::SmartLean, vec![])
                .unwrap();
            let owner = db
                .claim_global_index_operation(
                    &f.source,
                    &uuid::Uuid::new_v4().to_string(),
                    SourceOperationKind::Rebuild,
                    SourceOperationRoute::Coordinator,
                )
                .unwrap()
                .unwrap();
            system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
            let rss_before = system.process(pid).map_or(0, sysinfo::Process::memory);
            let mut rss_peak = rss_before;
            let _ = super::native::take_metadata_attempts();
            let mut passes = Vec::new();

            for pass in ["initial", "replay"] {
                let begin = Instant::now();
                let mut sampling = std::time::Duration::ZERO;
                let mut writer_timing =
                    crate::global_index::repository::GlobalIndexBatchTiming::default();
                let mut sampled_batches = 0;
                let sample_stride = writer_batches.div_ceil(8).max(1);
                for (batch_index, batch) in entries.chunks(writer_batch_size).enumerate() {
                    let write = || {
                        db.upsert_global_entries_batch_owned_timed(batch, &owner)
                            .unwrap()
                    };
                    let (_, timing) = if canonical_writer {
                        crate::global_index::repository::with_canonical_scope_decision_writes(write)
                    } else {
                        write()
                    };
                    writer_timing.add(timing);
                    if batch_index % sample_stride == 0 || batch_index + 1 == writer_batches {
                        let sample_begin = Instant::now();
                        system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
                        rss_peak =
                            rss_peak.max(system.process(pid).map_or(0, sysinfo::Process::memory));
                        sampling += sample_begin.elapsed();
                        sampled_batches += 1;
                    }
                }
                let instrumented = begin.elapsed();
                let elapsed = instrumented.saturating_sub(sampling).as_secs_f64();
                let pass_record = serde_json::json!({
                    "pass": pass,
                    "seconds": elapsed,
                    "candidatesPerSecond": n as f64 / elapsed,
                    "rssSamplingSeconds": sampling.as_secs_f64(),
                    "rssSampledBatches": sampled_batches,
                    "coveragePolicyLoadSeconds": writer_timing.coverage_policy_load.as_secs_f64(),
                    "coverageEvaluationSeconds": writer_timing.coverage_evaluation.as_secs_f64(),
                    "sqliteTransactionSeconds": writer_timing.sqlite_transaction.as_secs_f64(),
                    "connectionAcquisitionSeconds": writer_timing.connection_acquisition.as_secs_f64(),
                    "transactionAcquisitionSeconds": writer_timing.transaction_acquisition.as_secs_f64(),
                    "transactionSetupSeconds": writer_timing.transaction_setup.as_secs_f64(),
                    "entryUpsertSeconds": writer_timing.entry_upsert.as_secs_f64(),
                    "aiTaskQualificationSeconds": writer_timing.ai_task_qualification.as_secs_f64(),
                    "recordDecisionsSeconds": writer_timing.record_decisions.as_secs_f64(),
                    "commitSeconds": writer_timing.commit.as_secs_f64(),
                    "timedWriterBatches": writer_batches,
                    "writerBatchSize": writer_batch_size,
                    "writerBatches": writer_batches
                });
                println!("GLOBAL_INDEX_WRITER_PAIR {pass_record}");
                passes.push(pass_record);
            }

            db.finish_global_index_operation(&owner, "ready", None, Some(1), Some(1), true)
                .unwrap();
            let conn = db.conn().unwrap();
            let rows: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM global_entries WHERE is_stale=0",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(rows, expected_rows, "no false exclusions in writer pair");
            assert!(
                !db.search_global_entries("README", 20, 0)
                    .unwrap()
                    .is_empty(),
                "FTS must expose visible filename rows"
            );
            conn.execute(
                "INSERT INTO global_entries_fts(global_entries_fts,rank) VALUES('integrity-check',1)",
                [],
            )
            .unwrap();
            assert_eq!(
                conn.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
                0
            );
            let fts_bytes: i64 = conn
                .query_row(
                    "SELECT COALESCE(SUM(pgsize),0) FROM dbstat WHERE name GLOB 'global_entries_fts*'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            let page_bytes: i64 = conn
                .query_row("PRAGMA page_count", [], |row| row.get::<_, i64>(0))
                .unwrap()
                * conn
                    .query_row("PRAGMA page_size", [], |row| row.get::<_, i64>(0))
                    .unwrap();
            let source_facts: (i64, i64) = conn
                .query_row(
                    "SELECT active_rows,active_ids FROM global_entry_source_facts WHERE volume_id=?1",
                    [&f.source.id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(source_facts, (expected_rows, expected_rows));
            drop(conn);
            let metadata_attempts = super::native::take_metadata_attempts();
            let wal_path = std::path::PathBuf::from(format!("{}-wal", db_path.display()));
            let database_file_bytes = fs::metadata(&db_path).map_or(0, |metadata| metadata.len());
            let wal_bytes = fs::metadata(&wal_path).map_or(0, |metadata| metadata.len());
            let shm_path = std::path::PathBuf::from(format!("{}-shm", db_path.display()));
            let shm_bytes = fs::metadata(&shm_path).map_or(0, |metadata| metadata.len());
            let record = serde_json::json!({
                "platform": std::env::consts::OS,
                "arch": std::env::consts::ARCH,
                "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
                "group": group,
                "repetition": repetition,
                "candidates": n,
                "rows": rows,
                "ftsBytes": fts_bytes,
                "databasePageBytes": page_bytes,
                "databaseFileBytes": database_file_bytes,
                "walBytes": wal_bytes,
                "shmBytes": shm_bytes,
                "databaseAndWalBytes": database_file_bytes + wal_bytes,
                "rssBeforeBytes": rss_before,
                "rssPeakSampledBytes": rss_peak,
                "nativeObserverMetadataAttempts": metadata_attempts,
                "sourceFactsActiveRows": source_facts.0,
                "passes": passes
            });
            println!("GLOBAL_INDEX_WRITER_RESULT {record}");
            records.push(record);
        }
    }
    if let Ok(path) = std::env::var("ZEN_GLOBAL_WRITER_REPORT") {
        fs::write(path, serde_json::to_vec_pretty(&records).unwrap()).unwrap();
    }
}

/// Isolates the marginal insert cost of the live search and source-facts
/// triggers. Trigger removal is confined to disposable test databases; this
/// never changes the production schema or application write path.
#[test]
#[ignore = "explicit bounded performance run"]
fn global_index_writer_trigger_contribution_25k() {
    use std::time::Instant;

    const N: usize = 25_000;
    let f = Fixture::new();
    let entries = (0..N)
        .map(|index| f.file(&format!("trigger-sample/entry{index}.txt")))
        .collect::<Vec<_>>();
    let mut records = Vec::new();
    for repetition in 0..3 {
        let mut modes = ["full_triggers", "without_fts", "without_source_stats"];
        modes.rotate_left(repetition);
        for mode in modes {
            let db_path = f.root.join(format!("trigger-{repetition}-{mode}.sqlite"));
            let db = Database::open(&db_path).unwrap();
            db.upsert_global_volume(&f.source).unwrap();
            if mode == "without_fts" {
                db.conn()
                    .unwrap()
                    .execute_batch(
                        "DROP TRIGGER global_entries_ai; DROP TRIGGER global_entries_ad; DROP TRIGGER global_entries_au;",
                    )
                    .unwrap();
            } else if mode == "without_source_stats" {
                db.conn()
                    .unwrap()
                    .execute_batch(
                        "DROP TRIGGER global_entry_source_facts_ai; DROP TRIGGER global_entry_source_facts_ad; DROP TRIGGER global_entry_source_facts_au; DROP TRIGGER global_entries_count_ai; DROP TRIGGER global_entries_count_ad; DROP TRIGGER global_entries_count_au;",
                    )
                    .unwrap();
            }

            let begin = Instant::now();
            let mut timing = crate::global_index::repository::GlobalIndexBatchTiming::default();
            for batch in entries.chunks(512) {
                let (_, batch_timing) = db.upsert_global_entries_batch_timed(batch).unwrap();
                timing.add(batch_timing);
            }
            let seconds = begin.elapsed().as_secs_f64();
            let conn = db.conn().unwrap();
            let fts_bytes: i64 = conn
                .query_row(
                    "SELECT COALESCE(SUM(pgsize),0) FROM dbstat WHERE name GLOB 'global_entries_fts*'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            let source_fact_rows: i64 = conn
                .query_row(
                    "SELECT active_rows FROM global_entry_source_facts WHERE volume_id=?1",
                    [&f.source.id],
                    |row| row.get(0),
                )
                .unwrap();
            let volume_entry_count: i64 = conn
                .query_row(
                    "SELECT entry_count FROM global_volumes WHERE id=?1",
                    [&f.source.id],
                    |row| row.get(0),
                )
                .unwrap();
            let database_bytes = conn
                .query_row("PRAGMA page_count", [], |row| row.get::<_, i64>(0))
                .unwrap()
                * conn
                    .query_row("PRAGMA page_size", [], |row| row.get::<_, i64>(0))
                    .unwrap();
            let database_file_bytes = fs::metadata(&db_path).map_or(0, |metadata| metadata.len());
            let wal_path = std::path::PathBuf::from(format!("{}-wal", db_path.display()));
            let wal_bytes = fs::metadata(&wal_path).map_or(0, |metadata| metadata.len());
            if mode == "full_triggers" {
                assert_eq!(source_fact_rows, N as i64);
                assert_eq!(volume_entry_count, N as i64);
                conn.execute(
                    "INSERT INTO global_entries_fts(global_entries_fts,rank) VALUES('integrity-check',1)",
                    [],
                )
                .unwrap();
            } else if mode == "without_source_stats" {
                assert_eq!(source_fact_rows, 0);
                assert_eq!(volume_entry_count, 0);
            }
            let record = serde_json::json!({
                "platform": std::env::consts::OS,
                "arch": std::env::consts::ARCH,
                "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
                "group": mode,
                "repetition": repetition,
                "candidates": N,
                "seconds": seconds,
                "candidatesPerSecond": N as f64 / seconds,
                "sqliteTransactionSeconds": timing.sqlite_transaction.as_secs_f64(),
                "coveragePolicyLoadSeconds": timing.coverage_policy_load.as_secs_f64(),
                "entryUpsertSeconds": timing.entry_upsert.as_secs_f64(),
                "commitSeconds": timing.commit.as_secs_f64(),
                "ftsBytes": fts_bytes,
                "databasePageBytes": database_bytes,
                "databaseFileBytes": database_file_bytes,
                "walBytes": wal_bytes,
                "databaseAndWalBytes": database_file_bytes + wal_bytes,
                "sourceFactsActiveRows": source_fact_rows,
                "volumeEntryCount": volume_entry_count
            });
            println!("GLOBAL_INDEX_TRIGGER_COST {record}");
            records.push(record);
            drop(conn);
            drop(db);
        }
    }
    if let Ok(path) = std::env::var("ZEN_GLOBAL_WRITER_TRIGGER_REPORT") {
        fs::write(path, serde_json::to_vec_pretty(&records).unwrap()).unwrap();
    }
}

/// Production repository A/B on a task-owned filesystem corpus; no MFT/Spotlight
/// enumeration and no user disks. Reuses ISG-00E's paired writer/FTS measurements,
/// but exercises the current native context observer and owned admission path.
#[test]
#[ignore = "explicit bounded performance run"]
fn smart_lean_ingestion_cost() {
    use std::time::Instant;
    let n = std::env::var("ZEN_SMART_LEAN_ENTRIES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(1000);
    assert!((1000..=200_000).contains(&n));
    let f = Fixture::new();
    f.file("project/package.json");
    f.file("project/node_modules/.package-lock.json");
    f.file("rust/Cargo.toml");
    f.file("rust/target/.rustc_info.json");
    let entries = (0..n)
        .map(|i| {
            f.file(&match i % 10 {
                0..=2 => format!("project/node_modules/lib/dependency{i}.js"),
                3 => format!("rust/target/debug/generated{i}.o"),
                4 => format!("project/src/source{i}.rs"),
                5 => format!("project/docs/README{i}.md"),
                6 => format!("documents/report{i}.pdf"),
                7 => format!("Projects/Windows/README{i}.md"),
                8 => format!("Design/build/logo{i}.ai"),
                _ => format!("资料/作品{i}.png"),
            })
        })
        .collect::<Vec<_>>();
    let expected_lean = (0..n).filter(|i| i % 10 >= 4).count() as i64;
    let mut rows = Vec::new();
    for repetition in 0..3 {
        // Three paired groups preserve ISG-00E's ungated writer control as
        // well as the selectable Broad/Smart modes. Rotate ordering each round.
        let raw = (CoverageMode::BroadCoverage, true);
        let broad = (CoverageMode::BroadCoverage, false);
        let lean = (CoverageMode::SmartLean, false);
        let modes = match repetition % 3 {
            0 => [raw, broad, lean],
            1 => [lean, raw, broad],
            _ => [broad, lean, raw],
        };
        for (mode, raw_baseline) in modes {
            let group = if raw_baseline {
                "ungated_repository"
            } else if mode == CoverageMode::BroadCoverage {
                "broad_coverage"
            } else {
                "smart_lean"
            };
            let db =
                Database::open(f.root.join(format!("bench-{repetition}-{group}.sqlite"))).unwrap();
            db.upsert_global_volume(&f.source).unwrap();
            db.save_global_index_coverage(1, mode, vec![]).unwrap();
            let owner = db
                .claim_global_index_operation(
                    &f.source,
                    &uuid::Uuid::new_v4().to_string(),
                    SourceOperationKind::Rebuild,
                    SourceOperationRoute::Coordinator,
                )
                .unwrap()
                .unwrap();
            let mut system = sysinfo::System::new();
            let pid = sysinfo::get_current_pid().unwrap();
            system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
            let rss_before = system.process(pid).map_or(0, sysinfo::Process::memory);
            let mut rss_peak = rss_before;
            let _ = super::native::take_metadata_attempts();
            let begin = Instant::now();
            let mut sampling = std::time::Duration::ZERO;
            let mut sampled_batches = 0;
            let mut writer_timing =
                crate::global_index::repository::GlobalIndexBatchTiming::default();
            let mut timed_writer_batches = 0u64;
            for (batch_index, batch) in entries.chunks(512).enumerate() {
                if raw_baseline {
                    let (_, timing) = db.upsert_global_entries_batch_timed(batch).unwrap();
                    writer_timing.add(timing);
                    timed_writer_batches += 1;
                } else {
                    let (_, timing) = db
                        .upsert_global_entries_batch_owned_timed(batch, &owner)
                        .unwrap();
                    writer_timing.add(timing);
                    timed_writer_batches += 1;
                }
                // Process inspection on hosted Windows is expensive. Keep its
                // cost explicit and outside admission timing, with bounded RSS
                // samples instead of inspecting the process after every batch.
                if batch_index % 32 == 0 || batch_index + 1 == n.div_ceil(512) {
                    let sample_begin = Instant::now();
                    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
                    rss_peak =
                        rss_peak.max(system.process(pid).map_or(0, sysinfo::Process::memory));
                    sampling += sample_begin.elapsed();
                    sampled_batches += 1;
                }
            }
            db.finish_global_index_operation(&owner, "ready", None, Some(1), Some(1), true)
                .unwrap();
            let instrumented = begin.elapsed();
            let elapsed = instrumented.saturating_sub(sampling).as_secs_f64();
            let metadata_attempts = super::native::take_metadata_attempts();
            let conn = db.conn().unwrap();
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM global_entries WHERE is_stale=0",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            let expected = if mode == CoverageMode::SmartLean {
                expected_lean
            } else {
                n as i64
            };
            assert_eq!(
                count, expected,
                "no false exclusions in the literal corpus ledger"
            );
            let mismatches:i64=conn.query_row("SELECT COUNT(*) FROM global_entries WHERE is_stale=0 AND (path LIKE '%node_modules%' OR path LIKE '%/target/%' OR path LIKE '%\\target\\%')",[],|r|r.get(0)).unwrap();
            if mode == CoverageMode::SmartLean {
                assert_eq!(mismatches, 0);
            }
            assert_eq!(
                conn.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| r
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
                0
            );
            conn.execute("INSERT INTO global_entries_fts(global_entries_fts,rank) VALUES('integrity-check',1)",[]).unwrap();
            let fts:i64=conn.query_row("SELECT COALESCE(SUM(pgsize),0) FROM dbstat WHERE name GLOB 'global_entries_fts*'",[],|r|r.get(0)).unwrap();
            let pages: i64 = conn
                .query_row("PRAGMA page_count", [], |r| r.get(0))
                .unwrap();
            let page_size: i64 = conn
                .query_row("PRAGMA page_size", [], |r| r.get(0))
                .unwrap();
            let observation = db.global_index_coverage().unwrap().sources.remove(0);
            drop(conn);
            let reindex = if raw_baseline {
                serde_json::Value::Null
            } else {
                let reindex_mode = if mode == CoverageMode::SmartLean {
                    CoverageMode::BroadCoverage
                } else {
                    CoverageMode::SmartLean
                };
                let reindex_begin = Instant::now();
                db.save_global_index_coverage(
                    db.global_index_coverage().unwrap().policy.revision,
                    reindex_mode,
                    vec![],
                )
                .unwrap();
                let owner = db
                    .claim_global_index_operation(
                        &f.source,
                        &uuid::Uuid::new_v4().to_string(),
                        SourceOperationKind::Rebuild,
                        SourceOperationRoute::Coordinator,
                    )
                    .unwrap()
                    .unwrap();
                let retired = db
                    .mark_global_entries_stale_for_volume_owned(&owner)
                    .unwrap();
                for batch in entries.chunks(512) {
                    db.upsert_global_entries_batch_owned(batch, &owner).unwrap();
                }
                db.finish_global_index_operation(&owner, "ready", None, Some(2), Some(2), true)
                    .unwrap();
                let reindex_seconds = reindex_begin.elapsed().as_secs_f64();
                let reindex_metadata_attempts = super::native::take_metadata_attempts();
                let conn = db.conn().unwrap();
                let reindex_active: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM global_entries WHERE is_stale=0",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert_eq!(
                    reindex_active,
                    if reindex_mode == CoverageMode::SmartLean {
                        expected_lean
                    } else {
                        n as i64
                    }
                );
                let reindex_retained: i64 = conn
                    .query_row("SELECT COUNT(*) FROM global_entries", [], |r| r.get(0))
                    .unwrap();
                conn.execute("INSERT INTO global_entries_fts(global_entries_fts,rank) VALUES('integrity-check',1)",[]).unwrap();
                assert_eq!(
                    conn.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| r
                        .get::<_, i64>(
                        0
                    ))
                    .unwrap(),
                    0
                );
                serde_json::json!({"mode":reindex_mode,"seconds":reindex_seconds,"metadataAttempts":reindex_metadata_attempts,"retiredRows":retired,"activeRows":reindex_active,"retainedRows":reindex_retained})
            };
            let candidates_per_second = n as f64 / elapsed;
            let record = serde_json::json!({"platform":std::env::consts::OS,"arch":std::env::consts::ARCH,"profile":if cfg!(debug_assertions) {"debug"} else {"release"},"group":group,"mode":mode,"repetition":repetition,"candidates":n,"candidatesPerSecond":candidates_per_second,"rows":count,"ftsBytes":fts,"dbPageBytes":pages*page_size,"seconds":elapsed,"instrumentedSeconds":instrumented.as_secs_f64(),"rssSamplingSeconds":sampling.as_secs_f64(),"rssSampledBatches":sampled_batches,"coveragePolicyLoadSeconds":writer_timing.coverage_policy_load.as_secs_f64(),"coverageEvaluationSeconds":writer_timing.coverage_evaluation.as_secs_f64(),"sqliteTransactionSeconds":writer_timing.sqlite_transaction.as_secs_f64(),"connectionAcquisitionSeconds":writer_timing.connection_acquisition.as_secs_f64(),"transactionAcquisitionSeconds":writer_timing.transaction_acquisition.as_secs_f64(),"transactionSetupSeconds":writer_timing.transaction_setup.as_secs_f64(),"entryUpsertSeconds":writer_timing.entry_upsert.as_secs_f64(),"aiTaskQualificationSeconds":writer_timing.ai_task_qualification.as_secs_f64(),"recordDecisionsSeconds":writer_timing.record_decisions.as_secs_f64(),"commitSeconds":writer_timing.commit.as_secs_f64(),"timedWriterBatches":timed_writer_batches,"excludedObservations":observation.excluded,"undetermined":observation.undetermined,"falseExclusions":0,"rawEnumeration":"not measured","nativeObserverMetadataAttempts":metadata_attempts,"rssBeforeBytes":rss_before,"rssPeakSampledBytes":rss_peak,"writerBatches":n.div_ceil(512),"wakeCycles":"not measured","reindex":reindex});
            println!("SMART_LEAN_AB {record}");
            rows.push(record);
        }
    }
    if let Ok(path) = std::env::var("ZEN_SMART_LEAN_REPORT") {
        fs::write(path, serde_json::to_vec_pretty(&rows).unwrap()).unwrap();
    }
}

#[test]
fn source_revoke_and_failed_reconcile_cannot_publish_policy_completion() {
    let f = Fixture::new();
    let e = f.file("docs/important.md");
    f.db.save_global_index_coverage(1, CoverageMode::SmartLean, vec![])
        .unwrap();
    let owner = f.owner();
    f.db.upsert_global_entries_batch_owned(std::slice::from_ref(&e), &owner)
        .unwrap();
    f.db.finish_global_index_operation(
        &owner,
        "permission_required",
        Some("controlled_fixture_permission"),
        None,
        None,
        true,
    )
    .unwrap();
    assert!(f.db.global_index_coverage().unwrap().sources[0].pending);
    assert!(f
        .db
        .global_index_operation_state(&f.source.id)
        .unwrap()
        .unwrap()
        .rebuild_pending());
    let owner = f.owner();
    f.db.set_global_volume_enabled(&f.source.id, false).unwrap();
    assert!(f
        .db
        .upsert_global_entries_batch_owned(std::slice::from_ref(&e), &owner)
        .is_err());
    assert!(f
        .db
        .search_global_entries("important", 10, 0)
        .unwrap()
        .is_empty());
    f.db.set_global_volume_enabled(&f.source.id, true).unwrap();
    assert!(f
        .db
        .upsert_global_entries_batch_owned(&[e], &owner)
        .is_err());
}
#[test]
fn rule_specificity_and_identity_replacement_are_conservative() {
    let f = Fixture::new();
    let dependency = f.dependencies();
    let outer = bind_rule(&f.source, &f.root.join("project").to_string_lossy(), false).unwrap();
    let inner = bind_rule(
        &f.source,
        &f.root.join("project/node_modules/lib").to_string_lossy(),
        true,
    )
    .unwrap();
    let p = CoveragePolicy {
        mode: CoverageMode::Custom,
        rules: vec![outer, inner],
        ..CoveragePolicy::default()
    };
    assert_eq!(
        observe_batch(&p, &f.source, std::slice::from_ref(&dependency)),
        vec![Decision::Include]
    );
    // Keep the original directory alive, so its inode/file ID cannot be reused.
    fs::rename(
        f.root.join("project/node_modules/lib"),
        f.root.join("original-lib"),
    )
    .unwrap();
    let replacement = f.file("project/node_modules/lib/noise.js");
    assert_eq!(
        observe_batch(&p, &f.source, &[replacement]),
        vec![Decision::Undetermined("rule_directory_identity_changed")]
    );
}
#[test]
fn hardlink_search_admission_uses_native_index_policy() {
    let f = Fixture::new();
    let e = f.dependencies();
    let alias_path = f.root.join("user-copy.js");
    fs::hard_link(&e.path, &alias_path).unwrap();
    let original_decision = observe_batch(
        &CoveragePolicy::default(),
        &f.source,
        std::slice::from_ref(&e),
    )[0]
    .clone();
    let mut alias = e;
    alias.path = alias_path.to_string_lossy().into_owned();
    alias.name = "user-copy.js".into();
    alias.extension = "js".into();
    super::native::enrich_fixture_identity(&mut alias);
    let alias_decision = observe_batch(&CoveragePolicy::default(), &f.source, &[alias])[0].clone();
    // Read-only filename indexing need not open a handle just to reject
    // hardlinks. Each visible name still follows its own Index Scope path.
    #[cfg(windows)]
    {
        assert_eq!(original_decision, Decision::Exclude("npm_dependencies"));
        assert_eq!(alias_decision, Decision::Include);
    }
    #[cfg(not(windows))]
    {
        assert!(matches!(original_decision, Decision::Undetermined(_)));
        assert!(matches!(alias_decision, Decision::Undetermined(_)));
    }
}

#[cfg(windows)]
#[test]
fn mft_fast_path_uses_native_attributes_without_extra_handle_checks() {
    let f = Fixture::new();
    let mut file = f.file("docs/ordinary.txt");
    let policy = CoveragePolicy::default();
    super::native::take_metadata_attempts();
    assert_eq!(
        observe_batch(&policy, &f.source, std::slice::from_ref(&file)),
        vec![Decision::Include]
    );
    let fast_metadata = super::native::take_metadata_attempts();
    file.source_provider = crate::global_index::PROVIDER_WINDOWS_RECURSIVE_FALLBACK.to_string();
    let _ = observe_batch(&policy, &f.source, std::slice::from_ref(&file));
    let fallback_metadata = super::native::take_metadata_attempts();
    assert!(
        fast_metadata < fallback_metadata,
        "MFT filename admission should avoid the extra per-file handle check"
    );

    file.source_provider = crate::global_index::PROVIDER_WINDOWS_MFT_USN.to_string();
    for flag in [0x400, 0x1000, 0x40000, 0x400000] {
        file.file_attributes = flag;
        assert_eq!(
            observe_batch(&policy, &f.source, std::slice::from_ref(&file)),
            vec![Decision::Undetermined(
                "identity_permission_or_alias_unverified"
            )]
        );
    }
}

#[test]
fn resolved_uncertainty_does_not_make_cumulative_observations_a_permanent_blocker() {
    let f = Fixture::new();
    let entry = f.file("resolved.txt");
    fs::remove_file(&entry.path).unwrap();
    let owner = f.owner();
    f.db.upsert_global_entries_batch_owned(std::slice::from_ref(&entry), &owner)
        .unwrap();
    f.db.finish_global_index_operation(&owner, "ready", None, Some(1), Some(1), false)
        .unwrap();
    assert!(!f.db.global_index_status().unwrap().collection_complete);
    fs::write(&entry.path, b"fixture").unwrap();
    let owner =
        f.db.claim_global_index_operation(
            &f.source,
            &uuid::Uuid::new_v4().to_string(),
            SourceOperationKind::Incremental,
            SourceOperationRoute::Coordinator,
        )
        .unwrap()
        .unwrap();
    f.db.upsert_global_entries_batch_owned(std::slice::from_ref(&entry), &owner)
        .unwrap();
    f.db.finish_global_index_operation(&owner, "ready", None, None, Some(2), false)
        .unwrap();
    let snapshot = f.db.global_index_coverage().unwrap();
    assert_eq!(
        snapshot.sources[0].undetermined, 1,
        "observation counter remains cumulative"
    );
    assert!(
        !snapshot.sources[0].has_undetermined,
        "current repository truth has resolved the uncertainty"
    );
    assert!(f.db.global_index_status().unwrap().collection_complete);
}
#[test]
fn policy_retirement_preserves_managed_ai_corrections_and_references() {
    let f = Fixture::new();
    let entry = f.dependencies();
    f.db.upsert_global_entries_batch(std::slice::from_ref(&entry))
        .unwrap();
    {
        let conn = f.db.conn().unwrap();
        conn.execute("INSERT INTO managed_scopes(id,path,enabled,created_at,updated_at) VALUES('scope',?1,1,1,1)",[f.root.to_string_lossy().as_ref()]).unwrap();
        conn.execute("INSERT INTO managed_entries(id,global_entry_id,managed_scope_id,enabled,created_at,updated_at) VALUES('managed',?1,'scope',1,1,1)",[entry.entry_id()]).unwrap();
        conn.execute("INSERT INTO ai_analysis_state(global_entry_id,status,classification_json,user_corrected,updated_at) VALUES(?1,'completed','user-owned-correction',1,1)",[entry.entry_id()]).unwrap();
    }
    f.db.save_global_index_coverage(1, CoverageMode::SmartLean, vec![])
        .unwrap();
    let owner = f.owner();
    f.db.mark_global_entries_stale_for_volume_owned(&owner)
        .unwrap();
    f.db.upsert_global_entries_batch_owned(std::slice::from_ref(&entry), &owner)
        .unwrap();
    f.db.finish_global_index_operation(&owner, "ready", None, Some(1), Some(1), true)
        .unwrap();
    let conn = f.db.conn().unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT enabled FROM managed_entries WHERE id='managed'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(conn.query_row("SELECT classification_json,user_corrected FROM ai_analysis_state WHERE global_entry_id=?1",[entry.entry_id()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?))).unwrap(),("user-owned-correction".into(),1));
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
}

#[test]
fn in_source_cmake_build_never_excludes_the_project_sources() {
    let f = Fixture::new();
    f.file("project/CMakeLists.txt");
    f.file("project/CMakeCache.txt");
    let generated = f.file("project/CMakeFiles/generated.o");
    let source = f.file("project/src/main.cpp");
    let readme = f.file("project/README.md");
    assert_eq!(
        observe_batch(
            &CoveragePolicy::default(),
            &f.source,
            &[generated, source, readme]
        ),
        vec![
            Decision::Exclude("cmake_build_output"),
            Decision::Include,
            Decision::Include
        ]
    );
}
