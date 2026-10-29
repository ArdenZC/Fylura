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
        fs::create_dir_all(&root).unwrap();
        let db = Database::open(root.join("fixture.sqlite")).unwrap();
        let mut source = crate::global_index::tests::test_volume();
        source.mount_path = root.to_string_lossy().into_owned();
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
        GlobalEntryInput::from_path(&self.source.id, &full, &self.source.provider)
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
        "资料/设计稿.png",
        "Program Files2/notes.txt",
        "node_modules/notes.md",
        "build/report.pdf",
        "target/notes.md",
        ".venv/notes.txt",
    ];
    let entries = names.iter().map(|p| f.file(p)).collect::<Vec<_>>();
    assert_eq!(
        observe_batch(&CoveragePolicy::default(), &f.source, &entries),
        vec![Decision::Include; names.len()]
    );
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
                0..=3 => format!("project/node_modules/lib/dependency{i}.js"),
                4..=5 => format!("rust/target/debug/generated{i}.o"),
                6 => format!("documents/report{i}.pdf"),
                7 => format!("Projects/Windows/README{i}.md"),
                8 => format!("Design/build/logo{i}.ai"),
                _ => format!("资料/作品{i}.png"),
            })
        })
        .collect::<Vec<_>>();
    let expected_lean = (0..n).filter(|i| i % 10 >= 6).count() as i64;
    let mut rows = Vec::new();
    for repetition in 0..3 {
        // Alternate order to reduce warm-cache ordering bias.
        let modes = if repetition % 2 == 0 {
            [CoverageMode::BroadCoverage, CoverageMode::SmartLean]
        } else {
            [CoverageMode::SmartLean, CoverageMode::BroadCoverage]
        };
        for mode in modes {
            let db =
                Database::open(f.root.join(format!("bench-{repetition}-{mode:?}.sqlite"))).unwrap();
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
            for batch in entries.chunks(512) {
                db.upsert_global_entries_batch_owned(batch, &owner).unwrap();
                system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
                rss_peak = rss_peak.max(system.process(pid).map_or(0, sysinfo::Process::memory));
            }
            db.finish_global_index_operation(&owner, "ready", None, Some(1), Some(1), true)
                .unwrap();
            let elapsed = begin.elapsed().as_secs_f64();
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
            let record = serde_json::json!({"platform":std::env::consts::OS,"mode":mode,"repetition":repetition,"candidates":n,"rows":count,"ftsBytes":fts,"dbPageBytes":pages*page_size,"seconds":elapsed,"excludedObservations":observation.excluded,"undetermined":observation.undetermined,"falseExclusions":0,"rawEnumeration":"not measured","nativeObserverMetadataAttempts":metadata_attempts,"rssBeforeBytes":rss_before,"rssPeakSampledBytes":rss_peak,"writerBatches":n.div_ceil(512),"wakeCycles":"not measured"});
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
fn hardlinks_are_fenced_on_native_filesystems() {
    let f = Fixture::new();
    let e = f.dependencies();
    fs::hard_link(&e.path, f.root.join("user-copy.js")).unwrap();
    assert!(matches!(
        observe_batch(&CoveragePolicy::default(), &f.source, &[e])[0],
        Decision::Undetermined(_)
    ));
}
