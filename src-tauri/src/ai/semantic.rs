//! Canonical, advisory file semantics for Managed AI and explicit one-off tasks.
//!
//! The provider response deliberately contains no durable IDs, filesystem
//! destinations, operation IDs, or permission claims. The worker adds the
//! source binding after validating the response. Managed AI persists the envelope
//! in its analysis-state row; one-off tasks bind it to an existing Plan source.

use crate::{
    db::{Lifecycle, Purpose, RiskLevel, SuggestedAction},
    file_naming::{normalize_proposed_file_name, ExtensionChangePolicy},
};
use serde::{Deserialize, Serialize};

pub(crate) const FILE_TYPES: &[&str] = &[
    "Document",
    "Image",
    "Video",
    "Audio",
    "Code",
    "ArchivePackage",
    "Installer",
    "Spreadsheet",
    "Presentation",
    "Other",
];
const MAX_SEMANTIC_JSON_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct SemanticSourceBinding {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) global_entry_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) managed_scope_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) one_off: Option<OneOffSemanticBinding>,
    pub(crate) input_fingerprint: String,
    pub(crate) provider: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct OneOffSemanticBinding {
    pub(crate) version: u32,
    pub(crate) plan_id: String,
    pub(crate) source_id: String,
}

impl SemanticSourceBinding {
    pub(crate) fn ref_id(&self) -> String {
        match &self.one_off {
            Some(source) => format!("one-off:{}", source.source_id),
            None => format!("managed:{}", self.global_entry_id),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct SemanticAssessmentV1 {
    pub(crate) version: i32,
    pub(crate) source_binding: SemanticSourceBinding,
    pub(crate) ref_id: String,
    pub(crate) file_type: String,
    pub(crate) purpose: Purpose,
    pub(crate) lifecycle: Lifecycle,
    pub(crate) context: String,
    pub(crate) risk_level: RiskLevel,
    pub(crate) suggested_action: SuggestedAction,
    pub(crate) target_template: Option<String>,
    pub(crate) suggested_name: Option<String>,
    pub(crate) confidence: f64,
    pub(crate) reason: String,
    pub(crate) keywords: Vec<String>,
    pub(crate) requires_confirmation: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ProviderAssessmentV1 {
    version: i32,
    ref_id: String,
    file_type: String,
    purpose: String,
    lifecycle: String,
    context: String,
    risk_level: String,
    suggested_action: String,
    #[serde(default)]
    target_template: Option<String>,
    #[serde(default)]
    suggested_name: Option<String>,
    confidence: f64,
    reason: String,
    #[serde(default)]
    keywords: Vec<String>,
    requires_confirmation: bool,
}

/// The exact pre-V1 response shape that was persisted before semantic
/// authority was introduced. It is accepted only as an input to migration;
/// the canonical form written back is always V1.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ProviderAssessmentV0 {
    #[serde(default)]
    version: Option<i32>,
    ref_id: String,
    file_type: String,
    purpose: String,
    lifecycle: String,
    risk_level: String,
    suggested_action: String,
    confidence: f64,
    reason: String,
}

impl SemanticAssessmentV1 {
    pub(crate) fn parse_provider_response(
        response: &str,
        source_binding: SemanticSourceBinding,
        source_name: &str,
        source_extension: &str,
        source_is_directory: bool,
    ) -> Result<Self, String> {
        if response.len() > MAX_SEMANTIC_JSON_BYTES {
            return Err("managed_ai_response_too_large".to_string());
        }
        let value: serde_json::Value = serde_json::from_str(response)
            .map_err(|error| format!("managed_ai_invalid_json: {error}"))?;
        let version = read_semantic_version(&value)?;
        let expected_ref = source_binding.ref_id();

        let (
            ref_id,
            file_type,
            purpose,
            lifecycle,
            context,
            risk_level,
            suggested_action,
            mut target_template,
            mut suggested_name,
            confidence,
            reason,
            keywords,
            mut requires_confirmation,
        ) = match version {
            Some(1) => {
                let parsed: ProviderAssessmentV1 = serde_json::from_value(value)
                    .map_err(|error| format!("managed_ai_invalid_v1: {error}"))?;
                if parsed.version != 1 {
                    return Err("managed_ai_unsupported_semantic_version".to_string());
                }
                (
                    parsed.ref_id,
                    parsed.file_type,
                    parsed.purpose,
                    parsed.lifecycle,
                    parsed.context,
                    parsed.risk_level,
                    parsed.suggested_action,
                    parsed.target_template,
                    parsed.suggested_name,
                    parsed.confidence,
                    parsed.reason,
                    parsed.keywords,
                    parsed.requires_confirmation,
                )
            }
            None | Some(0) => {
                let parsed: ProviderAssessmentV0 = serde_json::from_value(value)
                    .map_err(|error| format!("managed_ai_invalid_v0: {error}"))?;
                if parsed.version.is_some_and(|value| value != 0) {
                    return Err("managed_ai_unsupported_semantic_version".to_string());
                }
                (
                    parsed.ref_id,
                    parsed.file_type,
                    parsed.purpose,
                    parsed.lifecycle,
                    String::new(),
                    parsed.risk_level,
                    parsed.suggested_action,
                    None,
                    None,
                    parsed.confidence,
                    parsed.reason,
                    Vec::new(),
                    true,
                )
            }
            _ => return Err("managed_ai_unsupported_semantic_version".to_string()),
        };

        if ref_id != expected_ref {
            return Err("managed_ai_ref_id_mismatch".to_string());
        }
        if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
            return Err("managed_ai_confidence_out_of_range".to_string());
        }
        validate_text("reason", &reason, 512, false)?;
        validate_text("context", &context, 512, true)?;
        if keywords.len() > 16 {
            return Err("managed_ai_too_many_keywords".to_string());
        }
        for keyword in &keywords {
            validate_text("keyword", keyword, 80, false)?;
        }

        let file_type = canonical_file_type(&file_type)
            .ok_or_else(|| "managed_ai_invalid_file_type".to_string())?;
        let purpose = canonical_purpose(&purpose);
        let lifecycle = canonical_lifecycle(&lifecycle);
        let risk_level = canonical_risk(&risk_level);
        let mut suggested_action = canonical_action(&suggested_action);
        let mut force_review = purpose.is_invalid()
            || lifecycle.is_invalid()
            || risk_level.is_invalid()
            || suggested_action.is_invalid();
        if purpose.as_str() == "Unknown"
            || lifecycle.as_str() == "Unknown"
            || risk_level.as_str() == "Unknown"
        {
            force_review = true;
        }

        if let Some(template) = target_template.as_deref() {
            if !valid_target_template(template) {
                target_template = None;
                force_review = true;
            }
        }
        if let Some(name) = suggested_name.as_deref() {
            if name.chars().count() > 255
                || name.chars().any(char::is_control)
                || normalize_proposed_file_name(
                    source_name,
                    source_extension,
                    name,
                    source_is_directory,
                    ExtensionChangePolicy::Preserve,
                )
                .is_err()
            {
                suggested_name = None;
                force_review = true;
            } else if let Ok(normalized) = normalize_proposed_file_name(
                source_name,
                source_extension,
                name,
                source_is_directory,
                ExtensionChangePolicy::Preserve,
            ) {
                suggested_name = Some(normalized);
            }
        }

        if matches!(suggested_action.as_str(), "Unknown") {
            force_review = true;
        }
        if matches!(
            suggested_action.as_str(),
            "Move" | "MoveAndRename" | "Archive"
        ) && target_template.is_none()
        {
            force_review = true;
        }
        if risk_level.as_str() != "Normal" {
            force_review = true;
        }
        if force_review {
            suggested_action = SuggestedAction::Review;
            requires_confirmation = true;
        }
        if confidence < 0.8 {
            requires_confirmation = true;
        }
        if matches!(suggested_action.as_str(), "Review" | "DeleteCandidate") {
            requires_confirmation = true;
        }

        Ok(Self {
            version: 1,
            source_binding,
            ref_id,
            file_type,
            purpose,
            lifecycle,
            context,
            risk_level,
            suggested_action,
            target_template,
            suggested_name,
            confidence,
            reason: reason.trim().to_string(),
            keywords,
            requires_confirmation,
        })
    }

    pub(crate) fn decode_stored(
        raw: &str,
        expected_binding: SemanticSourceBinding,
        source_name: &str,
        source_extension: &str,
        source_is_directory: bool,
    ) -> Result<Self, String> {
        if raw.len() > MAX_SEMANTIC_JSON_BYTES {
            return Err("managed_ai_stored_semantic_too_large".to_string());
        }
        let value: serde_json::Value = serde_json::from_str(raw)
            .map_err(|error| format!("managed_ai_stored_semantic_invalid: {error}"))?;
        match read_semantic_version(&value)? {
            Some(1) => {
                let assessment: Self = serde_json::from_value(value)
                    .map_err(|error| format!("managed_ai_stored_v1_invalid: {error}"))?;
                if assessment.source_binding != expected_binding
                    || assessment.ref_id != expected_binding.ref_id()
                    || !assessment.confidence.is_finite()
                    || !(0.0..=1.0).contains(&assessment.confidence)
                    || !valid_target_template_option(assessment.target_template.as_deref())
                    || canonical_file_type(&assessment.file_type).as_deref()
                        != Some(assessment.file_type.as_str())
                    || assessment.purpose.is_invalid()
                    || assessment.lifecycle.is_invalid()
                    || assessment.risk_level.is_invalid()
                    || assessment.suggested_action.is_invalid()
                    || (assessment.purpose.as_str() == "Unknown"
                        || assessment.lifecycle.as_str() == "Unknown"
                        || assessment.risk_level.as_str() == "Unknown"
                        || assessment.suggested_action.as_str() == "Unknown")
                        && (assessment.suggested_action.as_str() != "Review"
                            || !assessment.requires_confirmation)
                {
                    return Err("managed_ai_stored_v1_binding_or_value_mismatch".to_string());
                }
                validate_text("reason", &assessment.reason, 512, false)?;
                validate_text("context", &assessment.context, 512, true)?;
                if assessment.keywords.len() > 16 {
                    return Err("managed_ai_stored_v1_keywords_invalid".to_string());
                }
                for keyword in &assessment.keywords {
                    validate_text("keyword", keyword, 80, false)?;
                }
                if let Some(name) = assessment.suggested_name.as_deref() {
                    let normalized = normalize_proposed_file_name(
                        source_name,
                        source_extension,
                        name,
                        source_is_directory,
                        ExtensionChangePolicy::Preserve,
                    )
                    .map_err(|_| "managed_ai_stored_v1_filename_invalid".to_string())?;
                    if name.chars().count() > 255
                        || name.chars().any(char::is_control)
                        || normalized != name
                    {
                        return Err("managed_ai_stored_v1_filename_invalid".to_string());
                    }
                }
                if (assessment.risk_level.as_str() != "Normal"
                    && (assessment.suggested_action.as_str() != "Review"
                        || !assessment.requires_confirmation))
                    || (assessment.confidence < 0.8 && !assessment.requires_confirmation)
                    || (matches!(
                        assessment.suggested_action.as_str(),
                        "Review" | "DeleteCandidate"
                    ) && !assessment.requires_confirmation)
                    || (matches!(
                        assessment.suggested_action.as_str(),
                        "Move" | "MoveAndRename" | "Archive"
                    ) && assessment.target_template.is_none())
                {
                    return Err("managed_ai_stored_v1_safety_invariant_failed".to_string());
                }
                Ok(assessment)
            }
            None | Some(0) => {
                let migrated = Self::parse_provider_response(
                    raw,
                    expected_binding,
                    source_name,
                    source_extension,
                    source_is_directory,
                )?;
                Ok(migrated)
            }
            _ => Err("managed_ai_stored_semantic_version_unsupported".to_string()),
        }
    }

    pub(crate) fn canonical_json(&self) -> Result<String, String> {
        serde_json::to_string(self)
            .map_err(|error| format!("managed_ai_result_encode_failed: {error}"))
    }

    pub(crate) fn fingerprint(&self) -> String {
        blake3::hash(self.canonical_json().unwrap_or_default().as_bytes())
            .to_hex()
            .to_string()
    }
}

fn validate_text(
    field: &str,
    value: &str,
    max_chars: usize,
    allow_empty: bool,
) -> Result<(), String> {
    if (!allow_empty && value.trim().is_empty())
        || value.chars().count() > max_chars
        || value.chars().any(char::is_control)
    {
        return Err(format!("managed_ai_invalid_{field}"));
    }
    Ok(())
}

fn read_semantic_version(value: &serde_json::Value) -> Result<Option<i64>, String> {
    match value.get("version") {
        None => Ok(None),
        Some(version) => version
            .as_i64()
            .map(Some)
            .ok_or_else(|| "managed_ai_unsupported_semantic_version".to_string()),
    }
}

fn normalized_enum(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn canonical_file_type(value: &str) -> Option<String> {
    FILE_TYPES
        .iter()
        .find(|candidate| normalized_enum(candidate) == normalized_enum(value))
        .map(|candidate| (*candidate).to_string())
}

fn canonical_purpose(value: &str) -> Purpose {
    let canonical = match normalized_enum(value).as_str() {
        "project" => "Project",
        "teaching" => "Teaching",
        "study" => "Study",
        "work" => "Work",
        "personal" => "Personal",
        "career" => "Career",
        "finance" => "Finance",
        "identity" => "Identity",
        "media" => "Media",
        "installer" => "Installer",
        "temporary" => "Temporary",
        "archive" => "Archive",
        "document" => "Document",
        "duplicatereview" => "Duplicate Review",
        "unknown" => "Unknown",
        _ => "Unknown",
    };
    Purpose::from(canonical)
}

fn canonical_lifecycle(value: &str) -> Lifecycle {
    let canonical = match normalized_enum(value).as_str() {
        "inbox" => "Inbox",
        "active" => "Active",
        "reference" => "Reference",
        "archive" => "Archive",
        "disposable" => "Disposable",
        "duplicate" => "Duplicate",
        "sensitive" => "Sensitive",
        "trashreview" => "TrashReview",
        "unknown" => "Unknown",
        _ => "Unknown",
    };
    Lifecycle::from(canonical)
}

fn canonical_risk(value: &str) -> RiskLevel {
    let canonical = match normalized_enum(value).as_str() {
        "low" | "normal" => "Normal",
        "sensitive" => "Sensitive",
        "system" => "System",
        "medium" | "high" | "caution" => "Caution",
        "unknown" => "Unknown",
        _ => "Unknown",
    };
    RiskLevel::from(canonical)
}

fn canonical_action(value: &str) -> SuggestedAction {
    let canonical = match normalized_enum(value).as_str() {
        "keep" => "Keep",
        "rename" => "Rename",
        "move" => "Move",
        "moveandrename" => "MoveAndRename",
        "archive" => "Archive",
        "review" => "Review",
        "deletecandidate" | "delete" => "DeleteCandidate",
        "unknown" => "Unknown",
        _ => "Review",
    };
    SuggestedAction::from(canonical)
}

fn valid_target_template_option(value: Option<&str>) -> bool {
    value.is_none_or(valid_target_template)
}

fn valid_target_template(value: &str) -> bool {
    if value.is_empty()
        || value.len() > 512
        || value.starts_with('/')
        || value.starts_with('\\')
        || value.contains('\\')
        || value.contains(':')
        || value.chars().any(char::is_control)
    {
        return false;
    }
    let segments = value.split('/').collect::<Vec<_>>();
    !segments.is_empty()
        && segments.iter().all(|segment| {
            !segment.is_empty()
                && *segment != "."
                && *segment != ".."
                && segment.chars().count() <= 120
                && !segment.contains(['<', '>', '"', '|', '?', '*'])
                && !segment.ends_with(['.', ' '])
                && !is_windows_reserved_segment(segment)
        })
}

fn is_windows_reserved_segment(segment: &str) -> bool {
    let base = segment
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(['.', ' '])
        .to_ascii_uppercase();
    matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            base.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding() -> SemanticSourceBinding {
        SemanticSourceBinding {
            one_off: None,
            global_entry_id: "entry-1".to_string(),
            managed_scope_id: "scope-1".to_string(),
            input_fingerprint: "fingerprint-1".to_string(),
            provider: "local".to_string(),
        }
    }

    fn provider_v1(target: &str) -> String {
        serde_json::json!({
            "version": 1,
            "refId": "managed:entry-1",
            "fileType": "Document",
            "purpose": "Work",
            "lifecycle": "Active",
            "context": "project notes",
            "riskLevel": "Normal",
            "suggestedAction": "Move",
            "targetTemplate": target,
            "suggestedName": "notes.txt",
            "confidence": 0.95,
            "reason": "work notes",
            "keywords": ["notes"],
            "requiresConfirmation": false
        })
        .to_string()
    }

    #[test]
    fn provider_semantics_are_bound_and_paths_remain_relative() {
        let parsed = SemanticAssessmentV1::parse_provider_response(
            &provider_v1("Work/{year}"),
            binding(),
            "source.txt",
            "txt",
            false,
        )
        .expect("valid V1 response");
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.source_binding.managed_scope_id, "scope-1");
        assert_eq!(parsed.target_template.as_deref(), Some("Work/{year}"));
        assert_eq!(parsed.suggested_action.as_str(), "Move");
        assert!(!parsed.requires_confirmation);
    }

    #[test]
    fn malformed_authority_keys_and_non_relative_targets_fail_closed() {
        for (key, value) in [
            ("operationId", serde_json::json!("op-1")),
            ("batchId", serde_json::json!("batch-1")),
            ("operationBatchId", serde_json::json!("batch-1")),
            ("journalId", serde_json::json!("journal-1")),
            ("inputFingerprint", serde_json::json!("attacker-value")),
            (
                "sourceBinding",
                serde_json::json!({"globalEntryId":"other"}),
            ),
            ("targetPath", serde_json::json!("C:/outside")),
            ("overwrite", serde_json::json!(true)),
            ("deleteAllowed", serde_json::json!(true)),
            ("trashAllowed", serde_json::json!(true)),
            ("execute", serde_json::json!(true)),
            ("shellCommand", serde_json::json!("format C:")),
            ("script", serde_json::json!("remove everything")),
            ("toolCall", serde_json::json!({"name":"shell"})),
        ] {
            let mut authority: serde_json::Value =
                serde_json::from_str(&provider_v1("Work")).expect("test response JSON");
            authority
                .as_object_mut()
                .expect("test object")
                .insert(key.to_string(), value);
            assert!(
                SemanticAssessmentV1::parse_provider_response(
                    &authority.to_string(),
                    binding(),
                    "source.txt",
                    "txt",
                    false
                )
                .is_err(),
                "authority key {key} must be rejected"
            );
        }

        for target in [
            "../outside",
            "/absolute",
            "C:/outside",
            "\\\\server\\share",
            "Work//2026",
        ] {
            let parsed = SemanticAssessmentV1::parse_provider_response(
                &provider_v1(target),
                binding(),
                "source.txt",
                "txt",
                false,
            )
            .expect("unsafe hint is downgraded to review");
            assert_eq!(parsed.suggested_action.as_str(), "Review", "{target}");
            assert!(parsed.target_template.is_none(), "{target}");
            assert!(parsed.requires_confirmation, "{target}");
        }
    }

    #[test]
    fn v0_is_migrated_to_a_confirmation_required_v1_envelope() {
        let raw = r#"{"refId":"managed:entry-1","fileType":"document","purpose":"work","lifecycle":"active","riskLevel":"low","suggestedAction":"keep","confidence":0.9,"reason":"legacy"}"#;
        let parsed =
            SemanticAssessmentV1::decode_stored(raw, binding(), "source.txt", "txt", false)
                .expect("legacy state migrates");
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.risk_level.as_str(), "Normal");
        assert!(parsed.requires_confirmation);
        assert!(parsed.canonical_json().unwrap().contains("\"version\":1"));
    }

    #[test]
    fn malformed_or_misbound_stored_v0_and_v1_fail_closed() {
        let malformed_v0 = r#"{"refId":"managed:entry-1","fileType":"document","purpose":"work","lifecycle":"active","riskLevel":"low","suggestedAction":"keep","confidence":0.9,"reason":"legacy","operationId":"op-1"}"#;
        assert!(SemanticAssessmentV1::decode_stored(
            malformed_v0,
            binding(),
            "source.txt",
            "txt",
            false
        )
        .is_err());

        let provider = SemanticAssessmentV1::parse_provider_response(
            &provider_v1("Work"),
            binding(),
            "source.txt",
            "txt",
            false,
        )
        .expect("valid provider semantics");
        let canonical = provider.canonical_json().expect("canonical envelope");
        let mut wrong_binding = binding();
        wrong_binding.input_fingerprint = "stale-fingerprint".to_string();
        assert!(SemanticAssessmentV1::decode_stored(
            &canonical,
            wrong_binding,
            "source.txt",
            "txt",
            false
        )
        .is_err());
        let mut authority: serde_json::Value =
            serde_json::from_str(&canonical).expect("canonical envelope JSON");
        authority
            .as_object_mut()
            .expect("canonical object")
            .insert("journalId".to_string(), serde_json::json!("journal-1"));
        assert!(SemanticAssessmentV1::decode_stored(
            &authority.to_string(),
            binding(),
            "source.txt",
            "txt",
            false
        )
        .is_err());

        let mut unknown_purpose: serde_json::Value =
            serde_json::from_str(&canonical).expect("canonical envelope JSON");
        unknown_purpose["purpose"] = serde_json::json!("Unknown");
        assert!(SemanticAssessmentV1::decode_stored(
            &unknown_purpose.to_string(),
            binding(),
            "source.txt",
            "txt",
            false
        )
        .is_err());

        for invalid_version in [
            r#"{"version":null,"refId":"managed:entry-1","fileType":"document","purpose":"work","lifecycle":"active","riskLevel":"low","suggestedAction":"keep","confidence":0.9,"reason":"legacy"}"#,
            r#"{"version":2,"refId":"managed:entry-1","fileType":"document","purpose":"work","lifecycle":"active","riskLevel":"low","suggestedAction":"keep","confidence":0.9,"reason":"legacy"}"#,
        ] {
            assert!(SemanticAssessmentV1::parse_provider_response(
                invalid_version,
                binding(),
                "source.txt",
                "txt",
                false
            )
            .is_err());
        }
    }

    #[test]
    fn extension_change_and_risky_semantics_cannot_be_accepted_as_normal_moves() {
        let extension = SemanticAssessmentV1::parse_provider_response(
            &provider_v1("Work"),
            binding(),
            "source.txt",
            "txt",
            false,
        )
        .expect("baseline");
        assert_eq!(extension.suggested_name.as_deref(), Some("notes.txt"));

        let risky = provider_v1("Work").replace("\"Normal\"", "\"high\"");
        let risky = SemanticAssessmentV1::parse_provider_response(
            &risky,
            binding(),
            "source.txt",
            "txt",
            false,
        )
        .expect("risky response is retained conservatively");
        assert_eq!(risky.risk_level.as_str(), "Caution");
        assert_eq!(risky.suggested_action.as_str(), "Review");
        assert!(risky.requires_confirmation);

        let renamed = provider_v1("Work").replace("notes.txt", "notes.exe");
        let renamed = SemanticAssessmentV1::parse_provider_response(
            &renamed,
            binding(),
            "source.txt",
            "txt",
            false,
        )
        .expect("extension change is downgraded");
        assert!(renamed.suggested_name.is_none());
        assert_eq!(renamed.suggested_action.as_str(), "Review");

        let delete = provider_v1("Work").replace("\"Move\"", "\"DeleteCandidate\"");
        let delete = SemanticAssessmentV1::parse_provider_response(
            &delete,
            binding(),
            "source.txt",
            "txt",
            false,
        )
        .expect("delete suggestion remains advisory");
        assert_eq!(delete.suggested_action.as_str(), "DeleteCandidate");
        assert!(delete.requires_confirmation);
    }
}
