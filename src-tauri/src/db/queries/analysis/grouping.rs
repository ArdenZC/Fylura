//! Complete-run projections over the existing Finding ledger, independent of pages.
use super::*;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisFindingCategoryDto {
    pub category: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisFindingGroupDto {
    pub tier: String,
    pub count: i64,
    pub excluded_count: i64,
    pub selectable_count: i64,
    pub exact_reclaimable_bytes: i64,
    pub potential_reclaimable_bytes: i64,
    pub categories: Vec<AnalysisFindingCategoryDto>,
}

pub(super) fn summarize(
    conn: &Connection,
    run: &str,
) -> Result<Vec<AnalysisFindingGroupDto>, DbError> {
    ["safe", "review", "caution"].into_iter().map(|tier| {
        let (count, excluded_count, selectable_count) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(CASE WHEN d.decision IN ('dismissed','snoozed') AND (d.decision != 'snoozed' OR d.snoozed_until > unixepoch()) THEN 1 ELSE 0 END),0),
             COALESCE(SUM(CASE WHEN f.executable=1 AND f.action_kind='safe_trash_candidate' AND
             (COALESCE(d.decision,'open') NOT IN ('dismissed','snoozed') OR (d.decision='snoozed' AND d.snoozed_until<=unixepoch())) AND
             (f.tier='safe' OR (f.tier='review' AND d.decision='acknowledged')) THEN 1 ELSE 0 END),0)
             FROM analysis_findings f LEFT JOIN analysis_finding_decisions d ON d.finding_key=f.finding_key
             WHERE f.run_id=?1 AND f.status='active' AND f.tier=?2",
            params![run,tier], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        let mut statement = conn.prepare("SELECT category,COUNT(*) FROM analysis_findings WHERE run_id=?1 AND status='active' AND tier=?2 AND NOT EXISTS (SELECT 1 FROM analysis_finding_decisions d WHERE d.finding_key=analysis_findings.finding_key AND (d.decision='dismissed' OR (d.decision='snoozed' AND d.snoozed_until>unixepoch()))) GROUP BY category ORDER BY COUNT(*) DESC,category")?;
        let categories = statement.query_map(params![run,tier], |r| Ok(AnalysisFindingCategoryDto {category:r.get(0)?,count:r.get(1)?}))?.collect::<Result<Vec<_>,_>>()?;
        let (exact, potential) = analysis_reclaimable_totals_for_tier(conn, run, Some(tier))?;
        Ok(AnalysisFindingGroupDto { tier:tier.into(),count,excluded_count,selectable_count,exact_reclaimable_bytes:exact,potential_reclaimable_bytes:potential,categories })
    }).collect()
}
