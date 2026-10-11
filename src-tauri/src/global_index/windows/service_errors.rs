//! Stable semantic mapping using the existing v3 error_code field only.
use super::service::IndexServiceResponse;
use crate::db::DbError;
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
        GlobalIndexError::PermissionRequired(_) | GlobalIndexError::WindowsIo(_) => (
            "index_permission_required",
            INDEX_STATUS_PERMISSION_REQUIRED,
        ),
        GlobalIndexError::Provider(_) => ("index_provider_error", INDEX_STATUS_ERROR),
        GlobalIndexError::DatabaseBusy(_) => ("index_database_busy", INDEX_STATUS_ERROR),
        GlobalIndexError::ServiceBusy => ("index_service_busy", INDEX_STATUS_ERROR),
        GlobalIndexError::ServiceOutcomeUnknown(_) => ("index_outcome_unknown", INDEX_STATUS_ERROR),
        GlobalIndexError::Database(DbError::Sqlite(rusqlite::Error::SqliteFailure(error, _)))
            if matches!(
                error.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            ) =>
        {
            ("index_database_busy", INDEX_STATUS_ERROR)
        }
        GlobalIndexError::Database(_) => ("index_database_error", INDEX_STATUS_ERROR),
    }
}

pub(super) fn decode(response: &IndexServiceResponse) -> GlobalIndexError {
    let message = response
        .message
        .clone()
        .or_else(|| response.error_code.clone())
        .unwrap_or_else(|| "Windows index service request failed".to_string());
    match response.error_code.as_deref() {
        Some("index_service_busy") => GlobalIndexError::ServiceBusy,
        Some("index_database_busy") => GlobalIndexError::DatabaseBusy(message),
        Some("index_database_error") => GlobalIndexError::Database(DbError::Validation(message)),
        Some("index_outcome_unknown") => GlobalIndexError::ServiceOutcomeUnknown(message),
        Some("index_paused") => GlobalIndexError::Paused,
        Some("index_rebuild_required") => GlobalIndexError::RebuildRequired(message),
        Some("index_permission_required") => GlobalIndexError::PermissionRequired(message),
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
    fn service_busy_decodes_as_a_typed_request_rejection() {
        let response = IndexServiceResponse {
            protocol_version: IPC_PROTOCOL_VERSION,
            request_id: "request-contender".into(),
            ok: false,
            error_code: Some("index_service_busy".into()),
            message: Some("Another global index operation is already running".into()),
            status: Some("indexing".into()),
        };

        assert!(matches!(decode(&response), GlobalIndexError::ServiceBusy));
        println!(
            "issue328_service_busy request_id=request-contender service_status=indexing request_result=rejected error_category=service_busy"
        );
    }

    #[test]
    fn provider_permission_and_service_busy_codes_remain_distinct() {
        for (code, expected) in [
            ("index_service_busy", "service_busy"),
            ("index_provider_error", "provider"),
            ("index_permission_required", "permission"),
        ] {
            let response = IndexServiceResponse {
                protocol_version: IPC_PROTOCOL_VERSION,
                request_id: "typed-response".into(),
                ok: false,
                error_code: Some(code.into()),
                message: Some("diagnostic text is not the classifier".into()),
                status: Some("indexing".into()),
            };
            let decoded = decode(&response);
            match expected {
                "service_busy" => assert!(matches!(decoded, GlobalIndexError::ServiceBusy)),
                "provider" => assert!(matches!(decoded, GlobalIndexError::Provider(_))),
                "permission" => assert!(matches!(decoded, GlobalIndexError::PermissionRequired(_))),
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn database_busy_locked_provider_permission_and_unknown_are_distinct() {
        let responses = [
            ("index_service_busy", "service"),
            ("index_database_busy", "database_busy"),
            ("index_database_error", "database"),
            ("index_provider_error", "provider"),
            ("index_permission_required", "permission"),
            ("index_outcome_unknown", "unknown"),
        ];
        for (code, expected) in responses {
            let response = IndexServiceResponse {
                protocol_version: IPC_PROTOCOL_VERSION,
                request_id: "error-taxonomy".into(),
                ok: false,
                error_code: Some(code.into()),
                message: Some("typed error fixture".into()),
                status: Some(INDEX_STATUS_ERROR.into()),
            };
            let decoded = decode(&response);
            match expected {
                "service" => assert!(matches!(decoded, GlobalIndexError::ServiceBusy)),
                "database_busy" => {
                    assert!(matches!(decoded, GlobalIndexError::DatabaseBusy(_)))
                }
                "database" => assert!(matches!(
                    decoded,
                    GlobalIndexError::Database(DbError::Validation(_))
                )),
                "provider" => assert!(matches!(decoded, GlobalIndexError::Provider(_))),
                "permission" => assert!(matches!(decoded, GlobalIndexError::PermissionRequired(_))),
                "unknown" => assert!(matches!(
                    decoded,
                    GlobalIndexError::ServiceOutcomeUnknown(_)
                )),
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn sqlite_busy_and_locked_codes_keep_database_identity_over_ipc_v3() {
        for sqlite_code in [rusqlite::ffi::SQLITE_BUSY, rusqlite::ffi::SQLITE_LOCKED] {
            let error = GlobalIndexError::Database(DbError::Sqlite(
                rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(sqlite_code), None),
            ));
            let (code, status) = classify(&error);
            assert_eq!(code, "index_database_busy");
            assert_eq!(status, INDEX_STATUS_ERROR);
            let response = IndexServiceResponse {
                protocol_version: IPC_PROTOCOL_VERSION,
                request_id: "sqlite-busy-taxonomy".into(),
                ok: false,
                error_code: Some(code.into()),
                message: Some(error.to_string()),
                status: Some(status.into()),
            };
            assert!(matches!(
                decode(&response),
                GlobalIndexError::DatabaseBusy(_)
            ));
        }
    }
}
