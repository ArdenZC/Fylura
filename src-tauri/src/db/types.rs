use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database pool error: {0}")]
    Pool(#[from] r2d2::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Validation(String),
}

#[derive(Debug, Clone)]
pub(crate) struct IndexedFileRow {
    pub(crate) id: String,
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) extension: String,
    pub(crate) size: i64,
    pub(crate) mtime: i64,
    pub(crate) ctime: i64,
    pub(crate) is_dir: bool,
    pub(crate) state_code: i64,
    pub(crate) file_type: String,
    pub(crate) purpose: String,
    pub(crate) lifecycle: String,
    pub(crate) context: String,
    pub(crate) risk_level: String,
    pub(crate) suggested_action: String,
    pub(crate) suggested_target_path: String,
    pub(crate) suggested_name: String,
    pub(crate) confidence: f64,
    pub(crate) classification_reason: String,
    pub(crate) classification_status: String,
    pub(crate) matched_rules: String,
    pub(crate) requires_confirmation: bool,
    pub(crate) content_hash: String,
    pub(crate) is_duplicate: bool,
    pub(crate) is_stale: bool,
    pub(crate) last_seen_at: i64,
    pub(crate) last_classified_at: i64,
    pub(crate) classified_rule_version: String,
    pub(crate) last_classified_mtime: i64,
    pub(crate) last_classified_size: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InsertFileRequest {
    pub id: String,
    pub path: String,
    pub name: String,
    pub extension: String,
    pub size: i64,
    pub mtime: i64,
    #[serde(default)]
    pub ctime: i64,
    pub is_dir: bool,
    pub state_code: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSearchResult {
    pub id: String,
    pub path: String,
    pub name: String,
    pub extension: String,
    pub size: i64,
    pub mtime: i64,
    pub is_dir: bool,
    pub state_code: i64,
    pub rank: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileRecordDto {
    pub id: String,
    pub name: String,
    pub path: String,
    pub directory: String,
    pub extension: String,
    pub size: i64,
    pub file_type: String,
    pub purpose: String,
    pub lifecycle: String,
    pub context: String,
    pub risk_level: String,
    pub hash: Option<String>,
    pub created_at: String,
    pub modified_at: String,
    pub scanned_at: String,
    pub last_seen_at: String,
    pub is_hidden: bool,
    pub is_deleted: bool,
    pub is_duplicate: bool,
    pub suggested_action: String,
    pub suggested_target_path: String,
    pub suggested_name: String,
    pub confidence: f64,
    pub classification_reason: String,
    pub classification_status: String,
    pub matched_rules: Vec<String>,
    pub requires_confirmation: bool,
    pub last_opened_at: Option<String>,
    pub open_count: i64,
    pub indexed_at: String,
    pub source_id: Option<String>,
    pub is_stale: bool,
    pub state_code: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PagedFilesResult {
    pub files: Vec<FileRecordDto>,
    pub total: i64,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct OperationPreviewDto {
    pub id: String,
    #[serde(rename = "fileId")]
    pub file_id: String,
    pub operation_type: String,
    pub source_path: String,
    pub target_path: String,
    pub old_name: String,
    pub new_name: String,
    pub status: String,
    pub risk_level: String,
    pub confidence: f64,
    pub requires_confirmation: bool,
    pub suggested_action: String,
    pub is_duplicate: bool,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_by_default: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_executable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocking_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editable_new_name: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_parent_exists: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub will_create_parent: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict_policy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub will_copy: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub will_move: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub will_download: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub materialization_requirement: Option<String>,
    /// V2.1 names are explicit and camelCase for new clients. The snake_case
    /// fields above remain during the compatibility window used by existing
    /// renderer projections.
    #[serde(
        rename = "materializationRequirement",
        skip_serializing_if = "Option::is_none"
    )]
    pub materialization_requirement_v2: Option<String>,
    #[serde(
        rename = "operationFingerprint",
        skip_serializing_if = "Option::is_none"
    )]
    pub operation_fingerprint: Option<String>,
    #[serde(
        rename = "crossVolumeCopyRequired",
        skip_serializing_if = "Option::is_none"
    )]
    pub cross_volume_copy_required: Option<bool>,
    #[serde(
        rename = "metadataDegradationPossible",
        skip_serializing_if = "Option::is_none"
    )]
    pub metadata_degradation_possible: Option<bool>,
    #[serde(
        rename = "sourceRetirementCapability",
        skip_serializing_if = "Option::is_none"
    )]
    pub source_retirement_capability: Option<String>,
    #[serde(
        rename = "sourceRetirementEligible",
        skip_serializing_if = "Option::is_none"
    )]
    pub source_retirement_eligible: Option<bool>,
    #[serde(
        rename = "sourceRetirementProbeRequired",
        skip_serializing_if = "Option::is_none"
    )]
    pub source_retirement_probe_required: Option<bool>,
    #[serde(
        rename = "providerCoordination",
        skip_serializing_if = "Option::is_none"
    )]
    pub provider_coordination: Option<bool>,
    #[serde(
        rename = "sourceIdentityFingerprint",
        skip_serializing_if = "Option::is_none"
    )]
    pub source_identity_fingerprint: Option<String>,
    #[serde(
        rename = "providerIdentityFingerprint",
        skip_serializing_if = "Option::is_none"
    )]
    pub provider_identity_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub will_replace: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub will_trash: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum MaterializationRequirement {
    #[serde(rename = "none")]
    None,
    #[serde(rename = "metadata_only")]
    MetadataOnly,
    #[serde(rename = "explicit_download_required")]
    ExplicitDownloadRequired,
    #[serde(rename = "provider_managed")]
    ProviderManaged,
    #[serde(rename = "unknown")]
    Unknown,
}

impl MaterializationRequirement {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::MetadataOnly => "metadata_only",
            Self::ExplicitDownloadRequired => "explicit_download_required",
            Self::ProviderManaged => "provider_managed",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationPreviewScopeResult {
    pub previews: Vec<OperationPreviewDto>,
    pub total: i64,
    pub limit: u32,
    pub offset: u32,
    pub truncated: bool,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsSummary {
    pub total_files: i64,
    pub total_size: i64,
    pub disk_total_size: i64,
    pub disk_free_size: i64,
    pub disk_usage_ratio: f64,
    pub duplicate_files: i64,
    pub large_files: i64,
    pub sensitive_files: i64,
    pub needs_confirmation: i64,
    pub by_type: HashMap<String, i64>,
    pub by_lifecycle: HashMap<String, i64>,
    pub last_scanned_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Rule {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub source: RuleSource,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub priority: f64,
    #[serde(default)]
    pub weight: f64,
    #[serde(default = "default_or", alias = "rootOperator")]
    pub root_operator: RuleOperator,
    #[serde(default)]
    pub groups: Vec<RuleConditionGroup>,
    #[serde(default)]
    pub action: RuleAction,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RuleConditionGroup {
    pub id: String,
    #[serde(default = "default_and")]
    pub operator: RuleOperator,
    #[serde(default)]
    pub conditions: Vec<RuleCondition>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RuleCondition {
    pub id: String,
    pub field: ConditionField,
    pub operator: ConditionOperator,
    pub value: Value,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct RuleAction {
    #[serde(default)]
    pub purpose: Option<Purpose>,
    #[serde(default)]
    pub lifecycle: Option<Lifecycle>,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default, alias = "riskLevel")]
    pub risk_level: Option<RiskLevel>,
    #[serde(default, alias = "suggestedAction")]
    pub suggested_action: Option<SuggestedAction>,
    #[serde(default, alias = "targetTemplate")]
    pub target_template: Option<String>,
    #[serde(default, alias = "renameTemplate")]
    pub rename_template: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleExecutionSummary {
    pub scanned: i64,
    pub updated: i64,
    pub skipped: i64,
    pub needs_confirmation: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_batches: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_files: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuleExecutionMode {
    #[default]
    InboxOnly,
    AllChangedOrRuleChanged,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchIndexOptimizeReport {
    pub trigger: String,
    pub duration_ms: u128,
    pub success: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LibraryScope {
    CurrentScan {
        #[serde(default)]
        roots: Vec<String>,
        #[serde(default, rename = "scanSessionId")]
        scan_session_id: Option<String>,
    },
    Roots {
        #[serde(default)]
        roots: Vec<String>,
    },
    All,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileLibraryFilter {
    #[serde(default)]
    pub library_filter: Option<LibraryFilter>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibraryFilter {
    All,
    Active,
    Archive,
    Review,
    Duplicate,
    Sensitive,
}

pub(crate) struct RuleSqlRow {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) source: String,
    pub(crate) enabled: bool,
    pub(crate) priority: f64,
    pub(crate) weight: f64,
    pub(crate) root_operator: String,
    pub(crate) groups_json: String,
    pub(crate) action_json: String,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
}

#[derive(Debug, Clone)]
pub(crate) struct BuiltinClassification {
    pub(crate) action: RuleAction,
    pub(crate) confidence: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct ClassificationUpdate {
    pub(crate) file_type: String,
    pub(crate) purpose: String,
    pub(crate) lifecycle: String,
    pub(crate) context: String,
    pub(crate) risk_level: String,
    pub(crate) suggested_action: String,
    pub(crate) suggested_target_path: String,
    pub(crate) suggested_name: String,
    pub(crate) confidence: f64,
    pub(crate) classification_reason: String,
    pub(crate) classification_status: String,
    pub(crate) matched_rules: String,
    pub(crate) requires_confirmation: bool,
}

fn default_true() -> bool {
    true
}

fn default_or() -> RuleOperator {
    RuleOperator::Or
}

fn default_and() -> RuleOperator {
    RuleOperator::And
}
macro_rules! domain_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub enum $name {
            $($variant,)+
            Invalid(String),
        }

        impl $name {
            pub fn as_str(&self) -> &str {
                match self {
                    $(Self::$variant => $value,)+
                    Self::Invalid(value) => value,
                }
            }
            pub fn is_invalid(&self) -> bool { matches!(self, Self::Invalid(_)) }

            fn from_value(value: String) -> Self {
                match value.as_str() {
                    $($value => Self::$variant,)+
                    _ => Self::Invalid(value),
                }
            }
        }

        impl std::ops::Deref for $name {
            type Target = str;
            fn deref(&self) -> &Self::Target { self.as_str() }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self { Self::from_value(value) }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self { Self::from_value(value.to_string()) }
        }

        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let value = String::deserialize(deserializer)?;
                Ok(Self::from_value(value))
            }
        }
    };
}

domain_enum!(Purpose {
    Project => "Project", Teaching => "Teaching", Study => "Study", Work => "Work",
    Personal => "Personal", Career => "Career", Finance => "Finance", Identity => "Identity",
    Media => "Media", Installer => "Installer", Temporary => "Temporary", Archive => "Archive",
    Document => "Document", DuplicateReview => "Duplicate Review", Unknown => "Unknown"
});
domain_enum!(Lifecycle {
    Inbox => "Inbox", Active => "Active", Reference => "Reference", Archive => "Archive",
    Disposable => "Disposable", Duplicate => "Duplicate", Sensitive => "Sensitive",
    TrashReview => "TrashReview", Unknown => "Unknown"
});
domain_enum!(RiskLevel {
    Normal => "Normal", Sensitive => "Sensitive", System => "System", Caution => "Caution",
    Unknown => "Unknown"
});
domain_enum!(SuggestedAction {
    Keep => "Keep", Rename => "Rename", Move => "Move", MoveAndRename => "MoveAndRename",
    Archive => "Archive", Review => "Review", DeleteCandidate => "DeleteCandidate",
    Unknown => "Unknown"
});
domain_enum!(RuleSource {
    System => "system", User => "user", Session => "session", Ai => "ai",
    Learned => "learned", Unknown => "unknown"
});
domain_enum!(RuleOperator {
    And => "AND", Or => "OR", Unknown => "UNKNOWN"
});
domain_enum!(ConditionField {
    Name => "name", Extension => "extension", FileType => "file_type", Path => "path",
    Directory => "directory", Size => "size", ModifiedAt => "modified_at",
    IsDuplicate => "is_duplicate", RiskLevel => "risk_level", Unknown => "unknown"
});
domain_enum!(ConditionOperator {
    Contains => "contains", Equals => "equals", StartsWith => "startsWith",
    EndsWith => "endsWith", Is => "is", GreaterThan => "greaterThan",
    LessThan => "lessThan", OlderThanDays => "olderThanDays",
    NewerThanDays => "newerThanDays", Unknown => "unknown"
});

#[allow(clippy::derivable_impls)]
impl Default for RuleSource {
    fn default() -> Self {
        Self::Unknown
    }
}

#[allow(clippy::derivable_impls)]
impl Default for RuleOperator {
    fn default() -> Self {
        Self::And
    }
}

#[cfg(test)]
mod domain_enum_tests {
    use super::*;

    #[test]
    fn rule_domain_enums_serialize_with_frontend_values() {
        assert_eq!(
            serde_json::to_string(&Purpose::Project).unwrap(),
            r#""Project""#
        );
        assert_eq!(
            serde_json::to_string(&Lifecycle::Reference).unwrap(),
            r#""Reference""#
        );
        assert_eq!(
            serde_json::to_string(&RiskLevel::System).unwrap(),
            r#""System""#
        );
        assert_eq!(
            serde_json::to_string(&SuggestedAction::MoveAndRename).unwrap(),
            r#""MoveAndRename""#
        );
        assert_eq!(
            serde_json::to_string(&RuleSource::Learned).unwrap(),
            r#""learned""#
        );
        assert_eq!(serde_json::to_string(&RuleOperator::Or).unwrap(), r#""OR""#);
        assert_eq!(
            serde_json::to_string(&ConditionField::ModifiedAt).unwrap(),
            r#""modified_at""#
        );
        assert_eq!(
            serde_json::to_string(&ConditionOperator::OlderThanDays).unwrap(),
            r#""olderThanDays""#
        );
    }

    #[test]
    fn invalid_domain_values_remain_typed_until_validation_or_migration() {
        let field: ConditionField = serde_json::from_str(r#""not-a-field""#).unwrap();
        let action: SuggestedAction = serde_json::from_str(r#""not-an-action""#).unwrap();

        assert!(field.is_invalid());
        assert!(action.is_invalid());
    }
}
