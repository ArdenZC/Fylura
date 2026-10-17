//! Durable, opaque SourceUID observations and per-root epoch claims.
//!
//! These writes intentionally remain a materialization beside the legacy
//! `files` reader. They never switch the Library generation authority.

use crate::{
    db::{DbError, InsertFileRequest},
    fs_safety::PhysicalFileIdentity,
    ids::new_job_id,
};
use rusqlite::{params, OptionalExtension, Transaction};

const SOURCE_UID_PHYSICAL_LOOKUP_SQL: &str = r#"
SELECT source_uid, verification_digest, verification_state
FROM library_source_incarnations
WHERE physical_platform = ?1 AND physical_volume_id = ?2
  AND physical_object_id = ?3
ORDER BY created_at, source_uid
"#;

#[derive(Debug, Clone)]
pub(crate) struct SourceObservationOutcome {
    pub source_uid: String,
    /// Existing path rows are quarantined when their fact lineage cannot be
    /// proved against the observed physical object.
    pub block_legacy_row: bool,
    pub verification_state: &'static str,
    pub changed: bool,
}

pub(crate) struct SourceObservationContext<'a> {
    pub root_id: &'a str,
    pub root_epoch: i64,
    pub owner_kind: &'a str,
    pub owner_revision: i64,
    pub legacy_row_exists: bool,
    pub observed_at: i64,
}

#[derive(Debug)]
struct PhysicalClaim {
    platform: String,
    volume_id: String,
    object_id: String,
    digest: String,
}

#[derive(Debug)]
struct ExistingAttribution {
    source_uid: Option<String>,
    claim_state: String,
}

pub(crate) fn advance_root_epoch_tx(
    tx: &Transaction<'_>,
    root_id: &str,
    owner_kind: &str,
    owner_revision: i64,
    observed_at: i64,
) -> Result<i64, DbError> {
    tx.execute(
        r#"
        INSERT OR IGNORE INTO library_root_epoch_state (
            root_id, epoch, last_owner_kind, last_owner_revision, updated_at
        )
        SELECT id, 0, ?2, ?3, ?4
        FROM scan_roots
        WHERE id = ?1 AND source_kind = 'file_library'
        "#,
        params![root_id, owner_kind, owner_revision, observed_at],
    )?;
    let changed = tx.execute(
        r#"
        UPDATE library_root_epoch_state
        SET epoch = epoch + 1,
            last_owner_kind = ?2,
            last_owner_revision = ?3,
            updated_at = ?4
        WHERE root_id = ?1
        "#,
        params![root_id, owner_kind, owner_revision, observed_at],
    )?;
    if changed != 1 {
        return Err(DbError::Validation(format!(
            "SourceUID root epoch owner is missing for root {root_id}."
        )));
    }
    tx.query_row(
        "SELECT epoch FROM library_root_epoch_state WHERE root_id = ?1",
        [root_id],
        |row| row.get(0),
    )
    .map_err(DbError::from)
}

pub(crate) fn managed_root_ids_for_path_tx(
    tx: &Transaction<'_>,
    path: &str,
) -> Result<Vec<(String, i64)>, DbError> {
    let mut statement = tx.prepare(
        "SELECT id, normalized_path, revision FROM scan_roots \
         WHERE source_kind = 'file_library' AND enabled = 1 ORDER BY id",
    )?;
    let roots = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(roots
        .into_iter()
        .filter(|(_, root, _)| path_is_within(root, path))
        .map(|(id, _, revision)| (id, revision))
        .collect())
}

pub(crate) fn observe_source_for_root_tx(
    tx: &Transaction<'_>,
    file: &InsertFileRequest,
    physical: Option<&PhysicalFileIdentity>,
    context: SourceObservationContext<'_>,
) -> Result<SourceObservationOutcome, DbError> {
    let SourceObservationContext {
        root_id,
        root_epoch,
        owner_kind,
        owner_revision,
        legacy_row_exists,
        observed_at,
    } = context;
    let physical_claim = physical.and_then(physical_claim);
    let (root_prior_uid, root_prior_state) = tx
        .query_row(
            r#"
            SELECT source_uid, claim_state FROM library_source_root_claims
            WHERE root_id = ?1 AND observed_path = ?2
              AND claim_state != 'absent'
            ORDER BY CASE claim_state
                WHEN 'present' THEN 0
                WHEN 'unverified' THEN 1
                WHEN 'conflicted' THEN 2
                ELSE 3
            END, root_epoch DESC, rowid DESC
            LIMIT 1
            "#,
            params![root_id, file.path],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?
        .map_or((None, None), |(uid, state)| (Some(uid), Some(state)));
    let reusable_unverified_uid = (physical_claim.is_none()
        && root_prior_state.as_deref() == Some("unverified"))
    .then_some(root_prior_uid.as_deref())
    .flatten();
    let (source_uid, verification_state) =
        if physical_claim.is_none() && root_prior_state.as_deref() == Some("conflicted") {
            (
                root_prior_uid
                    .clone()
                    .expect("conflicted path claim has a source uid"),
                "conflicted",
            )
        } else {
            resolve_source_uid_tx(
                tx,
                physical_claim.as_ref(),
                reusable_unverified_uid,
                observed_at,
            )?
        };
    let claim_state = match verification_state {
        "verified" => "present",
        "conflicted" => "conflicted",
        _ => "unverified",
    };

    let attribution = latest_attribution_tx(tx, &file.id)?;
    let path_prior_uid = root_prior_uid.or_else(|| {
        attribution
            .as_ref()
            .and_then(|claim| claim.source_uid.clone())
    });
    // Option::or eagerly ran the cross-root SQL even when the root or legacy
    // attribution already supplied the answer. Keep the precedence unchanged,
    // but avoid that extra query while the scan owns SQLite's writer lock.
    let path_prior_uid = match path_prior_uid {
        Some(uid) => Some(uid),
        None => latest_source_uid_for_path_tx(tx, &file.path)?,
    };

    let identity_changed = path_prior_uid
        .as_deref()
        .is_some_and(|prior_uid| prior_uid != source_uid);
    let path_claim_conflicted = identity_changed
        || verification_state == "conflicted"
        || (verification_state == "unverified"
            && path_prior_uid.as_deref() != Some(source_uid.as_str()));
    if path_claim_conflicted {
        mark_path_claims_conflicted_tx(
            tx,
            &file.path,
            root_id,
            root_epoch,
            owner_kind,
            owner_revision,
            observed_at,
        )?;
        if legacy_row_exists {
            conflict_attribution_tx(
                tx,
                &file.id,
                &file.path,
                path_prior_uid.as_deref(),
                physical_claim.as_ref().map(|claim| claim.digest.as_str()),
                observed_at,
            )?;
        }
    }

    let attribution_matches = attribution.as_ref().is_some_and(|claim| {
        claim.source_uid.as_deref() == Some(source_uid.as_str())
            && matches!(claim.claim_state.as_str(), "verified" | "user_confirmed")
    });
    let block_legacy_row = verification_state != "verified"
        || (legacy_row_exists && (identity_changed || !attribution_matches));

    let inherited_legacy_id =
        if attribution_matches || (!legacy_row_exists && verification_state == "verified") {
            Some(file.id.as_str())
        } else {
            None
        };
    let observation_digest = observation_digest(file, physical_claim.as_ref(), verification_state)?;
    let previous_observation = tx
        .query_row(
            r#"
            SELECT observation_revision, observation_digest
            FROM library_source_observations
            WHERE source_uid = ?1
            ORDER BY observation_revision DESC LIMIT 1
            "#,
            params![source_uid],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let previous_claim_state = tx
        .query_row(
            "SELECT claim_state FROM library_source_root_claims WHERE root_id = ?1 AND source_uid = ?2 AND observed_path = ?3",
            params![root_id, source_uid, file.path],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let (observation_revision, observation_changed) = match previous_observation {
        Some((revision, digest)) if digest == observation_digest => (revision, false),
        Some((revision, _)) => (revision + 1, true),
        None => (1, true),
    };
    if observation_changed {
        tx.execute(
            r#"
            INSERT INTO library_source_observations (
                source_uid, observation_revision, legacy_file_id_snapshot,
                observed_path, name, extension, size, mtime, ctime, is_dir,
                physical_claim_digest, observation_digest, observed_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            "#,
            params![
                source_uid,
                observation_revision,
                inherited_legacy_id,
                file.path,
                file.name,
                file.extension,
                file.size.max(0),
                file.mtime,
                file.ctime,
                i64::from(file.is_dir),
                physical_claim.as_ref().map(|claim| claim.digest.as_str()),
                observation_digest,
                observed_at,
            ],
        )?;
        tx.execute(
            "UPDATE library_source_incarnations SET source_revision = source_revision + 1, updated_at = ?2 WHERE source_uid = ?1",
            params![source_uid, observed_at],
        )?;
    }

    if let Some(legacy_file_id) = inherited_legacy_id {
        ensure_verified_attribution_tx(
            tx,
            legacy_file_id,
            &file.path,
            &source_uid,
            physical_claim.as_ref().map(|claim| claim.digest.as_str()),
            observed_at,
        )?;
    } else if !attribution_matches {
        ensure_unresolved_attribution_tx(tx, &file.id, &file.path, observed_at)?;
    }

    tx.execute(
        r#"
        INSERT INTO library_source_root_claims (
            root_id, source_uid, observed_path, root_epoch, owner_kind,
            owner_revision, observation_revision, claim_state, observed_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        ON CONFLICT(root_id, source_uid, observed_path) DO UPDATE SET
            root_epoch = excluded.root_epoch,
            owner_kind = excluded.owner_kind,
            owner_revision = excluded.owner_revision,
            observation_revision = excluded.observation_revision,
            claim_state = excluded.claim_state,
            observed_at = excluded.observed_at
        WHERE excluded.root_epoch > library_source_root_claims.root_epoch
        "#,
        params![
            root_id,
            source_uid,
            file.path,
            root_epoch,
            owner_kind,
            owner_revision,
            observation_revision,
            claim_state,
            observed_at,
        ],
    )?;

    Ok(SourceObservationOutcome {
        source_uid,
        block_legacy_row,
        verification_state,
        changed: observation_changed
            || identity_changed
            || previous_claim_state.as_deref() != Some(claim_state),
    })
}

/// Records the identity boundary for a neutral File Library row created when
/// an existing legacy row cannot be proven to describe the observed object.
/// This row contains no inherited user facts, so verified physical identity
/// can safely bind future facts created against the current object.
pub(crate) fn ensure_neutral_source_attribution_tx(
    tx: &Transaction<'_>,
    file_id: &str,
    path: &str,
    source_uid: &str,
    physical: Option<&PhysicalFileIdentity>,
    verified: bool,
    observed_at: i64,
) -> Result<(), DbError> {
    let physical_claim = physical.and_then(physical_claim);
    if verified {
        if let Some(physical_claim) = physical_claim.as_ref() {
            return ensure_verified_attribution_tx(
                tx,
                file_id,
                path,
                source_uid,
                Some(&physical_claim.digest),
                observed_at,
            );
        }
    }
    ensure_unresolved_attribution_tx(tx, file_id, path, observed_at)
}

pub(crate) fn verify_legacy_source_identity_tx(
    tx: &Transaction<'_>,
    legacy_file_id: &str,
    physical: Option<&PhysicalFileIdentity>,
) -> Result<Option<String>, DbError> {
    let Some(physical) = physical.and_then(physical_claim) else {
        return Ok(None);
    };
    let Some(attribution) = latest_attribution_tx(tx, legacy_file_id)? else {
        return Ok(None);
    };
    if !matches!(
        attribution.claim_state.as_str(),
        "verified" | "user_confirmed"
    ) {
        return Ok(None);
    }
    let Some(source_uid) = attribution.source_uid else {
        return Ok(None);
    };
    let identity_matches: bool = tx.query_row(
        r#"
        SELECT EXISTS(
            SELECT 1 FROM library_source_incarnations
            WHERE source_uid = ?1 AND verification_state = 'verified'
              AND physical_platform = ?2 AND physical_volume_id = ?3
              AND physical_object_id = ?4 AND verification_digest = ?5
        )
        "#,
        params![
            source_uid,
            physical.platform,
            physical.volume_id,
            physical.object_id,
            physical.digest,
        ],
        |row| row.get(0),
    )?;
    Ok(identity_matches.then_some(source_uid))
}

pub(crate) fn mark_root_path_absent_tx(
    tx: &Transaction<'_>,
    root_id: &str,
    path: &str,
    root_epoch: i64,
    owner_kind: &str,
    owner_revision: i64,
    observed_at: i64,
) -> Result<usize, DbError> {
    Ok(tx.execute(
        r#"
        UPDATE library_source_root_claims
        SET root_epoch = ?3, owner_kind = ?4, owner_revision = ?5,
            claim_state = 'absent', observed_at = ?6
        WHERE root_id = ?1 AND observed_path = ?2
          AND claim_state IN ('present','unverified')
          AND root_epoch < ?3
        "#,
        params![
            root_id,
            path,
            root_epoch,
            owner_kind,
            owner_revision,
            observed_at
        ],
    )?)
}

pub(crate) fn reconcile_root_membership_tx(
    tx: &Transaction<'_>,
    root_id: &str,
    run_id: &str,
    root_epoch: i64,
    owner_revision: i64,
    observed_at: i64,
) -> Result<usize, DbError> {
    let changed = tx.execute(
        r#"
        UPDATE library_source_root_claims AS claim
        SET root_epoch = ?3, owner_kind = 'scan', owner_revision = ?4,
            claim_state = 'absent', observed_at = ?5
        WHERE claim.root_id = ?1
          AND claim.claim_state IN ('present','unverified')
          AND NOT EXISTS (
              SELECT 1 FROM scan_seen AS seen
              WHERE seen.run_id = ?2 AND seen.observed_path = claim.observed_path
          )
        "#,
        params![root_id, run_id, root_epoch, owner_revision, observed_at],
    )?;
    tx.execute(
        r#"
        UPDATE library_source_incarnations
        SET verification_state = 'retired', source_revision = source_revision + 1,
            updated_at = ?1
        WHERE verification_state = 'verified'
          AND EXISTS (
              SELECT 1 FROM library_source_root_claims AS claim
              WHERE claim.source_uid = library_source_incarnations.source_uid
          )
          AND NOT EXISTS (
              SELECT 1 FROM library_source_root_claims AS claim
              WHERE claim.source_uid = library_source_incarnations.source_uid
                AND claim.claim_state IN ('present','unverified','conflicted')
          )
        "#,
        params![observed_at],
    )?;
    Ok(changed)
}

fn physical_claim(identity: &PhysicalFileIdentity) -> Option<PhysicalClaim> {
    let platform = identity.platform_kind.as_str();
    let volume_id = identity.platform_volume_id.as_deref()?.trim();
    let object_id = identity.platform_file_id.as_deref()?.trim();
    let created_ns = identity.created_ns?;
    if volume_id.is_empty() || object_id.is_empty() {
        return None;
    }
    let digest =
        blake3::hash(format!("{platform}\0{volume_id}\0{object_id}\0{created_ns}").as_bytes())
            .to_hex()
            .to_string();
    Some(PhysicalClaim {
        platform: platform.to_string(),
        volume_id: volume_id.to_string(),
        object_id: object_id.to_string(),
        digest,
    })
}

fn resolve_source_uid_tx(
    tx: &Transaction<'_>,
    physical: Option<&PhysicalClaim>,
    reusable_unverified_uid: Option<&str>,
    observed_at: i64,
) -> Result<(String, &'static str), DbError> {
    let Some(physical) = physical else {
        if let Some(source_uid) = reusable_unverified_uid {
            return Ok((source_uid.to_string(), "unverified"));
        }
        let source_uid = new_job_id("source-uid");
        tx.execute(
            r#"
            INSERT INTO library_source_incarnations (
                source_uid, verification_state, created_at, updated_at
            ) VALUES (?1, 'unverified', ?2, ?2)
            "#,
            params![source_uid, observed_at],
        )?;
        return Ok((source_uid, "unverified"));
    };

    let mut statement = tx.prepare(SOURCE_UID_PHYSICAL_LOOKUP_SQL)?;
    let matching_ids = statement
        .query_map(
            params![physical.platform, physical.volume_id, physical.object_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);

    if let Some((source_uid, _, state)) = matching_ids
        .iter()
        .find(|(_, digest, _)| digest.as_deref() == Some(physical.digest.as_str()))
    {
        if state == "verified" {
            return Ok((source_uid.clone(), "verified"));
        }
        if state == "retired" {
            tx.execute(
                "UPDATE library_source_incarnations SET verification_state='verified', source_revision=source_revision+1, updated_at=?2 WHERE source_uid=?1 AND verification_state='retired'",
                params![source_uid, observed_at],
            )?;
            return Ok((source_uid.clone(), "verified"));
        }
        return Ok((source_uid.clone(), "conflicted"));
    }

    let live_collision = if matching_ids.is_empty() {
        false
    } else {
        let mut live = false;
        for (source_uid, _, state) in &matching_ids {
            let has_live_claim: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM library_source_root_claims WHERE source_uid=?1 AND claim_state IN ('present','unverified','conflicted'))",
                params![source_uid],
                |row| row.get(0),
            )?;
            live |= has_live_claim || state == "conflicted";
        }
        live
    };
    let state = if live_collision {
        "conflicted"
    } else {
        "verified"
    };
    if live_collision {
        tx.execute(
            r#"
            UPDATE library_source_incarnations
            SET verification_state='conflicted', source_revision=source_revision+1,
                updated_at=?1
            WHERE physical_platform=?2 AND physical_volume_id=?3 AND physical_object_id=?4
              AND verification_state='verified'
            "#,
            params![
                observed_at,
                physical.platform,
                physical.volume_id,
                physical.object_id
            ],
        )?;
    }
    let source_uid = new_job_id("source-uid");
    tx.execute(
        r#"
        INSERT INTO library_source_incarnations (
            source_uid, verification_state, physical_platform, physical_volume_id,
            physical_object_id, verification_digest, source_revision, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?7)
        "#,
        params![
            source_uid,
            state,
            physical.platform,
            physical.volume_id,
            physical.object_id,
            physical.digest,
            observed_at,
        ],
    )?;
    Ok((source_uid, state))
}

fn latest_attribution_tx(
    tx: &Transaction<'_>,
    legacy_file_id: &str,
) -> Result<Option<ExistingAttribution>, DbError> {
    tx.query_row(
        r#"
        SELECT source_uid, claim_state
        FROM library_legacy_attributions
        WHERE legacy_file_id = ?1
        ORDER BY decision_revision DESC, rowid DESC LIMIT 1
        "#,
        params![legacy_file_id],
        |row| {
            Ok(ExistingAttribution {
                source_uid: row.get(0)?,
                claim_state: row.get(1)?,
            })
        },
    )
    .optional()
    .map_err(DbError::from)
}

pub(crate) fn has_source_uid_attribution_tx(
    tx: &Transaction<'_>,
    file_id: &str,
) -> Result<bool, DbError> {
    Ok(latest_attribution_tx(tx, file_id)?.is_some())
}

fn latest_source_uid_for_path_tx(
    tx: &Transaction<'_>,
    path: &str,
) -> Result<Option<String>, DbError> {
    let mut statement = tx.prepare(
        r#"
        SELECT source_uid
        FROM library_source_root_claims
        WHERE observed_path = ?1 AND claim_state IN ('present','unverified')
        GROUP BY source_uid
        ORDER BY source_uid
        LIMIT 2
        "#,
    )?;
    let source_uids = statement
        .query_map(params![path], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok((source_uids.len() == 1).then(|| source_uids[0].clone()))
}

fn ensure_verified_attribution_tx(
    tx: &Transaction<'_>,
    legacy_file_id: &str,
    legacy_path: &str,
    source_uid: &str,
    physical_digest: Option<&str>,
    observed_at: i64,
) -> Result<(), DbError> {
    let current = latest_attribution_tx(tx, legacy_file_id)?;
    if current.is_some() {
        return Ok(());
    }
    let evidence = attribution_evidence(source_uid, legacy_path, physical_digest);
    tx.execute(
        r#"
        INSERT INTO library_legacy_attributions (
            claim_id, legacy_file_id, legacy_path_snapshot, source_uid,
            claim_state, evidence_digest, decision_revision, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, 'verified', ?5, 1, ?6, ?6)
        "#,
        params![
            new_job_id("source-attribution"),
            legacy_file_id,
            legacy_path,
            source_uid,
            evidence,
            observed_at,
        ],
    )?;
    Ok(())
}

fn ensure_unresolved_attribution_tx(
    tx: &Transaction<'_>,
    legacy_file_id: &str,
    legacy_path: &str,
    observed_at: i64,
) -> Result<(), DbError> {
    if latest_attribution_tx(tx, legacy_file_id)?.is_some() {
        return Ok(());
    }
    tx.execute(
        r#"
        INSERT INTO library_legacy_attributions (
            claim_id, legacy_file_id, legacy_path_snapshot, source_uid,
            claim_state, evidence_digest, decision_revision, created_at, updated_at
        ) VALUES (?1, ?2, ?3, NULL, 'unresolved', NULL, 1, ?4, ?4)
        "#,
        params![
            new_job_id("source-attribution"),
            legacy_file_id,
            legacy_path,
            observed_at
        ],
    )?;
    Ok(())
}

fn conflict_attribution_tx(
    tx: &Transaction<'_>,
    legacy_file_id: &str,
    legacy_path: &str,
    previous_uid: Option<&str>,
    physical_digest: Option<&str>,
    observed_at: i64,
) -> Result<(), DbError> {
    let Some(previous_uid) = previous_uid else {
        return ensure_unresolved_attribution_tx(tx, legacy_file_id, legacy_path, observed_at);
    };
    let revision: i64 = tx.query_row(
        "SELECT COALESCE(MAX(decision_revision), 0) + 1 FROM library_legacy_attributions WHERE legacy_file_id=?1",
        params![legacy_file_id],
        |row| row.get(0),
    )?;
    let evidence = attribution_evidence(previous_uid, legacy_path, physical_digest);
    tx.execute(
        r#"
        INSERT INTO library_legacy_attributions (
            claim_id, legacy_file_id, legacy_path_snapshot, source_uid,
            claim_state, evidence_digest, decision_revision, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?4, 'conflicted', ?5, ?6, ?7, ?7)
        "#,
        params![
            new_job_id("source-attribution"),
            legacy_file_id,
            legacy_path,
            previous_uid,
            evidence,
            revision,
            observed_at,
        ],
    )?;
    Ok(())
}

fn mark_path_claims_conflicted_tx(
    tx: &Transaction<'_>,
    path: &str,
    current_root_id: &str,
    current_root_epoch: i64,
    owner_kind: &str,
    owner_revision: i64,
    observed_at: i64,
) -> Result<(), DbError> {
    let mut statement = tx.prepare(
        r#"
        SELECT DISTINCT root_id FROM library_source_root_claims
        WHERE observed_path = ?1 AND claim_state IN ('present','unverified')
        "#,
    )?;
    let root_ids = statement
        .query_map(params![path], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for root_id in root_ids {
        let (epoch, revision) = if root_id == current_root_id {
            (current_root_epoch, owner_revision)
        } else {
            // A newly observed physical object at the same managed path makes
            // older work in every overlapping root stale. Keep the claim but
            // fence its current scan/watcher owner and request reconciliation.
            tx.execute(
                r#"
                UPDATE scan_roots
                SET revision = revision + 1,
                    watcher_revision = watcher_revision + 1,
                    needs_reconciliation = 1,
                    health_status = CASE
                        WHEN health_status IN ('missing','permission_required') THEN health_status
                        ELSE 'reconciliation_required'
                    END,
                    updated_at = ?1
                WHERE id = ?2
                "#,
                params![observed_at, root_id],
            )?;
            let revision: i64 = tx.query_row(
                "SELECT revision FROM scan_roots WHERE id = ?1",
                params![root_id],
                |row| row.get(0),
            )?;
            let epoch = advance_root_epoch_tx(tx, &root_id, owner_kind, revision, observed_at)?;
            (epoch, revision)
        };
        tx.execute(
            r#"
            UPDATE library_source_root_claims
            SET root_epoch = ?3, owner_kind = ?4, owner_revision = ?5,
                claim_state = 'conflicted', observed_at = ?6
            WHERE root_id = ?1 AND observed_path = ?2
              AND claim_state IN ('present','unverified')
            "#,
            params![root_id, path, epoch, owner_kind, revision, observed_at],
        )?;
    }
    Ok(())
}

fn observation_digest(
    file: &InsertFileRequest,
    physical: Option<&PhysicalClaim>,
    verification_state: &str,
) -> Result<String, DbError> {
    let bytes = serde_json::to_vec(&(
        &file.path,
        &file.name,
        &file.extension,
        file.size,
        file.mtime,
        file.ctime,
        file.is_dir,
        verification_state,
        physical.map(|claim| claim.digest.as_str()),
    ))?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn attribution_evidence(source_uid: &str, path: &str, physical_digest: Option<&str>) -> String {
    blake3::hash(
        format!(
            "{source_uid}\0{path}\0{}",
            physical_digest.unwrap_or("unavailable")
        )
        .as_bytes(),
    )
    .to_hex()
    .to_string()
}

fn path_is_within(root: &str, path: &str) -> bool {
    let root = normalized_path_key(root);
    let path = normalized_path_key(path);
    if root == "/" {
        return path.starts_with('/');
    }
    path == root
        || path
            .strip_prefix(&root)
            .is_some_and(|suffix| suffix.starts_with('/') || suffix.starts_with('\\'))
}

fn normalized_path_key(path: &str) -> String {
    let mut normalized = path.replace('\\', "/");
    while normalized.len() > 1 && normalized.ends_with('/') {
        normalized.pop();
    }
    if cfg!(windows) {
        normalized.make_ascii_lowercase();
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::Database,
        fs_safety::{PhysicalFileIdentity, PhysicalPlatform},
    };
    use rusqlite::Connection;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_db_path(label: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "zen-canvas-source-identity-{label}-{}-{nonce}.sqlite3",
            std::process::id()
        ))
    }

    fn test_db(label: &str) -> Database {
        Database::open(test_db_path(label)).expect("open source identity test database")
    }

    fn seed_physical_lookup_row(
        conn: &Connection,
        source_uid: &str,
        verification_state: &str,
        physical_object_id: Option<&str>,
        verification_digest: Option<&str>,
        created_at: i64,
    ) {
        conn.execute(
            "INSERT INTO library_source_incarnations (
                source_uid, verification_state, physical_platform, physical_volume_id,
                physical_object_id, verification_digest, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![
                source_uid,
                verification_state,
                physical_object_id.map(|_| "unix"),
                physical_object_id.map(|_| "volume"),
                physical_object_id,
                verification_digest,
                created_at,
            ],
        )
        .expect("seed source physical lookup row");
    }

    fn physical_lookup_rows(conn: &Connection) -> Vec<(String, Option<String>, String)> {
        let mut statement = conn
            .prepare(SOURCE_UID_PHYSICAL_LOOKUP_SQL)
            .expect("prepare SourceUID physical lookup");
        statement
            .query_map(params!["unix", "volume", "object"], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .expect("query SourceUID physical lookup")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect SourceUID physical lookup rows")
    }

    fn physical_lookup_plan(conn: &Connection) -> Vec<String> {
        let mut statement = conn
            .prepare(&format!(
                "EXPLAIN QUERY PLAN {SOURCE_UID_PHYSICAL_LOOKUP_SQL}"
            ))
            .expect("prepare SourceUID physical lookup plan");
        statement
            .query_map(params!["unix", "volume", "object"], |row| row.get(3))
            .expect("query SourceUID physical lookup plan")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect SourceUID physical lookup plan")
    }

    #[test]
    fn schema_40_to_41_lookup_index_preserves_all_state_results_and_query_order() {
        let path = test_db_path("schema41-lookup-equivalence");
        let db = Database::open(&path).expect("create Schema41 source identity database");
        let conn = db.conn().expect("open source identity connection");
        seed_physical_lookup_row(
            &conn,
            "uid-a-retired",
            "retired",
            Some("object"),
            Some("digest-a"),
            10,
        );
        seed_physical_lookup_row(
            &conn,
            "uid-z-verified",
            "verified",
            Some("object"),
            Some("digest-a"),
            10,
        );
        seed_physical_lookup_row(
            &conn,
            "uid-b-conflicted",
            "conflicted",
            Some("object"),
            Some("digest-b"),
            11,
        );
        seed_physical_lookup_row(
            &conn,
            "uid-d-retired",
            "retired",
            Some("object"),
            Some("digest-b"),
            11,
        );
        seed_physical_lookup_row(
            &conn,
            "uid-c-unverified",
            "unverified",
            Some("object"),
            None,
            12,
        );
        seed_physical_lookup_row(
            &conn,
            "uid-other-object",
            "verified",
            Some("other-object"),
            Some("digest-other"),
            1,
        );
        seed_physical_lookup_row(&conn, "uid-null-identity", "unverified", None, None, 0);

        crate::db::schema_global_generations::drop_for_test(&conn);
        conn.execute_batch(
            "DROP INDEX idx_library_source_physical_lookup; PRAGMA user_version = 40;",
        )
        .expect("construct exact Schema40 lookup fixture");
        let before_plan = physical_lookup_plan(&conn);
        let before_rows = physical_lookup_rows(&conn);
        assert!(before_plan
            .iter()
            .any(|detail| { detail.contains("SCAN library_source_incarnations") }));
        assert!(before_plan
            .iter()
            .any(|detail| detail.contains("USE TEMP B-TREE FOR ORDER BY")));
        assert_eq!(
            before_rows,
            vec![
                (
                    "uid-a-retired".to_string(),
                    Some("digest-a".to_string()),
                    "retired".to_string(),
                ),
                (
                    "uid-z-verified".to_string(),
                    Some("digest-a".to_string()),
                    "verified".to_string(),
                ),
                (
                    "uid-b-conflicted".to_string(),
                    Some("digest-b".to_string()),
                    "conflicted".to_string(),
                ),
                (
                    "uid-d-retired".to_string(),
                    Some("digest-b".to_string()),
                    "retired".to_string(),
                ),
                (
                    "uid-c-unverified".to_string(),
                    None,
                    "unverified".to_string(),
                ),
            ]
        );
        drop(conn);
        drop(db);

        let migrated = Database::open(&path).expect("migrate Schema40 lookup fixture to 41");
        let conn = migrated.conn().expect("inspect Schema41 lookup fixture");
        let after_plan = physical_lookup_plan(&conn);
        let after_rows = physical_lookup_rows(&conn);
        assert_eq!(after_rows, before_rows);
        assert!(after_plan
            .iter()
            .any(|detail| { detail.contains("USING INDEX idx_library_source_physical_lookup") }));
        assert!(!after_plan
            .iter()
            .any(|detail| detail.contains("USE TEMP B-TREE FOR ORDER BY")));

        let same_digest_duplicate = conn.execute(
            "INSERT INTO library_source_incarnations (
                source_uid, verification_state, physical_platform, physical_volume_id,
                physical_object_id, verification_digest, created_at, updated_at
             ) VALUES ('uid-duplicate-verified', 'verified', 'unix', 'volume', 'object', 'digest-a', 20, 20)",
            [],
        );
        assert!(same_digest_duplicate.is_err());
        conn.execute(
            "INSERT INTO library_source_incarnations (
                source_uid, verification_state, physical_platform, physical_volume_id,
                physical_object_id, verification_digest, created_at, updated_at
             ) VALUES ('uid-distinct-verified', 'verified', 'unix', 'volume', 'object', 'digest-distinct', 21, 21)",
            [],
        )
        .expect("verified uniqueness index permits a distinct digest");
        drop(conn);
        drop(migrated);
        let _ = std::fs::remove_file(path);
    }

    fn seed_root(db: &Database, root_id: &str, path: &str) {
        db.conn()
            .expect("open root fixture connection")
            .execute(
                "INSERT INTO scan_roots (id, normalized_path, display_name, source_kind, enabled, health_status, needs_reconciliation, created_at, updated_at) VALUES (?1, ?2, ?3, 'file_library', 1, 'healthy', 0, 1, 1)",
                params![root_id, path, root_id],
            )
            .expect("insert source identity root");
    }

    fn file_request(path: &str) -> InsertFileRequest {
        InsertFileRequest {
            id: path.to_string(),
            path: path.to_string(),
            name: "identity.txt".to_string(),
            extension: "txt".to_string(),
            size: 1,
            mtime: 1,
            ctime: 1,
            is_dir: false,
            state_code: 0,
        }
    }

    fn physical_identity(created_ns: i64) -> PhysicalFileIdentity {
        PhysicalFileIdentity {
            size: 1,
            modified_ns: Some(1),
            created_ns: Some(created_ns),
            platform_kind: PhysicalPlatform::Unix,
            platform_volume_id: Some("volume".to_string()),
            platform_file_id: Some("object".to_string()),
            physical_key: Some("unix:v1:volume:object".to_string()),
            link_count: Some(1),
        }
    }

    #[test]
    fn cross_root_path_fallback_preserves_replacement_conflicts() {
        let db = test_db("cross-root-fallback");
        seed_root(&db, "parent", "/managed");
        seed_root(&db, "child", "/managed/nested");
        let mut conn = db.conn().expect("identity connection");
        let tx = conn.transaction().expect("identity transaction");
        let mut file = file_request("/managed/nested/identity.txt");
        let first_epoch = advance_root_epoch_tx(&tx, "parent", "scan", 1, 10).unwrap();
        let first = observe_source_for_root_tx(
            &tx,
            &file,
            Some(&physical_identity(100)),
            SourceObservationContext {
                root_id: "parent",
                root_epoch: first_epoch,
                owner_kind: "scan",
                owner_revision: 1,
                legacy_row_exists: false,
                observed_at: 10,
            },
        )
        .unwrap();
        // This root has neither a prior path claim nor a legacy attribution.
        // Only the cross-root fallback can discover the replaced source.
        file.id = "new-legacy-row".to_string();
        let mut replacement = physical_identity(200);
        replacement.platform_file_id = Some("replacement-object".to_string());
        let second_epoch = advance_root_epoch_tx(&tx, "child", "scan", 1, 11).unwrap();
        let second = observe_source_for_root_tx(
            &tx,
            &file,
            Some(&replacement),
            SourceObservationContext {
                root_id: "child",
                root_epoch: second_epoch,
                owner_kind: "scan",
                owner_revision: 1,
                legacy_row_exists: false,
                observed_at: 11,
            },
        )
        .unwrap();
        assert_ne!(first.source_uid, second.source_uid);
        let old_claim: String = tx
            .query_row(
                "SELECT claim_state FROM library_source_root_claims WHERE root_id = 'parent' AND source_uid = ?1",
                params![first.source_uid],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(old_claim, "conflicted");
        tx.commit().unwrap();
    }

    #[test]
    fn unavailable_and_reused_native_identity_remain_non_actionable() {
        let db = test_db("unverified");
        seed_root(&db, "unverified-root", "/managed");
        let conn = db.conn().expect("open unverified writer connection");
        let tx = conn
            .unchecked_transaction()
            .expect("start identity transaction");
        let root_epoch = advance_root_epoch_tx(&tx, "unverified-root", "scan", 1, 10)
            .expect("advance unverified root epoch");
        let unavailable = observe_source_for_root_tx(
            &tx,
            &file_request("/managed/identity.txt"),
            None,
            SourceObservationContext {
                root_id: "unverified-root",
                root_epoch,
                owner_kind: "scan",
                owner_revision: 1,
                legacy_row_exists: true,
                observed_at: 10,
            },
        )
        .expect("write unverified source observation");
        assert!(unavailable.block_legacy_row);
        let unavailable_state: String = tx
            .query_row(
                "SELECT verification_state FROM library_source_incarnations WHERE source_uid = ?1",
                params![unavailable.source_uid],
                |row| row.get(0),
            )
            .expect("read unavailable identity state");
        assert_eq!(unavailable_state, "unverified");
        let retry_epoch = advance_root_epoch_tx(&tx, "unverified-root", "watcher", 2, 11)
            .expect("advance repeated unverified root epoch");
        let retry = observe_source_for_root_tx(
            &tx,
            &file_request("/managed/identity.txt"),
            None,
            SourceObservationContext {
                root_id: "unverified-root",
                root_epoch: retry_epoch,
                owner_kind: "watcher",
                owner_revision: 2,
                legacy_row_exists: true,
                observed_at: 11,
            },
        )
        .expect("retain unresolved identity handle");
        assert_eq!(retry.source_uid, unavailable.source_uid);
        assert!(
            !retry.changed,
            "unchanged unverified observation is idempotent"
        );
        let incarnation_count: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM library_source_incarnations WHERE verification_state = 'unverified'",
                [],
                |row| row.get(0),
            )
            .expect("count unresolved incarnations");
        assert_eq!(incarnation_count, 1);
        tx.commit()
            .expect("commit unavailable identity observation");
        drop(conn);

        let db = test_db("reused-object-id");
        seed_root(&db, "conflict-root", "/managed");
        let mut conn = db.conn().expect("open conflict writer connection");
        let tx = conn
            .transaction()
            .expect("start conflict identity transaction");
        let first_epoch = advance_root_epoch_tx(&tx, "conflict-root", "scan", 1, 20)
            .expect("advance initial conflict epoch");
        let first = observe_source_for_root_tx(
            &tx,
            &file_request("/managed/identity.txt"),
            Some(&physical_identity(100)),
            SourceObservationContext {
                root_id: "conflict-root",
                root_epoch: first_epoch,
                owner_kind: "scan",
                owner_revision: 1,
                legacy_row_exists: false,
                observed_at: 20,
            },
        )
        .expect("write initial verified identity");
        let replacement_epoch = advance_root_epoch_tx(&tx, "conflict-root", "watcher", 2, 21)
            .expect("advance reused identity epoch");
        let replacement = observe_source_for_root_tx(
            &tx,
            &file_request("/managed/identity.txt"),
            Some(&physical_identity(200)),
            SourceObservationContext {
                root_id: "conflict-root",
                root_epoch: replacement_epoch,
                owner_kind: "watcher",
                owner_revision: 2,
                legacy_row_exists: true,
                observed_at: 21,
            },
        )
        .expect("reject recycled native object id");
        assert!(replacement.block_legacy_row);
        assert_ne!(replacement.source_uid, first.source_uid);
        let states: Vec<String> = tx
            .prepare(
                "SELECT verification_state FROM library_source_incarnations WHERE physical_platform = 'unix' AND physical_volume_id = 'volume' AND physical_object_id = 'object' ORDER BY source_uid",
            )
            .expect("prepare conflicting identity states")
            .query_map([], |row| row.get(0))
            .expect("query conflicting identity states")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect conflicting identity states");
        assert_eq!(states, vec!["conflicted", "conflicted"]);
        let claim_state: String = tx
            .query_row(
                "SELECT claim_state FROM library_source_root_claims WHERE root_id = 'conflict-root' AND source_uid = ?1 AND observed_path = '/managed/identity.txt'",
                params![replacement.source_uid],
                |row| row.get(0),
            )
            .expect("read conflicting root claim");
        assert_eq!(claim_state, "conflicted");
        let unavailable_retry_epoch = advance_root_epoch_tx(&tx, "conflict-root", "watcher", 3, 22)
            .expect("advance unavailable retry epoch");
        let unavailable_retry = observe_source_for_root_tx(
            &tx,
            &file_request("/managed/identity.txt"),
            None,
            SourceObservationContext {
                root_id: "conflict-root",
                root_epoch: unavailable_retry_epoch,
                owner_kind: "watcher",
                owner_revision: 3,
                legacy_row_exists: true,
                observed_at: 22,
            },
        )
        .expect("retain conflicted identity when native identity is unavailable");
        assert_eq!(unavailable_retry.source_uid, replacement.source_uid);
        assert!(unavailable_retry.block_legacy_row);
        let retry_claim_state: String = tx
            .query_row(
                "SELECT claim_state FROM library_source_root_claims WHERE root_id = 'conflict-root' AND source_uid = ?1 AND observed_path = '/managed/identity.txt'",
                params![replacement.source_uid],
                |row| row.get(0),
            )
            .expect("read retry claim state");
        assert_eq!(retry_claim_state, "conflicted");
        tx.commit()
            .expect("commit conflicting identity observation");
    }
}
