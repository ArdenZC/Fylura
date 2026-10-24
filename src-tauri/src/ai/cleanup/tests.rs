use super::*;
use crate::ai::provider::AIProviderError;
use std::{
    collections::VecDeque,
    fs,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::Duration,
};

#[test]
fn output_names_persist_as_reveal_and_current_ai_cannot_enable_cleanup() {
    let fixture = new_cleanup_ai_fixture();
    for name in ["build", "dist", "target"] {
        let path = fixture.source_root.join(name);
        fs::create_dir_all(&path).expect("create ordinary user directory");
        fs::write(
            path.join("build-name-test.txt"),
            b"manually created content",
        )
        .expect("write ordinary user file");
    }
    let run = complete_cleanup_analysis_run(fixture.db(), &fixture.source_root);
    let page = fixture
        .db()
        .list_analysis_findings(
            &crate::db::AnalysisFindingFilter {
                run_id: Some(run.id.clone()),
                ..Default::default()
            },
            None,
            100,
        )
        .expect("load persisted findings");
    let outputs = page
        .findings
        .into_iter()
        .filter(|finding| matches!(finding.title.as_str(), "build" | "dist" | "target"))
        .collect::<Vec<_>>();
    assert_eq!(outputs.len(), 3, "small user directories remain scanned");
    for finding in outputs {
        assert_eq!(finding.tier, "review");
        assert_eq!(finding.action_kind, "reveal");
        assert_eq!(finding.confidence, "estimated");
        assert!(!finding.executable);
        assert_eq!(finding.exact_reclaimable_bytes, None);
        assert_eq!(finding.evidence_summary["trashAllowed"], false);
        assert_eq!(finding.evidence_summary["selectedByDefault"], false);
        let candidate = storage_candidate_from_analysis_finding(&finding).unwrap();
        let current_run = fixture.db().get_analysis_run(&run.id).unwrap();
        let provider =
            StaticCleanupProvider::new(vec![response_for_ids(std::slice::from_ref(&candidate.id))]);
        let published =
            analyze_fixture_candidates(fixture.db(), &current_run, &[candidate], &provider)
                .expect("AI assessment remains advisory");
        let assessed = &published[0];
        assert!(has_current_ai_assessment(fixture.db(), &assessed.id));
        assert_eq!(assessed.tier, "review");
        assert_eq!(assessed.action_kind, "reveal");
        assert!(!assessed.executable);
        let selection = crate::storage_analyzer::CleanupFindingSelection {
            finding_id: assessed.id.clone(),
            expected_revision: assessed.revision,
            review_confirmation: None,
        };
        // The same resolver gates both command preview and execution.
        let error =
            resolve_analysis_candidates_for_cleanup(fixture.db(), &run.id, &[selection], true)
                .expect_err("current AI and matching identity cannot prove basename safety");
        assert!(
            error.contains("no independent cleanup safety evidence"),
            "{error}"
        );
        let path = Path::new(assessed.path_snapshot.as_ref().unwrap());
        assert!(path.join("build-name-test.txt").exists());
    }
}

#[test]
fn ai_cannot_make_trash_disallowed_candidate_allowed() {
    let original = review_candidate("c1", "D:/Downloads/movie.mkv");
    let merged = merge_ai_cleanup_analysis(&original, &move_to_trash_output("c1"), None);
    assert!(!merged.trash_allowed);
    assert_ne!(merged.suggested_action, CleanupActionKind::MoveToTrash);
}

#[test]
fn semantic_delete_candidate_does_not_grant_cleanup_trash_authority() {
    let semantic = crate::ai::semantic::SemanticAssessmentV1::parse_provider_response(
            r#"{"version":1,"refId":"managed:entry-1","fileType":"Document","purpose":"Work","lifecycle":"Active","context":"","riskLevel":"Normal","suggestedAction":"DeleteCandidate","confidence":0.95,"reason":"possible cleanup candidate","keywords":[],"requiresConfirmation":false}"#,
            crate::ai::semantic::SemanticSourceBinding {
                global_entry_id: "entry-1".to_string(),
                managed_scope_id: "scope-1".to_string(),
                input_fingerprint: "fingerprint-1".to_string(),
                provider: "local".to_string(),
            },
            "archive.txt",
            "txt",
            false,
        )
        .expect("semantic suggestion remains advisory");
    assert_eq!(semantic.suggested_action.as_str(), "DeleteCandidate");
    assert!(semantic.requires_confirmation);

    // Analysis Finding is evaluated only by its own detector and existing
    // conservative merge contract; a semantic delete label cannot turn a
    // review-only finding into Safe Trash permission.
    let finding = review_candidate("c1", "D:/Downloads/archive.txt");
    let merged = merge_ai_cleanup_analysis(&finding, &move_to_trash_output("c1"), None);
    assert_eq!(merged.tier, CleanupTier::Review);
    assert!(!merged.trash_allowed);
    assert_ne!(merged.suggested_action, CleanupActionKind::MoveToTrash);
}

#[test]
fn ai_cannot_upgrade_caution_to_safe() {
    let original = caution_candidate("c1", "D:/VMs/demo.vhdx");
    let merged = merge_ai_cleanup_analysis(&original, &move_to_trash_output("c1"), None);
    assert_eq!(merged.tier, CleanupTier::Caution);
    assert!(!merged.trash_allowed);
}

#[test]
fn caution_cannot_be_selected_by_default() {
    let original = caution_candidate("c1", "D:/data/app.db");
    let mut output = move_to_trash_output("c1");
    output.selected_by_default = Some(true);
    let merged = merge_ai_cleanup_analysis(&original, &output, None);
    assert!(!merged.selected_by_default);
}

#[test]
fn program_files_cannot_move_to_trash() {
    let original = safe_candidate("c1", "C:/Program Files/App/cache");
    let merged = merge_ai_cleanup_analysis(&original, &move_to_trash_output("c1"), None);
    assert_eq!(merged.tier, CleanupTier::Caution);
    assert_ne!(merged.suggested_action, CleanupActionKind::MoveToTrash);
    assert!(!merged.trash_allowed);
}

#[test]
fn windows_path_cannot_move_to_trash() {
    let original = safe_candidate("c1", "C:/Windows/Temp/demo.tmp");
    let merged = merge_ai_cleanup_analysis(&original, &move_to_trash_output("c1"), None);
    assert_eq!(merged.tier, CleanupTier::Caution);
    assert_ne!(merged.suggested_action, CleanupActionKind::MoveToTrash);
}

#[test]
fn appdata_cannot_be_selected_by_default() {
    let original = safe_candidate("c1", "C:/Users/me/AppData/Local/App/cache");
    let merged = merge_ai_cleanup_analysis(&original, &move_to_trash_output("c1"), None);
    assert_eq!(merged.tier, CleanupTier::Review);
    assert!(!merged.selected_by_default);
}

#[test]
fn browser_profile_stays_caution() {
    let original = caution_candidate(
        "c1",
        "C:/Users/me/AppData/Local/Google/Chrome/User Data/Default",
    );
    let merged = merge_ai_cleanup_analysis(&original, &move_to_trash_output("c1"), None);
    assert_eq!(merged.tier, CleanupTier::Caution);
    assert!(!merged.trash_allowed);
}

#[test]
fn database_file_stays_caution() {
    let original = caution_candidate("c1", "D:/data/prod.sqlite3");
    let merged = merge_ai_cleanup_analysis(&original, &move_to_trash_output("c1"), None);
    assert_eq!(merged.tier, CleanupTier::Caution);
}

#[test]
fn virtual_machine_image_stays_caution() {
    let original = caution_candidate("c1", "D:/VMs/demo.qcow2");
    let merged = merge_ai_cleanup_analysis(&original, &move_to_trash_output("c1"), None);
    assert_eq!(merged.tier, CleanupTier::Caution);
}

#[test]
fn node_modules_can_remain_safe() {
    let original = safe_candidate("c1", "D:/Projects/demo/node_modules");
    let merged = merge_ai_cleanup_analysis(&original, &move_to_trash_output("c1"), None);
    assert_eq!(merged.tier, CleanupTier::Safe);
    assert_eq!(merged.suggested_action, CleanupActionKind::MoveToTrash);
    assert!(merged.trash_allowed);
}

#[test]
fn node_modules_can_receive_risk_note() {
    let original = safe_candidate("c1", "D:/Projects/demo/node_modules");
    let mut output = move_to_trash_output("c1");
    output.risk_note = Some("Check npm link and local patches first.".to_string());
    let merged = merge_ai_cleanup_analysis(&original, &output, None);
    assert_eq!(
        merged.risk_note.as_deref(),
        Some("Check npm link and local patches first.")
    );
}

#[test]
fn unknown_large_item_stays_review() {
    let original = review_candidate("c1", "D:/Downloads/archive.bin");
    let merged = merge_ai_cleanup_analysis(&original, &move_to_trash_output("c1"), None);
    assert_eq!(merged.tier, CleanupTier::Review);
    assert!(!merged.trash_allowed);
}

#[test]
fn illegal_candidate_id_is_ignored() {
    let coverage = cleanup_ai_coverage(&["c1".to_string()], vec![move_to_trash_output("other")])
        .expect("classify unknown provider identity");
    assert!(coverage.unique_outputs.is_empty());
    assert_eq!(coverage.counts.unknown, 1);
    assert_eq!(coverage.counts.omitted, 1);
}

#[test]
fn illegal_enum_falls_back_to_original() {
    let original = safe_candidate("c1", "D:/Projects/demo/node_modules");
    let mut output = move_to_trash_output("c1");
    output.tier = Some("Danger".to_string());
    output.suggested_action = Some("DeleteNow".to_string());
    let merged = merge_ai_cleanup_analysis(&original, &output, None);
    assert_eq!(merged.tier, CleanupTier::Safe);
    assert_eq!(merged.suggested_action, CleanupActionKind::MoveToTrash);
}

#[test]
fn api_key_is_redacted_from_errors() {
    let message = sanitize_ai_cleanup_error(
        "Provider rejected key sk-cleanup-secret".to_string(),
        "sk-cleanup-secret",
    );
    assert!(!message.contains("sk-cleanup-secret"));
    assert!(message.contains("[redacted]"));
}

#[test]
fn cleanup_markdown_wrapped_json_parses() {
    let content = format!("```json\n{}\n```", cleanup_response("c1"));
    let outputs = parse_ai_cleanup_analysis_response(&content).expect("parse markdown");
    assert_eq!(outputs[0].candidate_id, "c1");
}

#[test]
fn cleanup_thinking_wrapped_json_parses() {
    let content = format!(
        "<think>Risk analysis goes here.</think>\n{}",
        cleanup_response("c1")
    );
    let outputs = parse_ai_cleanup_analysis_response(&content).expect("strip thinking");
    assert_eq!(outputs[0].candidate_id, "c1");
}

#[test]
fn cleanup_direct_array_json_parses() {
    let content = r#"[{"candidateId":"c1","tier":"Safe","category":"AI category","suggestedAction":"MoveToTrash","confidence":0.95,"reason":"AI reason.","riskNote":"AI risk.","trashAllowed":true,"selectedByDefault":true}]"#;
    let outputs = parse_ai_cleanup_analysis_response(content).expect("parse direct array");
    assert_eq!(outputs[0].candidate_id, "c1");
}

#[test]
fn cleanup_nested_result_analyses_parses() {
    let content = r#"{"result":{"analyses":[{"candidateId":"c1","tier":"Safe","category":"AI category","suggestedAction":"MoveToTrash","confidence":0.95,"reason":"AI reason.","riskNote":"AI risk.","trashAllowed":true,"selectedByDefault":true}]}}"#;
    let outputs = parse_ai_cleanup_analysis_response(content).expect("parse result.analyses");
    assert_eq!(outputs[0].candidate_id, "c1");
}

#[test]
fn cleanup_provider_requests_use_cleanup_analysis_trace_context() {
    let fixture = new_cleanup_ai_fixture();
    let run = complete_cleanup_analysis_run(fixture.db(), &fixture.source_root);
    let candidates = active_cleanup_candidates(fixture.db(), &run.id);
    let ids = candidates
        .iter()
        .map(|candidate| candidate.id.clone())
        .collect::<Vec<_>>();
    let provider = StaticCleanupProvider::new(vec![response_for_ids(&ids)]);

    analyze_fixture_candidates(fixture.db(), &run, &candidates, &provider)
        .expect("publish cleanup assessment with trace context");

    let requests = provider.requests();
    assert_eq!(requests.len(), 1);
    let trace = requests[0]
        .provider_options
        .trace_context
        .as_ref()
        .expect("cleanup provider request has trace context");
    assert_eq!(trace.operation, AITraceOperation::CleanupAnalysis);
    assert_eq!(trace.job_id.as_deref(), Some(run.id.as_str()));
    assert_eq!(trace.target_count, Some(candidates.len()));
    assert_eq!(trace.batch_size, Some(candidates.len()));
    assert!(trace
        .batch_id
        .as_deref()
        .is_some_and(|value| value.contains("cleanup-ai-request")));
    assert!(!trace.include_sensitive_document_content);
    assert!(trace.redaction_secrets.is_empty());
}

#[test]
fn exact_coverage_publishes_only_unique_returned_candidates() {
    let fixture = new_cleanup_ai_fixture();
    let run = complete_cleanup_analysis_run(fixture.db(), &fixture.source_root);
    let candidates = active_cleanup_candidates(fixture.db(), &run.id);
    assert_eq!(candidates.len(), 3);
    let ids = candidates
        .iter()
        .map(|candidate| candidate.id.clone())
        .collect::<Vec<_>>();
    let mut assessed_output = cleanup_output(&ids[2], "exact candidate only");
    assessed_output["tier"] = serde_json::json!("Review");
    assessed_output["suggestedAction"] = serde_json::json!("Reveal");
    assessed_output["trashAllowed"] = serde_json::json!(false);
    assessed_output["selectedByDefault"] = serde_json::json!(false);
    let response = serde_json::json!({
        "analyses": [
            cleanup_output(&ids[0], "duplicate must not win"),
            cleanup_output(&ids[0], "duplicate must not overwrite"),
            assessed_output,
            cleanup_output("fabricated-candidate", "must be rejected")
        ]
    })
    .to_string();
    let provider = StaticCleanupProvider::new(vec![response]);
    let published = analyze_fixture_candidates(fixture.db(), &run, &candidates, &provider)
        .expect("publish exact coverage");

    assert_eq!(published.len(), 1);
    assert_eq!(published[0].id, ids[2]);
    let duplicate = fixture
        .db()
        .get_analysis_finding(&ids[0])
        .expect("read duplicate candidate")
        .expect("duplicate candidate exists");
    let omitted = fixture
        .db()
        .get_analysis_finding(&ids[1])
        .expect("read omitted candidate")
        .expect("omitted candidate exists");
    let assessed = fixture
        .db()
        .get_analysis_finding(&ids[2])
        .expect("read assessed candidate")
        .expect("assessed candidate exists");
    assert!(!has_current_ai_assessment(fixture.db(), &duplicate.id));
    assert!(!has_current_ai_assessment(fixture.db(), &omitted.id));
    assert!(!has_current_ai_assessment(
        fixture.db(),
        "fabricated-candidate"
    ));
    assert!(has_current_ai_assessment(fixture.db(), &assessed.id));
    assert_eq!(
        assessed.evidence_summary["aiAssessment"]["publication"]["compareAndSwap"],
        "succeeded"
    );
    for id in [&ids[0], &ids[1]] {
        assert_eq!(
            fixture
                .db()
                .list_analysis_finding_evidence(id)
                .expect("read candidate evidence")
                .iter()
                .filter(|item| item.evidence_kind == "ai_assessment")
                .count(),
            0
        );
    }
    assert_eq!(assessed.tier, "review");
    assert!(!assessed.executable);
    assert_eq!(assessed.action_kind, "reveal");
}

#[test]
fn reordered_provider_response_is_bound_by_exact_candidate_id() {
    let fixture = new_cleanup_ai_fixture();
    let run = complete_cleanup_analysis_run(fixture.db(), &fixture.source_root);
    let candidates = active_cleanup_candidates(fixture.db(), &run.id);
    let mut reversed = candidates
        .iter()
        .map(|candidate| candidate.id.clone())
        .collect::<Vec<_>>();
    reversed.reverse();
    let response = serde_json::json!({
            "analyses": reversed.iter().map(|id| cleanup_output(id, &format!("assessment for {id}"))).collect::<Vec<_>>()
        })
        .to_string();
    let provider = StaticCleanupProvider::new(vec![response]);
    let published = analyze_fixture_candidates(fixture.db(), &run, &candidates, &provider)
        .expect("publish reordered exact response");

    assert_eq!(
        published
            .iter()
            .map(|finding| finding.id.as_str())
            .collect::<Vec<_>>(),
        candidates
            .iter()
            .map(|candidate| candidate.id.as_str())
            .collect::<Vec<_>>()
    );
    for candidate in &candidates {
        let finding = fixture
            .db()
            .get_analysis_finding(&candidate.id)
            .expect("read reordered finding")
            .expect("finding exists");
        assert_eq!(
            finding.evidence_summary["aiAssessment"]["assessment"]["reason"].as_str(),
            Some(format!("assessment for {}", candidate.id).as_str())
        );
        assert!(has_current_ai_assessment(fixture.db(), &finding.id));
    }
}

#[test]
fn malformed_authority_fields_fail_closed_without_evidence() {
    let fixture = new_cleanup_ai_fixture();
    let run = complete_cleanup_analysis_run(fixture.db(), &fixture.source_root);
    let candidates = active_cleanup_candidates(fixture.db(), &run.id);
    let candidate = candidates[0].clone();
    let response = serde_json::json!({
        "analyses": [{
            "candidateId": candidate.id,
            "tier": "Safe",
            "trashAllowed": true,
            "execute": true,
            "deleteAllowed": true,
            "overwrite": true,
            "requiresConfirmation": false,
            "confirmationBypass": true,
            "operationId": "forged-operation",
            "journalState": "committed",
            "absolutePath": "F:/forged/target"
        }]
    })
    .to_string();
    let provider = StaticCleanupProvider::new(vec![response.clone(), response]);
    let result = analyze_fixture_candidates(
        fixture.db(),
        &run,
        std::slice::from_ref(&candidate),
        &provider,
    );

    assert!(result.is_err());
    assert_eq!(provider.calls.load(Ordering::Relaxed), 2);
    let finding = fixture
        .db()
        .get_analysis_finding(&candidate.id)
        .expect("read malformed candidate")
        .expect("candidate exists");
    assert_eq!(finding.status, "active");
    assert!(!has_current_ai_assessment(fixture.db(), &finding.id));
    assert_eq!(
        fixture
            .db()
            .list_analysis_finding_evidence(&candidate.id)
            .expect("read malformed evidence")
            .iter()
            .filter(|item| item.evidence_kind == "ai_assessment")
            .count(),
        0
    );
}

#[test]
fn source_change_during_provider_call_makes_result_stale() {
    let fixture = new_cleanup_ai_fixture();
    let run = complete_cleanup_analysis_run(fixture.db(), &fixture.source_root);
    let candidates = active_cleanup_candidates(fixture.db(), &run.id);
    let candidate = candidates[0].clone();
    let source_path = PathBuf::from(&candidate.path);
    let provider = StaticCleanupProvider::with_callback(
        vec![response_for_ids(std::slice::from_ref(&candidate.id))],
        move || {
            fs::write(source_path, b"changed source with a new size")
                .expect("change fixture source");
        },
    );

    let result = analyze_fixture_candidates(
        fixture.db(),
        &run,
        std::slice::from_ref(&candidate),
        &provider,
    );
    assert!(result.is_err());
    let finding = fixture
        .db()
        .get_analysis_finding(&candidate.id)
        .expect("read stale source finding")
        .expect("finding exists");
    assert_eq!(finding.status, "stale");
    assert!(!has_current_ai_assessment(fixture.db(), &finding.id));
    assert!(fixture
        .db()
        .list_analysis_finding_evidence(&candidate.id)
        .expect("read stale source evidence")
        .iter()
        .all(|item| item.evidence_kind != "ai_assessment"));
}

#[test]
fn provider_policy_change_during_provider_call_makes_result_stale() {
    let fixture = new_cleanup_ai_fixture();
    let run = complete_cleanup_analysis_run(fixture.db(), &fixture.source_root);
    let candidates = active_cleanup_candidates(fixture.db(), &run.id);
    let candidate = candidates[0].clone();
    let db_for_callback = fixture.db().clone();
    let provider = StaticCleanupProvider::with_callback(
        vec![response_for_ids(std::slice::from_ref(&candidate.id))],
        move || {
            let changed_settings = AISettings {
                enabled: true,
                cleanup_ai_enabled: true,
                model: "cleanup-policy-change-test".to_string(),
                ..AISettings::default()
            };
            let settings_json = serde_json::to_string(&changed_settings)
                .expect("serialize changed cleanup AI policy");
            let connection = db_for_callback
                .conn()
                .expect("open settings policy database connection");
            connection
                .execute(
                    "INSERT INTO app_settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    rusqlite::params![AI_SETTINGS_KEY, settings_json],
                )
                .expect("change cleanup AI provider policy");
        },
    );

    let result = analyze_fixture_candidates(
        fixture.db(),
        &run,
        std::slice::from_ref(&candidate),
        &provider,
    );
    assert!(result.is_err());
    let finding = fixture
        .db()
        .get_analysis_finding(&candidate.id)
        .expect("read policy-stale finding")
        .expect("candidate exists");
    assert_eq!(finding.status, "active");
    assert!(!has_current_ai_assessment(fixture.db(), &finding.id));
    assert!(fixture
        .db()
        .list_analysis_finding_evidence(&candidate.id)
        .expect("read policy-stale evidence")
        .iter()
        .all(|item| item.evidence_kind != "ai_assessment"));
}

#[test]
fn replaced_candidate_set_cannot_receive_old_provider_response() {
    let fixture = new_cleanup_ai_fixture();
    let first_run = complete_cleanup_analysis_run(fixture.db(), &fixture.source_root);
    let candidates = active_cleanup_candidates(fixture.db(), &first_run.id);
    let candidate = candidates[0].clone();
    let db_for_callback = fixture.db().clone();
    let source_root = fixture.source_root.clone();
    let provider = StaticCleanupProvider::with_callback(
        vec![response_for_ids(std::slice::from_ref(&candidate.id))],
        move || {
            complete_cleanup_analysis_run(&db_for_callback, &source_root);
        },
    );

    let result = analyze_fixture_candidates(
        fixture.db(),
        &first_run,
        std::slice::from_ref(&candidate),
        &provider,
    );
    assert!(result.is_err());
    let old = fixture
        .db()
        .get_analysis_finding(&candidate.id)
        .expect("read replaced candidate")
        .expect("old finding exists");
    assert_eq!(old.status, "superseded");
    assert!(!has_current_ai_assessment(fixture.db(), &old.id));
    assert!(fixture
        .db()
        .list_analysis_finding_evidence(&candidate.id)
        .expect("read replaced candidate evidence")
        .iter()
        .all(|item| item.evidence_kind != "ai_assessment"));
}

#[test]
fn overlapping_requests_commit_b_first_and_reject_a_as_stale() {
    let fixture = new_cleanup_ai_fixture();
    let run = complete_cleanup_analysis_run(fixture.db(), &fixture.source_root);
    let candidates = active_cleanup_candidates(fixture.db(), &run.id);
    let candidate = candidates[0].clone();
    let response = response_for_ids(std::slice::from_ref(&candidate.id));
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let provider_a = Arc::new(StaticCleanupProvider::with_callback(
        vec![response.clone()],
        move || {
            entered_tx.send(()).expect("signal provider A started");
            release_rx
                .recv_timeout(Duration::from_secs(10))
                .expect("release provider A after B publishes");
        },
    ));
    let db_a = fixture.db().clone();
    let run_a = run.clone();
    let candidate_a = candidate.clone();
    let handle_a = thread::spawn(move || {
        analyze_fixture_candidates(&db_a, &run_a, &[candidate_a], provider_a.as_ref())
    });
    entered_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("provider A captured its precondition");

    let provider_b = StaticCleanupProvider::new(vec![response]);
    let published_b = analyze_fixture_candidates(
        fixture.db(),
        &run,
        std::slice::from_ref(&candidate),
        &provider_b,
    )
    .expect("request B publishes first");
    assert_eq!(published_b.len(), 1);
    release_tx.send(()).expect("release request A");
    let result_a = handle_a.join().expect("join request A");
    assert!(result_a.is_err());

    let finding = fixture
        .db()
        .get_analysis_finding(&candidate.id)
        .expect("read concurrent finding")
        .expect("finding exists");
    assert!(has_current_ai_assessment(fixture.db(), &finding.id));
    assert_eq!(
        fixture
            .db()
            .list_analysis_finding_evidence(&candidate.id)
            .expect("read concurrent evidence")
            .iter()
            .filter(|item| item.evidence_kind == "ai_assessment")
            .count(),
        1
    );
}

#[test]
fn current_assessment_rejects_a_newer_finding_revision() {
    let fixture = new_cleanup_ai_fixture();
    let (_run, candidate, finding) = publish_single_cleanup_assessment(&fixture);
    assert!(has_current_ai_assessment(fixture.db(), &finding.id));

    let connection = fixture.db().conn().expect("open fixture database");
    connection
        .execute(
            "UPDATE analysis_findings SET revision = revision + 1 WHERE id = ?1",
            [&candidate.id],
        )
        .expect("advance finding revision");
    drop(connection);

    assert!(!has_current_ai_assessment(fixture.db(), &finding.id));
}

#[test]
fn current_assessment_rejects_changed_live_source_identity() {
    let fixture = new_cleanup_ai_fixture();
    let (_run, candidate, finding) = publish_single_cleanup_assessment(&fixture);
    assert!(has_current_ai_assessment(fixture.db(), &finding.id));

    fs::write(&candidate.path, b"changed after AI publication")
        .expect("change source after assessment publication");

    assert!(!has_current_ai_assessment(fixture.db(), &finding.id));
}

#[test]
fn current_assessment_rejects_a_newer_analysis_run_revision() {
    let fixture = new_cleanup_ai_fixture();
    let (run, _candidate, finding) = publish_single_cleanup_assessment(&fixture);
    assert!(has_current_ai_assessment(fixture.db(), &finding.id));

    let connection = fixture.db().conn().expect("open fixture database");
    connection
        .execute(
            "UPDATE analysis_runs SET revision = revision + 1 WHERE id = ?1",
            [&run.id],
        )
        .expect("advance Analysis Run revision");
    drop(connection);

    assert!(!has_current_ai_assessment(fixture.db(), &finding.id));
}

#[test]
fn current_assessment_rejects_a_changed_analysis_source_snapshot() {
    let fixture = new_cleanup_ai_fixture();
    let (run, _candidate, finding) = publish_single_cleanup_assessment(&fixture);
    assert!(has_current_ai_assessment(fixture.db(), &finding.id));

    let connection = fixture.db().conn().expect("open fixture database");
    connection
        .execute(
            "UPDATE analysis_runs SET source_snapshot_hash = 'changed-source-snapshot' WHERE id = ?1",
            [&run.id],
        )
        .expect("change Analysis Run source snapshot hash");
    drop(connection);

    assert!(!has_current_ai_assessment(fixture.db(), &finding.id));
}

#[test]
fn current_assessment_rejects_a_newer_detector_revision() {
    let fixture = new_cleanup_ai_fixture();
    let (run, _candidate, finding) = publish_single_cleanup_assessment(&fixture);
    assert!(has_current_ai_assessment(fixture.db(), &finding.id));

    let connection = fixture.db().conn().expect("open fixture database");
    connection
        .execute(
            "UPDATE analysis_run_detectors SET revision = revision + 1 WHERE run_id = ?1 AND detector_id = ?2",
            rusqlite::params![run.id, crate::analysis::CLEANUP_HEURISTICS_DETECTOR],
        )
        .expect("advance detector revision");
    drop(connection);

    assert!(!has_current_ai_assessment(fixture.db(), &finding.id));
}

#[test]
fn current_assessment_rejects_a_changed_detector_status() {
    let fixture = new_cleanup_ai_fixture();
    let (run, _candidate, finding) = publish_single_cleanup_assessment(&fixture);
    assert!(has_current_ai_assessment(fixture.db(), &finding.id));

    let connection = fixture.db().conn().expect("open fixture database");
    connection
        .execute(
            "UPDATE analysis_run_detectors SET status = 'failed', revision = revision + 1 WHERE run_id = ?1 AND detector_id = ?2",
            rusqlite::params![run.id, crate::analysis::CLEANUP_HEURISTICS_DETECTOR],
        )
        .expect("change detector status");
    drop(connection);

    assert!(!has_current_ai_assessment(fixture.db(), &finding.id));
}

#[test]
fn current_assessment_rejects_changed_provider_settings() {
    let fixture = new_cleanup_ai_fixture();
    let (_run, _candidate, finding) = publish_single_cleanup_assessment(&fixture);
    assert!(has_current_ai_assessment(fixture.db(), &finding.id));

    let changed_settings = AISettings {
        model: "changed-after-publication".to_string(),
        ..test_ai_settings()
    };
    let settings_json =
        serde_json::to_string(&changed_settings).expect("serialize changed AI settings");
    let connection = fixture.db().conn().expect("open fixture database");
    connection
        .execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![AI_SETTINGS_KEY, settings_json],
        )
        .expect("change persisted AI settings");
    drop(connection);

    assert!(!has_current_ai_assessment(fixture.db(), &finding.id));
}

#[test]
fn current_assessment_requires_matching_durable_evidence() {
    let fixture = new_cleanup_ai_fixture();
    let (_run, _candidate, finding) = publish_single_cleanup_assessment(&fixture);
    assert!(has_current_ai_assessment(fixture.db(), &finding.id));

    let connection = fixture.db().conn().expect("open fixture database");
    connection
        .execute(
            "UPDATE analysis_finding_evidence SET value_json = '{}' WHERE finding_id = ?1 AND evidence_kind = 'ai_assessment'",
            [&finding.id],
        )
        .expect("mismatch durable assessment evidence");
    drop(connection);

    assert!(!has_current_ai_assessment(fixture.db(), &finding.id));
}

#[test]
fn current_assessment_rejects_removed_durable_evidence() {
    let fixture = new_cleanup_ai_fixture();
    let (_run, _candidate, finding) = publish_single_cleanup_assessment(&fixture);
    assert!(has_current_ai_assessment(fixture.db(), &finding.id));

    let connection = fixture.db().conn().expect("open fixture database");
    connection
        .execute(
            "DELETE FROM analysis_finding_evidence WHERE finding_id = ?1 AND evidence_kind = 'ai_assessment'",
            [&finding.id],
        )
        .expect("remove durable assessment evidence");
    drop(connection);

    assert!(!has_current_ai_assessment(fixture.db(), &finding.id));
}

#[test]
fn legacy_misaligned_finding_identity_cannot_keep_a_current_assessment() {
    let fixture = new_cleanup_ai_fixture();
    let (_run, candidate, finding) = publish_single_cleanup_assessment(&fixture);
    assert!(has_current_ai_assessment(fixture.db(), &finding.id));

    let metadata = fs::metadata(&candidate.path).expect("read source metadata");
    let mut legacy_identity = finding.identity_snapshot.clone();
    legacy_identity["size"] = serde_json::json!(metadata.len() + 1);
    legacy_identity
        .as_object_mut()
        .expect("finding identity is an object")
        .remove("logicalSize");
    let identity_json =
        serde_json::to_string(&legacy_identity).expect("serialize legacy source identity");
    let connection = fixture.db().conn().expect("open fixture database");
    connection
        .execute(
            "UPDATE analysis_findings SET identity_snapshot_json = ?1 WHERE id = ?2",
            rusqlite::params![identity_json, finding.id],
        )
        .expect("restore misaligned legacy identity snapshot");
    drop(connection);

    assert!(!has_current_ai_assessment(fixture.db(), &finding.id));
}

#[test]
fn empty_candidate_request_never_calls_provider_or_publishes_evidence() {
    let fixture = new_cleanup_ai_fixture();
    let run = complete_cleanup_analysis_run(fixture.db(), &fixture.source_root);
    let provider = StaticCleanupProvider::new(Vec::new());
    let settings = test_ai_settings();
    let expected_revisions = HashMap::new();
    let settings_revision = crate::db::app_setting_value_fingerprint(None);
    let result = analyze_cleanup_candidates_with_provider(
        CleanupAiProviderContext {
            db: fixture.db(),
            job_id: &run.id,
            candidates: Vec::new(),
            expected_revisions: &expected_revisions,
            settings: &settings,
            settings_revision: &settings_revision,
            app_data_dir: None,
        },
        &provider,
    )
    .expect("empty request succeeds without a provider call");
    assert!(result.is_empty());
    assert_eq!(provider.calls.load(Ordering::Relaxed), 0);
}

#[test]
fn allocated_cleanup_size_does_not_replace_logical_source_length() {
    let fixture = new_cleanup_ai_fixture();
    let run = complete_cleanup_analysis_run(fixture.db(), &fixture.source_root);
    let candidates = active_cleanup_candidates(fixture.db(), &run.id);

    assert_eq!(candidates.len(), 3);
    for candidate in candidates {
        let finding = fixture
            .db()
            .get_analysis_finding(&candidate.id)
            .expect("read cleanup finding")
            .expect("cleanup finding exists");
        let metadata = fs::symlink_metadata(&candidate.path).expect("read fixture identity");

        assert!(metadata.is_file());
        assert_eq!(
            finding.identity_snapshot["size"].as_u64(),
            Some(candidate.size)
        );
        if candidate.size != metadata.len() {
            assert_eq!(
                finding.identity_snapshot["logicalSize"].as_u64(),
                Some(metadata.len())
            );
        } else {
            assert!(finding.identity_snapshot.get("logicalSize").is_none());
        }
        assert!(crate::analysis::finding_identity_matches(
            fixture.db(),
            &finding
        ));

        #[cfg(target_os = "macos")]
        assert_ne!(candidate.size, metadata.len());
    }
}

fn test_ai_settings() -> AISettings {
    AISettings {
        enabled: true,
        cleanup_ai_enabled: true,
        batch_size: 10,
        ..AISettings::default()
    }
}

fn publish_single_cleanup_assessment(
    fixture: &CleanupAiFixture,
) -> (AnalysisRunDto, StorageCandidate, AnalysisFindingDto) {
    let run = complete_cleanup_analysis_run(fixture.db(), &fixture.source_root);
    let candidate = active_cleanup_candidates(fixture.db(), &run.id)
        .into_iter()
        .next()
        .expect("fixture has a cleanup candidate");
    let provider =
        StaticCleanupProvider::new(vec![response_for_ids(std::slice::from_ref(&candidate.id))]);
    let published = analyze_fixture_candidates(
        fixture.db(),
        &run,
        std::slice::from_ref(&candidate),
        &provider,
    )
    .expect("publish exact candidate assessment");
    assert_eq!(published.len(), 1);
    let finding = fixture
        .db()
        .get_analysis_finding(&candidate.id)
        .expect("read published candidate")
        .expect("published candidate exists");
    (run, candidate, finding)
}

fn analyze_fixture_candidates(
    db: &Database,
    run: &AnalysisRunDto,
    candidates: &[StorageCandidate],
    provider: &dyn AIProvider,
) -> Result<Vec<AnalysisFindingDto>, String> {
    let expected_revisions = candidates
        .iter()
        .map(|candidate| {
            db.get_analysis_finding(&candidate.id)
                .map_err(|error| error.to_string())?
                .map(|finding| (finding.id, finding.revision))
                .ok_or_else(|| "fixture finding was not found".to_string())
        })
        .collect::<Result<HashMap<_, _>, _>>()?;
    let settings = test_ai_settings();
    let settings_revision = crate::db::app_setting_value_fingerprint(None);
    analyze_cleanup_candidates_with_provider(
        CleanupAiProviderContext {
            db,
            job_id: &run.id,
            candidates: candidates.to_vec(),
            expected_revisions: &expected_revisions,
            settings: &settings,
            settings_revision: &settings_revision,
            app_data_dir: None,
        },
        provider,
    )
}

fn new_cleanup_ai_fixture() -> CleanupAiFixture {
    let fixture_base = std::env::var_os("CLEANUP_AI_TEST_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".tmp-tests"));
    let fixture_base = if fixture_base.is_absolute() {
        fixture_base
    } else {
        std::env::current_dir()
            .expect("resolve cleanup AI fixture workspace")
            .join(fixture_base)
    };
    let root = fixture_base.join(new_job_id("cleanup-ai-fixture"));
    let source_root = root.join("Downloads");
    fs::create_dir_all(&source_root).expect("create isolated cleanup source root");
    for name in ["alpha.msi", "beta.msi", "gamma.msi"] {
        fs::write(
            source_root.join(name),
            format!("fixture content for {name}"),
        )
        .expect("write cleanup source file");
    }
    let mut fixture = CleanupAiFixture {
        root: root.clone(),
        source_root,
        db: None,
    };
    fixture.db =
        Some(Database::open(root.join("cleanup-ai.sqlite3")).expect("open fixture database"));
    fixture
}

fn complete_cleanup_analysis_run(db: &Database, source_root: &Path) -> AnalysisRunDto {
    let detector_id = crate::analysis::CLEANUP_HEURISTICS_DETECTOR;
    let run = db
        .start_analysis_run(
            &crate::db::StartAnalysisRunRequest {
                scope: crate::db::AnalysisScopeRequest {
                    kind: "approved_cleanup_paths".to_string(),
                    root_ids: Vec::new(),
                    paths: vec![source_root.to_string_lossy().into_owned()],
                },
                detector_ids: vec![detector_id.to_string()],
                request_key: Some(new_job_id("cleanup-ai-analysis")),
            },
            &[(detector_id.to_string(), 1)],
        )
        .expect("start durable cleanup analysis")
        .run;
    let running_run = db
        .claim_analysis_run(&run.id)
        .expect("claim cleanup analysis")
        .expect("cleanup analysis is queued");
    let detector = db
        .list_analysis_run_detectors(&run.id)
        .expect("list cleanup detector")
        .into_iter()
        .next()
        .expect("cleanup detector exists");
    let running_detector = db
        .set_analysis_detector_status(
            &run.id,
            &detector.detector_id,
            detector.revision,
            "running",
            0,
            0,
            0,
            0,
            None,
            None,
        )
        .expect("start cleanup detector");
    let drafts = crate::analysis::build_cleanup_findings(
        db,
        &running_run,
        &Arc::new(AtomicBool::new(false)),
    )
    .expect("run cleanup detector");
    let drafts = drafts
        .into_iter()
        .filter_map(|(id, draft)| (id == detector_id).then_some(draft))
        .collect::<Vec<_>>();
    assert!(
        !drafts.is_empty(),
        "fixture files must produce cleanup findings"
    );
    db.stage_analysis_findings(&run.id, &run.scope_hash, &drafts)
        .expect("stage cleanup detector findings");
    db.set_analysis_detector_status(
        &run.id,
        &detector.detector_id,
        running_detector.revision,
        "completed",
        drafts.len() as i64,
        drafts.len() as i64,
        drafts
            .iter()
            .filter_map(|draft| draft.exact_reclaimable_bytes)
            .sum(),
        drafts
            .iter()
            .map(|draft| draft.potential_reclaimable_bytes)
            .sum(),
        None,
        None,
    )
    .expect("complete cleanup detector");
    db.publish_analysis_run(&run.id)
        .expect("publish durable cleanup run");
    db.get_analysis_run(&run.id)
        .expect("read completed cleanup run")
}

fn active_cleanup_candidates(db: &Database, run_id: &str) -> Vec<StorageCandidate> {
    let page = db
        .list_analysis_findings(
            &crate::db::AnalysisFindingFilter {
                run_id: Some(run_id.to_string()),
                detector_id: Some(crate::analysis::CLEANUP_HEURISTICS_DETECTOR.to_string()),
                status: Some("active".to_string()),
                ..crate::db::AnalysisFindingFilter::default()
            },
            None,
            200,
        )
        .expect("list current cleanup findings");
    let mut candidates = page
        .findings
        .into_iter()
        .filter(|finding| {
            Path::new(finding.path_snapshot.as_deref().unwrap_or_default())
                .extension()
                .and_then(|extension| extension.to_str())
                == Some("msi")
        })
        .map(|finding| storage_candidate_from_analysis_finding(&finding).expect("project finding"))
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.name.cmp(&right.name));
    candidates
}

fn response_for_ids(ids: &[String]) -> String {
    serde_json::json!({
            "analyses": ids.iter().map(|id| cleanup_output(id, &format!("assessment for {id}"))).collect::<Vec<_>>()
        })
        .to_string()
}

fn cleanup_output(id: &str, reason: &str) -> serde_json::Value {
    serde_json::json!({
        "candidateId": id,
        "tier": "Safe",
        "category": "AI reviewed metadata",
        "suggestedAction": "MoveToTrash",
        "confidence": 0.95,
        "reason": reason,
        "riskNote": "Review project context before cleanup.",
        "trashAllowed": true,
        "selectedByDefault": true
    })
}

struct StaticCleanupProvider {
    responses: Mutex<VecDeque<String>>,
    callback: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    requests: Mutex<Vec<AIChatRequest>>,
    calls: AtomicUsize,
}

impl StaticCleanupProvider {
    fn new(responses: Vec<String>) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
            callback: Mutex::new(None),
            requests: Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
        }
    }

    fn with_callback(responses: Vec<String>, callback: impl FnOnce() + Send + 'static) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
            callback: Mutex::new(Some(Box::new(callback))),
            requests: Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
        }
    }

    fn requests(&self) -> Vec<AIChatRequest> {
        self.requests.lock().expect("provider request lock").clone()
    }
}

impl AIProvider for StaticCleanupProvider {
    fn chat_json(&self, request: AIChatRequest) -> Result<String, AIProviderError> {
        self.requests
            .lock()
            .expect("provider request lock")
            .push(request);
        self.calls.fetch_add(1, Ordering::Relaxed);
        if let Some(callback) = self.callback.lock().expect("provider callback lock").take() {
            callback();
        }
        self.responses
            .lock()
            .expect("provider response lock")
            .pop_front()
            .ok_or_else(|| AIProviderError::new("No test provider response remains."))
    }

    fn test_connection(
        &self,
    ) -> Result<super::super::schema::AIConnectionTestResult, AIProviderError> {
        Err(AIProviderError::new("Test provider does not connect."))
    }
}

struct CleanupAiFixture {
    root: PathBuf,
    source_root: PathBuf,
    db: Option<Database>,
}

impl CleanupAiFixture {
    fn db(&self) -> &Database {
        self.db.as_ref().expect("fixture database is open")
    }
}

impl Drop for CleanupAiFixture {
    fn drop(&mut self) {
        self.db.take();
        if self.root.exists() {
            fs::remove_dir_all(&self.root).expect("remove task-owned cleanup AI fixture");
        }
    }
}

fn safe_candidate(id: &str, path: &str) -> StorageCandidate {
    candidate(
        id,
        path,
        CleanupTier::Safe,
        CleanupActionKind::MoveToTrash,
        true,
        true,
    )
}

fn review_candidate(id: &str, path: &str) -> StorageCandidate {
    candidate(
        id,
        path,
        CleanupTier::Review,
        CleanupActionKind::Reveal,
        false,
        false,
    )
}

fn caution_candidate(id: &str, path: &str) -> StorageCandidate {
    candidate(
        id,
        path,
        CleanupTier::Caution,
        CleanupActionKind::Reveal,
        false,
        false,
    )
}

fn candidate(
    id: &str,
    path: &str,
    tier: CleanupTier,
    suggested_action: CleanupActionKind,
    trash_allowed: bool,
    selected_by_default: bool,
) -> StorageCandidate {
    StorageCandidate {
        id: id.to_string(),
        path: path.to_string(),
        name: path.rsplit('/').next().unwrap_or(path).to_string(),
        size: 820_000_000,
        tier,
        category: "Original category".to_string(),
        reason: "Original reason.".to_string(),
        suggested_action,
        risk_note: Some("Original risk.".to_string()),
        trash_allowed,
        selected_by_default,
    }
}

fn move_to_trash_output(id: &str) -> AICleanupAnalysisOutput {
    AICleanupAnalysisOutput {
        candidate_id: id.to_string(),
        tier: Some("Safe".to_string()),
        category: Some("AI category".to_string()),
        suggested_action: Some("MoveToTrash".into()),
        confidence: Some(0.95),
        reason: Some("AI reason.".to_string()),
        risk_note: Some("AI risk.".to_string()),
        trash_allowed: Some(true),
        selected_by_default: Some(true),
    }
}

fn cleanup_response(id: &str) -> String {
    format!(
        r#"{{"analyses":[{{"candidateId":"{id}","tier":"Safe","category":"AI category","suggestedAction":"MoveToTrash","confidence":0.95,"reason":"AI reason.","riskNote":"AI risk.","trashAllowed":true,"selectedByDefault":true}}]}}"#
    )
}
