-- index: idx_ai_job_items_status
CREATE INDEX idx_ai_job_items_status
            ON ai_job_items(status, updated_at);

-- index: idx_ai_jobs_entry_fingerprint
CREATE INDEX idx_ai_jobs_entry_fingerprint
            ON ai_jobs(global_entry_id, input_fingerprint);

-- index: idx_ai_jobs_status_created
CREATE INDEX idx_ai_jobs_status_created
            ON ai_jobs(status, created_at);

-- index: idx_analysis_finding_decisions_updated
CREATE INDEX idx_analysis_finding_decisions_updated
            ON analysis_finding_decisions(updated_at DESC, finding_key);

-- index: idx_analysis_finding_evidence_finding
CREATE INDEX idx_analysis_finding_evidence_finding
            ON analysis_finding_evidence(finding_id, created_at, id);

-- index: idx_analysis_finding_evidence_subject
CREATE INDEX idx_analysis_finding_evidence_subject
            ON analysis_finding_evidence(subject_kind, subject_id, finding_id);

-- index: idx_analysis_findings_active_page
CREATE INDEX idx_analysis_findings_active_page
            ON analysis_findings(status, tier, potential_reclaimable_bytes DESC, updated_at DESC, id);

-- index: idx_analysis_findings_key
CREATE INDEX idx_analysis_findings_key
            ON analysis_findings(finding_key, status, updated_at DESC);

-- index: idx_analysis_findings_run_detector
CREATE INDEX idx_analysis_findings_run_detector
            ON analysis_findings(run_id, detector_id, status);

-- index: idx_analysis_findings_subject
CREATE INDEX idx_analysis_findings_subject
            ON analysis_findings(primary_subject_kind, primary_subject_id, status);

-- index: idx_analysis_run_detectors_status
CREATE INDEX idx_analysis_run_detectors_status
            ON analysis_run_detectors(run_id, status, detector_id);

-- index: idx_analysis_runs_created
CREATE INDEX idx_analysis_runs_created
            ON analysis_runs(created_at DESC, id);

-- index: idx_analysis_runs_one_active_scope
CREATE UNIQUE INDEX idx_analysis_runs_one_active_scope
            ON analysis_runs(scope_hash, detector_set_hash)
            WHERE status IN ('queued', 'running', 'cancelling');

-- index: idx_automation_intents_active
CREATE INDEX idx_automation_intents_active ON automation_intents(archived_at,updated_at DESC,id);

-- index: idx_automation_runs_intent
CREATE INDEX idx_automation_runs_intent ON automation_runs(intent_id,created_at DESC,id);

-- index: idx_automation_runs_recent
CREATE INDEX idx_automation_runs_recent ON automation_runs(created_at DESC,id);

-- index: idx_automation_trigger_event_due
CREATE INDEX idx_automation_trigger_event_due ON automation_trigger_state(pending_event_due_at_ms) WHERE pending_event_due_at_ms IS NOT NULL;

-- index: idx_automation_trigger_schedule_due
CREATE INDEX idx_automation_trigger_schedule_due ON automation_trigger_state(next_due_at) WHERE next_due_at IS NOT NULL;

-- index: idx_classification_feedback_file_id
CREATE INDEX idx_classification_feedback_file_id
            ON classification_feedback(file_id);

-- index: idx_classification_history_confirmed
CREATE INDEX idx_classification_history_confirmed
            ON classification_history(user_confirmed);

-- index: idx_classification_history_file_id
CREATE INDEX idx_classification_history_file_id
            ON classification_history(file_id);

-- index: idx_classification_history_name
CREATE INDEX idx_classification_history_name
            ON classification_history(file_name);

-- index: idx_classification_history_source
CREATE INDEX idx_classification_history_source
            ON classification_history(source);

-- index: idx_cleanup_trash_batches_created_at
CREATE INDEX idx_cleanup_trash_batches_created_at
            ON cleanup_trash_batches(created_at DESC);

-- index: idx_cleanup_trash_items_batch_id
CREATE INDEX idx_cleanup_trash_items_batch_id
            ON cleanup_trash_items(batch_id);

-- index: idx_content_artifacts_root_status
CREATE INDEX idx_content_artifacts_root_status
            ON content_artifacts(scan_root_id, status, updated_at DESC);

-- index: idx_content_artifacts_status_updated
CREATE INDEX idx_content_artifacts_status_updated
            ON content_artifacts(status, updated_at DESC, file_id);

-- index: idx_content_run_items_file
CREATE INDEX idx_content_run_items_file
            ON content_run_items(file_id, updated_at DESC);

-- index: idx_content_run_items_run_status
CREATE INDEX idx_content_run_items_run_status
            ON content_run_items(run_id, status, ordinal, id);

-- index: idx_content_runs_status_updated
CREATE INDEX idx_content_runs_status_updated
            ON content_runs(status, updated_at DESC, id);

-- index: idx_content_scope_policies_enabled
CREATE INDEX idx_content_scope_policies_enabled
            ON content_scope_policies(enabled, updated_at DESC, scan_root_id);

-- index: idx_dedupe_run_errors_run
CREATE INDEX idx_dedupe_run_errors_run
            ON dedupe_run_errors(run_id, created_at, id);

-- index: idx_dedupe_runs_created
CREATE INDEX idx_dedupe_runs_created
            ON dedupe_runs(created_at DESC, id);

-- index: idx_dedupe_runs_one_active_scope
CREATE UNIQUE INDEX idx_dedupe_runs_one_active_scope
            ON dedupe_runs(scope_hash)
            WHERE status IN ('queued', 'running', 'cancelling');

-- index: idx_dedupe_runs_parent_scan
CREATE INDEX idx_dedupe_runs_parent_scan
            ON dedupe_runs(parent_scan_session_id, created_at DESC);

-- index: idx_duplicate_group_members_file
CREATE INDEX idx_duplicate_group_members_file
            ON duplicate_group_members(file_id, group_id);

-- index: idx_duplicate_group_members_physical
CREATE INDEX idx_duplicate_group_members_physical
            ON duplicate_group_members(group_id, physical_key);

-- index: idx_duplicate_groups_active_reclaimable
CREATE INDEX idx_duplicate_groups_active_reclaimable
            ON duplicate_groups(status, potential_reclaimable_bytes DESC, size_each DESC, id);

-- index: idx_file_fingerprints_full_hash
CREATE INDEX idx_file_fingerprints_full_hash
            ON file_fingerprints(size, full_hash) WHERE full_hash IS NOT NULL;

-- index: idx_file_fingerprints_physical
CREATE INDEX idx_file_fingerprints_physical
            ON file_fingerprints(physical_key) WHERE physical_key IS NOT NULL;

-- index: idx_file_fingerprints_prehash
CREATE INDEX idx_file_fingerprints_prehash
            ON file_fingerprints(size, prehash) WHERE prehash IS NOT NULL;

-- index: idx_file_fingerprints_validity
CREATE INDEX idx_file_fingerprints_validity
            ON file_fingerprints(size, modified_ns, fingerprint_status);

-- index: idx_file_user_tags_tag_file
CREATE INDEX idx_file_user_tags_tag_file
            ON file_user_tags(tag_id, file_id);

-- index: idx_files_action_mtime
CREATE INDEX idx_files_action_mtime
            ON files(is_stale, suggested_action, mtime DESC);

-- index: idx_files_active_mtime
CREATE INDEX idx_files_active_mtime
            ON files(is_stale, mtime DESC);

-- index: idx_files_classification_fingerprint
CREATE INDEX idx_files_classification_fingerprint ON files(last_classified_mtime, last_classified_size);

-- index: idx_files_classified_version
CREATE INDEX idx_files_classified_version ON files(classified_rule_version);

-- index: idx_files_dedupe
CREATE INDEX idx_files_dedupe
            ON files(size, content_hash)
            WHERE is_dir = 0 AND size > 0;

-- index: idx_files_extension
CREATE INDEX idx_files_extension ON files(extension);

-- index: idx_files_file_type
CREATE INDEX idx_files_file_type ON files(file_type);

-- index: idx_files_is_stale
CREATE INDEX idx_files_is_stale ON files(is_stale);

-- index: idx_files_last_classified_at
CREATE INDEX idx_files_last_classified_at ON files(last_classified_at DESC);

-- index: idx_files_last_seen_at
CREATE INDEX idx_files_last_seen_at ON files(last_seen_at DESC);

-- index: idx_files_lifecycle
CREATE INDEX idx_files_lifecycle ON files(lifecycle);

-- index: idx_files_lifecycle_mtime
CREATE INDEX idx_files_lifecycle_mtime
            ON files(is_stale, lifecycle, mtime DESC);

-- index: idx_files_mtime
CREATE INDEX idx_files_mtime ON files(mtime DESC);

-- index: idx_files_name
CREATE INDEX idx_files_name ON files(name);

-- index: idx_files_path
CREATE INDEX idx_files_path ON files(path);

-- index: idx_files_purpose
CREATE INDEX idx_files_purpose ON files(purpose);

-- index: idx_files_requires_confirmation
CREATE INDEX idx_files_requires_confirmation ON files(requires_confirmation);

-- index: idx_files_review_mtime
CREATE INDEX idx_files_review_mtime
            ON files(is_stale, requires_confirmation, suggested_action, mtime DESC);

-- index: idx_files_risk_level
CREATE INDEX idx_files_risk_level ON files(risk_level);

-- index: idx_files_risk_mtime
CREATE INDEX idx_files_risk_mtime
            ON files(is_stale, risk_level, mtime DESC);

-- index: idx_files_scope_path
CREATE INDEX idx_files_scope_path
            ON files(is_stale, path);

-- index: idx_global_entries_active_extension_order
CREATE INDEX idx_global_entries_active_extension_order
            ON global_entries(extension, modified_at_fs DESC, id ASC)
            WHERE is_stale = 0;

-- index: idx_global_entries_active_name_order
CREATE INDEX idx_global_entries_active_name_order
            ON global_entries(name_normalized, modified_at_fs DESC, id ASC)
            WHERE is_stale = 0;

-- index: idx_global_entries_modified_at
CREATE INDEX idx_global_entries_modified_at
            ON global_entries(modified_at_fs DESC);

-- index: idx_global_entries_name_normalized
CREATE INDEX idx_global_entries_name_normalized
            ON global_entries(name_normalized);

-- index: idx_global_entries_path_normalized
CREATE INDEX idx_global_entries_path_normalized
            ON global_entries(path_normalized);

-- index: idx_global_entries_platform_identity
CREATE INDEX idx_global_entries_platform_identity
            ON global_entries(volume_id, platform_file_id, parent_platform_file_id);

-- index: idx_global_entries_volume
CREATE INDEX idx_global_entries_volume
            ON global_entries(volume_id, is_stale);

-- index: idx_global_volumes_enabled
CREATE INDEX idx_global_volumes_enabled
            ON global_volumes(enabled, index_status);

-- index: idx_global_volumes_mount_path
CREATE INDEX idx_global_volumes_mount_path
            ON global_volumes(mount_path);

-- index: idx_library_files_confidence
CREATE INDEX idx_library_files_confidence
            ON files(is_stale, confidence DESC, id);

-- index: idx_library_files_created
CREATE INDEX idx_library_files_created
            ON files(is_stale, ctime DESC, id);

-- index: idx_library_files_modified
CREATE INDEX idx_library_files_modified
            ON files(is_stale, mtime DESC, id);

-- index: idx_library_files_name
CREATE INDEX idx_library_files_name
            ON files(is_stale, name COLLATE NOCASE, id);

-- index: idx_library_files_size
CREATE INDEX idx_library_files_size
            ON files(is_stale, size DESC, id);

-- index: idx_library_saved_views_position
CREATE INDEX idx_library_saved_views_position
            ON library_saved_views(position, updated_at DESC, id);

-- index: idx_managed_entries_global_entry
CREATE INDEX idx_managed_entries_global_entry
            ON managed_entries(global_entry_id, enabled);

-- index: idx_managed_entries_scope
CREATE INDEX idx_managed_entries_scope
            ON managed_entries(managed_scope_id, enabled);

-- index: idx_managed_scopes_enabled
CREATE INDEX idx_managed_scopes_enabled
            ON managed_scopes(enabled, updated_at DESC);

-- index: idx_operation_logs_batch_id
CREATE INDEX idx_operation_logs_batch_id ON operation_logs(batch_id);

-- index: idx_operation_logs_created_at
CREATE INDEX idx_operation_logs_created_at ON operation_logs(created_at DESC);

-- index: idx_operation_logs_restore_phase
CREATE INDEX idx_operation_logs_restore_phase ON operation_logs(restore_phase);

-- index: idx_operation_logs_restore_status
CREATE INDEX idx_operation_logs_restore_status ON operation_logs(restore_status);

-- index: idx_organization_plan_items_execution
CREATE INDEX idx_organization_plan_items_execution
            ON organization_plan_items(execution_id, validity, id);

-- index: idx_organization_plan_items_file
CREATE INDEX idx_organization_plan_items_file
            ON organization_plan_items(file_id_snapshot, plan_id);

-- index: idx_organization_plan_items_plan_state
CREATE INDEX idx_organization_plan_items_plan_state
            ON organization_plan_items(plan_id, validity, decision, ordinal, id);

-- index: idx_organization_plans_status_updated
CREATE INDEX idx_organization_plans_status_updated
            ON organization_plans(status, updated_at DESC, id);

-- index: idx_rule_proposals_status_updated
CREATE INDEX idx_rule_proposals_status_updated
            ON rule_proposals(status, updated_at DESC, id);

-- index: idx_rule_proposals_target
CREATE INDEX idx_rule_proposals_target
            ON rule_proposals(target_rule_id, status, updated_at DESC);

-- index: idx_rules_enabled
CREATE INDEX idx_rules_enabled ON rules(enabled);

-- index: idx_rules_priority
CREATE INDEX idx_rules_priority ON rules(priority DESC);

-- index: idx_rules_source
CREATE INDEX idx_rules_source ON rules(source);

-- index: idx_scan_roots_active_lease
CREATE INDEX idx_scan_roots_active_lease
            ON scan_roots(active_run_id, active_generation);

-- index: idx_scan_roots_enabled_health
CREATE INDEX idx_scan_roots_enabled_health
            ON scan_roots(enabled, health_status, updated_at DESC);

-- index: idx_scan_roots_reconciliation_enabled
CREATE INDEX idx_scan_roots_reconciliation_enabled
            ON scan_roots(enabled, needs_reconciliation, updated_at);

-- index: idx_scan_run_errors_run_created
CREATE INDEX idx_scan_run_errors_run_created
            ON scan_run_errors(run_id, created_at);

-- index: idx_scan_runs_one_active_per_root
CREATE UNIQUE INDEX idx_scan_runs_one_active_per_root
            ON scan_runs(scan_root_id)
            WHERE status IN ('queued', 'running', 'cancelling');

-- index: idx_scan_runs_root_created
CREATE INDEX idx_scan_runs_root_created
            ON scan_runs(scan_root_id, created_at DESC);

-- index: idx_scan_runs_session_status
CREATE INDEX idx_scan_runs_session_status
            ON scan_runs(parent_session_id, status, created_at DESC);

-- index: idx_scan_seen_run_path
CREATE INDEX idx_scan_seen_run_path
            ON scan_seen(run_id, observed_path);

-- index: idx_scan_session_roots_effective
CREATE INDEX idx_scan_session_roots_effective
            ON scan_session_roots(session_id, effective_index, effective_root_id);

-- index: idx_scan_session_roots_run
CREATE INDEX idx_scan_session_roots_run
            ON scan_session_roots(run_id, status);

-- index: idx_scan_sessions_status_created
CREATE INDEX idx_scan_sessions_status_created
            ON scan_sessions(status, created_at DESC);

-- index: idx_user_tags_name
CREATE INDEX idx_user_tags_name
            ON user_tags(normalized_name COLLATE NOCASE, id);

-- table: ai_analysis_state
CREATE TABLE ai_analysis_state (
            global_entry_id TEXT PRIMARY KEY REFERENCES global_entries(id) ON DELETE CASCADE,
            status TEXT NOT NULL DEFAULT 'pending',
            input_fingerprint TEXT NOT NULL DEFAULT '',
            provider TEXT NOT NULL DEFAULT 'local',
            model TEXT NOT NULL DEFAULT '',
            content_summary TEXT,
            classification_json TEXT,
            user_corrected INTEGER NOT NULL DEFAULT 0 CHECK (user_corrected IN (0, 1)),
            last_error TEXT,
            updated_at INTEGER NOT NULL
        );

-- table: ai_job_items
CREATE TABLE ai_job_items (
            id TEXT PRIMARY KEY,
            job_id TEXT NOT NULL REFERENCES ai_jobs(id) ON DELETE CASCADE,
            global_entry_id TEXT NOT NULL REFERENCES global_entries(id) ON DELETE CASCADE,
            status TEXT NOT NULL DEFAULT 'pending'
                CHECK (status IN ('pending', 'running', 'completed', 'failed', 'canceled', 'stale', 'blocked_by_policy')),
            last_error TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            UNIQUE(job_id, global_entry_id)
        );

-- table: ai_jobs
CREATE TABLE ai_jobs (
            id TEXT PRIMARY KEY,
            global_entry_id TEXT NOT NULL REFERENCES global_entries(id) ON DELETE CASCADE,
            managed_scope_id TEXT NOT NULL REFERENCES managed_scopes(id) ON DELETE CASCADE,
            input_fingerprint TEXT NOT NULL,
            provider TEXT NOT NULL,
            model TEXT NOT NULL DEFAULT '',
            processing_mode TEXT NOT NULL DEFAULT 'metadata',
            status TEXT NOT NULL DEFAULT 'pending'
                CHECK (status IN ('pending', 'running', 'completed', 'failed', 'canceled', 'stale', 'blocked_by_policy')),
            attempt_count INTEGER NOT NULL DEFAULT 0,
            last_error TEXT,
            created_at INTEGER NOT NULL,
            started_at INTEGER,
            completed_at INTEGER
        );

-- table: analysis_finding_decisions
CREATE TABLE analysis_finding_decisions (
            finding_key TEXT PRIMARY KEY,
            decision TEXT NOT NULL CHECK (decision IN ('open', 'acknowledged', 'dismissed', 'snoozed')),
            snoozed_until INTEGER,
            note TEXT,
            revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            CHECK (decision <> 'snoozed' OR snoozed_until IS NOT NULL)
        );

-- table: analysis_finding_evidence
CREATE TABLE analysis_finding_evidence (
            id TEXT PRIMARY KEY,
            finding_id TEXT NOT NULL REFERENCES analysis_findings(id) ON DELETE CASCADE,
            evidence_kind TEXT NOT NULL,
            subject_kind TEXT NOT NULL,
            subject_id TEXT,
            path_snapshot TEXT,
            value_json TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );

-- table: analysis_findings
CREATE TABLE analysis_findings (
            id TEXT PRIMARY KEY,
            finding_key TEXT NOT NULL,
            run_id TEXT NOT NULL REFERENCES analysis_runs(id) ON DELETE CASCADE,
            detector_id TEXT NOT NULL,
            detector_version INTEGER NOT NULL CHECK (detector_version > 0),
            scope_hash TEXT NOT NULL,
            status TEXT NOT NULL CHECK (status IN (
                'staged', 'active', 'stale', 'superseded', 'discarded'
            )),
            tier TEXT NOT NULL CHECK (tier IN ('safe', 'review', 'caution')),
            category TEXT NOT NULL,
            action_kind TEXT NOT NULL CHECK (action_kind IN (
                'reveal', 'review_duplicate_group', 'uninstall_advice',
                'app_internal_cleanup', 'safe_trash_candidate', 'none'
            )),
            title TEXT NOT NULL,
            reason TEXT NOT NULL,
            risk_note TEXT,
            confidence TEXT NOT NULL CHECK (confidence IN ('exact', 'estimated', 'unknown')),
            size_bytes INTEGER NOT NULL DEFAULT 0,
            exact_reclaimable_bytes INTEGER,
            potential_reclaimable_bytes INTEGER NOT NULL DEFAULT 0,
            requires_confirmation INTEGER NOT NULL DEFAULT 1 CHECK (requires_confirmation IN (0, 1)),
            executable INTEGER NOT NULL DEFAULT 0 CHECK (executable IN (0, 1)),
            primary_subject_kind TEXT NOT NULL,
            primary_subject_id TEXT NOT NULL,
            path_snapshot TEXT,
            identity_snapshot_json TEXT NOT NULL DEFAULT '{}',
            evidence_summary_json TEXT NOT NULL DEFAULT '{}',
            revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            published_at INTEGER,
            stale_at INTEGER,
            UNIQUE(run_id, finding_key)
        );

-- table: analysis_run_detectors
CREATE TABLE analysis_run_detectors (
            run_id TEXT NOT NULL REFERENCES analysis_runs(id) ON DELETE CASCADE,
            detector_id TEXT NOT NULL,
            detector_version INTEGER NOT NULL CHECK (detector_version > 0),
            status TEXT NOT NULL CHECK (status IN (
                'queued', 'running', 'completed', 'completed_with_warnings',
                'skipped', 'cancelled', 'failed', 'interrupted'
            )),
            revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
            scanned_subjects INTEGER NOT NULL DEFAULT 0,
            findings_staged INTEGER NOT NULL DEFAULT 0,
            findings_published INTEGER NOT NULL DEFAULT 0,
            exact_reclaimable_bytes INTEGER NOT NULL DEFAULT 0,
            potential_reclaimable_bytes INTEGER NOT NULL DEFAULT 0,
            started_at INTEGER,
            finished_at INTEGER,
            error_code TEXT,
            error_message TEXT,
            PRIMARY KEY (run_id, detector_id)
        );

-- table: analysis_runs
CREATE TABLE analysis_runs (
            id TEXT PRIMARY KEY,
            request_key TEXT NOT NULL,
            request_attempt INTEGER NOT NULL DEFAULT 1 CHECK (request_attempt > 0),
            scope_json TEXT NOT NULL,
            scope_hash TEXT NOT NULL,
            source_snapshot_json TEXT NOT NULL,
            source_snapshot_hash TEXT NOT NULL,
            detector_set_json TEXT NOT NULL,
            detector_set_hash TEXT NOT NULL,
            status TEXT NOT NULL CHECK (status IN (
                'queued', 'running', 'cancelling',
                'completed', 'completed_with_warnings',
                'cancelled', 'failed', 'interrupted'
            )),
            phase TEXT NOT NULL CHECK (phase IN (
                'preparing', 'running_detectors', 'finalizing', 'completed'
            )),
            revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
            cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK (cancel_requested IN (0, 1)),
            rerun_required INTEGER NOT NULL DEFAULT 0 CHECK (rerun_required IN (0, 1)),
            detectors_total INTEGER NOT NULL DEFAULT 0,
            detectors_completed INTEGER NOT NULL DEFAULT 0,
            detectors_failed INTEGER NOT NULL DEFAULT 0,
            findings_staged INTEGER NOT NULL DEFAULT 0,
            findings_published INTEGER NOT NULL DEFAULT 0,
            safe_count INTEGER NOT NULL DEFAULT 0,
            review_count INTEGER NOT NULL DEFAULT 0,
            caution_count INTEGER NOT NULL DEFAULT 0,
            exact_reclaimable_bytes INTEGER NOT NULL DEFAULT 0,
            potential_reclaimable_bytes INTEGER NOT NULL DEFAULT 0,
            warning_count INTEGER NOT NULL DEFAULT 0,
            error_count INTEGER NOT NULL DEFAULT 0,
            started_at INTEGER,
            finished_at INTEGER,
            last_checkpoint_at INTEGER,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            error_code TEXT,
            error_message TEXT,
            UNIQUE(request_key, request_attempt)
        );

-- table: app_settings
CREATE TABLE app_settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            , revision INTEGER NOT NULL DEFAULT 0);

-- table: automation_intents
CREATE TABLE "automation_intents" (
            id TEXT PRIMARY KEY,
            revision INTEGER NOT NULL CHECK(revision >= 1),
            title TEXT NOT NULL CHECK(length(trim(title)) BETWEEN 1 AND 120),
            workflow_kind TEXT NOT NULL CHECK(workflow_kind = 'organize_plan'),
            scope_query_json TEXT NOT NULL CHECK(json_valid(scope_query_json)),
            scope_fingerprint TEXT NOT NULL,
            trigger_json TEXT NOT NULL CHECK(json_valid(trigger_json) AND coalesce(
                json_extract(trigger_json,'$.version')=2 AND (
                    (json_extract(trigger_json,'$.kind') IN ('manual','managed_scope_change') AND
                     json_remove(trigger_json,'$.version','$.kind')='{}') OR
                    (json_extract(trigger_json,'$.kind')='schedule' AND
                     json_type(trigger_json,'$.timeZone')='text' AND
                     json_type(trigger_json,'$.localTime')='text' AND
                     json_type(trigger_json,'$.weekdays')='array' AND
                     json_array_length(trigger_json,'$.weekdays') BETWEEN 1 AND 7 AND
                     length(json_extract(trigger_json,'$.timeZone')) BETWEEN 1 AND 128 AND
                     json_extract(trigger_json,'$.localTime') GLOB '[0-2][0-9]:[0-5][0-9]' AND
                     substr(json_extract(trigger_json,'$.localTime'),1,2) <= '23' AND
                     (json_array_length(trigger_json,'$.weekdays') <= 0 OR (json_type(trigger_json,'$.weekdays[0]')='integer' AND json_extract(trigger_json,'$.weekdays[0]') BETWEEN 1 AND 7)) AND
                     (json_array_length(trigger_json,'$.weekdays') <= 1 OR (json_type(trigger_json,'$.weekdays[1]')='integer' AND json_extract(trigger_json,'$.weekdays[1]') BETWEEN 1 AND 7 AND json_extract(trigger_json,'$.weekdays[1]') > json_extract(trigger_json,'$.weekdays[0]'))) AND
                     (json_array_length(trigger_json,'$.weekdays') <= 2 OR (json_type(trigger_json,'$.weekdays[2]')='integer' AND json_extract(trigger_json,'$.weekdays[2]') BETWEEN 1 AND 7 AND json_extract(trigger_json,'$.weekdays[2]') > json_extract(trigger_json,'$.weekdays[1]'))) AND
                     (json_array_length(trigger_json,'$.weekdays') <= 3 OR (json_type(trigger_json,'$.weekdays[3]')='integer' AND json_extract(trigger_json,'$.weekdays[3]') BETWEEN 1 AND 7 AND json_extract(trigger_json,'$.weekdays[3]') > json_extract(trigger_json,'$.weekdays[2]'))) AND
                     (json_array_length(trigger_json,'$.weekdays') <= 4 OR (json_type(trigger_json,'$.weekdays[4]')='integer' AND json_extract(trigger_json,'$.weekdays[4]') BETWEEN 1 AND 7 AND json_extract(trigger_json,'$.weekdays[4]') > json_extract(trigger_json,'$.weekdays[3]'))) AND
                     (json_array_length(trigger_json,'$.weekdays') <= 5 OR (json_type(trigger_json,'$.weekdays[5]')='integer' AND json_extract(trigger_json,'$.weekdays[5]') BETWEEN 1 AND 7 AND json_extract(trigger_json,'$.weekdays[5]') > json_extract(trigger_json,'$.weekdays[4]'))) AND
                     (json_array_length(trigger_json,'$.weekdays') <= 6 OR (json_type(trigger_json,'$.weekdays[6]')='integer' AND json_extract(trigger_json,'$.weekdays[6]') BETWEEN 1 AND 7 AND json_extract(trigger_json,'$.weekdays[6]') > json_extract(trigger_json,'$.weekdays[5]'))) AND
                     json_remove(trigger_json,'$.version','$.kind','$.timeZone','$.localTime','$.weekdays')='{}')
                ),0)),
            policy_json TEXT NOT NULL CHECK(json_valid(policy_json) AND coalesce(
                json_extract(policy_json,'$.version')=1 AND json_extract(policy_json,'$.review')='required' AND
                json_type(policy_json,'$.autoExecute')='false' AND
                json_remove(policy_json,'$.version','$.review','$.autoExecute')='{}',0)),
            enabled INTEGER NOT NULL CHECK(enabled IN (0,1)),
            created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, archived_at INTEGER
        );

-- table: automation_runs
CREATE TABLE "automation_runs" (
            id TEXT PRIMARY KEY, request_key TEXT NOT NULL UNIQUE,
            intent_id TEXT NOT NULL REFERENCES "automation_intents"(id) ON DELETE RESTRICT,
            intent_revision INTEGER NOT NULL CHECK(intent_revision>=1),
            trigger_kind TEXT NOT NULL CHECK(trigger_kind IN ('manual','schedule','managed_scope_change')),
            scope_fingerprint TEXT NOT NULL, library_snapshot_revision INTEGER,
            status TEXT NOT NULL CHECK(status IN ('completed','blocked','failed')),
            result_plan_id TEXT, queued_analysis_count INTEGER NOT NULL DEFAULT 0 CHECK(queued_analysis_count>=0),
            requires_plan_refresh INTEGER NOT NULL CHECK(requires_plan_refresh IN (0,1)),
            analysis_blocker_code TEXT, error_code TEXT, created_at INTEGER NOT NULL, completed_at INTEGER NOT NULL,
            trigger_context_json TEXT NOT NULL DEFAULT '{"version":1,"kind":"manual"}' CHECK(json_valid(trigger_context_json))
        );

-- table: automation_trigger_state
CREATE TABLE automation_trigger_state (
            intent_id TEXT PRIMARY KEY REFERENCES automation_intents(id) ON DELETE RESTRICT,
            intent_revision INTEGER NOT NULL CHECK(intent_revision>=1),
            next_due_at INTEGER, pending_event_due_at_ms INTEGER,
            pending_root_revisions_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(pending_root_revisions_json)),
            consumed_root_revisions_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(consumed_root_revisions_json)),
            last_trigger_key TEXT, last_triggered_at INTEGER, last_error_code TEXT, updated_at INTEGER NOT NULL,
            claimed_cause_json TEXT CHECK(claimed_cause_json IS NULL OR json_valid(claimed_cause_json))
        );

-- table: classification_feedback
CREATE TABLE classification_feedback (
                id TEXT PRIMARY KEY,
                file_id TEXT NOT NULL,
                file_name TEXT NOT NULL,
                original_json TEXT NOT NULL,
                corrected_json TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );

-- table: classification_history
CREATE TABLE classification_history (
                id TEXT PRIMARY KEY,
                file_id TEXT NOT NULL,
                file_name TEXT NOT NULL,
                file_path TEXT NOT NULL,
                extension TEXT NOT NULL DEFAULT '',
                source TEXT NOT NULL,
                file_type TEXT NOT NULL,
                purpose TEXT NOT NULL,
                lifecycle TEXT NOT NULL,
                context TEXT NOT NULL DEFAULT '',
                risk_level TEXT NOT NULL,
                suggested_action TEXT NOT NULL,
                suggested_target_path TEXT NOT NULL DEFAULT '',
                suggested_name TEXT NOT NULL DEFAULT '',
                confidence REAL NOT NULL DEFAULT 0.5,
                reason TEXT NOT NULL DEFAULT '',
                keywords_json TEXT NOT NULL DEFAULT '[]',
                user_confirmed INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL
            );

-- table: cleanup_trash_batches
CREATE TABLE cleanup_trash_batches (
                id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL,
                root TEXT,
                total_items INTEGER NOT NULL,
                total_size INTEGER NOT NULL,
                status TEXT NOT NULL
            );

-- table: cleanup_trash_items
CREATE TABLE cleanup_trash_items (
                id TEXT PRIMARY KEY,
                batch_id TEXT NOT NULL,
                original_path TEXT NOT NULL,
                trash_path TEXT NOT NULL,
                name TEXT NOT NULL,
                size INTEGER NOT NULL,
                moved_at TEXT NOT NULL,
                restored_at TEXT,
                status TEXT NOT NULL,
                message TEXT
            , source_modified_ns TEXT, source_platform_file_id TEXT, source_quick_hash TEXT, trash_modified_ns TEXT, trash_platform_volume_id TEXT, trash_platform_file_id TEXT, trash_quick_hash TEXT, identity_status TEXT NOT NULL DEFAULT 'legacy_unverified', source_full_hash TEXT, trash_full_hash TEXT, source_claim_path TEXT, operation_phase TEXT NOT NULL DEFAULT 'completed', claim_created_at TEXT, claim_platform_file_id TEXT, claim_full_hash TEXT, source_platform_volume_id TEXT);

-- table: content_artifact_fts
CREATE VIRTUAL TABLE content_artifact_fts USING fts5(
            artifact_id UNINDEXED,
            summary,
            keywords,
            language,
            raw_text,
            tokenize='unicode61'
        );

-- table: content_artifact_fts_config
CREATE TABLE 'content_artifact_fts_config'(k PRIMARY KEY, v) WITHOUT ROWID;

-- table: content_artifact_fts_content
CREATE TABLE 'content_artifact_fts_content'(id INTEGER PRIMARY KEY, c0, c1, c2, c3, c4);

-- table: content_artifact_fts_data
CREATE TABLE 'content_artifact_fts_data'(id INTEGER PRIMARY KEY, block BLOB);

-- table: content_artifact_fts_docsize
CREATE TABLE 'content_artifact_fts_docsize'(id INTEGER PRIMARY KEY, sz BLOB);

-- table: content_artifact_fts_idx
CREATE TABLE 'content_artifact_fts_idx'(segid, term, pgno, PRIMARY KEY(segid, term)) WITHOUT ROWID;

-- table: content_artifacts
CREATE TABLE content_artifacts (
            id TEXT PRIMARY KEY,
            file_id TEXT NOT NULL UNIQUE REFERENCES files(id) ON DELETE CASCADE,
            scan_root_id TEXT REFERENCES scan_roots(id) ON DELETE SET NULL,
            source_size INTEGER NOT NULL,
            source_mtime INTEGER NOT NULL,
            source_is_dir INTEGER NOT NULL CHECK (source_is_dir IN (0, 1)),
            source_hash TEXT NOT NULL DEFAULT '',
            extractor_family TEXT NOT NULL,
            extractor_version TEXT NOT NULL,
            policy_revision INTEGER NOT NULL,
            provider_kind TEXT,
            provider_model TEXT,
            prompt_policy_version INTEGER,
            content_fingerprint TEXT NOT NULL,
            status TEXT NOT NULL CHECK (status IN ('current','stale','unsupported','blocked','failed')),
            summary TEXT,
            keywords_json TEXT NOT NULL DEFAULT '[]',
            language TEXT,
            truncated INTEGER NOT NULL DEFAULT 0 CHECK (truncated IN (0, 1)),
            text_retained INTEGER NOT NULL DEFAULT 0 CHECK (text_retained IN (0, 1)),
            raw_text TEXT,
            provenance_json TEXT NOT NULL DEFAULT '{}',
            catalog_revision INTEGER NOT NULL DEFAULT 1 CHECK (catalog_revision >= 1),
            revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            last_run_id TEXT
        );

-- table: content_catalog
CREATE TABLE content_catalog (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
            updated_at INTEGER NOT NULL
        );

-- table: content_run_items
CREATE TABLE content_run_items (
            id TEXT PRIMARY KEY,
            run_id TEXT NOT NULL REFERENCES content_runs(id) ON DELETE CASCADE,
            file_id TEXT NOT NULL REFERENCES files(id) ON DELETE CASCADE,
            ordinal INTEGER NOT NULL,
            status TEXT NOT NULL CHECK (status IN ('pending','running','completed','blocked','failed','cancelled','stale')),
            root_id TEXT,
            source_is_dir INTEGER NOT NULL DEFAULT 0 CHECK (source_is_dir IN (0, 1)),
            extractor_family TEXT,
            extractor_version TEXT,
            artifact_id TEXT,
            provider_status TEXT NOT NULL DEFAULT 'pending'
                CHECK (provider_status IN ('pending','running','completed','blocked','failed','cancelled','stale')),
            provider_revision INTEGER NOT NULL DEFAULT 1 CHECK (provider_revision >= 1),
            provider_owner TEXT,
            provider_completed_at INTEGER,
            policy_revision INTEGER NOT NULL DEFAULT 1 CHECK (policy_revision >= 1),
            error_code TEXT,
            error_detail TEXT,
            source_size INTEGER NOT NULL,
            source_mtime INTEGER NOT NULL,
            source_hash TEXT NOT NULL DEFAULT '',
            revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            UNIQUE(run_id, file_id),
            UNIQUE(run_id, ordinal)
        );

-- table: content_runs
CREATE TABLE content_runs (
            id TEXT PRIMARY KEY,
            scope_json TEXT NOT NULL,
            scope_fingerprint TEXT NOT NULL,
            mode TEXT NOT NULL CHECK (mode IN ('local','understand','local_and_understand')),
            provider_mode TEXT NOT NULL CHECK (provider_mode IN ('none','existing_interactive_provider')),
            status TEXT NOT NULL CHECK (status IN ('building','ready','running','completed','partially_completed','cancelling','cancelled','failed','stale')),
            expected_library_revision INTEGER NOT NULL,
            policy_fingerprint TEXT NOT NULL,
            candidate_fingerprint TEXT NOT NULL DEFAULT '',
            candidate_resolver TEXT NOT NULL DEFAULT '',
            confirmation INTEGER NOT NULL DEFAULT 0 CHECK (confirmation IN (0, 1)),
            byte_budget INTEGER NOT NULL DEFAULT 0,
            char_budget INTEGER NOT NULL DEFAULT 0,
            requested_count INTEGER NOT NULL DEFAULT 0,
            materialized_count INTEGER NOT NULL DEFAULT 0,
            completed_count INTEGER NOT NULL DEFAULT 0,
            blocked_count INTEGER NOT NULL DEFAULT 0,
            skipped_count INTEGER NOT NULL DEFAULT 0,
            failed_count INTEGER NOT NULL DEFAULT 0,
            provider_revision INTEGER NOT NULL DEFAULT 1 CHECK (provider_revision >= 1),
            provider_owner TEXT,
            provider_confirmed INTEGER NOT NULL DEFAULT 0 CHECK (provider_confirmed IN (0, 1)),
            cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK (cancel_requested IN (0, 1)),
            revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
            last_error_code TEXT,
            last_error_detail TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            completed_at INTEGER
        );

-- table: content_scope_policies
CREATE TABLE content_scope_policies (
            scan_root_id TEXT PRIMARY KEY REFERENCES scan_roots(id) ON DELETE CASCADE,
            enabled INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
            extractor_families_json TEXT NOT NULL DEFAULT '["txt","md","csv","pdf_text","docx","xlsx","pptx"]',
            max_bytes INTEGER NOT NULL DEFAULT 8388608 CHECK (max_bytes BETWEEN 1024 AND 67108864),
            max_chars INTEGER NOT NULL DEFAULT 32768 CHECK (max_chars BETWEEN 256 AND 262144),
            max_pages INTEGER NOT NULL DEFAULT 100 CHECK (max_pages BETWEEN 1 AND 1000),
            max_rows INTEGER NOT NULL DEFAULT 10000 CHECK (max_rows BETWEEN 1 AND 100000),
            raw_retention_mode TEXT NOT NULL DEFAULT 'none' CHECK (raw_retention_mode IN ('none','bounded')),
            raw_retention_chars INTEGER NOT NULL DEFAULT 0 CHECK (raw_retention_chars BETWEEN 0 AND 262144),
            local_allowed INTEGER NOT NULL DEFAULT 0 CHECK (local_allowed IN (0, 1)),
            cloud_allowed INTEGER NOT NULL DEFAULT 0 CHECK (cloud_allowed IN (0, 1)),
            policy_revision INTEGER NOT NULL DEFAULT 1 CHECK (policy_revision >= 1),
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );

-- table: dedupe_authority_state
CREATE TABLE dedupe_authority_state (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
            status TEXT NOT NULL CHECK (status IN ('healthy', 'rebuild_required', 'degraded')),
            last_authoritative_run_id TEXT,
            scope_hash TEXT NOT NULL DEFAULT '',
            updated_at INTEGER NOT NULL,
            FOREIGN KEY (last_authoritative_run_id) REFERENCES dedupe_runs(id) ON DELETE SET NULL
        );

-- table: dedupe_run_errors
CREATE TABLE dedupe_run_errors (
            id TEXT PRIMARY KEY,
            run_id TEXT NOT NULL,
            file_id TEXT,
            path_snapshot TEXT NOT NULL,
            phase TEXT NOT NULL,
            error_code TEXT NOT NULL,
            error_message TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            FOREIGN KEY (run_id) REFERENCES dedupe_runs(id) ON DELETE CASCADE
        );

-- table: dedupe_runs
CREATE TABLE dedupe_runs (
            id TEXT PRIMARY KEY,
            request_key TEXT NOT NULL,
            request_attempt INTEGER NOT NULL DEFAULT 1 CHECK (request_attempt > 0),
            parent_scan_session_id TEXT,
            scope_json TEXT NOT NULL,
            scope_hash TEXT NOT NULL,
            scope_snapshot_json TEXT NOT NULL DEFAULT '{}',
            scope_snapshot_hash TEXT NOT NULL DEFAULT '',
            status TEXT NOT NULL CHECK (status IN (
                'queued', 'running', 'cancelling',
                'completed', 'completed_with_warnings',
                'cancelled', 'failed', 'interrupted'
            )),
            phase TEXT NOT NULL CHECK (phase IN (
                'collecting', 'capturing_identity', 'prehashing',
                'full_hashing', 'building_groups', 'finalizing', 'completed'
            )),
            revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
            cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK (cancel_requested IN (0, 1)),
            rerun_required INTEGER NOT NULL DEFAULT 0 CHECK (rerun_required IN (0, 1)),
            candidate_files INTEGER NOT NULL DEFAULT 0,
            candidate_physical_objects INTEGER NOT NULL DEFAULT 0,
            candidate_bytes INTEGER NOT NULL DEFAULT 0,
            identity_verified_files INTEGER NOT NULL DEFAULT 0,
            identity_unknown_files INTEGER NOT NULL DEFAULT 0,
            hardlink_aliases INTEGER NOT NULL DEFAULT 0,
            prehashed_files INTEGER NOT NULL DEFAULT 0,
            prehash_pruned_files INTEGER NOT NULL DEFAULT 0,
            full_hashed_files INTEGER NOT NULL DEFAULT 0,
            duplicate_groups INTEGER NOT NULL DEFAULT 0,
            duplicate_members INTEGER NOT NULL DEFAULT 0,
            exact_reclaimable_bytes INTEGER NOT NULL DEFAULT 0,
            potential_reclaimable_bytes INTEGER NOT NULL DEFAULT 0,
            processed_files INTEGER NOT NULL DEFAULT 0,
            processed_bytes INTEGER NOT NULL DEFAULT 0,
            total_bytes INTEGER NOT NULL DEFAULT 0,
            warning_count INTEGER NOT NULL DEFAULT 0,
            error_count INTEGER NOT NULL DEFAULT 0,
            started_at INTEGER,
            finished_at INTEGER,
            last_checkpoint_at INTEGER,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            error_code TEXT,
            error_message TEXT, publication_mode TEXT NOT NULL DEFAULT 'diagnostic'
            CHECK (publication_mode IN ('authoritative', 'diagnostic')),
            UNIQUE(request_key, request_attempt)
        );

-- table: duplicate_group_members
CREATE TABLE duplicate_group_members (
            group_id TEXT NOT NULL,
            file_id TEXT NOT NULL,
            path_snapshot TEXT NOT NULL,
            physical_key TEXT,
            identity_status TEXT NOT NULL,
            is_hardlink_alias INTEGER NOT NULL DEFAULT 0 CHECK (is_hardlink_alias IN (0, 1)),
            size INTEGER NOT NULL,
            modified_ns INTEGER,
            verified_at INTEGER NOT NULL,
            PRIMARY KEY (group_id, file_id),
            FOREIGN KEY (group_id) REFERENCES duplicate_groups(id) ON DELETE CASCADE,
            FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE
        );

-- table: duplicate_groups
CREATE TABLE duplicate_groups (
            id TEXT PRIMARY KEY,
            size_each INTEGER NOT NULL,
            full_hash TEXT NOT NULL,
            full_hash_algorithm TEXT NOT NULL DEFAULT 'blake3',
            full_hash_version INTEGER NOT NULL DEFAULT 1,
            member_count INTEGER NOT NULL,
            physical_copy_count INTEGER NOT NULL,
            hardlink_alias_count INTEGER NOT NULL DEFAULT 0,
            exact_reclaimable_bytes INTEGER,
            potential_reclaimable_bytes INTEGER NOT NULL,
            reclaimable_confidence TEXT NOT NULL CHECK (
                reclaimable_confidence IN ('exact', 'estimated', 'unknown')
            ),
            status TEXT NOT NULL CHECK (status IN ('active', 'stale', 'superseded')),
            last_built_run_id TEXT NOT NULL,
            revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            last_verified_at INTEGER NOT NULL,
            UNIQUE(size_each, full_hash, full_hash_algorithm, full_hash_version),
            FOREIGN KEY (last_built_run_id) REFERENCES dedupe_runs(id)
        );

-- table: file_fingerprints
CREATE TABLE file_fingerprints (
            file_id TEXT PRIMARY KEY,
            path_snapshot TEXT NOT NULL,
            identity_status TEXT NOT NULL CHECK (identity_status IN (
                'verified', 'path_only', 'unsupported', 'missing', 'stale', 'error'
            )),
            platform_kind TEXT NOT NULL DEFAULT '',
            platform_volume_id TEXT,
            platform_file_id TEXT,
            physical_key TEXT,
            link_count INTEGER,
            size INTEGER NOT NULL,
            modified_ns INTEGER,
            prehash TEXT,
            prehash_algorithm TEXT NOT NULL DEFAULT 'blake3-head-tail',
            prehash_version INTEGER NOT NULL DEFAULT 1,
            prehash_sample_bytes INTEGER NOT NULL DEFAULT 4096,
            full_hash TEXT,
            full_hash_algorithm TEXT NOT NULL DEFAULT 'blake3',
            full_hash_version INTEGER NOT NULL DEFAULT 1,
            fingerprint_status TEXT NOT NULL CHECK (fingerprint_status IN (
                'identity_only', 'prehash_complete', 'complete', 'stale',
                'missing', 'unsupported', 'error'
            )),
            captured_at INTEGER NOT NULL,
            prehashed_at INTEGER,
            full_hashed_at INTEGER,
            last_verified_at INTEGER NOT NULL,
            error_code TEXT,
            error_message TEXT,
            revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
            FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE
        );

-- table: file_user_tags
CREATE TABLE file_user_tags (
            file_id TEXT NOT NULL
                REFERENCES files(id)
                ON UPDATE CASCADE
                ON DELETE CASCADE,
            tag_id TEXT NOT NULL
                REFERENCES user_tags(id)
                ON DELETE CASCADE,
            created_at INTEGER NOT NULL,
            PRIMARY KEY(file_id, tag_id)
        );

-- table: files
CREATE TABLE files (
                id TEXT PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL,
                extension TEXT NOT NULL DEFAULT '',
                size INTEGER NOT NULL DEFAULT 0,
                mtime INTEGER NOT NULL DEFAULT 0,
                is_dir INTEGER NOT NULL DEFAULT 0 CHECK (is_dir IN (0, 1)),
                state_code INTEGER NOT NULL DEFAULT 0
            , file_type TEXT NOT NULL DEFAULT 'Other', purpose TEXT NOT NULL DEFAULT 'Unknown', lifecycle TEXT NOT NULL DEFAULT 'Inbox', context TEXT NOT NULL DEFAULT '', risk_level TEXT NOT NULL DEFAULT 'Normal', suggested_action TEXT NOT NULL DEFAULT 'Keep', suggested_target_path TEXT NOT NULL DEFAULT '', suggested_name TEXT NOT NULL DEFAULT '', confidence REAL NOT NULL DEFAULT 0.5, classification_reason TEXT NOT NULL DEFAULT 'Indexed by Zen Canvas Tauri backend.', matched_rules TEXT NOT NULL DEFAULT '[]', requires_confirmation INTEGER NOT NULL DEFAULT 0, ctime INTEGER NOT NULL DEFAULT 0, is_stale INTEGER NOT NULL DEFAULT 0, last_seen_at INTEGER NOT NULL DEFAULT 0, last_classified_at INTEGER NOT NULL DEFAULT 0, classified_rule_version TEXT NOT NULL DEFAULT '', last_classified_mtime INTEGER NOT NULL DEFAULT 0, last_classified_size INTEGER NOT NULL DEFAULT 0, content_hash TEXT NOT NULL DEFAULT '', classification_status TEXT NOT NULL DEFAULT 'unclassified'
                CHECK (classification_status IN ('unclassified', 'classified')), filesystem_observation_key TEXT CHECK(filesystem_observation_key IS NULL OR length(filesystem_observation_key)=64));

-- table: files_fts
CREATE VIRTUAL TABLE files_fts USING fts5(
            name,
            path,
            content='files',
            content_rowid='rowid',
            tokenize='trigram'
        );

-- table: files_fts_config
CREATE TABLE 'files_fts_config'(k PRIMARY KEY, v) WITHOUT ROWID;

-- table: files_fts_data
CREATE TABLE 'files_fts_data'(id INTEGER PRIMARY KEY, block BLOB);

-- table: files_fts_docsize
CREATE TABLE 'files_fts_docsize'(id INTEGER PRIMARY KEY, sz BLOB);

-- table: files_fts_idx
CREATE TABLE 'files_fts_idx'(segid, term, pgno, PRIMARY KEY(segid, term)) WITHOUT ROWID;

-- table: global_entries
CREATE TABLE global_entries (
            id TEXT PRIMARY KEY,
            volume_id TEXT NOT NULL REFERENCES global_volumes(id) ON DELETE CASCADE,
            platform_file_id TEXT NOT NULL,
            parent_platform_file_id TEXT NOT NULL DEFAULT '',
            name TEXT NOT NULL,
            name_normalized TEXT NOT NULL,
            path TEXT NOT NULL,
            path_normalized TEXT NOT NULL,
            extension TEXT NOT NULL DEFAULT '',
            is_directory INTEGER NOT NULL DEFAULT 0 CHECK (is_directory IN (0, 1)),
            size INTEGER NOT NULL DEFAULT 0,
            created_at_fs INTEGER,
            modified_at_fs INTEGER,
            file_attributes INTEGER NOT NULL DEFAULT 0,
            is_hidden INTEGER NOT NULL DEFAULT 0 CHECK (is_hidden IN (0, 1)),
            is_system INTEGER NOT NULL DEFAULT 0 CHECK (is_system IN (0, 1)),
            is_stale INTEGER NOT NULL DEFAULT 0 CHECK (is_stale IN (0, 1)),
            source_provider TEXT NOT NULL,
            last_seen_at INTEGER NOT NULL
        );

-- table: global_entries_fts
CREATE VIRTUAL TABLE global_entries_fts USING fts5(
            name,
            path,
            extension,
            content='global_entries',
            content_rowid='rowid',
            tokenize='trigram'
        );

-- table: global_entries_fts_config
CREATE TABLE 'global_entries_fts_config'(k PRIMARY KEY, v) WITHOUT ROWID;

-- table: global_entries_fts_data
CREATE TABLE 'global_entries_fts_data'(id INTEGER PRIMARY KEY, block BLOB);

-- table: global_entries_fts_docsize
CREATE TABLE 'global_entries_fts_docsize'(id INTEGER PRIMARY KEY, sz BLOB);

-- table: global_entries_fts_idx
CREATE TABLE 'global_entries_fts_idx'(segid, term, pgno, PRIMARY KEY(segid, term)) WITHOUT ROWID;

-- table: global_volumes
CREATE TABLE global_volumes (
            id TEXT PRIMARY KEY,
            platform TEXT NOT NULL,
            stable_volume_id TEXT NOT NULL UNIQUE,
            display_name TEXT NOT NULL,
            mount_path TEXT NOT NULL,
            filesystem_type TEXT NOT NULL,
            drive_kind TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
            provider TEXT NOT NULL,
            index_status TEXT NOT NULL DEFAULT 'discovered',
            last_error TEXT,
            journal_id TEXT,
            journal_cursor TEXT,
            last_full_index_at INTEGER,
            last_incremental_sync_at INTEGER,
            entry_count INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );

-- table: library_query_state
CREATE TABLE library_query_state (
            singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
            revision INTEGER NOT NULL CHECK (revision >= 1),
            updated_at INTEGER NOT NULL
        );

-- table: library_saved_views
CREATE TABLE library_saved_views (
            id TEXT PRIMARY KEY,
            display_name TEXT NOT NULL,
            normalized_name TEXT NOT NULL COLLATE NOCASE UNIQUE,
            query_spec_version INTEGER NOT NULL CHECK (query_spec_version = 2),
            query_spec_json TEXT NOT NULL,
            position INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        , revision INTEGER NOT NULL DEFAULT 1);

-- table: managed_entries
CREATE TABLE managed_entries (
            id TEXT PRIMARY KEY,
            global_entry_id TEXT NOT NULL REFERENCES global_entries(id) ON DELETE CASCADE,
            managed_scope_id TEXT NOT NULL REFERENCES managed_scopes(id) ON DELETE CASCADE,
            enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            UNIQUE(global_entry_id, managed_scope_id)
        );

-- table: managed_scopes
CREATE TABLE managed_scopes (
            id TEXT PRIMARY KEY,
            path TEXT NOT NULL UNIQUE,
            global_entry_id TEXT REFERENCES global_entries(id) ON DELETE SET NULL,
            enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
            allow_local_ai INTEGER NOT NULL DEFAULT 1 CHECK (allow_local_ai IN (0, 1)),
            allow_cloud_ai INTEGER NOT NULL DEFAULT 0 CHECK (allow_cloud_ai IN (0, 1)),
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );

-- table: operation_batches
CREATE TABLE operation_batches (
                id TEXT PRIMARY KEY,
                created_at INTEGER NOT NULL,
                status TEXT NOT NULL
            );

-- table: operation_logs
CREATE TABLE operation_logs (
                id TEXT PRIMARY KEY,
                batch_id TEXT NOT NULL,
                operation_type TEXT NOT NULL,
                source_path TEXT NOT NULL,
                target_path TEXT NOT NULL,
                old_name TEXT NOT NULL,
                new_name TEXT NOT NULL,
                status TEXT NOT NULL,
                error_message TEXT,
                created_at INTEGER NOT NULL,
                can_undo INTEGER NOT NULL DEFAULT 0,
                path_before TEXT NOT NULL,
                path_after TEXT NOT NULL,
                name_before TEXT NOT NULL,
                name_after TEXT NOT NULL,
                can_restore INTEGER NOT NULL DEFAULT 0,
                restored_at INTEGER,
                restore_status TEXT NOT NULL DEFAULT 'not_restored',
                restore_error TEXT
            , source_size INTEGER, source_modified_ns TEXT, source_platform_file_id TEXT, source_quick_hash TEXT, target_platform_file_id TEXT, source_full_hash TEXT, target_full_hash TEXT, source_claim_path TEXT, operation_phase TEXT NOT NULL DEFAULT 'completed', claim_created_at TEXT, claim_platform_file_id TEXT, claim_full_hash TEXT, restore_claim_path TEXT, restore_phase TEXT NOT NULL DEFAULT 'idle', restore_claim_created_at TEXT, restore_claim_platform_file_id TEXT, restore_claim_full_hash TEXT, source_platform_volume_id TEXT, target_platform_volume_id TEXT, claim_platform_volume_id TEXT, restore_claim_platform_volume_id TEXT);

-- table: organization_plan_items
CREATE TABLE organization_plan_items (
            id TEXT PRIMARY KEY,
            plan_id TEXT NOT NULL REFERENCES organization_plans(id) ON DELETE CASCADE,
            ordinal INTEGER NOT NULL,
            file_id_snapshot TEXT NOT NULL,
            source_path_snapshot TEXT NOT NULL,
            source_name_snapshot TEXT NOT NULL,
            source_size_snapshot INTEGER NOT NULL,
            source_mtime_snapshot INTEGER NOT NULL,
            source_is_dir_snapshot INTEGER NOT NULL,
            proposal_fingerprint TEXT NOT NULL,
            proposal_kind TEXT NOT NULL CHECK (proposal_kind IN (
                'move', 'rename', 'move_rename', 'keep', 'blocked'
            )),
            proposed_target_directory TEXT NOT NULL,
            proposed_name TEXT NOT NULL,
            proposed_target_path TEXT NOT NULL,
            decision TEXT NOT NULL CHECK (decision IN (
                'undecided', 'accepted', 'kept', 'edited'
            )),
            edited_name TEXT,
            validity TEXT NOT NULL CHECK (validity IN (
                'ready', 'needs_analysis', 'needs_review', 'blocked',
                'stale', 'executing', 'executed', 'failed', 'skipped'
            )),
            confidence REAL NOT NULL,
            risk_level TEXT NOT NULL,
            requires_confirmation INTEGER NOT NULL,
            blocking_code TEXT,
            blocking_detail TEXT,
            authoritative_preview_id TEXT,
            operation_log_id TEXT,
            execution_id TEXT,
            revision INTEGER NOT NULL CHECK (revision >= 1),
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            UNIQUE(plan_id, ordinal),
            UNIQUE(plan_id, file_id_snapshot)
        );

-- table: organization_plans
CREATE TABLE organization_plans (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            status TEXT NOT NULL CHECK (status IN (
                'draft', 'building', 'ready', 'stale', 'executing',
                'partially_completed', 'completed', 'cancelled', 'failed'
            )),
            source_kind TEXT NOT NULL CHECK (source_kind IN ('explicit', 'all_matching')),
            source_query_spec_json TEXT,
            source_query_fingerprint TEXT,
            source_snapshot_revision INTEGER NOT NULL,
            requested_count INTEGER NOT NULL CHECK (requested_count >= 0),
            materialized_count INTEGER NOT NULL CHECK (materialized_count >= 0),
            planner_version INTEGER NOT NULL,
            revision INTEGER NOT NULL CHECK (revision >= 1),
            active_execution_id TEXT,
            active_operation_batch_id TEXT,
            last_error_code TEXT,
            last_error_detail TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            ready_at INTEGER,
            completed_at INTEGER
        );

-- table: rule_catalog_state
CREATE TABLE rule_catalog_state (
            singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
            revision INTEGER NOT NULL CHECK (revision >= 1),
            updated_at INTEGER NOT NULL
        );

-- table: rule_proposals
CREATE TABLE rule_proposals (
            id TEXT PRIMARY KEY,
            status TEXT NOT NULL CHECK (status IN (
                'draft', 'generating', 'ready', 'needs_clarification',
                'invalid', 'stale', 'applying', 'applied',
                'cancelled', 'failed'
            )),
            intent_kind TEXT NOT NULL CHECK (intent_kind IN ('create', 'update')),
            target_rule_id TEXT,
            base_rule_revision INTEGER,
            prompt TEXT NOT NULL,
            prompt_fingerprint TEXT NOT NULL,
            provider_kind TEXT,
            provider_preset TEXT,
            model TEXT,
            ast_version INTEGER NOT NULL,
            candidate_rule_json TEXT,
            candidate_fingerprint TEXT,
            summary TEXT,
            clarification_json TEXT NOT NULL DEFAULT '[]',
            validation_json TEXT NOT NULL DEFAULT '{}',
            applied_rule_id TEXT,
            revision INTEGER NOT NULL CHECK (revision >= 1),
            last_error_code TEXT,
            last_error_detail TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            generated_at INTEGER,
            applied_at INTEGER
        );

-- table: rules
CREATE TABLE rules (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT 'user',
                enabled INTEGER NOT NULL DEFAULT 1,
                priority REAL NOT NULL DEFAULT 0,
                weight REAL NOT NULL DEFAULT 0,
                root_operator TEXT NOT NULL DEFAULT 'AND',
                groups_json TEXT NOT NULL DEFAULT '[]',
                action_json TEXT NOT NULL DEFAULT '{}',
                created_at TEXT NOT NULL DEFAULT '',
                updated_at TEXT NOT NULL DEFAULT ''
            , ast_version INTEGER NOT NULL DEFAULT 1, revision INTEGER NOT NULL DEFAULT 1, origin_proposal_id TEXT);

-- table: scan_roots
CREATE TABLE scan_roots (
            id TEXT PRIMARY KEY,
            normalized_path TEXT NOT NULL UNIQUE,
            display_name TEXT NOT NULL,
            source_kind TEXT NOT NULL DEFAULT 'file_library',
            enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
            health_status TEXT NOT NULL DEFAULT 'unknown'
                CHECK (health_status IN (
                    'unknown', 'healthy', 'scanning', 'degraded',
                    'missing', 'permission_required', 'reconciliation_required'
                )),
            current_generation INTEGER NOT NULL DEFAULT 0 CHECK (current_generation >= 0),
            active_run_id TEXT,
            active_generation INTEGER,
            revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
            last_successful_generation INTEGER,
            last_full_scan_at INTEGER,
            needs_reconciliation INTEGER NOT NULL DEFAULT 1 CHECK (needs_reconciliation IN (0, 1)),
            last_error_code TEXT,
            last_error_message TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        , watcher_revision INTEGER NOT NULL DEFAULT 0 CHECK (watcher_revision >= 0), watcher_applied_revision INTEGER NOT NULL DEFAULT 0 CHECK (watcher_applied_revision >= 0), watcher_last_event_at INTEGER, watcher_last_applied_at INTEGER, watcher_last_error_code TEXT, watcher_last_error_message TEXT, watcher_rule_recovery_required INTEGER NOT NULL DEFAULT 0
            CHECK (watcher_rule_recovery_required IN (0, 1)), library_change_revision INTEGER NOT NULL DEFAULT 0 CHECK(library_change_revision>=0));

-- table: scan_run_errors
CREATE TABLE scan_run_errors (
            id TEXT PRIMARY KEY,
            run_id TEXT NOT NULL REFERENCES scan_runs(id) ON DELETE CASCADE,
            path TEXT,
            error_code TEXT NOT NULL,
            error_message TEXT,
            affects_coverage INTEGER NOT NULL DEFAULT 1
                CHECK (affects_coverage IN (0, 1)),
            created_at INTEGER NOT NULL
        );

-- table: scan_runs
CREATE TABLE scan_runs (
            id TEXT PRIMARY KEY,
            scan_root_id TEXT NOT NULL REFERENCES scan_roots(id) ON DELETE RESTRICT,
            generation INTEGER NOT NULL CHECK (generation >= 1),
            parent_session_id TEXT REFERENCES scan_sessions(id) ON DELETE SET NULL,
            lease_token TEXT NOT NULL UNIQUE,
            status TEXT NOT NULL
                CHECK (status IN (
                    'queued', 'running', 'cancelling', 'cancelled',
                    'completed', 'completed_with_warnings', 'failed',
                    'interrupted', 'requires_reconciliation'
                )),
            phase TEXT NOT NULL
                CHECK (phase IN (
                    'preparing', 'discovering', 'persisting',
                    'reconciling_missing', 'optimizing_search',
                    'finalizing', 'completed'
                )),
            scanned_files INTEGER NOT NULL DEFAULT 0,
            scanned_directories INTEGER NOT NULL DEFAULT 0,
            processed_bytes INTEGER NOT NULL DEFAULT 0,
            warnings_count INTEGER NOT NULL DEFAULT 0,
            errors_count INTEGER NOT NULL DEFAULT 0,
            metadata_error_count INTEGER NOT NULL DEFAULT 0,
            coverage_error_count INTEGER NOT NULL DEFAULT 0,
            coverage_complete INTEGER NOT NULL DEFAULT 0 CHECK (coverage_complete IN (0, 1)),
            stale_reconciliation_allowed INTEGER NOT NULL DEFAULT 0
                CHECK (stale_reconciliation_allowed IN (0, 1)),
            cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK (cancel_requested IN (0, 1)),
            revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
            started_at INTEGER,
            finished_at INTEGER,
            last_checkpoint_at INTEGER,
            error_code TEXT,
            error_message TEXT,
            result_json TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL, watcher_revision_at_start INTEGER NOT NULL DEFAULT 0 CHECK (watcher_revision_at_start >= 0),
            UNIQUE(scan_root_id, generation)
        );

-- table: scan_seen
CREATE TABLE scan_seen (
            run_id TEXT NOT NULL REFERENCES scan_runs(id) ON DELETE CASCADE,
            file_id TEXT NOT NULL,
            observed_path TEXT NOT NULL,
            observed_at INTEGER NOT NULL,
            PRIMARY KEY(run_id, file_id)
        );

-- table: scan_session_roots
CREATE TABLE scan_session_roots (
            session_id TEXT NOT NULL REFERENCES scan_sessions(id) ON DELETE CASCADE,
            requested_index INTEGER NOT NULL CHECK (requested_index >= 0),
            requested_path TEXT NOT NULL,
            normalized_requested_path TEXT NOT NULL,
            resolution TEXT NOT NULL
                CHECK (resolution IN (
                    'effective', 'duplicate_requested',
                    'nested_under_effective', 'invalid'
                )),
            effective_root_id TEXT REFERENCES scan_roots(id) ON DELETE RESTRICT,
            effective_path TEXT,
            effective_index INTEGER,
            run_id TEXT REFERENCES scan_runs(id) ON DELETE SET NULL,
            status TEXT NOT NULL
                CHECK (status IN (
                    'pending', 'queued', 'running', 'completed',
                    'completed_with_warnings', 'failed', 'cancelled',
                    'interrupted', 'requires_reconciliation', 'covered',
                    'duplicate', 'nested', 'invalid', 'cancelled_not_started'
                )),
            reason TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY(session_id, requested_index)
        );

-- table: scan_sessions
CREATE TABLE scan_sessions (
            id TEXT PRIMARY KEY,
            request_key TEXT UNIQUE,
            canonical_request_hash TEXT,
            status TEXT NOT NULL
                CHECK (status IN (
                    'queued', 'running', 'cancelling', 'cancelled',
                    'completed', 'completed_with_warnings', 'failed',
                    'interrupted', 'requires_reconciliation'
                )),
            phase TEXT NOT NULL DEFAULT 'preparing'
                CHECK (phase IN ('preparing', 'running', 'finalizing', 'completed')),
            cancel_requested INTEGER NOT NULL DEFAULT 0 CHECK (cancel_requested IN (0, 1)),
            requested_root_count INTEGER NOT NULL DEFAULT 0,
            effective_root_count INTEGER NOT NULL DEFAULT 0,
            completed_root_count INTEGER NOT NULL DEFAULT 0,
            failed_root_count INTEGER NOT NULL DEFAULT 0,
            cancelled_root_count INTEGER NOT NULL DEFAULT 0,
            covered_root_count INTEGER NOT NULL DEFAULT 0,
            unstarted_root_count INTEGER NOT NULL DEFAULT 0,
            dedupe_requested INTEGER NOT NULL DEFAULT 0 CHECK (dedupe_requested IN (0, 1)),
            dedupe_dispatch_state TEXT NOT NULL DEFAULT 'not_requested'
                CHECK (dedupe_dispatch_state IN (
                    'not_requested', 'pending', 'dispatching',
                    'dispatched', 'unknown', 'failed', 'suppressed'
                )),
            dedupe_attempt_count INTEGER NOT NULL DEFAULT 0,
            dedupe_job_id TEXT,
            dedupe_last_error TEXT,
            scanned_files INTEGER NOT NULL DEFAULT 0,
            scanned_directories INTEGER NOT NULL DEFAULT 0,
            warnings_count INTEGER NOT NULL DEFAULT 0,
            errors_count INTEGER NOT NULL DEFAULT 0,
            revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
            started_at INTEGER,
            finished_at INTEGER,
            last_checkpoint_at INTEGER,
            error_code TEXT,
            error_message TEXT,
            result_json TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );

-- table: user_tags
CREATE TABLE user_tags (
            id TEXT PRIMARY KEY,
            display_name TEXT NOT NULL,
            normalized_name TEXT NOT NULL COLLATE NOCASE UNIQUE,
            color_token TEXT NOT NULL DEFAULT 'neutral',
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        , revision INTEGER NOT NULL DEFAULT 1);

-- trigger: ai_jobs_canceled_terminal
CREATE TRIGGER ai_jobs_canceled_terminal
        BEFORE UPDATE OF status ON ai_jobs
        WHEN old.status = 'canceled' AND new.status <> 'canceled'
        BEGIN
            SELECT RAISE(ABORT, 'canceled AI jobs are terminal');
        END;

-- trigger: cleanup_items_batch_guard_insert
CREATE TRIGGER cleanup_items_batch_guard_insert
            BEFORE INSERT ON cleanup_trash_items
            WHEN NOT EXISTS (SELECT 1 FROM cleanup_trash_batches WHERE id = NEW.batch_id)
            BEGIN SELECT RAISE(ABORT, 'cleanup item batch does not exist'); END;

-- trigger: cleanup_items_phase_guard_insert
CREATE TRIGGER cleanup_items_phase_guard_insert
        BEFORE INSERT ON cleanup_trash_items
        WHEN NEW.operation_phase NOT IN ('prepared', 'source_claimed', 'copying',
            'target_committed', 'source_cleanup_pending', 'completed',
            'rolled_back', 'manual_review')
        BEGIN SELECT RAISE(ABORT, 'invalid cleanup operation phase'); END;

-- trigger: cleanup_items_phase_guard_update
CREATE TRIGGER cleanup_items_phase_guard_update
        BEFORE UPDATE OF operation_phase ON cleanup_trash_items
        WHEN NEW.operation_phase NOT IN ('prepared', 'source_claimed', 'copying',
            'target_committed', 'source_cleanup_pending', 'completed',
            'rolled_back', 'manual_review')
        BEGIN SELECT RAISE(ABORT, 'invalid cleanup operation phase'); END;

-- trigger: cleanup_items_status_guard_insert
CREATE TRIGGER cleanup_items_status_guard_insert
        BEFORE INSERT ON cleanup_trash_items
        WHEN NEW.status NOT IN ('pending', 'moved', 'restored', 'failed', 'missing',
            'manual_review', 'canceled')
        BEGIN SELECT RAISE(ABORT, 'invalid cleanup item status'); END;

-- trigger: cleanup_items_status_guard_update
CREATE TRIGGER cleanup_items_status_guard_update
        BEFORE UPDATE OF status ON cleanup_trash_items
        WHEN NEW.status NOT IN ('pending', 'moved', 'restored', 'failed', 'missing',
            'manual_review', 'canceled')
        BEGIN SELECT RAISE(ABORT, 'invalid cleanup item status'); END;

-- trigger: content_artifacts_file_changed
CREATE TRIGGER content_artifacts_file_changed
        AFTER UPDATE OF path, size, mtime, is_dir, is_stale, content_hash ON files
        WHEN EXISTS (SELECT 1 FROM content_artifacts WHERE file_id = NEW.id)
        BEGIN
            UPDATE content_artifacts
            SET status = 'stale', revision = revision + 1, updated_at = unixepoch()
            WHERE file_id = NEW.id;
        END;

-- trigger: content_catalog_artifact_delete
CREATE TRIGGER content_catalog_artifact_delete
        AFTER DELETE ON content_artifacts
        BEGIN
            UPDATE content_catalog SET revision=revision+1, updated_at=unixepoch() WHERE id=1;
        END;

-- trigger: content_catalog_artifact_insert
CREATE TRIGGER content_catalog_artifact_insert
        AFTER INSERT ON content_artifacts
        BEGIN
            UPDATE content_catalog SET revision=revision+1, updated_at=unixepoch() WHERE id=1;
        END;

-- trigger: content_catalog_artifact_update
CREATE TRIGGER content_catalog_artifact_update
        AFTER UPDATE ON content_artifacts
        BEGIN
            UPDATE content_catalog SET revision=revision+1, updated_at=unixepoch() WHERE id=1;
        END;

-- trigger: files_ad
CREATE TRIGGER files_ad AFTER DELETE ON files BEGIN
            INSERT INTO files_fts(files_fts, rowid, name, path)
            VALUES('delete', old.rowid, old.name, old.path);
        END;

-- trigger: files_ai
CREATE TRIGGER files_ai AFTER INSERT ON files BEGIN
            INSERT INTO files_fts(rowid, name, path) VALUES (new.rowid, new.name, new.path);
        END;

-- trigger: files_au
CREATE TRIGGER files_au AFTER UPDATE ON files BEGIN
            INSERT INTO files_fts(files_fts, rowid, name, path)
            VALUES('delete', old.rowid, old.name, old.path);
            INSERT INTO files_fts(rowid, name, path) VALUES (new.rowid, new.name, new.path);
        END;

-- trigger: global_entries_ad
CREATE TRIGGER global_entries_ad AFTER DELETE ON global_entries BEGIN
            INSERT INTO global_entries_fts(global_entries_fts, rowid, name, path, extension)
            VALUES ('delete', old.rowid, old.name, old.path, old.extension);
        END;

-- trigger: global_entries_ai
CREATE TRIGGER global_entries_ai AFTER INSERT ON global_entries BEGIN
            INSERT INTO global_entries_fts(rowid, name, path, extension)
            VALUES (new.rowid, new.name, new.path, new.extension);
        END;

-- trigger: global_entries_au
CREATE TRIGGER global_entries_au
        AFTER UPDATE OF name, path, extension ON global_entries
        WHEN old.name IS NOT new.name
          OR old.path IS NOT new.path
          OR old.extension IS NOT new.extension
        BEGIN
            INSERT INTO global_entries_fts(global_entries_fts, rowid, name, path, extension)
            VALUES ('delete', old.rowid, old.name, old.path, old.extension);
            INSERT INTO global_entries_fts(rowid, name, path, extension)
            VALUES (new.rowid, new.name, new.path, new.extension);
        END;

-- trigger: global_entries_count_ad
CREATE TRIGGER global_entries_count_ad
        AFTER DELETE ON global_entries
        WHEN old.is_stale = 0
        BEGIN
            UPDATE global_volumes
            SET entry_count = MAX(0, entry_count - 1), updated_at = unixepoch()
            WHERE id = old.volume_id;
        END;

-- trigger: global_entries_count_ai
CREATE TRIGGER global_entries_count_ai
        AFTER INSERT ON global_entries
        WHEN new.is_stale = 0
        BEGIN
            UPDATE global_volumes
            SET entry_count = entry_count + 1, updated_at = new.last_seen_at
            WHERE id = new.volume_id;
        END;

-- trigger: global_entries_count_au
CREATE TRIGGER global_entries_count_au
        AFTER UPDATE OF is_stale, volume_id ON global_entries
        WHEN old.is_stale IS NOT new.is_stale OR old.volume_id IS NOT new.volume_id
        BEGIN
            UPDATE global_volumes
            SET entry_count = MAX(0, entry_count - CASE WHEN old.is_stale = 0 THEN 1 ELSE 0 END),
                updated_at = unixepoch()
            WHERE id = old.volume_id;
            UPDATE global_volumes
            SET entry_count = entry_count + CASE WHEN new.is_stale = 0 THEN 1 ELSE 0 END,
                updated_at = unixepoch()
            WHERE id = new.volume_id;
        END;

-- trigger: operation_logs_batch_guard_insert
CREATE TRIGGER operation_logs_batch_guard_insert
            BEFORE INSERT ON operation_logs
            WHEN NOT EXISTS (SELECT 1 FROM operation_batches WHERE id = NEW.batch_id)
            BEGIN SELECT RAISE(ABORT, 'operation log batch does not exist'); END;

-- trigger: operation_logs_phase_guard_insert
CREATE TRIGGER operation_logs_phase_guard_insert
        BEFORE INSERT ON operation_logs
        WHEN NEW.operation_phase NOT IN ('prepared', 'source_claimed', 'copying',
            'target_committed', 'source_cleanup_pending', 'completed',
            'rolled_back', 'manual_review')
        BEGIN SELECT RAISE(ABORT, 'invalid operation phase'); END;

-- trigger: operation_logs_phase_guard_update
CREATE TRIGGER operation_logs_phase_guard_update
        BEFORE UPDATE OF operation_phase ON operation_logs
        WHEN NEW.operation_phase NOT IN ('prepared', 'source_claimed', 'copying',
            'target_committed', 'source_cleanup_pending', 'completed',
            'rolled_back', 'manual_review')
        BEGIN SELECT RAISE(ABORT, 'invalid operation phase'); END;

-- trigger: operation_logs_restore_phase_guard_insert
CREATE TRIGGER operation_logs_restore_phase_guard_insert
        BEFORE INSERT ON operation_logs
        WHEN NEW.restore_phase NOT IN ('idle', 'prepared', 'source_claimed', 'copying',
            'target_committed', 'source_cleanup_pending', 'completed',
            'rolled_back', 'manual_review')
        BEGIN SELECT RAISE(ABORT, 'invalid restore phase'); END;

-- trigger: operation_logs_restore_phase_guard_update
CREATE TRIGGER operation_logs_restore_phase_guard_update
        BEFORE UPDATE OF restore_phase ON operation_logs
        WHEN NEW.restore_phase NOT IN ('idle', 'prepared', 'source_claimed', 'copying',
            'target_committed', 'source_cleanup_pending', 'completed',
            'rolled_back', 'manual_review')
        BEGIN SELECT RAISE(ABORT, 'invalid restore phase'); END;

-- trigger: operation_logs_status_guard_insert
CREATE TRIGGER operation_logs_status_guard_insert
        BEFORE INSERT ON operation_logs
        WHEN NEW.status NOT IN ('pending', 'success', 'failed', 'skipped', 'manual_review')
          OR NEW.restore_status NOT IN ('not_restored', 'pending', 'restored', 'failed', 'unavailable', 'canceled', 'manual_review')
        BEGIN SELECT RAISE(ABORT, 'invalid operation log status'); END;

-- trigger: operation_logs_status_guard_update
CREATE TRIGGER operation_logs_status_guard_update
        BEFORE UPDATE OF status, restore_status ON operation_logs
        WHEN NEW.status NOT IN ('pending', 'success', 'failed', 'skipped', 'manual_review')
          OR NEW.restore_status NOT IN ('not_restored', 'pending', 'restored', 'failed', 'unavailable', 'canceled', 'manual_review')
        BEGIN SELECT RAISE(ABORT, 'invalid operation log status'); END;

-- view: active_duplicate_membership
CREATE VIEW active_duplicate_membership AS
        SELECT
            member.file_id,
            group_row.id AS group_id,
            group_row.size_each AS size,
            group_row.full_hash AS content_hash
        FROM duplicate_group_members AS member
        JOIN duplicate_groups AS group_row ON group_row.id = member.group_id
        WHERE group_row.status = 'active';
