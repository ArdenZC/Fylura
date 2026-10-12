use super::DbError;
use rusqlite::Connection;

pub(super) fn ensure_automation_schema(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(r#"
        CREATE TABLE IF NOT EXISTS automation_intents (
            id TEXT PRIMARY KEY,
            revision INTEGER NOT NULL CHECK(revision >= 1),
            title TEXT NOT NULL CHECK(length(trim(title)) BETWEEN 1 AND 120),
            workflow_kind TEXT NOT NULL CHECK(workflow_kind = 'organize_plan'),
            scope_query_json TEXT NOT NULL CHECK(json_valid(scope_query_json)),
            scope_fingerprint TEXT NOT NULL,
            trigger_json TEXT NOT NULL CHECK(json_valid(trigger_json) AND
                coalesce(json_extract(trigger_json, '$.version') = 1 AND
                json_extract(trigger_json, '$.kind') = 'manual' AND
                json_remove(trigger_json, '$.version', '$.kind') = '{}', 0)),
            policy_json TEXT NOT NULL CHECK(json_valid(policy_json) AND
                coalesce(json_extract(policy_json, '$.version') = 1 AND
                json_extract(policy_json, '$.review') = 'required' AND
                json_type(policy_json, '$.autoExecute') = 'false' AND
                json_remove(policy_json, '$.version', '$.review', '$.autoExecute') = '{}', 0)),
            enabled INTEGER NOT NULL CHECK(enabled IN (0,1)),
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            archived_at INTEGER
        );
        CREATE TABLE IF NOT EXISTS automation_runs (
            id TEXT PRIMARY KEY,
            request_key TEXT NOT NULL UNIQUE,
            intent_id TEXT NOT NULL REFERENCES automation_intents(id) ON DELETE RESTRICT,
            intent_revision INTEGER NOT NULL CHECK(intent_revision >= 1),
            trigger_kind TEXT NOT NULL CHECK(trigger_kind = 'manual'),
            scope_fingerprint TEXT NOT NULL,
            library_snapshot_revision INTEGER,
            status TEXT NOT NULL CHECK(status IN ('completed','blocked','failed')),
            result_plan_id TEXT,
            queued_analysis_count INTEGER NOT NULL DEFAULT 0 CHECK(queued_analysis_count >= 0),
            requires_plan_refresh INTEGER NOT NULL CHECK(requires_plan_refresh IN (0,1)),
            analysis_blocker_code TEXT,
            error_code TEXT,
            created_at INTEGER NOT NULL,
            completed_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_automation_intents_active ON automation_intents(archived_at, updated_at DESC, id);
        CREATE INDEX IF NOT EXISTS idx_automation_runs_recent ON automation_runs(created_at DESC, id);
        CREATE INDEX IF NOT EXISTS idx_automation_runs_intent ON automation_runs(intent_id, created_at DESC, id);
    "#)?;
    Ok(())
}
