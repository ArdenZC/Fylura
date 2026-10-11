//! Stable semantic mapping using the existing v3 error_code field only.
use super::service::IndexServiceResponse;
use crate::global_index::coordinator::GlobalIndexError;
use crate::global_index::models::{
    INDEX_STATUS_ERROR, INDEX_STATUS_PAUSED, INDEX_STATUS_PERMISSION_REQUIRED,
    INDEX_STATUS_REBUILD_REQUIRED,
};

pub(super) fn classify(error: &GlobalIndexError) -> (&'static str, &'static str) {
    match error {
        GlobalIndexError::Paused => ("index_paused", INDEX_STATUS_PAUSED),
        GlobalIndexError::RebuildRequired(_) => {
            ("index_rebuild_required", INDEX_STATUS_REBUILD_REQUIRED)
        }
        GlobalIndexError::Provider(_) | GlobalIndexError::WindowsIo(_) => (
            "index_permission_required",
            INDEX_STATUS_PERMISSION_REQUIRED,
        ),
        GlobalIndexError::Database(_) => ("index_failed", INDEX_STATUS_ERROR),
    }
}

pub(super) fn decode(response: &IndexServiceResponse) -> GlobalIndexError {
    let message = response
        .message
        .clone()
        .or_else(|| response.error_code.clone())
        .unwrap_or_else(|| "Windows index service request failed".to_string());
    match response.error_code.as_deref() {
        Some("index_paused") => GlobalIndexError::Paused,
        Some("index_rebuild_required") => GlobalIndexError::RebuildRequired(message),
        _ => GlobalIndexError::Provider(message),
    }
}

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
                error_code: Some(code.into()),
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
        for code in [None, Some("unknown"), Some("index_failed")] {
            let response = IndexServiceResponse {
                protocol_version: 3,
                request_id: "bounded".into(),
                ok: false,
                error_code: code.map(str::to_string),
                message: Some("indexing paused rebuild required".into()),
                status: Some("rebuild_required".into()),
            };
            assert!(matches!(decode(&response), GlobalIndexError::Provider(_)));
        }
    }

    #[test]
    fn service_busy_decodes_as_service_provider_contention() {
        let response = IndexServiceResponse {
            protocol_version: IPC_PROTOCOL_VERSION,
            request_id: "request-contender".into(),
            ok: false,
            error_code: Some("index_service_busy".into()),
            message: Some("Another global index operation is already running".into()),
            status: Some("indexing".into()),
        };

        match decode(&response) {
            GlobalIndexError::Provider(message) => {
                assert_eq!(message, "Another global index operation is already running")
            }
            other => panic!("service busy must remain a provider/service error: {other:?}"),
        }
        println!(
            "issue328_service_busy request_id=request-contender service_status=indexing request_result=rejected error_category=service_busy"
        );
    }
}
