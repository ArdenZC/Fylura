//! Stable semantic mapping using the existing v3 error_code field only.
use super::service::IndexServiceResponse;
use crate::global_index::coordinator::GlobalIndexError;
use crate::global_index::models::{
    INDEX_STATUS_ERROR, INDEX_STATUS_PAUSED, INDEX_STATUS_PERMISSION_REQUIRED,
    INDEX_STATUS_REBUILD_REQUIRED,
};

pub(super) fn classify(error: &GlobalIndexError) -> (String, &'static str) {
    let (code, status) = match error {
        GlobalIndexError::Paused => ("index_paused", INDEX_STATUS_PAUSED),
        GlobalIndexError::RebuildRequired(_) => {
            ("index_rebuild_required", INDEX_STATUS_REBUILD_REQUIRED)
        }
        GlobalIndexError::ServiceBusy(_) => ("index_service_busy", INDEX_STATUS_ERROR),
        GlobalIndexError::PermissionRequired(_) => (
            "index_permission_required",
            INDEX_STATUS_PERMISSION_REQUIRED,
        ),
        GlobalIndexError::Provider(_) => ("index_provider_error", INDEX_STATUS_ERROR),
        GlobalIndexError::WindowsIo(code) => {
            return (
                format!("index_windows_io:{code}"),
                if matches!(code, 5 | 1314) {
                    INDEX_STATUS_PERMISSION_REQUIRED
                } else {
                    INDEX_STATUS_ERROR
                },
            )
        }
        GlobalIndexError::Database(_) => ("index_failed", INDEX_STATUS_ERROR),
        // Client-local protocol/transport outcomes are never encoded as a remote outcome claim.
        GlobalIndexError::ServiceProtocol(_) | GlobalIndexError::ServiceOutcomeUnknown(_) => {
            ("index_provider_error", INDEX_STATUS_ERROR)
        }
    };
    (code.to_string(), status)
}

pub(super) fn known_code(code: &str) -> bool {
    matches!(
        code,
        "index_paused"
            | "index_rebuild_required"
            | "index_service_busy"
            | "index_permission_required"
            | "index_provider_error"
            | "index_failed"
    ) || windows_code(code).is_some()
}

fn windows_code(code: &str) -> Option<u32> {
    let number = code.strip_prefix("index_windows_io:")?;
    let value = number.parse::<u32>().ok()?;
    (value.to_string() == number).then_some(value)
}

pub(super) fn decode(response: &IndexServiceResponse) -> GlobalIndexError {
    let message = response
        .message
        .clone()
        .unwrap_or_else(|| "Windows index service request failed".into());
    match response.error_code.as_deref() {
        Some("index_paused") => GlobalIndexError::Paused,
        Some("index_rebuild_required") => GlobalIndexError::RebuildRequired(message),
        Some("index_service_busy") => GlobalIndexError::ServiceBusy(message),
        Some("index_permission_required") => GlobalIndexError::PermissionRequired(message),
        Some("index_provider_error") => GlobalIndexError::Provider(message),
        Some("index_failed") => GlobalIndexError::Database(crate::db::DbError::Validation(message)),
        Some(code) if windows_code(code).is_some() => {
            GlobalIndexError::WindowsIo(windows_code(code).unwrap())
        }
        _ => GlobalIndexError::ServiceProtocol("missing or unknown remote error_code".into()),
    }
}

#[cfg(test)]
#[path = "service_errors_p1_scope_tests.rs"]
mod p1_scope_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::global_index::windows::service::IPC_PROTOCOL_VERSION;
    #[test]
    fn v3_roundtrip_preserves_typed_recovery_and_ignores_message_text() {
        for error in [
            GlobalIndexError::Paused,
            GlobalIndexError::RebuildRequired("arbitrary diagnostic".into()),
            GlobalIndexError::Provider("indexing paused rebuild required 1181".into()),
        ] {
            let (code, status) = classify(&error);
            let response = IndexServiceResponse {
                protocol_version: IPC_PROTOCOL_VERSION,
                request_id: "bounded".into(),
                ok: false,
                error_code: Some(code),
                message: Some(error.to_string()),
                status: Some(status.into()),
            };
            let json = serde_json::to_value(&response).unwrap();
            assert_eq!(json.as_object().unwrap().len(), 6);
            assert_eq!(json["protocol_version"], 3);
            let decoded = decode(&serde_json::from_value(json).unwrap());
            assert_eq!(
                std::mem::discriminant(&decoded),
                std::mem::discriminant(&error)
            );
        }
        for code in [None, Some("unknown"), Some("index_outcome_unknown")] {
            let response = IndexServiceResponse {
                protocol_version: 3,
                request_id: "bounded".into(),
                ok: false,
                error_code: code.map(str::to_string),
                message: Some("indexing paused rebuild required".into()),
                status: Some("rebuild_required".into()),
            };
            assert!(matches!(
                decode(&response),
                GlobalIndexError::ServiceProtocol(_)
            ));
        }
    }

    #[test]
    fn service_busy_decodes_as_request_contention() {
        let response = IndexServiceResponse {
            protocol_version: IPC_PROTOCOL_VERSION,
            request_id: "request-contender".into(),
            ok: false,
            error_code: Some("index_service_busy".into()),
            message: Some("Another global index operation is already running".into()),
            status: Some("indexing".into()),
        };

        match decode(&response) {
            GlobalIndexError::ServiceBusy(message) => {
                assert_eq!(message, "Another global index operation is already running")
            }
            other => panic!("service busy must preserve request identity: {other:?}"),
        }
        println!(
            "issue328_service_busy request_id=request-contender service_status=indexing request_result=rejected error_category=service_busy"
        );
    }
}
