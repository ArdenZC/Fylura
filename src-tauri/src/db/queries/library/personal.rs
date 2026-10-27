//! U4 user-authored metadata. SQLite / managed IDs remain the sole authority.
use super::*;
use crate::db::queries::library_identity::verify_legacy_source_identity_tx;

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PersonalMetadataRequest {
    Get {
        #[serde(rename = "fileId")]
        file_id: String,
    },
    Update {
        #[serde(rename = "fileId")]
        file_id: String,
        note: String,
        favorite: bool,
        #[serde(rename = "expectedRevision")]
        expected_revision: i64,
    },
    List {
        favorites: bool,
        offset: u32,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalMetadata {
    pub file_id: String,
    pub note: String,
    pub favorite: bool,
    pub revision: i64,
    pub editable: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalMetadataItem {
    pub file_id: String,
    pub name: String,
    pub is_stale: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonalMetadataResponse {
    pub metadata: Option<PersonalMetadata>,
    pub items: Vec<PersonalMetadataItem>,
    pub has_more: bool,
}

// Versioned user metadata stored in the existing SQLite key/value authority.
// SourceUID survives supported managed rename/move/Restore and cannot be a Browse ID.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersonalRecord {
    version: u32,
    note: String,
    favorite: bool,
    revision: i64,
}
fn personal_key(source_uid: &str) -> String {
    format!("file_personal_metadata:v1:{source_uid}")
}

// The latest attribution must still name this incarnation; a path is never identity.
fn attributed_source(tx: &Transaction<'_>, file_id: &str) -> Result<Option<String>, DbError> {
    Ok(tx.query_row(
        "SELECT source_uid FROM library_legacy_attributions WHERE rowid=(SELECT rowid FROM library_legacy_attributions WHERE legacy_file_id=?1 ORDER BY decision_revision DESC,rowid DESC LIMIT 1)
         AND claim_state IN ('verified','user_confirmed') AND source_uid IS NOT NULL", [file_id], |row| row.get(0),
    ).optional()?)
}

fn personal_metadata(tx: &Transaction<'_>, file_id: &str) -> Result<PersonalMetadata, DbError> {
    let (path, stale): (String, bool) = tx
        .query_row(
            "SELECT path,is_stale FROM files WHERE id=?1",
            [file_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or_else(|| DbError::Validation("library_file_not_found".into()))?;
    let source_uid = attributed_source(tx, file_id)?;
    let physical = if stale {
        None
    } else {
        crate::fs_safety::capture_source_object_identity(std::path::Path::new(&path)).ok()
    };
    let live_uid = verify_legacy_source_identity_tx(tx, file_id, physical.as_ref())?;
    let editable = source_uid.is_some() && live_uid == source_uid && !stale;
    let saved: Option<String> = match source_uid.as_ref() {
        Some(uid) => tx
            .query_row(
                "SELECT value FROM app_settings WHERE key=?1",
                [personal_key(uid)],
                |row| row.get(0),
            )
            .optional()?,
        None => None,
    };
    let mut result = PersonalMetadata {
        file_id: file_id.into(),
        note: String::new(),
        favorite: false,
        revision: 0,
        editable,
    };
    if let Some(saved) = saved {
        let record: PersonalRecord = serde_json::from_str(&saved)
            .map_err(|_| DbError::Validation("library_personal_invalid_record".into()))?;
        if record.version != 1 || record.revision < 1 || record.note.chars().count() > 16_000 {
            return Err(DbError::Validation(
                "library_personal_invalid_record".into(),
            ));
        }
        if !stale && live_uid != source_uid {
            return Err(DbError::Validation(
                "library_personal_identity_unavailable".into(),
            ));
        }
        result.note = record.note;
        result.favorite = record.favorite;
        result.revision = record.revision;
    }
    Ok(result)
}

impl Database {
    pub fn file_library_personal_metadata(
        &self,
        request: PersonalMetadataRequest,
    ) -> Result<PersonalMetadataResponse, DbError> {
        let mut conn = self.conn()?;
        let behavior = if matches!(&request, PersonalMetadataRequest::Update { .. }) {
            rusqlite::TransactionBehavior::Immediate
        } else {
            rusqlite::TransactionBehavior::Deferred
        };
        let tx = conn.transaction_with_behavior(behavior)?;
        let mut response = PersonalMetadataResponse {
            metadata: None,
            items: vec![],
            has_more: false,
        };
        match request {
            PersonalMetadataRequest::Get { file_id } => {
                let id = validate_file_id(&file_id)?;
                response.metadata = Some(personal_metadata(&tx, &id)?);
            }
            PersonalMetadataRequest::Update {
                file_id,
                note,
                favorite,
                expected_revision,
            } => {
                let id = validate_file_id(&file_id)?;
                if note.chars().count() > 16_000
                    || expected_revision < 0
                    || expected_revision == i64::MAX
                    || note.contains('\0')
                {
                    return Err(DbError::Validation("library_personal_invalid_note".into()));
                }
                let current = personal_metadata(&tx, &id)?;
                if !current.editable || current.revision != expected_revision {
                    return Err(DbError::Validation(
                        "library_personal_conflict_or_unavailable".into(),
                    ));
                }
                let uid = attributed_source(&tx, &id)?.ok_or_else(|| {
                    DbError::Validation("library_personal_identity_unavailable".into())
                })?;
                let value = serde_json::to_string(&PersonalRecord {
                    version: 1,
                    note,
                    favorite,
                    revision: current.revision + 1,
                })
                .map_err(|_| DbError::Validation("library_personal_invalid_record".into()))?;
                tx.execute("INSERT INTO app_settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![personal_key(&uid),value])?;
                // Empty notes retain a revision tombstone to reject stale writers.
                response.metadata = Some(personal_metadata(&tx, &id)?);
            }
            PersonalMetadataRequest::List { favorites, offset } => {
                // Drive from the small indexed namespace, never from every managed file.
                let mut stmt = tx.prepare("SELECT f.id,f.name,f.is_stale FROM app_settings s
                    CROSS JOIN library_legacy_attributions a INDEXED BY idx_library_attributions_source
                    CROSS JOIN files f
                    WHERE s.key >= 'file_personal_metadata:v1:' AND s.key < 'file_personal_metadata:v1;'
                    AND a.source_uid=substr(s.key,length('file_personal_metadata:v1:')+1)
                    AND f.id=a.legacy_file_id AND a.claim_state IN ('verified','user_confirmed')
                    AND a.rowid=(SELECT rowid FROM library_legacy_attributions WHERE legacy_file_id=f.id ORDER BY decision_revision DESC,rowid DESC LIMIT 1)
                    AND json_valid(s.value) AND json_extract(CASE WHEN json_valid(s.value) THEN s.value ELSE '{}' END,'$.version')=1
                    AND ((?1=1 AND json_extract(CASE WHEN json_valid(s.value) THEN s.value ELSE '{}' END,'$.favorite')=1) OR (?1=0 AND length(json_extract(CASE WHEN json_valid(s.value) THEN s.value ELSE '{}' END,'$.note'))>0))
                    ORDER BY f.name COLLATE NOCASE,f.id LIMIT 41 OFFSET ?2")?;
                response.items = stmt
                    .query_map(params![favorites, offset], |row| {
                        Ok(PersonalMetadataItem {
                            file_id: row.get(0)?,
                            name: row.get(1)?,
                            is_stale: row.get(2)?,
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                response.has_more = response.items.len() > 40;
                response.items.truncate(40);
            }
        }
        tx.commit()?;
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::queries::library_identity::{
        advance_root_epoch_tx, observe_source_for_root_tx, SourceObservationContext,
    };
    use std::{fs, path::PathBuf};

    struct Fixture {
        db: Database,
        root: PathBuf,
        id: String,
    }
    impl Fixture {
        fn new() -> Self {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join(".tmp-tests")
                .join(format!("u4-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&root).unwrap();
            let path = root.join("note.txt");
            fs::write(&path, b"U4 owned fixture").unwrap();
            let db = Database::open(root.join("db.sqlite")).unwrap();
            let id = "u4-managed-file".to_string();
            let file = InsertFileRequest {
                id: id.clone(),
                path: path.to_string_lossy().into_owned(),
                name: "note.txt".into(),
                extension: "txt".into(),
                size: 16,
                mtime: 1,
                ctime: 1,
                is_dir: false,
                state_code: 0,
            };
            db.insert_file(file.clone()).unwrap();
            let mut conn = db.conn().unwrap();
            let tx = conn.transaction().unwrap();
            tx.execute("INSERT INTO scan_roots(id,normalized_path,display_name,source_kind,enabled,health_status,needs_reconciliation,created_at,updated_at) VALUES('u4-root',?1,'U4','file_library',1,'healthy',0,1,1)", [root.to_string_lossy().as_ref()]).unwrap();
            let epoch = advance_root_epoch_tx(&tx, "u4-root", "scan", 1, 1).unwrap();
            let physical = crate::fs_safety::capture_source_object_identity(&path).unwrap();
            assert!(
                physical.created_ns.is_some(),
                "fixture filesystem must expose birth time"
            );
            let outcome = observe_source_for_root_tx(
                &tx,
                &file,
                Some(&physical),
                SourceObservationContext {
                    root_id: "u4-root",
                    root_epoch: epoch,
                    owner_kind: "scan",
                    owner_revision: 1,
                    legacy_row_exists: false,
                    observed_at: 1,
                },
            )
            .unwrap();
            assert_eq!(outcome.verification_state, "verified");
            tx.commit().unwrap();
            drop(conn);
            Self { db, root, id }
        }
        fn get(&self) -> PersonalMetadata {
            self.db
                .file_library_personal_metadata(PersonalMetadataRequest::Get {
                    file_id: self.id.clone(),
                })
                .unwrap()
                .metadata
                .unwrap()
        }
        fn write(
            &self,
            revision: i64,
            note: &str,
            favorite: bool,
        ) -> Result<PersonalMetadataResponse, DbError> {
            self.db
                .file_library_personal_metadata(PersonalMetadataRequest::Update {
                    file_id: self.id.clone(),
                    note: note.into(),
                    favorite,
                    expected_revision: revision,
                })
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn u4_lifecycle_reopen_cas_delete_and_folder_settings_isolation() {
        let f = Fixture::new();
        assert!(f.get().editable);
        f.db.conn().unwrap().execute("INSERT INTO app_settings(key,value) VALUES('file_workspace_places','folder favorites unchanged')",[]).unwrap();
        f.write(0, "private user note", true).unwrap();
        assert!(f.write(0, "stale writer", false).is_err());
        assert!(f.write(1, &"x".repeat(16001), true).is_err());
        assert_eq!(f.get().note, "private user note");
        let reopened = Database::open(f.root.join("db.sqlite")).unwrap();
        assert_eq!(
            reopened
                .file_library_personal_metadata(PersonalMetadataRequest::Get {
                    file_id: f.id.clone()
                })
                .unwrap()
                .metadata
                .unwrap()
                .note,
            "private user note"
        );
        let saved =
            f.db.file_library_personal_metadata(PersonalMetadataRequest::List {
                favorites: true,
                offset: 0,
            })
            .unwrap();
        assert_eq!(saved.items.len(), 1);
        f.write(1, "", false).unwrap();
        assert_eq!(f.get().revision, 2);
        assert!(f.write(0, "ABA stale", true).is_err());
        assert!(f
            .db
            .file_library_personal_metadata(PersonalMetadataRequest::List {
                favorites: false,
                offset: 0
            })
            .unwrap()
            .items
            .is_empty());
        let value: String =
            f.db.conn()
                .unwrap()
                .query_row(
                    "SELECT value FROM app_settings WHERE key='file_workspace_places'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
        assert_eq!(value, "folder favorites unchanged");
    }

    #[test]
    fn u4_rename_move_and_stale_restore_keep_source_notes_but_replacement_does_not() {
        let mut f = Fixture::new();
        f.write(0, "bound to source", true).unwrap();
        let target = f.root.join("renamed.txt");
        fs::rename(f.root.join("note.txt"), &target).unwrap();
        assert!(f
            .db
            .update_file_after_successful_operation(
                &f.id,
                &f.root.join("note.txt").to_string_lossy(),
                &target.to_string_lossy(),
                "renamed.txt"
            )
            .unwrap());
        f.id =
            f.db.conn()
                .unwrap()
                .query_row("SELECT id FROM files WHERE name='renamed.txt'", [], |row| {
                    row.get(0)
                })
                .unwrap();
        assert_eq!(f.get().note, "bound to source");
        assert!(f.get().editable);
        f.db.conn()
            .unwrap()
            .execute("UPDATE files SET is_stale=1 WHERE id=?1", [&f.id])
            .unwrap();
        assert!(!f.get().editable);
        assert_eq!(f.get().note, "bound to source");
        assert!(f.write(1, "blocked stale", false).is_err());
        f.db.conn()
            .unwrap()
            .execute("UPDATE files SET is_stale=0 WHERE id=?1", [&f.id])
            .unwrap();
        assert_eq!(f.get().note, "bound to source");
        fs::rename(&target, f.root.join("old-object.txt")).unwrap();
        fs::write(&target, b"different physical file").unwrap();
        assert!(f
            .db
            .file_library_personal_metadata(PersonalMetadataRequest::Get {
                file_id: f.id.clone()
            })
            .is_err());
        assert!(f.write(1, "do not leak", true).is_err());
    }

    #[test]
    fn u4_ephemeral_missing_conflicted_and_unknown_contract_fail_closed() {
        let f = Fixture::new();
        assert!(f
            .db
            .file_library_personal_metadata(PersonalMetadataRequest::Get {
                file_id: "browse-session-entry".into()
            })
            .is_err());
        f.write(0, "keep private", true).unwrap();
        f.db.conn().unwrap().execute("UPDATE library_legacy_attributions SET claim_state='conflicted' WHERE legacy_file_id=?1",[&f.id]).unwrap();
        assert_eq!(f.get().note, "");
        assert!(!f.get().editable);
        assert!(f.write(0, "conflict", true).is_err());
        assert!(f
            .db
            .file_library_personal_metadata(PersonalMetadataRequest::List {
                favorites: true,
                offset: 0
            })
            .unwrap()
            .items
            .is_empty());
        assert!(serde_json::from_str::<PersonalMetadataRequest>(
            r#"{"kind":"get","fileId":"x","path":"arbitrary"}"#
        )
        .is_err());
    }
}
