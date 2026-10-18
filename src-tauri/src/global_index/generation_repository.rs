//! Dormant Schema 42 candidate storage. No production caller or publish API.
#![allow(
    dead_code,
    reason = "P3-A storage is deliberately dormant until Owner-authorized P3-B"
)]

use super::models::{normalize_path, unix_now, GlobalEntryInput};
use super::repository::{ensure_source_operation_owner_tx, SourceOperationOwner};
use crate::db::{Database, DbError};
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};

pub(crate) const MAX_CANDIDATE_BATCH: usize = 256;

#[derive(Debug, Clone, Copy)]
pub(crate) enum CaptureKind {
    Initial,
    Rebuild,
    Reconcile,
}

impl CaptureKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Initial => "initial",
            Self::Rebuild => "rebuild",
            Self::Reconcile => "reconcile",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct GenerationCandidate {
    pub id: String,
    pub owner: SourceOperationOwner,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GenerationManifest {
    pub count: i64,
    pub digest: String,
}

fn invalid(message: &str) -> DbError {
    DbError::Validation(format!("global_index_candidate_{message}"))
}

fn encoded_entry(entry: &GlobalEntryInput) -> Result<String, DbError> {
    serde_json::to_string(entry).map_err(|_| invalid("metadata_encoding"))
}

// Domain-separated, length-framed, sorted logical IDs + complete canonical
// metadata. Callers supply the capture manifest; seal recomputes from SQLite.
fn add_digest(hasher: &mut blake3::Hasher, id: &str, json: &str) {
    for value in [id, json] {
        hasher.update(&(value.len() as u64).to_le_bytes());
        hasher.update(value.as_bytes());
    }
}

pub(crate) fn generation_manifest(
    entries: &[GlobalEntryInput],
) -> Result<GenerationManifest, DbError> {
    let mut rows = entries
        .iter()
        .map(|e| Ok((e.entry_id(), encoded_entry(e)?)))
        .collect::<Result<Vec<_>, DbError>>()?;
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    if rows.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(invalid("duplicate_identity"));
    }
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"zen-global-generation-manifest-v1\0");
    for (id, json) in &rows {
        add_digest(&mut hasher, id, json);
    }
    Ok(GenerationManifest {
        count: rows.len() as i64,
        digest: hasher.finalize().to_hex().to_string(),
    })
}

fn ensure_candidate(tx: &Transaction<'_>, candidate: &GenerationCandidate) -> Result<(), DbError> {
    ensure_source_operation_owner_tx(tx, &candidate.owner)?;
    let valid: bool = tx.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM global_index_generations g
            JOIN global_index_generation_heads h ON h.volume_id = g.volume_id
            JOIN global_volumes v ON v.id = g.volume_id
            WHERE g.generation_id = ?1 AND g.volume_id = ?2
              AND g.operation_revision = ?3 AND g.operation_owner_id = ?4
              AND h.candidate_generation_id = g.generation_id
              AND g.state = 'staging' AND g.stable_volume_id = v.stable_volume_id
              AND g.source_provider = v.provider)",
        params![
            candidate.id,
            candidate.owner.volume_id,
            candidate.owner.revision,
            candidate.owner.operation_id
        ],
        |r| r.get(0),
    )?;
    if !valid {
        return Err(invalid("not_current_staging_owner"));
    }
    Ok(())
}

impl Database {
    pub(crate) fn begin_global_index_candidate(
        &self,
        owner: &SourceOperationOwner,
        kind: CaptureKind,
    ) -> Result<GenerationCandidate, DbError> {
        let id = uuid::Uuid::new_v4().to_string();
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_source_operation_owner_tx(&tx, owner)?;
        tx.execute(
            "INSERT INTO global_index_generation_heads(volume_id) VALUES (?1)
             ON CONFLICT(volume_id) DO NOTHING",
            [&owner.volume_id],
        )?;
        let pending: Option<String> = tx.query_row(
            "SELECT candidate_generation_id FROM global_index_generation_heads WHERE volume_id = ?1",
            [&owner.volume_id], |r| r.get(0),
        )?;
        if pending.is_some() {
            return Err(invalid("recovery_required"));
        }
        // UNIQUE(source, operation_revision) forbids replay/ABA even after abort.
        tx.execute(
            "INSERT INTO global_index_generations(
                generation_id, volume_id, stable_volume_id, source_provider,
                operation_revision, operation_owner_id, rebuild_intent_revision,
                capture_kind, created_at)
             SELECT ?1, id, stable_volume_id, provider, operation_revision,
                    operation_owner_id, rebuild_intent_revision, ?5, ?6
             FROM global_volumes WHERE id = ?2 AND operation_revision = ?3
               AND operation_owner_id = ?4",
            params![
                id,
                owner.volume_id,
                owner.revision,
                owner.operation_id,
                kind.as_str(),
                unix_now()
            ],
        )?;
        tx.execute(
            "UPDATE global_index_generation_heads SET candidate_generation_id = ?2 WHERE volume_id = ?1",
            params![owner.volume_id, id],
        )?;
        tx.commit()?;
        Ok(GenerationCandidate {
            id,
            owner: owner.clone(),
        })
    }

    pub(crate) fn stage_global_index_candidate_batch(
        &self,
        candidate: &GenerationCandidate,
        entries: &[GlobalEntryInput],
    ) -> Result<(), DbError> {
        if entries.is_empty() || entries.len() > MAX_CANDIDATE_BATCH {
            return Err(invalid("batch_size"));
        }
        // Prepare metadata outside the bounded writer reservation.
        let encoded = entries
            .iter()
            .map(|entry| {
                if entry.volume_id != candidate.owner.volume_id {
                    return Err(invalid("foreign_volume"));
                }
                Ok((entry, entry.entry_id(), encoded_entry(entry)?))
            })
            .collect::<Result<Vec<_>, DbError>>()?;
        if encoded.iter().map(|(_, _, json)| json.len()).sum::<usize>() > 2 * 1024 * 1024 {
            return Err(invalid("batch_bytes"));
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_candidate(&tx, candidate)?;
        let provider: String = tx.query_row(
            "SELECT source_provider FROM global_index_generations WHERE generation_id = ?1",
            [&candidate.id],
            |r| r.get(0),
        )?;
        for (entry, id, json) in encoded {
            if entry.source_provider != provider {
                return Err(invalid("foreign_provider"));
            }
            tx.execute(
                "INSERT INTO global_index_generation_entries(
                    generation_id, logical_entry_id, name, name_normalized,
                    path, path_normalized, extension, metadata_json)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    candidate.id,
                    id,
                    entry.name,
                    entry.name.to_lowercase(),
                    entry.path,
                    normalize_path(&entry.path),
                    entry.extension,
                    json
                ],
            )?;
        }
        tx.execute(
            "UPDATE global_index_generations
             SET entry_count = entry_count + ?2,
                 materialization_revision = materialization_revision + 1
             WHERE generation_id = ?1",
            params![candidate.id, entries.len() as i64],
        )?;
        tx.commit()?;
        // Deliberately no legacy/Managed/AI writes and no work notification.
        Ok(())
    }

    pub(crate) fn seal_global_index_candidate(
        &self,
        candidate: &GenerationCandidate,
        expected: &GenerationManifest,
        journal_id: Option<&str>,
        journal_cursor: Option<&str>,
    ) -> Result<(), DbError> {
        // Materialization verification is an O(N) read, never an O(N) writer.
        // Concurrent appends change materialization_revision; CAS below rejects
        // an obsolete proof without retrying or holding the WAL writer during it.
        let (revision, count, digest) = {
            let mut conn = self.conn()?;
            let tx = conn.transaction()?;
            let (revision, claimed_count): (i64, i64) = tx.query_row(
                "SELECT materialization_revision, entry_count FROM global_index_generations WHERE generation_id = ?1",
                [&candidate.id], |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let mut hasher = blake3::Hasher::new();
            hasher.update(b"zen-global-generation-manifest-v1\0");
            let mut count = 0i64;
            let mut stmt = tx.prepare(
                "SELECT logical_entry_id, metadata_json FROM global_index_generation_entries
                 WHERE generation_id = ?1 ORDER BY logical_entry_id",
            )?;
            let mut rows = stmt.query([&candidate.id])?;
            while let Some(row) = rows.next()? {
                let id: String = row.get(0)?;
                let json: String = row.get(1)?;
                add_digest(&mut hasher, &id, &json);
                count += 1;
            }
            if count != claimed_count {
                return Err(invalid("materialized_count_mismatch"));
            }
            (revision, count, hasher.finalize().to_hex().to_string())
        };
        if count != expected.count || digest != expected.digest {
            return Err(invalid("manifest_mismatch"));
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_candidate(&tx, candidate)?;
        let changed = tx.execute(
            "UPDATE global_index_generations
             SET state = 'sealed', capture_complete = 1, manifest_digest = ?3,
                 journal_id = ?4, journal_cursor = ?5, sealed_at = ?6
             WHERE generation_id = ?1 AND materialization_revision = ?2 AND entry_count = ?7",
            params![
                candidate.id,
                revision,
                digest,
                journal_id,
                journal_cursor,
                unix_now(),
                count
            ],
        )?;
        if changed != 1 {
            return Err(invalid("materialization_changed"));
        }
        tx.commit()?;
        // Sealed != published. No live checkpoint, intent or owner is completed.
        Ok(())
    }

    pub(crate) fn abort_global_index_candidate(
        &self,
        candidate: &GenerationCandidate,
        reason: &str,
    ) -> Result<(), DbError> {
        if reason.is_empty() || reason.len() > 1024 {
            return Err(invalid("abort_reason"));
        }
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_candidate(&tx, candidate)?;
        tx.execute(
            "UPDATE global_index_generations SET state = 'aborted', abort_reason = ?2
             WHERE generation_id = ?1",
            params![candidate.id, reason],
        )?;
        tx.execute(
            "UPDATE global_index_generation_heads SET candidate_generation_id = NULL
             WHERE volume_id = ?1 AND candidate_generation_id = ?2",
            params![candidate.owner.volume_id, candidate.id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Only a newly admitted successor may detach an abandoned candidate.
    /// Service UNKNOWN/Direct and process liveness rules stay in the existing
    /// source claim; this API cannot grant admission or reclaim an owner.
    pub(crate) fn recover_global_index_candidate(
        &self,
        owner: &SourceOperationOwner,
    ) -> Result<bool, DbError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure_source_operation_owner_tx(&tx, owner)?;
        let prior: Option<(String, i64)> = tx
            .query_row(
                "SELECT g.generation_id, g.operation_revision
             FROM global_index_generation_heads h JOIN global_index_generations g
               ON g.generation_id = h.candidate_generation_id AND g.volume_id = h.volume_id
             WHERE h.volume_id = ?1",
                [&owner.volume_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((id, revision)) = prior else {
            return Ok(false);
        };
        if revision >= owner.revision {
            return Err(invalid("successor_required"));
        }
        tx.execute(
            "UPDATE global_index_generations SET state = 'aborted', abort_reason = 'superseded_owner'
             WHERE generation_id = ?1 AND state = 'staging'", [&id],
        )?;
        // Sealed metadata remains immutable; detaching does not publish it.
        tx.execute(
            "UPDATE global_index_generation_heads SET candidate_generation_id = NULL
             WHERE volume_id = ?1 AND candidate_generation_id = ?2",
            params![owner.volume_id, id],
        )?;
        tx.commit()?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests;
