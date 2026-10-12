-- D4-S5 disposable SQL prototype only. Not product DDL and not a Schema version.
--
-- D4-S4's global clock remains present as a historical diagnostic counter but
-- is deliberately not part of D4-S5 current visibility. A root authority
-- revision invalidates only that root; a source authority revision invalidates
-- every root membership which depends on that same source incarnation.

CREATE TABLE d4s5_root_authority (
    root_id TEXT PRIMARY KEY REFERENCES scan_roots(id) ON DELETE RESTRICT,
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1)
);

CREATE TABLE d4s5_source_authority (
    source_id TEXT PRIMARY KEY REFERENCES file_sources(source_id) ON DELETE RESTRICT,
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1)
);

-- One event ID is one logical mutation of one authority scope. A claim revoke
-- may update file_sources and append the revoke row, but it advances the
-- source-scoped revision once because both DML statements share this event ID.
CREATE TABLE d4s5_authority_events (
    event_id TEXT PRIMARY KEY,
    domain TEXT NOT NULL CHECK (domain IN ('root', 'source')),
    object_id TEXT NOT NULL,
    revision_before INTEGER NOT NULL CHECK (revision_before >= 1),
    revision_after INTEGER NOT NULL CHECK (revision_after = revision_before + 1),
    source_revision_after INTEGER,
    CHECK ((domain = 'source' AND source_revision_after IS NOT NULL)
        OR (domain = 'root' AND source_revision_after IS NULL)),
    UNIQUE (domain, object_id, revision_after)
);

-- Immutable publication-time scopes. The per-root seal is distinct from the
-- source seal so overlapping roots can independently retain membership while
-- depending on the same immutable source identity evidence.
CREATE TABLE d4s5_manifest_root_seals (
    manifest_id TEXT NOT NULL,
    root_id TEXT NOT NULL,
    generation_id TEXT NOT NULL,
    root_authority_revision INTEGER NOT NULL CHECK (root_authority_revision >= 1),
    root_revision_at_publish INTEGER NOT NULL,
    watcher_revision_at_publish INTEGER NOT NULL,
    PRIMARY KEY (manifest_id, root_id),
    FOREIGN KEY (manifest_id, root_id)
        REFERENCES manifest_root_heads(manifest_id, root_id) ON DELETE RESTRICT,
    FOREIGN KEY (generation_id, root_id)
        REFERENCES source_generations(generation_id, root_id) ON DELETE RESTRICT
);

CREATE TABLE d4s5_manifest_source_seals (
    manifest_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    path_entry_id TEXT NOT NULL,
    source_authority_revision INTEGER NOT NULL CHECK (source_authority_revision >= 1),
    source_revision_at_publish INTEGER NOT NULL,
    PRIMARY KEY (manifest_id, source_id, path_entry_id),
    FOREIGN KEY (manifest_id, source_id, path_entry_id)
        REFERENCES d4s4_manifest_source_evidence(manifest_id, source_id, path_entry_id)
        ON DELETE RESTRICT
);

CREATE INDEX idx_d4s5_root_seals_root
    ON d4s5_manifest_root_seals(root_id, manifest_id, generation_id);
CREATE INDEX idx_d4s5_source_seals_source
    ON d4s5_manifest_source_seals(source_id, path_entry_id, manifest_id);

CREATE TRIGGER d4s5_root_authority_insert_guard
BEFORE INSERT ON d4s5_root_authority
WHEN d4s5_publish_authorized() <> 1
BEGIN
    SELECT RAISE(ABORT, 'd4s5: root authority bootstrap requires trusted publication');
END;
CREATE TRIGGER d4s5_root_authority_update_guard
BEFORE UPDATE ON d4s5_root_authority
WHEN d4s5_evidence_authorized() <> 1
  OR NEW.revision <> OLD.revision + 1
  OR NOT EXISTS (
      SELECT 1 FROM d4s5_authority_events e
      WHERE e.event_id = d4s5_event_id()
        AND e.domain = 'root' AND e.object_id = OLD.root_id
        AND e.revision_before = OLD.revision AND e.revision_after = NEW.revision
  )
BEGIN
    SELECT RAISE(ABORT, 'd4s5: root revision requires one matching authority event');
END;
CREATE TRIGGER d4s5_root_authority_delete_guard
BEFORE DELETE ON d4s5_root_authority
BEGIN
    SELECT RAISE(ABORT, 'd4s5: root authority is retained');
END;

CREATE TRIGGER d4s5_source_authority_insert_guard
BEFORE INSERT ON d4s5_source_authority
WHEN d4s5_publish_authorized() <> 1
BEGIN
    SELECT RAISE(ABORT, 'd4s5: source authority bootstrap requires trusted publication');
END;
CREATE TRIGGER d4s5_source_authority_update_guard
BEFORE UPDATE ON d4s5_source_authority
WHEN d4s5_evidence_authorized() <> 1
  OR NEW.revision <> OLD.revision + 1
  OR NOT EXISTS (
      SELECT 1 FROM d4s5_authority_events e
      WHERE e.event_id = d4s5_event_id()
        AND e.domain = 'source' AND e.object_id = OLD.source_id
        AND e.revision_before = OLD.revision AND e.revision_after = NEW.revision
        AND e.source_revision_after = (
            SELECT source_revision FROM file_sources WHERE source_id = OLD.source_id
        )
  )
BEGIN
    SELECT RAISE(ABORT, 'd4s5: source revision requires one matching authority event');
END;
CREATE TRIGGER d4s5_source_authority_delete_guard
BEFORE DELETE ON d4s5_source_authority
BEGIN
    SELECT RAISE(ABORT, 'd4s5: source authority is retained');
END;

CREATE TRIGGER d4s5_event_insert_guard
BEFORE INSERT ON d4s5_authority_events
WHEN d4s5_evidence_authorized() <> 1
  OR d4s5_event_id() IS NULL
  OR NEW.event_id <> d4s5_event_id()
BEGIN
    SELECT RAISE(ABORT, 'd4s5: authority events require the scoped mutation entry point');
END;
CREATE TRIGGER d4s5_event_update_guard
BEFORE UPDATE ON d4s5_authority_events
BEGIN
    SELECT RAISE(ABORT, 'd4s5: authority events are immutable');
END;
CREATE TRIGGER d4s5_event_delete_guard
BEFORE DELETE ON d4s5_authority_events
BEGIN
    SELECT RAISE(ABORT, 'd4s5: authority events are retained');
END;

CREATE TRIGGER d4s5_root_seal_insert_guard
BEFORE INSERT ON d4s5_manifest_root_seals
WHEN d4s5_publish_authorized() <> 1 OR d4s4_publish_authorized() <> 1
BEGIN
    SELECT RAISE(ABORT, 'd4s5: root seals require the trusted publication transaction');
END;
CREATE TRIGGER d4s5_root_seal_update_guard
BEFORE UPDATE ON d4s5_manifest_root_seals
BEGIN
    SELECT RAISE(ABORT, 'd4s5: published root seals are immutable');
END;
CREATE TRIGGER d4s5_root_seal_delete_guard
BEFORE DELETE ON d4s5_manifest_root_seals
BEGIN
    SELECT RAISE(ABORT, 'd4s5: published root seals are retained');
END;

CREATE TRIGGER d4s5_source_seal_insert_guard
BEFORE INSERT ON d4s5_manifest_source_seals
WHEN d4s5_publish_authorized() <> 1 OR d4s4_publish_authorized() <> 1
BEGIN
    SELECT RAISE(ABORT, 'd4s5: source seals require the trusted publication transaction');
END;
CREATE TRIGGER d4s5_source_seal_update_guard
BEFORE UPDATE ON d4s5_manifest_source_seals
BEGIN
    SELECT RAISE(ABORT, 'd4s5: published source seals are immutable');
END;
CREATE TRIGGER d4s5_source_seal_delete_guard
BEFORE DELETE ON d4s5_manifest_source_seals
BEGIN
    SELECT RAISE(ABORT, 'd4s5: published source seals are retained');
END;

-- The AFTER trigger runs inside the same transaction as D4-S4 publication.
-- The seal and pointer therefore cannot become visible separately.
CREATE TRIGGER d4s5_seal_manifest_on_publish
AFTER UPDATE OF status ON visibility_manifests
WHEN NEW.status = 'published' AND OLD.status <> 'published'
 AND d4s4_publish_authorized() = 1 AND d4s5_publish_authorized() = 1
BEGIN
    INSERT OR IGNORE INTO d4s5_root_authority(root_id, revision)
    SELECT root_id, 1 FROM manifest_root_heads WHERE manifest_id = NEW.manifest_id;

    INSERT OR IGNORE INTO d4s5_source_authority(source_id, revision)
    SELECT source_id, 1 FROM d4s4_manifest_source_evidence
    WHERE manifest_id = NEW.manifest_id;

    INSERT INTO d4s5_manifest_root_seals(
        manifest_id, root_id, generation_id, root_authority_revision,
        root_revision_at_publish, watcher_revision_at_publish
    )
    SELECT NEW.manifest_id, mr.root_id, mr.generation_id, ra.revision,
           re.root_revision, re.watcher_revision
    FROM manifest_root_heads mr
    JOIN d4s5_root_authority ra ON ra.root_id = mr.root_id
    JOIN d4s4_manifest_root_evidence re
      ON re.manifest_id = mr.manifest_id AND re.root_id = mr.root_id
    WHERE mr.manifest_id = NEW.manifest_id;

    INSERT INTO d4s5_manifest_source_seals(
        manifest_id, source_id, path_entry_id, source_authority_revision,
        source_revision_at_publish
    )
    SELECT se.manifest_id, se.source_id, se.path_entry_id, sa.revision,
           se.source_revision
    FROM d4s4_manifest_source_evidence se
    JOIN d4s5_source_authority sa ON sa.source_id = se.source_id
    WHERE se.manifest_id = NEW.manifest_id;
END;

CREATE TRIGGER d4s5_file_source_mutation_guard
BEFORE UPDATE OF status, identity_confidence, source_revision, retired_at ON file_sources
WHEN EXISTS (
    SELECT 1 FROM active_visibility_pointer p
    JOIN manifest_file_paths mfp ON mfp.manifest_id = p.manifest_id
    WHERE mfp.source_id = OLD.source_id
)
 AND (
    NEW.status IS NOT OLD.status
    OR NEW.identity_confidence IS NOT OLD.identity_confidence
    OR NEW.source_revision IS NOT OLD.source_revision
    OR NEW.retired_at IS NOT OLD.retired_at
 )
 AND (
    d4s4_evidence_change_authorized() <> 1
    OR d4s5_evidence_authorized() <> 1
    OR d4s5_event_id() IS NULL
    OR EXISTS (SELECT 1 FROM d4s5_authority_events WHERE event_id = d4s5_event_id())
 )
BEGIN
    SELECT RAISE(ABORT, 'd4s5: source evidence mutation requires a fresh scoped event');
END;

CREATE TRIGGER d4s5_file_source_revision_step
BEFORE UPDATE OF status, identity_confidence, source_revision, retired_at ON file_sources
WHEN (
    NEW.status IS NOT OLD.status
    OR NEW.identity_confidence IS NOT OLD.identity_confidence
    OR NEW.source_revision IS NOT OLD.source_revision
    OR NEW.retired_at IS NOT OLD.retired_at
 ) AND NEW.source_revision <> OLD.source_revision + 1
BEGIN
    SELECT RAISE(ABORT, 'd4s5: source evidence changes advance source_revision exactly once');
END;

CREATE TRIGGER d4s5_file_source_advance_scope
AFTER UPDATE OF status, identity_confidence, source_revision, retired_at ON file_sources
WHEN (
    NEW.status IS NOT OLD.status
    OR NEW.identity_confidence IS NOT OLD.identity_confidence
    OR NEW.source_revision IS NOT OLD.source_revision
    OR NEW.retired_at IS NOT OLD.retired_at
 ) AND d4s4_evidence_change_authorized() = 1
  AND d4s5_evidence_authorized() = 1
  AND NOT EXISTS (SELECT 1 FROM d4s5_authority_events WHERE event_id = d4s5_event_id())
BEGIN
    INSERT INTO d4s5_authority_events(
        event_id, domain, object_id, revision_before, revision_after, source_revision_after
    )
    SELECT d4s5_event_id(), 'source', NEW.source_id, revision, revision + 1, NEW.source_revision
    FROM d4s5_source_authority WHERE source_id = NEW.source_id;

    UPDATE d4s5_source_authority SET revision = revision + 1
    WHERE source_id = NEW.source_id;
END;

CREATE TRIGGER d4s5_root_mutation_guard
BEFORE UPDATE OF normalized_path, source_kind, enabled, health_status,
                 needs_reconciliation, watcher_revision, watcher_applied_revision ON scan_roots
WHEN EXISTS (
    SELECT 1 FROM active_visibility_pointer p
    JOIN manifest_root_heads mr ON mr.manifest_id = p.manifest_id
    WHERE mr.root_id = OLD.id
)
 AND (
    NEW.normalized_path IS NOT OLD.normalized_path
    OR NEW.source_kind IS NOT OLD.source_kind
    OR NEW.enabled IS NOT OLD.enabled
    OR NEW.health_status IS NOT OLD.health_status
    OR NEW.needs_reconciliation IS NOT OLD.needs_reconciliation
    OR NEW.watcher_revision IS NOT OLD.watcher_revision
    OR NEW.watcher_applied_revision IS NOT OLD.watcher_applied_revision
 )
 AND (
    d4s4_evidence_change_authorized() <> 1
    OR d4s5_evidence_authorized() <> 1
    OR d4s5_event_id() IS NULL
    OR EXISTS (SELECT 1 FROM d4s5_authority_events WHERE event_id = d4s5_event_id())
 )
BEGIN
    SELECT RAISE(ABORT, 'd4s5: root evidence mutation requires a fresh scoped event');
END;

CREATE TRIGGER d4s5_root_advance_scope
AFTER UPDATE OF normalized_path, source_kind, enabled, health_status,
                needs_reconciliation, watcher_revision, watcher_applied_revision ON scan_roots
WHEN (
    NEW.normalized_path IS NOT OLD.normalized_path
    OR NEW.source_kind IS NOT OLD.source_kind
    OR NEW.enabled IS NOT OLD.enabled
    OR NEW.health_status IS NOT OLD.health_status
    OR NEW.needs_reconciliation IS NOT OLD.needs_reconciliation
    OR NEW.watcher_revision IS NOT OLD.watcher_revision
    OR NEW.watcher_applied_revision IS NOT OLD.watcher_applied_revision
 ) AND d4s4_evidence_change_authorized() = 1
  AND d4s5_evidence_authorized() = 1
  AND NOT EXISTS (SELECT 1 FROM d4s5_authority_events WHERE event_id = d4s5_event_id())
BEGIN
    INSERT INTO d4s5_authority_events(
        event_id, domain, object_id, revision_before, revision_after, source_revision_after
    )
    SELECT d4s5_event_id(), 'root', NEW.id, revision, revision + 1, NULL
    FROM d4s5_root_authority WHERE root_id = NEW.id;

    UPDATE d4s5_root_authority SET revision = revision + 1
    WHERE root_id = NEW.id;
END;

-- A published identity claim can only be added/revoked after the same source
-- event has advanced source_revision and its scoped authority revision.
CREATE TRIGGER d4s5_claim_insert_guard
BEFORE INSERT ON source_identity_claims
WHEN EXISTS (
    SELECT 1 FROM active_visibility_pointer p
    JOIN manifest_file_paths mfp ON mfp.manifest_id = p.manifest_id
    WHERE mfp.source_id = NEW.source_id
)
 AND (
    d4s4_evidence_change_authorized() <> 1
    OR d4s5_evidence_authorized() <> 1
    OR NOT EXISTS (
        SELECT 1 FROM d4s5_authority_events e
        JOIN file_sources fs ON fs.source_id = e.object_id
        JOIN d4s5_source_authority sa ON sa.source_id = e.object_id
        WHERE e.event_id = d4s5_event_id() AND e.domain = 'source'
          AND e.object_id = NEW.source_id
          AND e.source_revision_after = fs.source_revision
          AND e.revision_after = sa.revision
    )
 )
BEGIN
    SELECT RAISE(ABORT, 'd4s5: published claim insertion requires matching source revision event');
END;

CREATE TRIGGER d4s5_claim_revocation_guard
BEFORE INSERT ON d4s4_claim_revocations
WHEN NOT EXISTS (
    SELECT 1 FROM source_identity_claims sic
    WHERE sic.claim_id = NEW.claim_id
)
 OR (
    EXISTS (
        SELECT 1 FROM active_visibility_pointer p
        JOIN manifest_file_paths mfp ON mfp.manifest_id = p.manifest_id
        JOIN source_identity_claims sic ON sic.source_id = mfp.source_id
        WHERE sic.claim_id = NEW.claim_id
    )
    AND (
        d4s4_evidence_change_authorized() <> 1
        OR d4s5_evidence_authorized() <> 1
        OR NOT EXISTS (
            SELECT 1 FROM source_identity_claims sic
            JOIN file_sources fs ON fs.source_id = sic.source_id
            JOIN d4s5_authority_events e ON e.object_id = sic.source_id
            JOIN d4s5_source_authority sa ON sa.source_id = sic.source_id
            WHERE sic.claim_id = NEW.claim_id
              AND e.event_id = d4s5_event_id() AND e.domain = 'source'
              AND e.source_revision_after = fs.source_revision
              AND e.revision_after = sa.revision
        )
    )
 )
BEGIN
    SELECT RAISE(ABORT, 'd4s5: published claim revocation requires matching source event');
END;

-- Per-root authority view. It intentionally reads the relation-level root
-- heads and members instead of trusting the union path projection alone.
CREATE VIEW d4s5_effective_root_members AS
SELECT DISTINCT
    p.manifest_id, p.revision AS manifest_revision, mr.root_id, mr.generation_id,
    gm.source_id, gm.path_entry_id, mfp.path_key, mfp.projected_path,
    fs.file_id, rs.root_authority_revision, ra.revision AS current_root_authority_revision,
    ss.source_authority_revision, sa.revision AS current_source_authority_revision
FROM active_visibility_pointer p
JOIN manifest_root_heads mr ON mr.manifest_id = p.manifest_id
JOIN source_generations sg
  ON sg.generation_id = mr.generation_id AND sg.root_id = mr.root_id
JOIN source_generation_members gm ON gm.generation_id = mr.generation_id
JOIN manifest_file_paths mfp
  ON mfp.manifest_id = p.manifest_id
 AND mfp.source_id = gm.source_id AND mfp.path_entry_id = gm.path_entry_id
JOIN file_sources fs ON fs.source_id = gm.source_id
JOIN files f ON f.id = fs.file_id AND f.path = mfp.projected_path
JOIN file_path_entries pe
  ON pe.path_entry_id = gm.path_entry_id AND pe.source_id = gm.source_id
JOIN file_physical_identity_epochs pi
  ON pi.physical_identity_id = pe.physical_identity_id
JOIN scan_roots r ON r.id = mr.root_id
JOIN scan_runs sr ON sr.id = sg.run_id AND sr.scan_root_id = mr.root_id
JOIN d4s4_manifest_root_evidence re
  ON re.manifest_id = p.manifest_id AND re.root_id = mr.root_id
JOIN d4s5_manifest_root_seals rs
  ON rs.manifest_id = p.manifest_id AND rs.root_id = mr.root_id
 AND rs.generation_id = mr.generation_id
JOIN d4s5_root_authority ra ON ra.root_id = mr.root_id
JOIN d4s4_manifest_source_evidence se
  ON se.manifest_id = p.manifest_id
 AND se.source_id = gm.source_id AND se.path_entry_id = gm.path_entry_id
JOIN d4s5_manifest_source_seals ss
  ON ss.manifest_id = se.manifest_id
 AND ss.source_id = se.source_id AND ss.path_entry_id = se.path_entry_id
JOIN d4s5_source_authority sa ON sa.source_id = fs.source_id
WHERE rs.root_authority_revision = ra.revision
  AND ss.source_authority_revision = sa.revision
  AND ss.source_revision_at_publish = fs.source_revision
  AND se.source_revision = fs.source_revision
  AND se.source_status = fs.status AND fs.status = 'observed'
  AND se.identity_confidence = fs.identity_confidence
  AND fs.identity_confidence IN ('native_verified', 'user_claimed')
  AND sg.status = 'published'
  AND sr.status = 'completed' AND sr.coverage_complete = 1
  AND sr.watcher_revision_at_start = re.watcher_revision
  AND sg.watcher_revision = re.watcher_revision
  AND r.watcher_revision = re.watcher_revision
  AND r.watcher_applied_revision = re.watcher_revision
  AND r.normalized_path = re.normalized_path
  AND r.source_kind = re.source_kind AND r.enabled = re.enabled
  AND r.enabled = 1 AND r.health_status = 'healthy'
  AND re.health_status = 'healthy'
  AND r.needs_reconciliation = 0 AND re.needs_reconciliation = 0
  AND pe.identity_status = 'verified'
  AND pe.evidence_digest = gm.identity_digest
  AND pe.evidence_digest = se.path_evidence_digest
  AND pe.physical_identity_id = se.physical_identity_id
  AND pi.identity_epoch = se.identity_epoch
  AND pi.platform_file_identity = se.platform_file_identity
  AND pi.evidence_digest = se.physical_evidence_digest
  AND (fs.identity_confidence <> 'user_claimed' OR EXISTS (
      SELECT 1 FROM source_identity_claims sic
      WHERE sic.source_id = gm.source_id AND sic.path_entry_id = gm.path_entry_id
        AND sic.claim_kind = 'explicit_legacy_claim'
        AND NOT EXISTS (
            SELECT 1 FROM d4s4_claim_revocations cr WHERE cr.claim_id = sic.claim_id
        )
  ))
  AND NOT EXISTS (
      SELECT 1 FROM d4s4_claim_revocations cr
      JOIN source_identity_claims sic ON sic.claim_id = cr.claim_id
      WHERE sic.source_id = gm.source_id AND sic.path_entry_id = gm.path_entry_id
  );

-- All current read/search entry points in this prototype compose from the same
-- per-root relation. The projection remains a path lookup, not authority.
CREATE VIEW d4s5_effective_file_sources AS
SELECT DISTINCT manifest_id, manifest_revision, source_id, path_entry_id,
       path_key, projected_path, file_id
FROM d4s5_effective_root_members;

-- A diagnostic view shows stale union projection rows that have no eligible
-- current root authority; those rows are history, not readable current files.
CREATE VIEW d4s5_unqualified_projection_rows AS
SELECT mfp.manifest_id, mfp.source_id, mfp.path_entry_id, mfp.path_key
FROM active_visibility_pointer p
JOIN manifest_file_paths mfp ON mfp.manifest_id = p.manifest_id
WHERE NOT EXISTS (
    SELECT 1 FROM d4s5_effective_file_sources e
    WHERE e.manifest_id = mfp.manifest_id
      AND e.source_id = mfp.source_id AND e.path_entry_id = mfp.path_entry_id
);
