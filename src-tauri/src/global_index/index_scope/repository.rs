use super::*;
use crate::{
    db::{Database, DbError},
    global_index::GlobalEntryInput,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

const KEY: &str = "global_index_coverage_v1";
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageSourceProgress {
    pub source_id: String,
    pub pending: bool,
    /// Counts are admitted candidate observations in this rebuild, not distinct file totals.
    pub included: i64,
    pub excluded: i64,
    pub undetermined: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageSnapshot {
    pub policy: CoveragePolicy,
    pub sources: Vec<CoverageSourceProgress>,
}
pub(crate) fn load_policy(conn: &Connection) -> Result<CoveragePolicy, DbError> {
    let raw: Option<String> = conn
        .query_row("SELECT value FROM app_settings WHERE key=?1", [KEY], |r| {
            r.get(0)
        })
        .optional()?;
    let policy: CoveragePolicy = raw
        .map(|v| serde_json::from_str(&v))
        .transpose()
        .map_err(|e| DbError::Validation(e.to_string()))?
        .unwrap_or_default();
    if policy.version != POLICY_VERSION {
        return Err(DbError::Validation(
            "coverage_policy_version_unsupported".into(),
        ));
    }
    Ok(policy)
}
impl Database {
    pub fn global_index_coverage(&self) -> Result<CoverageSnapshot, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let policy = load_policy(&tx)?;
        let sources = tx.prepare("SELECT id,coverage_pending,coverage_included,coverage_excluded,coverage_undetermined FROM global_volumes ORDER BY id")?.query_map([], |r| Ok(CoverageSourceProgress { source_id:r.get(0)?, pending:r.get(1)?, included:r.get(2)?, excluded:r.get(3)?, undetermined:r.get(4)? }))?.collect::<Result<Vec<_>,_>>()?;
        tx.commit()?;
        Ok(CoverageSnapshot { policy, sources })
    }
    pub fn save_global_index_coverage(
        &self,
        expected_revision: u64,
        mode: CoverageMode,
        rules: Vec<CoverageRule>,
    ) -> Result<(), DbError> {
        if rules.len() > 128 {
            return Err(DbError::Validation("coverage_rule_limit".into()));
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old = load_policy(&tx)?;
        if old.revision != expected_revision {
            return Err(DbError::Validation("coverage_revision_conflict".into()));
        }
        // Rules are bound by the command's native observer. Revalidate authorization
        // inside the same transaction that publishes the policy and fences all writers.
        for r in &rules {
            let allowed:bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM global_volumes WHERE id=?1 AND stable_volume_id=?2 AND enabled=1 AND (platform!='windows' OR scope_authorization_state='authorized'))",params![r.source_id,r.stable_volume_id],|row|row.get(0))?;
            if !allowed {
                return Err(DbError::Validation("coverage_source_not_authorized".into()));
            }
        }
        let policy = CoveragePolicy {
            version: POLICY_VERSION,
            revision: old
                .revision
                .checked_add(1)
                .ok_or_else(|| DbError::Validation("coverage_revision_exhausted".into()))?,
            mode,
            rules,
        };
        let raw = serde_json::to_string(&policy).map_err(|e| DbError::Validation(e.to_string()))?;
        tx.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![KEY,raw])?;
        // Constant work per source; no global_entries DELETE/UPDATE, no FK/FTS churn.
        // Pending hides old-policy results until the owned provider rebuild completes.
        tx.execute_batch(
            "UPDATE global_volumes SET coverage_pending=1,
            coverage_included=0,coverage_excluded=0,coverage_undetermined=0,
            rebuild_intent_revision=rebuild_intent_revision+1,
            operation_revision=operation_revision+1,
            operation_owner_id=NULL,operation_owner_pid=NULL,operation_owner_started_at=NULL,
            operation_owner_route=NULL,operation_kind=NULL,operation_state=NULL,
            index_status=CASE WHEN enabled=1 THEN 'rebuild_required' ELSE index_status END;",
        )?;
        tx.commit()?;
        Ok(())
    }
}
/// Admission is the authoritative row boundary for every producer (Service,
/// Direct MFT/USN and Spotlight). Retired rows retain every FK and user state.
/// Minimal ancestors reuse the same repository with an explicitly hidden state.
pub(crate) fn record_decisions(
    conn: &Connection,
    entries: &[GlobalEntryInput],
    decisions: &[Decision],
) -> Result<(), DbError> {
    for (entry, d) in entries.iter().zip(decisions) {
        let mut counts = [0i64; 3];
        counts[if matches!(d, Decision::Undetermined(_)) {
            2
        } else if d.visible() {
            0
        } else {
            1
        }] = 1;
        conn.execute("UPDATE global_volumes SET coverage_included=coverage_included+?2,coverage_excluded=coverage_excluded+?3,coverage_undetermined=coverage_undetermined+?4 WHERE id=?1",params![entry.volume_id, counts[0],counts[1],counts[2]])?;
        if !d.visible() {
            conn.execute(
                "UPDATE global_entries SET is_stale=1,scope_decision=?2 WHERE id=?1",
                params![
                    entry.entry_id(),
                    if entry.is_directory {
                        "minimal_ancestor"
                    } else {
                        d.reason()
                    }
                ],
            )?;
        } else {
            conn.execute(
                "UPDATE global_entries SET scope_decision=?2 WHERE id=?1",
                params![
                    entry.entry_id(),
                    if matches!(d, Decision::Undetermined(_)) {
                        "undetermined"
                    } else {
                        "include"
                    }
                ],
            )?;
        }
    }
    Ok(())
}
