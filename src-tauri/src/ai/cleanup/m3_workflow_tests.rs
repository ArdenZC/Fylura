use super::*;

#[test]
fn m3_complete_group_summary_is_independent_of_finding_page() {
    let fixture = new_cleanup_ai_fixture();
    for index in 0..210 {
        fs::write(
            fixture.source_root.join(format!("installer-{index}.msi")),
            b"installer fixture",
        )
        .unwrap();
    }
    let run = complete_cleanup_analysis_run(fixture.db(), &fixture.source_root);
    let page = fixture
        .db()
        .list_analysis_findings(
            &crate::db::AnalysisFindingFilter {
                run_id: Some(run.id.clone()),
                status: Some("active".into()),
                ..Default::default()
            },
            None,
            1,
        )
        .unwrap();
    assert_eq!(page.findings.len(), 1);
    assert!(page.next_cursor.is_some());
    assert_eq!(page.run_revision, Some(run.revision));
    let groups = page.groups.unwrap();
    assert_eq!(groups.len(), 3);
    assert_eq!(
        groups.iter().map(|g| g.count).sum::<i64>(),
        run.findings_published
    );
    assert!(run.findings_published > 200);
    assert_eq!(
        groups
            .iter()
            .map(|g| g.categories.iter().map(|c| c.count).sum::<i64>())
            .sum::<i64>(),
        run.findings_published
    );
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn m3_independent_ai_batches_keep_both_unchanged_findings_current() {
    let fixture = new_cleanup_ai_fixture();
    let run = complete_cleanup_analysis_run(fixture.db(), &fixture.source_root);
    let candidates = active_cleanup_candidates(fixture.db(), &run.id);
    assert!(candidates.len() >= 2);
    for candidate in &candidates[..2] {
        let current = fixture.db().get_analysis_run(&run.id).unwrap();
        let provider =
            StaticCleanupProvider::new(vec![response_for_ids(std::slice::from_ref(&candidate.id))]);
        analyze_fixture_candidates(
            fixture.db(),
            &current,
            std::slice::from_ref(candidate),
            &provider,
        )
        .unwrap();
    }
    for candidate in &candidates[..2] {
        assert!(has_current_ai_assessment(fixture.db(), &candidate.id));
    }
    fs::write(&candidates[0].path, b"changed source identity and length").unwrap();
    assert!(!has_current_ai_assessment(fixture.db(), &candidates[0].id));
    assert!(has_current_ai_assessment(fixture.db(), &candidates[1].id));
}

// The older AI fixture helper deliberately returns only installer files.
// Product workflow fixtures need directory and Review Findings as well.
fn m3_all_cleanup_candidates(db: &Database, run_id: &str) -> Vec<StorageCandidate> {
    let mut candidates = Vec::new();
    let mut cursor = None;
    loop {
        let page = db
            .list_analysis_findings(
                &crate::db::AnalysisFindingFilter {
                    run_id: Some(run_id.to_string()),
                    detector_id: Some(crate::analysis::CLEANUP_HEURISTICS_DETECTOR.to_string()),
                    status: Some("active".to_string()),
                    ..Default::default()
                },
                cursor.as_deref(),
                200,
            )
            .unwrap();
        candidates.extend(
            page.findings
                .iter()
                .map(|finding| storage_candidate_from_analysis_finding(finding).unwrap()),
        );
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    candidates
}

#[test]
fn m3_workflow_fixture_includes_real_dependency_directory_findings() {
    let fixture = new_cleanup_ai_fixture();
    let cache = m3_project_cache(&fixture);
    let run = complete_cleanup_analysis_run(fixture.db(), &fixture.source_root);
    let candidates = m3_all_cleanup_candidates(fixture.db(), &run.id);
    assert!(
        candidates.iter().any(
            |c| crate::path_identity::normalize_for_compare(Path::new(&c.path))
                == crate::path_identity::normalize_for_compare(&cache)
        ),
        "directory Finding must not be lost by installer-only fixture filtering"
    );
}

fn m3_project_cache(fixture: &CleanupAiFixture) -> PathBuf {
    let project = fixture.source_root.join("project");
    fs::create_dir_all(project.join("node_modules/package")).unwrap();
    fs::write(project.join("package.json"), br#"{"name":"fixture"}"#).unwrap();
    fs::write(
        project.join("node_modules/package/index.js"),
        b"restorable dependency bytes",
    )
    .unwrap();
    project.join("node_modules")
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn m3_real_folder_scan_ai_group_review_safe_trash_history_and_restore() {
    use crate::storage_analyzer::{
        execute_analysis_cleanup_selections, preview_analysis_cleanup_selections,
        restore_cleanup_trash_items_for_db, CleanupFindingSelection,
    };
    let fixture = new_cleanup_ai_fixture();
    let cache = m3_project_cache(&fixture);
    fs::create_dir_all(fixture.source_root.join("personal/build")).unwrap();
    fs::write(
        fixture.source_root.join("personal/build/manual.txt"),
        b"keep personal work",
    )
    .unwrap();
    let run = complete_cleanup_analysis_run(fixture.db(), &fixture.source_root);
    let connection = fixture.db().conn().unwrap();
    for table in ["files", "scan_roots", "global_volumes", "global_entries"] {
        assert_eq!(
            connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0,
            "{table}"
        );
    }
    drop(connection);
    let candidates = m3_all_cleanup_candidates(fixture.db(), &run.id);
    let safe = candidates
        .iter()
        .find(|c| {
            crate::path_identity::normalize_for_compare(Path::new(&c.path))
                == crate::path_identity::normalize_for_compare(&cache)
        })
        .unwrap();
    let review = candidates
        .iter()
        .find(|c| c.tier == CleanupTier::Review)
        .unwrap();
    let provider = StaticCleanupProvider::new(vec![response_for_ids(&[
        safe.id.clone(),
        review.id.clone(),
    ])]);
    analyze_fixture_candidates(
        fixture.db(),
        &run,
        &[safe.clone(), review.clone()],
        &provider,
    )
    .unwrap();
    let safe_finding = fixture
        .db()
        .get_analysis_finding(&safe.id)
        .unwrap()
        .unwrap();
    let review_finding = fixture
        .db()
        .get_analysis_finding(&review.id)
        .unwrap()
        .unwrap();
    assert!(
        safe_finding.evidence_summary["aiAssessment"]["assessment"]["reason"]
            .as_str()
            .is_some_and(|reason| !reason.is_empty())
    );
    assert!(has_current_ai_assessment(fixture.db(), &safe.id));
    assert!(has_current_ai_assessment(fixture.db(), &review.id));
    fixture
        .db()
        .set_analysis_finding_decision(&review_finding.finding_key, "dismissed", None, None, 0)
        .unwrap();
    let safe_selection = CleanupFindingSelection {
        finding_id: safe.id.clone(),
        expected_revision: safe_finding.revision,
        review_confirmation: None,
    };
    let kept = CleanupFindingSelection {
        finding_id: review.id.clone(),
        expected_revision: review_finding.revision,
        review_confirmation: None,
    };
    let preview = preview_analysis_cleanup_selections(
        fixture.db(),
        &run.id,
        &[safe_selection.clone(), kept],
        None,
    )
    .unwrap();
    assert_eq!(preview.previews.len(), 2);
    assert_eq!(
        preview
            .previews
            .iter()
            .filter(|p| p.is_executable == Some(true))
            .count(),
        1
    );
    let result =
        execute_analysis_cleanup_selections(fixture.db(), &run.id, &[safe_selection], None)
            .unwrap();
    assert_eq!((result.moved, result.skipped, result.failed), (1, 0, 0));
    let batch = fixture
        .db()
        .cleanup_trash_items_for_batch(result.batch_id.as_deref().unwrap())
        .unwrap();
    assert_eq!(batch.len(), 1);
    assert_eq!(
        result.logs[0].candidate_id.as_deref(),
        Some(safe.id.as_str())
    );
    assert!(!cache.exists());
    assert!(Path::new(&review.path).exists());
    let restored =
        restore_cleanup_trash_items_for_db(vec![batch[0].id.clone()], fixture.db()).unwrap();
    assert_eq!(restored.restored, 1);
    assert_eq!(
        fs::read(cache.join("package/index.js")).unwrap(),
        b"restorable dependency bytes"
    );
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn m3_safe_candidates_need_no_ai_key_and_stale_sources_are_refused() {
    use crate::storage_analyzer::{
        execute_analysis_cleanup_selections, preview_analysis_cleanup_selections,
        restore_cleanup_trash_items_for_db, CleanupFindingSelection,
    };
    let fixture = new_cleanup_ai_fixture();
    let cache = m3_project_cache(&fixture);
    let run = complete_cleanup_analysis_run(fixture.db(), &fixture.source_root);
    let candidate = m3_all_cleanup_candidates(fixture.db(), &run.id)
        .into_iter()
        .find(|c| {
            crate::path_identity::normalize_for_compare(Path::new(&c.path))
                == crate::path_identity::normalize_for_compare(&cache)
        })
        .unwrap();
    assert!(!has_current_ai_assessment(fixture.db(), &candidate.id));
    let finding = fixture
        .db()
        .get_analysis_finding(&candidate.id)
        .unwrap()
        .unwrap();
    let selection = CleanupFindingSelection {
        finding_id: candidate.id.clone(),
        expected_revision: finding.revision,
        review_confirmation: None,
    };
    let preview = preview_analysis_cleanup_selections(
        fixture.db(),
        &run.id,
        std::slice::from_ref(&selection),
        None,
    )
    .unwrap();
    assert_eq!(preview.previews[0].is_executable, Some(true));
    let result = execute_analysis_cleanup_selections(
        fixture.db(),
        &run.id,
        std::slice::from_ref(&selection),
        None,
    )
    .unwrap();
    assert_eq!(result.moved, 1);
    restore_cleanup_trash_items_for_db(vec![result.logs[0].item_id.clone().unwrap()], fixture.db())
        .unwrap();
    // A new deterministic scan is the only new source authority after Restore.
    let run = complete_cleanup_analysis_run(fixture.db(), &fixture.source_root);
    let finding = fixture
        .db()
        .list_analysis_findings(
            &crate::db::AnalysisFindingFilter {
                run_id: Some(run.id.clone()),
                tier: Some("safe".into()),
                ..Default::default()
            },
            None,
            200,
        )
        .unwrap()
        .findings
        .into_iter()
        .find(|f| f.path_snapshot.as_deref() == Some(candidate.path.as_str()))
        .unwrap();
    fs::remove_dir_all(&cache).unwrap();
    let stale = CleanupFindingSelection {
        finding_id: finding.id,
        expected_revision: finding.revision,
        review_confirmation: None,
    };
    let preview = preview_analysis_cleanup_selections(
        fixture.db(),
        &run.id,
        std::slice::from_ref(&stale),
        None,
    )
    .unwrap();
    assert_eq!(preview.previews[0].is_executable, Some(false));
    assert!(execute_analysis_cleanup_selections(fixture.db(), &run.id, &[stale], None).is_err());
}

#[test]
fn m3_deterministic_safe_resolution_is_independent_of_ai_and_refuses_changed_files() {
    use crate::storage_analyzer::{
        resolve_analysis_candidates_for_cleanup, CleanupFindingSelection,
    };
    let fixture = new_cleanup_ai_fixture();
    let run = complete_cleanup_analysis_run(fixture.db(), &fixture.source_root);
    let candidate = active_cleanup_candidates(fixture.db(), &run.id)
        .into_iter()
        .find(|c| c.tier == CleanupTier::Safe)
        .unwrap();
    let finding = fixture
        .db()
        .get_analysis_finding(&candidate.id)
        .unwrap()
        .unwrap();
    let selection = CleanupFindingSelection {
        finding_id: finding.id,
        expected_revision: finding.revision,
        review_confirmation: None,
    };
    assert_eq!(
        resolve_analysis_candidates_for_cleanup(
            fixture.db(),
            &run.id,
            std::slice::from_ref(&selection),
            true
        )
        .unwrap()
        .len(),
        1
    );
    fs::write(&candidate.path, b"different file bytes and size after scan").unwrap();
    assert!(
        resolve_analysis_candidates_for_cleanup(fixture.db(), &run.id, &[selection], true).is_err()
    );
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn m3_partial_preparation_returns_results_bound_to_the_right_finding() {
    use crate::storage_analyzer::{
        move_cleanup_candidates_to_safe_trash_for_candidates,
        resolve_analysis_candidates_for_cleanup, restore_cleanup_trash_items_for_db,
        CleanupFindingSelection,
    };
    let fixture = new_cleanup_ai_fixture();
    let run = complete_cleanup_analysis_run(fixture.db(), &fixture.source_root);
    let candidates = active_cleanup_candidates(fixture.db(), &run.id)
        .into_iter()
        .filter(|c| c.tier == CleanupTier::Safe)
        .take(2)
        .collect::<Vec<_>>();
    assert_eq!(candidates.len(), 2);
    let selections = candidates
        .iter()
        .map(|candidate| {
            let finding = fixture
                .db()
                .get_analysis_finding(&candidate.id)
                .unwrap()
                .unwrap();
            CleanupFindingSelection {
                finding_id: finding.id,
                expected_revision: finding.revision,
                review_confirmation: None,
            }
        })
        .collect::<Vec<_>>();
    let resolved =
        resolve_analysis_candidates_for_cleanup(fixture.db(), &run.id, &selections, true).unwrap();
    // A real source disappears between resolution and the engine's preparation.
    fs::remove_file(&resolved[1].path).unwrap();
    let result = move_cleanup_candidates_to_safe_trash_for_candidates(
        resolved.iter().map(|c| c.id.clone()).collect(),
        &resolved,
        fixture.db(),
        None,
    )
    .unwrap();
    assert_eq!((result.moved, result.skipped, result.failed), (1, 1, 0));
    assert_eq!(
        result.logs[0].candidate_id.as_deref(),
        Some(resolved[1].id.as_str())
    );
    assert_eq!(result.logs[0].status, "skipped");
    assert_eq!(
        result.logs[1].candidate_id.as_deref(),
        Some(resolved[0].id.as_str())
    );
    assert_eq!(result.logs[1].status, "success");
    assert_eq!(
        restore_cleanup_trash_items_for_db(
            vec![result.logs[1].item_id.clone().unwrap()],
            fixture.db()
        )
        .unwrap()
        .restored,
        1
    );
    assert!(Path::new(&resolved[0].path).exists());
    assert!(!Path::new(&resolved[1].path).exists());
}
