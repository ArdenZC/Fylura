mod automation;
mod schema_automation;
mod schema_automation_triggers;
pub use automation::*;
mod classification;
mod commands;
mod connection;
mod learning;
mod queries;
mod schema;
mod schema_global_generations;
mod types;

#[cfg(test)]
pub(crate) fn drop_source_writer_schema_for_test(conn: &rusqlite::Connection) {
    schema_global_generations::drop_for_test(conn);
    conn.execute_batch(
        "DROP INDEX IF EXISTS idx_library_source_root_claims_source_state;
         DROP INDEX IF EXISTS idx_library_source_root_claims_root_path;
         DROP INDEX IF EXISTS idx_library_source_root_claims_path_state;
         DROP INDEX IF EXISTS idx_library_source_physical_verified;
         DROP INDEX IF EXISTS idx_library_source_physical_lookup;
         DROP TABLE IF EXISTS library_source_root_claims;
         DROP TABLE IF EXISTS library_root_epoch_state;",
    )
    .expect("drop schema 41 source writer objects from historical test fixture");
}

pub(crate) use automation::apply_watcher_root_transitions;
pub(crate) fn app_setting_value_fingerprint(value: Option<&str>) -> String {
    let mut hasher = blake3::Hasher::new();
    match value {
        Some(value) => {
            hasher.update(b"value:");
            hasher.update(value.as_bytes());
        }
        None => {
            hasher.update(b"missing");
        }
    }
    hasher.finalize().to_hex().to_string()
}

pub use classification::engine::{ExecuteRulesForScopeV2Request, RuleExecutionResultV2};
pub(crate) use classification::normalized_file_type;
pub(crate) use classification::{build_target_path, OrganizeRootConfig};
#[cfg(test)]
pub(crate) use classification::{rule_version_for_rules, translate_template};
pub use commands::*;
pub use connection::Database;
pub use learning::*;
pub(crate) use queries::analysis::{
    bump_dedupe_authority_tx, invalidate_analysis_findings_for_file_tx,
    invalidate_analysis_findings_for_group_tx, AnalysisAiAssessmentPublication,
    AnalysisAiFindingPrecondition, AnalysisAiPublicationBatch, AnalysisFindingFilter,
    AnalysisPublishOutcome, FindingDraft, FindingEvidenceDraft, ManagedAnalysisFile,
    ManagedAnalysisFingerprint,
};
pub use queries::analysis::{
    AnalysisDetectorDto, AnalysisFindingDecisionDto, AnalysisFindingDto,
    AnalysisFindingEvidenceDto, AnalysisFindingPageDto, AnalysisRunDto, AnalysisScopeRequest,
    DedupeAuthorityDto, StartAnalysisRunRequest,
};
pub(crate) use queries::dedupe::{
    BuiltGroup, BuiltMember, DedupeCandidate, DedupeCheckpoint, FingerprintCas, FingerprintRow,
    PublishOutcome, PREHASH_MIN_SIZE, PREHASH_SAMPLE_BYTES,
};
pub use queries::dedupe::{
    DedupeGroupDto, DedupeGroupMemberDto, DedupeGroupPageDto, DedupeRunDto, DedupeScopeRequest,
    StartDedupeRunRequest,
};
pub use queries::library::*;
pub(crate) use queries::library::{current_library_revision, resolve_scope};
pub use queries::organization::*;
pub use queries::rule_proposals::*;
pub(crate) use queries::rules_repo::bump_catalog_revision_unconditional;
pub use queries::rules_repo::*;
pub use queries::scan;
pub(crate) use queries::{
    bool_to_i64, current_unix_seconds, indexed_file_from_row, infer_file_type, normalize_path_text,
    optimize_search_index_after_bulk_upsert, parent_directory, path_lookup_candidates, push_unique,
    scoped_files_sql, trim_trailing_path_separators, unix_seconds_to_iso,
};
#[cfg(test)]
pub(crate) use queries::{build_fts_query, upsert_files_by_paths_with_optional_optimize};
pub use queries::{
    emit_search_index_optimized, run_search_index_optimize, upsert_files_by_paths_for_db,
    upsert_files_by_paths_for_db_with_warnings, WatcherUpsertWarning,
};
pub use types::*;

const CLASSIFY_BATCH_SIZE: usize = 500;
const OPTIMIZE_AFTER_UPSERT_THRESHOLD: usize = 500;
pub const SEARCH_INDEX_OPTIMIZED_EVENT: &str = "search-index-optimized";

const CLASSIFICATION_STATUS_UNCLASSIFIED: &str = "unclassified";
const CLASSIFICATION_STATUS_CLASSIFIED: &str = "classified";

#[cfg(test)]
mod tests;
