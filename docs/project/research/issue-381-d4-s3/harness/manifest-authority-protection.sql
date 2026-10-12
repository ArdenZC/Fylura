-- Research-only DML protection for the D4-S2 physical model.
-- This file is applied only to disposable Schema 37 harness databases.
-- A publishing caller must run the independent Python/SQL Oracle before
-- transitioning a ready manifest to published, then CAS the active pointer in
-- the same SQLite transaction.

CREATE TRIGGER d4s3_manifest_no_direct_published_insert
BEFORE INSERT ON visibility_manifests
WHEN NEW.status = 'published'
BEGIN
    SELECT RAISE(ABORT, 'd4s3: direct published manifest insert forbidden');
END;

CREATE TRIGGER d4s3_manifest_freeze_update
BEFORE UPDATE ON visibility_manifests
WHEN OLD.status = 'published'
BEGIN
    SELECT RAISE(ABORT, 'd4s3: published manifest is immutable');
END;

CREATE TRIGGER d4s3_manifest_freeze_delete
BEFORE DELETE ON visibility_manifests
WHEN OLD.status = 'published'
BEGIN
    SELECT RAISE(ABORT, 'd4s3: published manifest is immutable');
END;

CREATE TRIGGER d4s3_manifest_publish_guard
BEFORE UPDATE OF status ON visibility_manifests
WHEN NEW.status = 'published' AND OLD.status <> 'published'
BEGIN
    SELECT CASE WHEN OLD.status <> 'ready'
        THEN RAISE(ABORT, 'd4s3: only a ready manifest can publish') END;

    SELECT CASE WHEN NEW.parent_manifest_id IS NOT (
        SELECT manifest_id FROM active_visibility_pointer WHERE singleton_id = 1
    ) THEN RAISE(ABORT, 'd4s3: manifest parent is not the current pointer') END;

    SELECT CASE WHEN EXISTS (
        SELECT 1
        FROM manifest_root_heads AS mr
        LEFT JOIN source_generations AS sg ON sg.generation_id = mr.generation_id
        LEFT JOIN scan_runs AS sr ON sr.id = sg.run_id
        WHERE mr.manifest_id = NEW.manifest_id
          AND (
              sg.generation_id IS NULL
              OR sg.status NOT IN ('complete', 'published')
              OR sr.id IS NULL
              OR sr.status <> 'completed'
              OR sr.coverage_complete <> 1
              OR sg.member_count <> (
                  SELECT count(*) FROM source_generation_members AS gm
                  WHERE gm.generation_id = sg.generation_id
              )
          )
    ) THEN RAISE(ABORT, 'd4s3: incomplete or inconsistent root generation') END;

    SELECT CASE WHEN EXISTS (
        SELECT 1
        FROM manifest_root_heads AS mr
        JOIN source_generation_members AS gm ON gm.generation_id = mr.generation_id
        LEFT JOIN file_path_entries AS pe
          ON pe.path_entry_id = gm.path_entry_id AND pe.source_id = gm.source_id
        LEFT JOIN file_sources AS fs ON fs.source_id = gm.source_id
        LEFT JOIN file_physical_identity_epochs AS pi
          ON pi.physical_identity_id = pe.physical_identity_id
        WHERE mr.manifest_id = NEW.manifest_id
          AND (
              pe.path_entry_id IS NULL
              OR pe.identity_status <> 'verified'
              OR pe.physical_identity_id IS NULL
              OR pe.evidence_digest IS NULL OR pe.evidence_digest = ''
              OR gm.identity_digest IS NULL OR gm.identity_digest = ''
              OR gm.identity_digest <> pe.evidence_digest
              OR pi.physical_identity_id IS NULL
              OR pi.identity_epoch IS NULL OR pi.identity_epoch = ''
              OR pi.platform_file_identity IS NULL OR pi.platform_file_identity = ''
              OR fs.status <> 'observed'
              OR fs.identity_confidence NOT IN ('native_verified', 'user_claimed')
              OR (fs.identity_confidence = 'user_claimed' AND NOT EXISTS (
                  SELECT 1 FROM source_identity_claims AS sic
                  WHERE sic.source_id = gm.source_id
                    AND sic.path_entry_id = gm.path_entry_id
                    AND sic.claim_kind = 'explicit_legacy_claim'
              ))
          )
    ) THEN RAISE(ABORT, 'd4s3: source or path identity evidence is invalid') END;

    SELECT CASE WHEN EXISTS (
        SELECT gm.source_id
        FROM manifest_root_heads AS mr
        JOIN source_generation_members AS gm ON gm.generation_id = mr.generation_id
        WHERE mr.manifest_id = NEW.manifest_id
        GROUP BY gm.source_id
        HAVING count(DISTINCT gm.path_entry_id) <> 1
    ) THEN RAISE(ABORT, 'd4s3: source has conflicting current path entries') END;

    -- The two EXCEPT directions are the publication completeness contract:
    -- no selected member may be omitted, and no unselected projection may be
    -- published. DISTINCT collapses the same member observed by nested roots.
    SELECT CASE WHEN EXISTS (
        SELECT source_id, path_entry_id, path_key, path_text FROM (
            SELECT DISTINCT gm.source_id, gm.path_entry_id, pe.path_key, pe.path_text
            FROM manifest_root_heads AS mr
            JOIN source_generation_members AS gm ON gm.generation_id = mr.generation_id
            JOIN file_path_entries AS pe
              ON pe.path_entry_id = gm.path_entry_id AND pe.source_id = gm.source_id
            WHERE mr.manifest_id = NEW.manifest_id
        )
        EXCEPT
        SELECT source_id, path_entry_id, path_key, projected_path
        FROM manifest_file_paths WHERE manifest_id = NEW.manifest_id
    ) THEN RAISE(ABORT, 'd4s3: manifest projection omits a selected member') END;

    SELECT CASE WHEN EXISTS (
        SELECT source_id, path_entry_id, path_key, projected_path
        FROM manifest_file_paths WHERE manifest_id = NEW.manifest_id
        EXCEPT
        SELECT source_id, path_entry_id, path_key, path_text FROM (
            SELECT DISTINCT gm.source_id, gm.path_entry_id, pe.path_key, pe.path_text
            FROM manifest_root_heads AS mr
            JOIN source_generation_members AS gm ON gm.generation_id = mr.generation_id
            JOIN file_path_entries AS pe
              ON pe.path_entry_id = gm.path_entry_id AND pe.source_id = gm.source_id
            WHERE mr.manifest_id = NEW.manifest_id
        )
    ) THEN RAISE(ABORT, 'd4s3: manifest projection contains an unselected member or wrong path') END;
END;

CREATE TRIGGER d4s3_generation_no_direct_published_insert
BEFORE INSERT ON source_generations
WHEN NEW.status = 'published'
BEGIN
    SELECT RAISE(ABORT, 'd4s3: direct published generation insert forbidden');
END;

CREATE TRIGGER d4s3_generation_publish_transition_guard
BEFORE UPDATE ON source_generations
WHEN NEW.status = 'published' AND OLD.status <> 'published'
BEGIN
    SELECT CASE WHEN OLD.status <> 'complete'
        OR NEW.generation_id IS NOT OLD.generation_id
        OR NEW.root_id IS NOT OLD.root_id
        OR NEW.run_id IS NOT OLD.run_id
        OR NEW.root_revision IS NOT OLD.root_revision
        OR NEW.watcher_revision IS NOT OLD.watcher_revision
        OR NEW.member_count IS NOT OLD.member_count
        OR NEW.member_digest IS NOT OLD.member_digest
        OR NEW.completed_at IS NOT OLD.completed_at
        THEN RAISE(ABORT, 'd4s3: invalid generation seal transition') END;
END;

CREATE TRIGGER d4s3_generation_freeze_update
BEFORE UPDATE ON source_generations
WHEN OLD.status = 'published'
  OR EXISTS (
      SELECT 1 FROM manifest_root_heads AS mr
      JOIN visibility_manifests AS vm ON vm.manifest_id = mr.manifest_id
      WHERE mr.generation_id = OLD.generation_id AND vm.status = 'published'
  )
BEGIN
    SELECT RAISE(ABORT, 'd4s3: published generation is immutable');
END;

CREATE TRIGGER d4s3_generation_freeze_delete
BEFORE DELETE ON source_generations
WHEN OLD.status = 'published'
  OR EXISTS (
      SELECT 1 FROM manifest_root_heads AS mr
      JOIN visibility_manifests AS vm ON vm.manifest_id = mr.manifest_id
      WHERE mr.generation_id = OLD.generation_id AND vm.status = 'published'
  )
BEGIN
    SELECT RAISE(ABORT, 'd4s3: published generation is immutable');
END;

CREATE TRIGGER d4s3_member_freeze_insert
BEFORE INSERT ON source_generation_members
WHEN EXISTS (
    SELECT 1 FROM source_generations AS sg
    WHERE sg.generation_id = NEW.generation_id AND sg.status = 'published'
)
  OR EXISTS (
      SELECT 1 FROM manifest_root_heads AS mr
      JOIN visibility_manifests AS vm ON vm.manifest_id = mr.manifest_id
      WHERE mr.generation_id = NEW.generation_id AND vm.status = 'published'
  )
BEGIN
    SELECT RAISE(ABORT, 'd4s3: published generation membership is immutable');
END;

CREATE TRIGGER d4s3_member_freeze_update
BEFORE UPDATE ON source_generation_members
WHEN EXISTS (
    SELECT 1 FROM source_generations AS sg
    WHERE sg.generation_id IN (OLD.generation_id, NEW.generation_id)
      AND sg.status = 'published'
)
  OR EXISTS (
      SELECT 1 FROM manifest_root_heads AS mr
      JOIN visibility_manifests AS vm ON vm.manifest_id = mr.manifest_id
      WHERE mr.generation_id IN (OLD.generation_id, NEW.generation_id)
        AND vm.status = 'published'
  )
BEGIN
    SELECT RAISE(ABORT, 'd4s3: published generation membership is immutable');
END;

CREATE TRIGGER d4s3_member_freeze_delete
BEFORE DELETE ON source_generation_members
WHEN EXISTS (
    SELECT 1 FROM source_generations AS sg
    WHERE sg.generation_id = OLD.generation_id AND sg.status = 'published'
)
  OR EXISTS (
      SELECT 1 FROM manifest_root_heads AS mr
      JOIN visibility_manifests AS vm ON vm.manifest_id = mr.manifest_id
      WHERE mr.generation_id = OLD.generation_id AND vm.status = 'published'
  )
BEGIN
    SELECT RAISE(ABORT, 'd4s3: published generation membership is immutable');
END;

CREATE TRIGGER d4s3_root_head_freeze_insert
BEFORE INSERT ON manifest_root_heads
WHEN EXISTS (SELECT 1 FROM visibility_manifests AS vm
             WHERE vm.manifest_id = NEW.manifest_id AND vm.status = 'published')
BEGIN
    SELECT RAISE(ABORT, 'd4s3: published manifest root heads are immutable');
END;

CREATE TRIGGER d4s3_root_head_freeze_update
BEFORE UPDATE ON manifest_root_heads
WHEN EXISTS (SELECT 1 FROM visibility_manifests AS vm
             WHERE vm.manifest_id = OLD.manifest_id AND vm.status = 'published')
  OR EXISTS (SELECT 1 FROM visibility_manifests AS vm
             WHERE vm.manifest_id = NEW.manifest_id AND vm.status = 'published')
BEGIN
    SELECT RAISE(ABORT, 'd4s3: published manifest root heads are immutable');
END;

CREATE TRIGGER d4s3_root_head_freeze_delete
BEFORE DELETE ON manifest_root_heads
WHEN EXISTS (SELECT 1 FROM visibility_manifests AS vm
             WHERE vm.manifest_id = OLD.manifest_id AND vm.status = 'published')
BEGIN
    SELECT RAISE(ABORT, 'd4s3: published manifest root heads are immutable');
END;

CREATE TRIGGER d4s3_path_freeze_insert
BEFORE INSERT ON manifest_file_paths
WHEN EXISTS (SELECT 1 FROM visibility_manifests AS vm
             WHERE vm.manifest_id = NEW.manifest_id AND vm.status = 'published')
BEGIN
    SELECT RAISE(ABORT, 'd4s3: published manifest paths are immutable');
END;

CREATE TRIGGER d4s3_path_freeze_update
BEFORE UPDATE ON manifest_file_paths
WHEN EXISTS (SELECT 1 FROM visibility_manifests AS vm
             WHERE vm.manifest_id = OLD.manifest_id AND vm.status = 'published')
  OR EXISTS (SELECT 1 FROM visibility_manifests AS vm
             WHERE vm.manifest_id = NEW.manifest_id AND vm.status = 'published')
BEGIN
    SELECT RAISE(ABORT, 'd4s3: published manifest paths are immutable');
END;

CREATE TRIGGER d4s3_path_freeze_delete
BEFORE DELETE ON manifest_file_paths
WHEN EXISTS (SELECT 1 FROM visibility_manifests AS vm
             WHERE vm.manifest_id = OLD.manifest_id AND vm.status = 'published')
BEGIN
    SELECT RAISE(ABORT, 'd4s3: published manifest paths are immutable');
END;

CREATE TRIGGER d4s3_pointer_initial_insert_guard
BEFORE INSERT ON active_visibility_pointer
WHEN NOT EXISTS (
    SELECT 1 FROM visibility_manifests AS vm
    WHERE vm.manifest_id = NEW.manifest_id
      AND vm.status = 'published'
      AND vm.parent_manifest_id IS NULL
)
BEGIN
    SELECT RAISE(ABORT, 'd4s3: initial pointer must select a root published manifest');
END;

CREATE TRIGGER d4s3_pointer_update_guard
BEFORE UPDATE ON active_visibility_pointer
WHEN NEW.singleton_id <> OLD.singleton_id
  OR NEW.revision <> OLD.revision + 1
  OR NEW.manifest_id = OLD.manifest_id
  OR NOT EXISTS (
      SELECT 1 FROM visibility_manifests AS vm
      WHERE vm.manifest_id = NEW.manifest_id
        AND vm.status = 'published'
        AND vm.parent_manifest_id = OLD.manifest_id
  )
BEGIN
    SELECT RAISE(ABORT, 'd4s3: pointer update must be a published-child CAS');
END;

CREATE TRIGGER d4s3_pointer_no_delete
BEFORE DELETE ON active_visibility_pointer
BEGIN
    SELECT RAISE(ABORT, 'd4s3: active visibility pointer cannot be deleted');
END;
