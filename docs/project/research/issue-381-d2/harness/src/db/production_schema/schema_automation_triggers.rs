//! Schema 37 only: preserve Intent/Run history while expanding trigger contracts.
use super::DbError;
use rusqlite::Connection;

pub(super) fn migrate_automation_triggers(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(r#"
        CREATE TABLE automation_intents_37 (
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
        INSERT INTO automation_intents_37
            SELECT id,revision,title,workflow_kind,scope_query_json,scope_fingerprint,
                '{"version":2,"kind":"manual"}',policy_json,enabled,created_at,updated_at,archived_at
            FROM automation_intents;
        CREATE TABLE automation_runs_37 (
            id TEXT PRIMARY KEY, request_key TEXT NOT NULL UNIQUE,
            intent_id TEXT NOT NULL REFERENCES automation_intents_37(id) ON DELETE RESTRICT,
            intent_revision INTEGER NOT NULL CHECK(intent_revision>=1),
            trigger_kind TEXT NOT NULL CHECK(trigger_kind IN ('manual','schedule','managed_scope_change')),
            scope_fingerprint TEXT NOT NULL, library_snapshot_revision INTEGER,
            status TEXT NOT NULL CHECK(status IN ('completed','blocked','failed')),
            result_plan_id TEXT, queued_analysis_count INTEGER NOT NULL DEFAULT 0 CHECK(queued_analysis_count>=0),
            requires_plan_refresh INTEGER NOT NULL CHECK(requires_plan_refresh IN (0,1)),
            analysis_blocker_code TEXT, error_code TEXT, created_at INTEGER NOT NULL, completed_at INTEGER NOT NULL,
            trigger_context_json TEXT NOT NULL DEFAULT '{"version":1,"kind":"manual"}' CHECK(json_valid(trigger_context_json))
        );
        INSERT INTO automation_runs_37 SELECT *, '{"version":1,"kind":"manual"}' FROM automation_runs;
        DROP TABLE automation_runs;
        DROP TABLE automation_intents;
        ALTER TABLE automation_intents_37 RENAME TO automation_intents;
        ALTER TABLE automation_runs_37 RENAME TO automation_runs;
        CREATE INDEX idx_automation_intents_active ON automation_intents(archived_at,updated_at DESC,id);
        CREATE INDEX idx_automation_runs_recent ON automation_runs(created_at DESC,id);
        CREATE INDEX idx_automation_runs_intent ON automation_runs(intent_id,created_at DESC,id);
        ALTER TABLE scan_roots ADD COLUMN library_change_revision INTEGER NOT NULL DEFAULT 0 CHECK(library_change_revision>=0);
        ALTER TABLE files ADD COLUMN filesystem_observation_key TEXT CHECK(filesystem_observation_key IS NULL OR length(filesystem_observation_key)=64);
        CREATE TABLE automation_trigger_state (
            intent_id TEXT PRIMARY KEY REFERENCES automation_intents(id) ON DELETE RESTRICT,
            intent_revision INTEGER NOT NULL CHECK(intent_revision>=1),
            next_due_at INTEGER, pending_event_due_at_ms INTEGER,
            pending_root_revisions_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(pending_root_revisions_json)),
            consumed_root_revisions_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(consumed_root_revisions_json)),
            last_trigger_key TEXT, last_triggered_at INTEGER, last_error_code TEXT, updated_at INTEGER NOT NULL,
            claimed_cause_json TEXT CHECK(claimed_cause_json IS NULL OR json_valid(claimed_cause_json))
        );
        CREATE INDEX idx_automation_trigger_schedule_due ON automation_trigger_state(next_due_at) WHERE next_due_at IS NOT NULL;
        CREATE INDEX idx_automation_trigger_event_due ON automation_trigger_state(pending_event_due_at_ms) WHERE pending_event_due_at_ms IS NOT NULL;
    "#)?;
    Ok(())
}
