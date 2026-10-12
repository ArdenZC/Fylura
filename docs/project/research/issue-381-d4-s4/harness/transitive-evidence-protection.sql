-- D4-S4 disposable SQLite prototype only. This is not production DDL.
-- Hashes are computed by the trusted Python publication Oracle while holding
-- BEGIN IMMEDIATE; SQLite triggers enforce relational guards and a connection
-- local authorization capability, but do not implement SHA-256.

CREATE TABLE d4s4_authority_clock (
    singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
    revision INTEGER NOT NULL CHECK (revision >= 1)
);
INSERT INTO d4s4_authority_clock(singleton_id, revision) VALUES (1, 1);

CREATE TABLE d4s4_manifest_authority_seals (
    manifest_id TEXT PRIMARY KEY REFERENCES visibility_manifests(manifest_id) ON DELETE RESTRICT,
    authority_revision INTEGER NOT NULL CHECK (authority_revision >= 1),
    root_count INTEGER NOT NULL CHECK (root_count >= 0),
    root_map_digest TEXT NOT NULL,
    member_count INTEGER NOT NULL CHECK (member_count >= 0),
    member_digest TEXT NOT NULL,
    source_count INTEGER NOT NULL CHECK (source_count >= 0),
    source_evidence_digest TEXT NOT NULL,
    sealed_at INTEGER NOT NULL
);

CREATE TABLE d4s4_manifest_root_evidence (
    manifest_id TEXT NOT NULL,
    root_id TEXT NOT NULL,
    generation_id TEXT NOT NULL,
    root_revision INTEGER NOT NULL,
    watcher_revision INTEGER NOT NULL,
    normalized_path TEXT NOT NULL,
    source_kind TEXT NOT NULL,
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    health_status TEXT NOT NULL,
    needs_reconciliation INTEGER NOT NULL CHECK (needs_reconciliation IN (0, 1)),
    PRIMARY KEY (manifest_id, root_id),
    FOREIGN KEY (manifest_id, root_id)
        REFERENCES manifest_root_heads(manifest_id, root_id) ON DELETE RESTRICT
);

CREATE TABLE d4s4_manifest_source_evidence (
    manifest_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    path_entry_id TEXT NOT NULL,
    source_revision INTEGER NOT NULL,
    source_status TEXT NOT NULL,
    identity_confidence TEXT NOT NULL,
    physical_identity_id TEXT NOT NULL,
    identity_epoch TEXT NOT NULL,
    platform_file_identity TEXT NOT NULL,
    physical_evidence_digest TEXT NOT NULL,
    path_evidence_digest TEXT NOT NULL,
    claim_ids_json TEXT NOT NULL,
    evidence_commitment TEXT NOT NULL,
    PRIMARY KEY (manifest_id, source_id, path_entry_id),
    FOREIGN KEY (manifest_id, source_id)
        REFERENCES manifest_file_paths(manifest_id, source_id) ON DELETE RESTRICT,
    FOREIGN KEY (path_entry_id, source_id)
        REFERENCES file_path_entries(path_entry_id, source_id) ON DELETE RESTRICT
);

-- A claim is retained as historical evidence. Revocation is an append-only
-- event; deleting the claim row is never the revocation operation.
CREATE TABLE d4s4_claim_revocations (
    claim_id TEXT PRIMARY KEY REFERENCES source_identity_claims(claim_id) ON DELETE RESTRICT,
    revoked_at INTEGER NOT NULL,
    reason TEXT NOT NULL
);

CREATE TRIGGER d4s4_clock_guard
BEFORE UPDATE ON d4s4_authority_clock
WHEN d4s4_evidence_change_authorized() <> 1
 AND d4s4_publish_authorized() <> 1
BEGIN
    SELECT RAISE(ABORT, 'd4s4: authority clock is service-owned');
END;

CREATE TRIGGER d4s4_manifest_seal_insert_guard
BEFORE INSERT ON d4s4_manifest_authority_seals
WHEN d4s4_publish_authorized() <> 1
BEGIN
    SELECT RAISE(ABORT, 'd4s4: authority seal requires trusted publication');
END;
CREATE TRIGGER d4s4_manifest_seal_update_guard
BEFORE UPDATE ON d4s4_manifest_authority_seals
BEGIN
    SELECT RAISE(ABORT, 'd4s4: authority seals are immutable');
END;
CREATE TRIGGER d4s4_manifest_seal_delete_guard
BEFORE DELETE ON d4s4_manifest_authority_seals
BEGIN
    SELECT RAISE(ABORT, 'd4s4: authority seals are retained');
END;

CREATE TRIGGER d4s4_root_evidence_insert_guard
BEFORE INSERT ON d4s4_manifest_root_evidence
WHEN d4s4_publish_authorized() <> 1
BEGIN
    SELECT RAISE(ABORT, 'd4s4: root evidence requires trusted publication');
END;
CREATE TRIGGER d4s4_manifest_root_evidence_update_guard
BEFORE UPDATE ON d4s4_manifest_root_evidence
BEGIN
    SELECT RAISE(ABORT, 'd4s4: root evidence is immutable');
END;
CREATE TRIGGER d4s4_root_evidence_delete_guard
BEFORE DELETE ON d4s4_manifest_root_evidence
BEGIN
    SELECT RAISE(ABORT, 'd4s4: root evidence is retained');
END;

CREATE TRIGGER d4s4_source_evidence_insert_guard
BEFORE INSERT ON d4s4_manifest_source_evidence
WHEN d4s4_publish_authorized() <> 1
BEGIN
    SELECT RAISE(ABORT, 'd4s4: source evidence requires trusted publication');
END;
CREATE TRIGGER d4s4_source_evidence_update_guard
BEFORE UPDATE ON d4s4_manifest_source_evidence
BEGIN
    SELECT RAISE(ABORT, 'd4s4: source evidence is immutable');
END;
CREATE TRIGGER d4s4_source_evidence_delete_guard
BEFORE DELETE ON d4s4_manifest_source_evidence
BEGIN
    SELECT RAISE(ABORT, 'd4s4: source evidence is retained');
END;

-- Terminal run coverage and status are immutable once a published Manifest
-- cites the run. A correction is a new run, never an in-place rewrite.
CREATE TRIGGER d4s4_published_run_coverage_immutable
BEFORE UPDATE OF status, coverage_complete, stale_reconciliation_allowed,
                 watcher_revision_at_start ON scan_runs
WHEN EXISTS (
    SELECT 1 FROM source_generations sg
    JOIN manifest_root_heads mr ON mr.generation_id = sg.generation_id
    JOIN visibility_manifests vm ON vm.manifest_id = mr.manifest_id
    WHERE sg.run_id = OLD.id AND vm.status = 'published'
)
 AND (
    NEW.status IS NOT OLD.status
    OR NEW.coverage_complete IS NOT OLD.coverage_complete
    OR NEW.stale_reconciliation_allowed IS NOT OLD.stale_reconciliation_allowed
    OR NEW.watcher_revision_at_start IS NOT OLD.watcher_revision_at_start
 )
BEGIN
    SELECT RAISE(ABORT, 'd4s4: published scan coverage evidence is immutable');
END;

-- Mutable source status/confidence is permitted only through the service's
-- explicit evidence-mutation path. It increments the authority clock, making
-- every cursor and current read based on the prior seal stale until republish.
CREATE TRIGGER d4s4_file_source_evidence_update_guard
BEFORE UPDATE OF status, identity_confidence, source_revision, retired_at ON file_sources
WHEN EXISTS (
    SELECT 1 FROM manifest_file_paths mfp
    JOIN visibility_manifests vm ON vm.manifest_id = mfp.manifest_id
    WHERE mfp.source_id = OLD.source_id AND vm.status = 'published'
)
 AND (
    NEW.status IS NOT OLD.status
    OR NEW.identity_confidence IS NOT OLD.identity_confidence
    OR NEW.source_revision IS NOT OLD.source_revision
    OR NEW.retired_at IS NOT OLD.retired_at
 )
 AND d4s4_evidence_change_authorized() <> 1
BEGIN
    SELECT RAISE(ABORT, 'd4s4: source evidence change requires authority invalidation');
END;

CREATE TRIGGER d4s4_file_source_revision_step
BEFORE UPDATE OF status, identity_confidence, retired_at ON file_sources
WHEN (
    NEW.status IS NOT OLD.status
    OR NEW.identity_confidence IS NOT OLD.identity_confidence
    OR NEW.retired_at IS NOT OLD.retired_at
 ) AND NEW.source_revision <> OLD.source_revision + 1
BEGIN
    SELECT RAISE(ABORT, 'd4s4: semantic source change must advance source_revision by one');
END;

CREATE TRIGGER d4s4_file_source_evidence_invalidate
AFTER UPDATE OF status, identity_confidence, source_revision, retired_at ON file_sources
WHEN d4s4_evidence_change_authorized() = 1
 AND (
    NEW.status IS NOT OLD.status
    OR NEW.identity_confidence IS NOT OLD.identity_confidence
    OR NEW.source_revision IS NOT OLD.source_revision
    OR NEW.retired_at IS NOT OLD.retired_at
 )
 AND EXISTS (
    SELECT 1 FROM active_visibility_pointer p
    JOIN manifest_file_paths mfp ON mfp.manifest_id = p.manifest_id
    WHERE mfp.source_id = OLD.source_id
 )
BEGIN
    UPDATE d4s4_authority_clock SET revision = revision + 1 WHERE singleton_id = 1;
END;

-- Root path/ownership/coverage and watcher progress are transitive evidence.
-- Operational scan lease revision alone is deliberately not used as a
-- continuous visibility version: it changes during ordinary run admission.
CREATE TRIGGER d4s4_root_evidence_update_guard
BEFORE UPDATE OF normalized_path, source_kind, enabled, health_status,
                 needs_reconciliation, watcher_revision,
                 watcher_applied_revision ON scan_roots
WHEN EXISTS (
    SELECT 1 FROM manifest_root_heads mr
    JOIN visibility_manifests vm ON vm.manifest_id = mr.manifest_id
    WHERE mr.root_id = OLD.id AND vm.status = 'published'
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
 AND d4s4_evidence_change_authorized() <> 1
BEGIN
    SELECT RAISE(ABORT, 'd4s4: root evidence change requires authority invalidation');
END;

CREATE TRIGGER d4s4_root_evidence_invalidate
AFTER UPDATE OF normalized_path, source_kind, enabled, health_status,
                needs_reconciliation, watcher_revision,
                watcher_applied_revision ON scan_roots
WHEN d4s4_evidence_change_authorized() = 1
 AND (
    NEW.normalized_path IS NOT OLD.normalized_path
    OR NEW.source_kind IS NOT OLD.source_kind
    OR NEW.enabled IS NOT OLD.enabled
    OR NEW.health_status IS NOT OLD.health_status
    OR NEW.needs_reconciliation IS NOT OLD.needs_reconciliation
    OR NEW.watcher_revision IS NOT OLD.watcher_revision
    OR NEW.watcher_applied_revision IS NOT OLD.watcher_applied_revision
 )
 AND EXISTS (
    SELECT 1 FROM active_visibility_pointer p
    JOIN manifest_root_heads mr ON mr.manifest_id = p.manifest_id
    WHERE mr.root_id = OLD.id
 )
BEGIN
    UPDATE d4s4_authority_clock SET revision = revision + 1 WHERE singleton_id = 1;
END;

-- Physical identity observations are append-only after path-entry creation.
-- A new incarnation/epoch adds new evidence; historical evidence is not edited.
CREATE TRIGGER d4s4_physical_identity_immutable_update
BEFORE UPDATE ON file_physical_identity_epochs
WHEN EXISTS (
    SELECT 1 FROM file_path_entries pe
    WHERE pe.physical_identity_id = OLD.physical_identity_id
)
BEGIN
    SELECT RAISE(ABORT, 'd4s4: referenced physical identity evidence is immutable');
END;
CREATE TRIGGER d4s4_physical_identity_immutable_delete
BEFORE DELETE ON file_physical_identity_epochs
WHEN EXISTS (
    SELECT 1 FROM file_path_entries pe
    WHERE pe.physical_identity_id = OLD.physical_identity_id
)
BEGIN
    SELECT RAISE(ABORT, 'd4s4: referenced physical identity evidence is retained');
END;

-- Claims are evidence records, not mutable flags. Revoke by appending a row to
-- d4s4_claim_revocations through the authority-mutation API.
CREATE TRIGGER d4s4_identity_claim_immutable_update
BEFORE UPDATE ON source_identity_claims
BEGIN
    SELECT RAISE(ABORT, 'd4s4: identity claims are immutable evidence');
END;
CREATE TRIGGER d4s4_identity_claim_insert_guard
BEFORE INSERT ON source_identity_claims
WHEN EXISTS (
    SELECT 1 FROM manifest_file_paths mfp
    JOIN visibility_manifests vm ON vm.manifest_id = mfp.manifest_id
    WHERE mfp.source_id = NEW.source_id AND vm.status = 'published'
 ) AND d4s4_evidence_change_authorized() <> 1
BEGIN
    SELECT RAISE(ABORT, 'd4s4: new claim requires authority invalidation');
END;
CREATE TRIGGER d4s4_identity_claim_immutable_delete
BEFORE DELETE ON source_identity_claims
BEGIN
    SELECT RAISE(ABORT, 'd4s4: identity claims are retained; append a revocation');
END;
CREATE TRIGGER d4s4_identity_claim_insert_invalidate
AFTER INSERT ON source_identity_claims
WHEN d4s4_evidence_change_authorized() = 1
 AND EXISTS (
    SELECT 1 FROM active_visibility_pointer p
    JOIN manifest_file_paths mfp ON mfp.manifest_id = p.manifest_id
    WHERE mfp.source_id = NEW.source_id
 )
BEGIN
    UPDATE d4s4_authority_clock SET revision = revision + 1 WHERE singleton_id = 1;
END;
CREATE TRIGGER d4s4_claim_revocation_insert_guard
BEFORE INSERT ON d4s4_claim_revocations
WHEN d4s4_evidence_change_authorized() <> 1
BEGIN
    SELECT RAISE(ABORT, 'd4s4: claim revocation requires authority invalidation');
END;
CREATE TRIGGER d4s4_claim_revocation_invalidate
AFTER INSERT ON d4s4_claim_revocations
WHEN EXISTS (
    SELECT 1 FROM active_visibility_pointer p
    JOIN manifest_file_paths mfp ON mfp.manifest_id = p.manifest_id
    JOIN source_identity_claims sic ON sic.source_id = mfp.source_id
    WHERE sic.claim_id = NEW.claim_id
)
BEGIN
    UPDATE d4s4_authority_clock SET revision = revision + 1 WHERE singleton_id = 1;
END;

CREATE TRIGGER d4s4_manifest_publish_capability
BEFORE UPDATE OF status ON visibility_manifests
WHEN NEW.status = 'published' AND OLD.status <> 'published'
 AND d4s4_publish_authorized() <> 1
BEGIN
    SELECT RAISE(ABORT, 'd4s4: Manifest publication requires trusted publisher');
END;
CREATE TRIGGER d4s4_generation_publish_capability
BEFORE UPDATE OF status ON source_generations
WHEN NEW.status = 'published' AND OLD.status <> 'published'
 AND d4s4_publish_authorized() <> 1
BEGIN
    SELECT RAISE(ABORT, 'd4s4: generation publication requires trusted publisher');
END;
CREATE TRIGGER d4s4_pointer_insert_capability
BEFORE INSERT ON active_visibility_pointer
WHEN d4s4_publish_authorized() <> 1
BEGIN
    SELECT RAISE(ABORT, 'd4s4: pointer publication requires trusted publisher');
END;
CREATE TRIGGER d4s4_pointer_update_capability
BEFORE UPDATE ON active_visibility_pointer
WHEN d4s4_publish_authorized() <> 1
BEGIN
    SELECT RAISE(ABORT, 'd4s4: pointer CAS requires trusted publisher');
END;

-- Identity/path edits update files_fts in the same transaction and therefore
-- require the same publication entry point. Independent user metadata and tag
-- rows remain mutable without changing authority or the pointer.
CREATE TRIGGER d4s4_file_identity_update_guard
BEFORE UPDATE OF path, name ON files
WHEN EXISTS (
    SELECT 1 FROM file_sources fs
    JOIN manifest_file_paths mfp ON mfp.source_id = fs.source_id
    JOIN active_visibility_pointer p ON p.manifest_id = mfp.manifest_id
    WHERE fs.file_id = OLD.id
)
 AND (NEW.path IS NOT OLD.path OR NEW.name IS NOT OLD.name)
 AND d4s4_publish_authorized() <> 1
BEGIN
    SELECT RAISE(ABORT, 'd4s4: current file identity edits require trusted publication');
END;

CREATE VIEW d4s4_effective_file_sources AS
SELECT DISTINCT e.source_id, e.path_entry_id, e.path_key, e.projected_path,
       e.file_id, e.manifest_id, e.manifest_revision,
       seal.authority_revision
FROM effective_file_sources e
JOIN d4s4_manifest_authority_seals seal ON seal.manifest_id = e.manifest_id
JOIN d4s4_authority_clock clock ON clock.singleton_id = 1
JOIN file_sources fs ON fs.source_id = e.source_id
JOIN d4s4_manifest_source_evidence se
  ON se.manifest_id = e.manifest_id
 AND se.source_id = e.source_id
 AND se.path_entry_id = e.path_entry_id
 AND se.source_revision = fs.source_revision
 AND se.source_status = fs.status
 AND se.identity_confidence = fs.identity_confidence
JOIN file_path_entries pe
  ON pe.path_entry_id = e.path_entry_id AND pe.source_id = e.source_id
JOIN file_physical_identity_epochs pi
  ON pi.physical_identity_id = pe.physical_identity_id
 AND pi.physical_identity_id = se.physical_identity_id
 AND pi.identity_epoch = se.identity_epoch
 AND pi.platform_file_identity = se.platform_file_identity
 AND pi.evidence_digest = se.physical_evidence_digest
 AND pe.evidence_digest = se.path_evidence_digest
WHERE seal.authority_revision = clock.revision
  AND NOT EXISTS (
      SELECT 1 FROM d4s4_claim_revocations cr
      JOIN source_identity_claims sic ON sic.claim_id = cr.claim_id
      WHERE sic.source_id = e.source_id AND sic.path_entry_id = e.path_entry_id
  )
  AND NOT EXISTS (
      SELECT 1
      FROM manifest_root_heads mr
      JOIN d4s4_manifest_root_evidence re
        ON re.manifest_id = mr.manifest_id AND re.root_id = mr.root_id
      JOIN scan_roots r ON r.id = mr.root_id
      JOIN source_generations sg ON sg.generation_id = mr.generation_id
      JOIN scan_runs sr ON sr.id = sg.run_id
      WHERE mr.manifest_id = e.manifest_id
        AND (
            r.normalized_path IS NOT re.normalized_path
            OR r.source_kind IS NOT re.source_kind
            OR r.enabled IS NOT re.enabled
            OR r.health_status IS NOT re.health_status
            OR r.needs_reconciliation IS NOT re.needs_reconciliation
            OR r.watcher_revision IS NOT re.watcher_revision
            OR sg.root_revision IS NOT re.root_revision
            OR sg.watcher_revision IS NOT re.watcher_revision
            OR sr.status <> 'completed'
            OR sr.coverage_complete <> 1
            OR sr.watcher_revision_at_start <> sg.watcher_revision
        )
  );

CREATE VIEW d4s4_oracle_current_authority AS
SELECT p.manifest_id, p.revision AS pointer_revision,
       c.revision AS authority_revision,
       seal.root_count, seal.root_map_digest,
       seal.member_count, seal.member_digest,
       seal.source_count, seal.source_evidence_digest,
       (SELECT count(*) FROM d4s4_effective_file_sources e
        WHERE e.manifest_id = p.manifest_id) AS visible_source_count
FROM active_visibility_pointer p
JOIN d4s4_manifest_authority_seals seal ON seal.manifest_id = p.manifest_id
JOIN d4s4_authority_clock c ON c.singleton_id = 1
WHERE seal.authority_revision = c.revision;
