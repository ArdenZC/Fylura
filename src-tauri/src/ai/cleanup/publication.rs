use std::collections::HashSet;

use super::*;

#[derive(Debug, Clone)]
struct CleanupAiCandidateSnapshot {
    candidate: StorageCandidate,
    finding: AnalysisFindingDto,
    detector: AnalysisDetectorDto,
    precondition: AnalysisAiFindingPrecondition,
    candidate_identity_fingerprint: String,
    source_fingerprint: String,
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct CleanupAiCoverageCounts {
    pub(super) requested: usize,
    pub(super) returned: usize,
    pub(super) omitted: usize,
    pub(super) duplicated: usize,
    pub(super) unknown: usize,
}

#[derive(Debug, Default)]
pub(super) struct CleanupAiCoverage {
    pub(super) unique_outputs: HashMap<String, AICleanupAnalysisOutput>,
    pub(super) counts: CleanupAiCoverageCounts,
}

pub(super) struct CleanupAiProviderContext<'a> {
    pub(super) db: &'a Database,
    pub(super) job_id: &'a str,
    pub(super) candidates: Vec<StorageCandidate>,
    pub(super) expected_revisions: &'a HashMap<String, i64>,
    pub(super) settings: &'a AISettings,
    pub(super) settings_revision: &'a str,
    pub(super) app_data_dir: Option<PathBuf>,
}

struct CleanupAiEvidenceInput<'a> {
    finding: &'a AnalysisFindingDto,
    detector: &'a AnalysisDetectorDto,
    run: &'a AnalysisRunDto,
    settings: &'a AISettings,
    settings_revision: &'a str,
    request_id: &'a str,
    batch_id: &'a str,
    requested_ids: &'a [String],
    candidate_set_fingerprint: &'a str,
    counts: &'a CleanupAiCoverageCounts,
    output: &'a AICleanupAnalysisOutput,
    merged: &'a StorageCandidate,
    candidate_identity_fingerprint: &'a str,
    source_fingerprint: &'a str,
    request_candidate_set_fingerprint: &'a str,
}

pub(super) fn analyze_cleanup_candidates_with_configured_provider(
    db: &Database,
    job_id: &str,
    candidates: Vec<StorageCandidate>,
    expected_revisions: &HashMap<String, i64>,
    settings: &AISettings,
    settings_revision: &str,
    app_data_dir: Option<PathBuf>,
) -> Result<Vec<AnalysisFindingDto>, String> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let readiness = crate::ai::readiness::cleanup_ai_readiness_from_settings(
        settings.clone(),
        settings_revision,
    );
    if readiness.state != crate::ai::readiness::AIReadinessState::Ready {
        return Err(format!(
            "AI cleanup analysis is not ready: {}",
            readiness.reason
        ));
    }
    let provider: Box<dyn AIProvider> = match settings.provider {
        AIProviderKind::OpenAICompatible => {
            Box::new(OpenAICompatibleProvider::new(settings.clone()))
        }
        AIProviderKind::Ollama => Box::new(OllamaProvider::new(settings.clone())),
    };
    analyze_cleanup_candidates_with_provider(
        CleanupAiProviderContext {
            db,
            job_id,
            candidates,
            expected_revisions,
            settings,
            settings_revision,
            app_data_dir,
        },
        provider.as_ref(),
    )
}

pub(super) fn analyze_cleanup_candidates_with_provider(
    context: CleanupAiProviderContext<'_>,
    provider: &dyn AIProvider,
) -> Result<Vec<AnalysisFindingDto>, String> {
    let CleanupAiProviderContext {
        db,
        job_id,
        candidates,
        expected_revisions,
        settings,
        settings_revision,
        app_data_dir,
    } = context;
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let run = db
        .get_analysis_run(job_id)
        .map_err(|error| error.to_string())?;
    if run.scope.get("kind").and_then(serde_json::Value::as_str) != Some("approved_cleanup_paths")
        || !matches!(run.status.as_str(), "completed" | "completed_with_warnings")
    {
        return Err("Cleanup AI Analysis Run is no longer publishable.".to_string());
    }
    let detector_rows = db
        .list_analysis_run_detectors(job_id)
        .map_err(|error| error.to_string())?;
    let snapshots = capture_cleanup_ai_preconditions(
        db,
        &run,
        &detector_rows,
        &candidates,
        expected_revisions,
    )?;
    let request_id = new_job_id("cleanup-ai-request");
    let all_candidate_ids = snapshots
        .iter()
        .map(|snapshot| snapshot.finding.id.clone())
        .collect::<Vec<_>>();
    let request_candidate_set_fingerprint = candidate_set_fingerprint(&all_candidate_ids)?;
    let mut publications = Vec::new();
    let mut total_coverage = CleanupAiCoverageCounts::default();
    let batch_size = settings.batch_size.max(1);
    for (batch_index, batch) in snapshots.chunks(batch_size).enumerate() {
        let batch_candidates = batch
            .iter()
            .map(|snapshot| snapshot.candidate.clone())
            .collect::<Vec<_>>();
        let batch_id = format!("{request_id}-batch-{}", batch_index + 1);
        let trace_context = AITraceContext {
            operation: AITraceOperation::CleanupAnalysis,
            job_id: Some(job_id.to_string()),
            batch_id: Some(batch_id.clone()),
            target_count: Some(batch_candidates.len()),
            batch_size: Some(batch_candidates.len()),
            ..Default::default()
        };
        let content = call_ai_cleanup_provider(
            provider,
            settings,
            &batch_candidates,
            false,
            trace_context.clone(),
        )?;
        let outputs = match parse_ai_cleanup_analysis_response(&content) {
            Ok(outputs) => outputs,
            Err(_) => {
                let retry_content = call_ai_cleanup_provider(
                    provider,
                    settings,
                    &batch_candidates,
                    true,
                    trace_context,
                )?;
                parse_ai_cleanup_analysis_response(&retry_content).map_err(|error| {
                    format!(
                        "{error} 已尝试清洗和重试，但仍失败。建议关闭 thinking，或换用 deepseek-v4-flash / qwen-plus 等更稳定的非思考模型。"
                    )
                })?
            }
        };
        let batch_candidate_ids = batch
            .iter()
            .map(|snapshot| snapshot.finding.id.clone())
            .collect::<Vec<_>>();
        let coverage = cleanup_ai_coverage(&batch_candidate_ids, outputs)?;
        total_coverage.requested += coverage.counts.requested;
        total_coverage.returned += coverage.counts.returned;
        total_coverage.omitted += coverage.counts.omitted;
        total_coverage.duplicated += coverage.counts.duplicated;
        total_coverage.unknown += coverage.counts.unknown;

        let batch_candidate_set_fingerprint = candidate_set_fingerprint(&batch_candidate_ids)?;
        for snapshot in batch {
            let Some(output) = coverage.unique_outputs.get(&snapshot.finding.id) else {
                continue;
            };
            let merged =
                merge_ai_cleanup_analysis(&snapshot.candidate, output, app_data_dir.as_ref());
            let evidence = cleanup_ai_assessment_evidence(CleanupAiEvidenceInput {
                finding: &snapshot.finding,
                detector: &snapshot.detector,
                run: &run,
                settings,
                settings_revision,
                request_id: &request_id,
                batch_id: &batch_id,
                requested_ids: &batch_candidate_ids,
                candidate_set_fingerprint: &batch_candidate_set_fingerprint,
                counts: &coverage.counts,
                output,
                merged: &merged,
                candidate_identity_fingerprint: &snapshot.candidate_identity_fingerprint,
                source_fingerprint: &snapshot.source_fingerprint,
                request_candidate_set_fingerprint: &request_candidate_set_fingerprint,
            });
            publications.push(AnalysisAiAssessmentPublication {
                finding_id: snapshot.finding.id.clone(),
                requested_tier: tier_to_string(&merged.tier).to_ascii_lowercase(),
                requested_trash_allowed: merged.trash_allowed,
                evidence,
            });
        }
    }

    eprintln!(
        "Cleanup AI coverage request={request_id} requested={} returned={} omitted={} duplicated={} unknown={}",
        total_coverage.requested,
        total_coverage.returned,
        total_coverage.omitted,
        total_coverage.duplicated,
        total_coverage.unknown,
    );
    if publications.is_empty() {
        return Ok(Vec::new());
    }

    revalidate_cleanup_ai_request(db, &run, &snapshots, expected_revisions, settings_revision)?;

    let batch = AnalysisAiPublicationBatch {
        run_id: run.id.clone(),
        expected_run_revision: run.revision,
        expected_source_snapshot_hash: run.source_snapshot_hash.clone(),
        expected_detector_set_hash: run.detector_set_hash.clone(),
        settings_key: AI_SETTINGS_KEY.to_string(),
        expected_settings_revision: settings_revision.to_string(),
        expected_candidate_set_fingerprint: request_candidate_set_fingerprint,
        preconditions: snapshots
            .iter()
            .map(|snapshot| snapshot.precondition.clone())
            .collect(),
        assessments: publications,
    };
    db.publish_analysis_ai_assessments_cas(&batch)
        .map_err(|error| {
            eprintln!("Cleanup AI publication request={request_id} outcome=stale_or_invalid");
            error.to_string()
        })
}

fn capture_cleanup_ai_preconditions(
    db: &Database,
    run: &AnalysisRunDto,
    detector_rows: &[AnalysisDetectorDto],
    candidates: &[StorageCandidate],
    expected_revisions: &HashMap<String, i64>,
) -> Result<Vec<CleanupAiCandidateSnapshot>, String> {
    let mut snapshots = Vec::with_capacity(candidates.len());
    let mut seen = HashSet::with_capacity(candidates.len());
    for candidate in candidates {
        if candidate.id.trim().is_empty() || !seen.insert(candidate.id.clone()) {
            return Err("Cleanup AI request contains duplicate candidate identity.".to_string());
        }
        let finding = db
            .get_analysis_finding(&candidate.id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Cleanup AI finding disappeared before provider request.".to_string())?;
        let expected_revision = expected_revisions
            .get(&finding.id)
            .copied()
            .ok_or_else(|| "Cleanup AI request manifest is incomplete.".to_string())?;
        if finding.id != candidate.id
            || finding.run_id != run.id
            || finding.status != "active"
            || finding.revision != expected_revision
        {
            return Err("Cleanup AI finding changed before provider request.".to_string());
        }
        let detector = detector_rows
            .iter()
            .find(|detector| {
                detector.detector_id == finding.detector_id
                    && detector.detector_version == finding.detector_version
            })
            .cloned()
            .ok_or_else(|| "Cleanup AI detector binding is unavailable.".to_string())?;
        if !matches!(
            detector.status.as_str(),
            "completed" | "completed_with_warnings"
        ) || !crate::analysis::finding_identity_matches(db, &finding)
        {
            return Err("Cleanup AI source or detector binding is no longer current.".to_string());
        }
        let source_fingerprint = fingerprint_json(&finding.identity_snapshot)?;
        let candidate_identity_fingerprint =
            candidate_identity_fingerprint(&finding, expected_revision, &source_fingerprint)?;
        let identity_snapshot_json =
            serde_json::to_string(&finding.identity_snapshot).map_err(|error| error.to_string())?;
        let precondition = AnalysisAiFindingPrecondition {
            finding_id: finding.id.clone(),
            finding_key: finding.finding_key.clone(),
            run_id: finding.run_id.clone(),
            detector_id: finding.detector_id.clone(),
            detector_version: finding.detector_version,
            detector_status: detector.status.clone(),
            detector_revision: detector.revision,
            scope_hash: finding.scope_hash.clone(),
            expected_revision,
            primary_subject_kind: finding.primary_subject_kind.clone(),
            primary_subject_id: finding.primary_subject_id.clone(),
            path_snapshot: finding.path_snapshot.clone(),
            identity_snapshot_json,
        };
        snapshots.push(CleanupAiCandidateSnapshot {
            candidate: candidate.clone(),
            finding,
            detector,
            precondition,
            candidate_identity_fingerprint,
            source_fingerprint,
        });
    }
    if snapshots.len() != candidates.len() || snapshots.len() != expected_revisions.len() {
        return Err("Cleanup AI request manifest does not match its candidate set.".to_string());
    }
    Ok(snapshots)
}

fn revalidate_cleanup_ai_request(
    db: &Database,
    expected_run: &AnalysisRunDto,
    snapshots: &[CleanupAiCandidateSnapshot],
    expected_revisions: &HashMap<String, i64>,
    expected_settings_revision: &str,
) -> Result<(), String> {
    let (_, current_settings_revision) =
        get_ai_settings_for_db_with_revision(db).map_err(|error| error.to_string())?;
    if current_settings_revision != expected_settings_revision {
        return Err(
            "Cleanup AI response is stale because provider policy changed; it was not published."
                .to_string(),
        );
    }
    let current_run = db
        .get_analysis_run(&expected_run.id)
        .map_err(|error| error.to_string())?;
    if current_run.status != expected_run.status
        || !matches!(
            current_run.status.as_str(),
            "completed" | "completed_with_warnings"
        )
        || current_run.revision != expected_run.revision
        || current_run.source_snapshot_hash != expected_run.source_snapshot_hash
        || current_run.detector_set_hash != expected_run.detector_set_hash
    {
        return Err(
            "Cleanup AI response is stale because the Analysis Run changed; it was not published."
                .to_string(),
        );
    }
    let selections = snapshots
        .iter()
        .map(
            |snapshot| crate::storage_analyzer::CleanupFindingSelection {
                finding_id: snapshot.finding.id.clone(),
                expected_revision: snapshot.precondition.expected_revision,
                review_confirmation: None,
            },
        )
        .collect::<Vec<_>>();
    let current_candidates =
        resolve_analysis_candidates_for_cleanup(db, &expected_run.id, &selections, false)?;
    if current_candidates.len() != snapshots.len() {
        return Err("Cleanup AI response is stale because candidate coverage changed; it was not published.".to_string());
    }
    let current_detectors = db
        .list_analysis_run_detectors(&expected_run.id)
        .map_err(|error| error.to_string())?;
    for snapshot in snapshots {
        let current_finding = db
            .get_analysis_finding(&snapshot.finding.id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "Cleanup AI response is stale because a candidate disappeared; it was not published.".to_string())?;
        let current_detector = current_detectors
            .iter()
            .find(|detector| detector.detector_id == snapshot.detector.detector_id)
            .ok_or_else(|| {
                "Cleanup AI response is stale because detector state changed; it was not published."
                    .to_string()
            })?;
        if !same_cleanup_ai_precondition(
            &current_finding,
            current_detector,
            &snapshot.precondition,
        )? || !crate::analysis::finding_identity_matches(db, &current_finding)
        {
            return Err("Cleanup AI response is stale because candidate identity or source changed; it was not published.".to_string());
        }
        let expected_revision = expected_revisions
            .get(&current_finding.id)
            .copied()
            .ok_or_else(|| "Cleanup AI response is stale because candidate coverage changed; it was not published.".to_string())?;
        if current_finding.revision != expected_revision
            || current_detector.revision != snapshot.detector.revision
            || current_detector.status != snapshot.detector.status
        {
            return Err("Cleanup AI response is stale because revision or detector state changed; it was not published.".to_string());
        }
    }
    Ok(())
}

fn same_cleanup_ai_precondition(
    finding: &AnalysisFindingDto,
    detector: &AnalysisDetectorDto,
    expected: &AnalysisAiFindingPrecondition,
) -> Result<bool, String> {
    let identity_snapshot_json =
        serde_json::to_string(&finding.identity_snapshot).map_err(|error| error.to_string())?;
    Ok(finding.id == expected.finding_id
        && finding.finding_key == expected.finding_key
        && finding.run_id == expected.run_id
        && finding.detector_id == expected.detector_id
        && finding.detector_version == expected.detector_version
        && finding.scope_hash == expected.scope_hash
        && finding.status == "active"
        && finding.revision == expected.expected_revision
        && finding.primary_subject_kind == expected.primary_subject_kind
        && finding.primary_subject_id == expected.primary_subject_id
        && finding.path_snapshot == expected.path_snapshot
        && identity_snapshot_json == expected.identity_snapshot_json
        && detector.detector_id == expected.detector_id
        && detector.detector_version == expected.detector_version
        && detector.status == expected.detector_status
        && detector.revision == expected.detector_revision)
}

pub(super) fn cleanup_ai_coverage(
    requested_ids: &[String],
    outputs: Vec<AICleanupAnalysisOutput>,
) -> Result<CleanupAiCoverage, String> {
    let requested = requested_ids.iter().cloned().collect::<HashSet<_>>();
    if requested.len() != requested_ids.len() {
        return Err("Cleanup AI request contains duplicate candidate identity.".to_string());
    }
    let mut outputs_by_id: HashMap<String, Vec<AICleanupAnalysisOutput>> = HashMap::new();
    let mut coverage = CleanupAiCoverage {
        counts: CleanupAiCoverageCounts {
            requested: requested_ids.len(),
            ..CleanupAiCoverageCounts::default()
        },
        ..CleanupAiCoverage::default()
    };
    for output in outputs {
        if !requested.contains(&output.candidate_id) {
            coverage.counts.unknown += 1;
            continue;
        }
        outputs_by_id
            .entry(output.candidate_id.clone())
            .or_default()
            .push(output);
    }
    for id in requested_ids {
        match outputs_by_id.remove(id).unwrap_or_default().as_slice() {
            [output] => {
                coverage.counts.returned += 1;
                coverage.unique_outputs.insert(id.clone(), output.clone());
            }
            [] => coverage.counts.omitted += 1,
            _ => coverage.counts.duplicated += 1,
        }
    }
    Ok(coverage)
}

fn cleanup_ai_assessment_evidence(input: CleanupAiEvidenceInput<'_>) -> serde_json::Value {
    serde_json::json!({
        "schema": "cleanup_ai_assessment_v1",
        "coverage": {
            "status": "returned_exactly_once",
            "candidateId": input.finding.id,
            "returnedCandidateId": input.output.candidate_id,
            "requestId": input.request_id,
            "batchId": input.batch_id,
            "requestedCandidateIds": input.requested_ids,
            "candidateSetFingerprint": input.candidate_set_fingerprint,
            "requestCandidateSetFingerprint": input.request_candidate_set_fingerprint,
            "requestedCount": input.counts.requested,
            "returnedCount": input.counts.returned,
            "omittedCount": input.counts.omitted,
            "duplicateCount": input.counts.duplicated,
            "unknownCount": input.counts.unknown,
            "analysisRunId": input.run.id,
            "analysisRunRevision": input.run.revision,
            "analysisSourceSnapshotHash": input.run.source_snapshot_hash,
            "detectorSetHash": input.run.detector_set_hash,
            "detectorId": input.detector.detector_id,
            "detectorVersion": input.detector.detector_version,
            "detectorRevision": input.detector.revision,
            "expectedFindingRevision": input.finding.revision,
            "candidateIdentityFingerprint": input.candidate_identity_fingerprint,
            "sourceFingerprint": input.source_fingerprint,
            "providerPolicyRevision": input.settings_revision,
            "provider": format!("{:?}", input.settings.provider),
            "model": input.settings.model,
        },
        "assessment": {
            "candidateId": input.output.candidate_id,
            "tier": tier_to_string(&input.merged.tier),
            "category": input.merged.category,
            "suggestedAction": action_to_string(&input.merged.suggested_action),
            "confidence": input.output.confidence.map(|value| value.clamp(0.0, 1.0)),
            "reason": input.merged.reason,
            "riskNote": input.merged.risk_note,
            "trashAllowed": input.merged.trash_allowed,
            "selectedByDefault": input.merged.selected_by_default
        }
    })
}

fn candidate_set_fingerprint(candidate_ids: &[String]) -> Result<String, String> {
    let mut sorted_ids = candidate_ids.to_vec();
    sorted_ids.sort();
    sorted_ids.dedup();
    if sorted_ids.len() != candidate_ids.len() {
        return Err("Cleanup AI candidate set contains duplicate identities.".to_string());
    }
    let serialized = serde_json::to_string(&sorted_ids).map_err(|error| error.to_string())?;
    Ok(blake3::hash(serialized.as_bytes()).to_hex().to_string())
}

fn fingerprint_json(value: &serde_json::Value) -> Result<String, String> {
    let serialized = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    Ok(blake3::hash(&serialized).to_hex().to_string())
}

fn candidate_identity_fingerprint(
    finding: &AnalysisFindingDto,
    expected_revision: i64,
    source_fingerprint: &str,
) -> Result<String, String> {
    let identity = serde_json::json!({
        "findingId": finding.id,
        "findingKey": finding.finding_key,
        "analysisRunId": finding.run_id,
        "expectedRevision": expected_revision,
        "detectorId": finding.detector_id,
        "detectorVersion": finding.detector_version,
        "primarySubjectKind": finding.primary_subject_kind,
        "primarySubjectId": finding.primary_subject_id,
        "sourceFingerprint": source_fingerprint
    });
    fingerprint_json(&identity)
}

/// Reports whether the current durable Cleanup Finding has a live AI assessment.
/// Callers must use this backend predicate rather than interpreting evidence JSON.
pub(crate) fn has_current_ai_assessment(db: &Database, finding_id: &str) -> bool {
    let Ok(Some(finding)) = db.get_analysis_finding(finding_id) else {
        return false;
    };
    if finding.id != finding_id
        || finding.status != "active"
        || !crate::analysis::finding_identity_matches(db, &finding)
    {
        return false;
    }
    let Some(assessment) = finding.evidence_summary.get("aiAssessment") else {
        return false;
    };
    if assessment["schema"] != "cleanup_ai_assessment_v1"
        || assessment["assessment"].as_object().is_none()
    {
        return false;
    }
    let coverage = &assessment["coverage"];
    let publication = &assessment["publication"];
    let Some(requested_ids) = coverage["requestedCandidateIds"].as_array() else {
        return false;
    };
    let Some(ids) = requested_ids
        .iter()
        .map(|id| id.as_str().map(str::to_string))
        .collect::<Option<Vec<_>>>()
    else {
        return false;
    };
    let Some(requested_count) = u64::try_from(ids.len()).ok() else {
        return false;
    };
    if ids.is_empty()
        || ids.iter().any(String::is_empty)
        || ids.len() != requested_ids.len()
        || coverage["requestedCount"].as_u64() != Some(requested_count)
        || ids.iter().filter(|id| *id == &finding.id).count() != 1
        || candidate_set_fingerprint(&ids).ok().as_deref()
            != coverage["candidateSetFingerprint"].as_str()
        || coverage["returnedCount"]
            .as_u64()
            .is_none_or(|count| count == 0 || count > requested_count)
        || coverage["status"] != "returned_exactly_once"
        || coverage["candidateId"] != finding.id
        || coverage["returnedCandidateId"] != finding.id
        || assessment["assessment"]["candidateId"] != finding.id
        || coverage["analysisRunId"] != finding.run_id
        || coverage["detectorId"] != finding.detector_id
        || coverage["detectorVersion"].as_i64() != Some(finding.detector_version)
        || publication["outcome"] != "published"
        || publication["compareAndSwap"] != "succeeded"
        || publication["findingId"] != finding.id
        || publication["analysisRunId"] != finding.run_id
        || publication["detectorId"] != finding.detector_id
        || publication["detectorVersion"].as_i64() != Some(finding.detector_version)
        || publication["publishedRevision"].as_i64() != Some(finding.revision)
    {
        return false;
    }
    let Some(expected_revision) = coverage["expectedFindingRevision"].as_i64() else {
        return false;
    };
    if expected_revision.checked_add(1) != Some(finding.revision)
        || publication["expectedRevision"].as_i64() != Some(expected_revision)
    {
        return false;
    }
    let Ok(source_fingerprint) = fingerprint_json(&finding.identity_snapshot) else {
        return false;
    };
    let Ok(candidate_fingerprint) =
        candidate_identity_fingerprint(&finding, expected_revision, &source_fingerprint)
    else {
        return false;
    };
    if coverage["sourceFingerprint"].as_str() != Some(source_fingerprint.as_str())
        || coverage["candidateIdentityFingerprint"].as_str() != Some(candidate_fingerprint.as_str())
    {
        return false;
    }
    let Ok(run) = db.get_analysis_run(&finding.run_id) else {
        return false;
    };
    let Some(expected_run_revision) = coverage["analysisRunRevision"].as_i64() else {
        return false;
    };
    if run.id != finding.run_id
        || run.scope.get("kind").and_then(serde_json::Value::as_str)
            != Some("approved_cleanup_paths")
        || !matches!(run.status.as_str(), "completed" | "completed_with_warnings")
        // Other Finding batches may advance this aggregate revision. Bind
        // currentness to this Finding's exact revision, source/detector/policy
        // facts, and a run which has not rewound before its publication.
        || expected_run_revision.checked_add(1).is_none_or(|published| run.revision < published)
        || run.source_snapshot_hash != coverage["analysisSourceSnapshotHash"]
        || run.detector_set_hash != coverage["detectorSetHash"]
    {
        return false;
    }
    let Ok(detectors) = db.list_analysis_run_detectors(&finding.run_id) else {
        return false;
    };
    let Some(detector) = detectors.iter().find(|detector| {
        detector.detector_id == finding.detector_id
            && detector.detector_version == finding.detector_version
    }) else {
        return false;
    };
    if !matches!(
        detector.status.as_str(),
        "completed" | "completed_with_warnings"
    ) || coverage["detectorRevision"].as_i64() != Some(detector.revision)
    {
        return false;
    }
    let Ok((_, settings_revision)) = get_ai_settings_for_db_with_revision(db) else {
        return false;
    };
    if coverage["providerPolicyRevision"].as_str() != Some(settings_revision.as_str()) {
        return false;
    }
    let Ok(evidence) = db.list_analysis_finding_evidence(&finding.id) else {
        return false;
    };
    evidence
        .iter()
        .filter(|item| {
            item.evidence_kind == "ai_assessment"
                && item.subject_kind == "analysis_finding"
                && item.subject_id.as_deref() == Some(finding.id.as_str())
                && item.value == *assessment
        })
        .count()
        == 1
}
