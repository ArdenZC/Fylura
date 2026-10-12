-- D4-S6 research-only publication fence; applied to disposable Schema 37 DBs.
-- SHA-256 remains an application Oracle responsibility: SQLite core has no
-- built-in SHA-256 function. Relational seal completeness is checked in SQL
-- before either initial-pointer insertion or successor-pointer CAS.

CREATE VIEW d4s6_selected_root_members AS
SELECT mr.manifest_id, mr.root_id, mr.generation_id, gm.source_id, gm.path_entry_id
FROM manifest_root_heads mr
JOIN source_generation_members gm ON gm.generation_id = mr.generation_id;

CREATE VIEW d4s6_selected_sources AS
SELECT DISTINCT manifest_id, source_id, path_entry_id
FROM d4s6_selected_root_members;

-- One relational oracle is shared by both pointer paths so their membership,
-- seal-count, and revision rules cannot drift apart.
CREATE VIEW d4s6_manifest_seal_completeness AS
SELECT vm.manifest_id,
       CASE WHEN
         NOT EXISTS (
           SELECT 1 FROM d4s4_manifest_authority_seals a
           WHERE a.manifest_id = vm.manifest_id
             AND a.root_map_digest = vm.root_map_digest
             AND a.root_count = (SELECT count(*) FROM manifest_root_heads mr
                                 WHERE mr.manifest_id = vm.manifest_id)
             AND a.member_count = (SELECT count(*) FROM d4s6_selected_sources s
                                  WHERE s.manifest_id = vm.manifest_id)
             AND a.source_count = (SELECT count(*) FROM d4s4_manifest_source_evidence se
                                  WHERE se.manifest_id = vm.manifest_id)
         )
         OR EXISTS (
           SELECT root_id, generation_id FROM manifest_root_heads
           WHERE manifest_id = vm.manifest_id
           EXCEPT
           SELECT root_id, generation_id FROM d4s5_manifest_root_seals
           WHERE manifest_id = vm.manifest_id
         )
         OR EXISTS (
           SELECT root_id, generation_id FROM d4s5_manifest_root_seals
           WHERE manifest_id = vm.manifest_id
           EXCEPT
           SELECT root_id, generation_id FROM manifest_root_heads
           WHERE manifest_id = vm.manifest_id
         )
         OR EXISTS (
           SELECT source_id, path_entry_id FROM d4s6_selected_sources
           WHERE manifest_id = vm.manifest_id
           EXCEPT
           SELECT source_id, path_entry_id FROM manifest_file_paths
           WHERE manifest_id = vm.manifest_id
         )
         OR EXISTS (
           SELECT source_id, path_entry_id FROM manifest_file_paths
           WHERE manifest_id = vm.manifest_id
           EXCEPT
           SELECT source_id, path_entry_id FROM d4s6_selected_sources
           WHERE manifest_id = vm.manifest_id
         )
         OR EXISTS (
           SELECT source_id, path_entry_id FROM d4s6_selected_sources
           WHERE manifest_id = vm.manifest_id
           EXCEPT
           SELECT source_id, path_entry_id FROM d4s4_manifest_source_evidence
           WHERE manifest_id = vm.manifest_id
         )
         OR EXISTS (
           SELECT source_id, path_entry_id FROM d4s4_manifest_source_evidence
           WHERE manifest_id = vm.manifest_id
           EXCEPT
           SELECT source_id, path_entry_id FROM d4s6_selected_sources
           WHERE manifest_id = vm.manifest_id
         )
         OR EXISTS (
           SELECT source_id, path_entry_id FROM d4s4_manifest_source_evidence
           WHERE manifest_id = vm.manifest_id
           EXCEPT
           SELECT source_id, path_entry_id FROM d4s5_manifest_source_seals
           WHERE manifest_id = vm.manifest_id
         )
         OR EXISTS (
           SELECT source_id, path_entry_id FROM d4s5_manifest_source_seals
           WHERE manifest_id = vm.manifest_id
           EXCEPT
           SELECT source_id, path_entry_id FROM d4s4_manifest_source_evidence
           WHERE manifest_id = vm.manifest_id
         )
         OR EXISTS (
           SELECT 1 FROM d4s5_manifest_root_seals rs
           LEFT JOIN d4s4_manifest_root_evidence re
             ON re.manifest_id = rs.manifest_id AND re.root_id = rs.root_id
           LEFT JOIN d4s5_root_authority ra ON ra.root_id = rs.root_id
           WHERE rs.manifest_id = vm.manifest_id
             AND (re.root_id IS NULL OR ra.root_id IS NULL
               OR rs.generation_id <> re.generation_id
               OR rs.root_authority_revision <> ra.revision
               OR rs.root_revision_at_publish <> re.root_revision
               OR rs.watcher_revision_at_publish <> re.watcher_revision)
         )
         OR EXISTS (
           SELECT 1 FROM d4s5_manifest_source_seals ss
           LEFT JOIN d4s4_manifest_source_evidence se
             ON se.manifest_id = ss.manifest_id
            AND se.source_id = ss.source_id AND se.path_entry_id = ss.path_entry_id
           LEFT JOIN d4s5_source_authority sa ON sa.source_id = ss.source_id
           WHERE ss.manifest_id = vm.manifest_id
             AND (se.source_id IS NULL OR sa.source_id IS NULL
               OR ss.source_revision_at_publish <> se.source_revision
               OR ss.source_authority_revision <> sa.revision)
         )
         OR (SELECT count(*) FROM d4s5_manifest_root_seals
             WHERE manifest_id = vm.manifest_id)
              <> (SELECT count(*) FROM manifest_root_heads
                  WHERE manifest_id = vm.manifest_id)
         OR (SELECT count(*) FROM d4s5_manifest_source_seals
             WHERE manifest_id = vm.manifest_id)
              <> (SELECT count(*) FROM d4s6_selected_sources
                  WHERE manifest_id = vm.manifest_id)
       THEN 0 ELSE 1 END AS complete
FROM visibility_manifests vm;

CREATE TRIGGER d4s6_manifest_publish_authorization
BEFORE UPDATE OF status ON visibility_manifests
WHEN NEW.status = 'published' AND OLD.status <> 'published'
BEGIN
    SELECT CASE WHEN d4s4_publish_authorized() <> 1
        OR d4s5_publish_authorized() <> 1
        OR d4s6_publish_authorized() <> 1
        THEN RAISE(ABORT, 'd4s6: publication requires S4, S5, and fenced publisher capabilities') END;
END;

CREATE TRIGGER d4s6_initial_pointer_fence
BEFORE INSERT ON active_visibility_pointer
BEGIN
    SELECT CASE WHEN d4s4_publish_authorized() <> 1
        OR d4s5_publish_authorized() <> 1
        OR d4s6_publish_authorized() <> 1
        THEN RAISE(ABORT, 'd4s6: initial pointer requires the fenced publication entry') END;
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM visibility_manifests vm
        JOIN d4s6_manifest_seal_completeness c ON c.manifest_id = vm.manifest_id
        WHERE vm.manifest_id = NEW.manifest_id AND vm.status = 'published'
          AND vm.parent_manifest_id IS NULL AND c.complete = 1
    ) THEN RAISE(ABORT, 'd4s6: initial Manifest has incomplete authority seals') END;
END;

CREATE TRIGGER d4s6_successor_pointer_fence
BEFORE UPDATE ON active_visibility_pointer
BEGIN
    SELECT CASE WHEN d4s4_publish_authorized() <> 1
        OR d4s5_publish_authorized() <> 1
        OR d4s6_publish_authorized() <> 1
        THEN RAISE(ABORT, 'd4s6: pointer CAS requires the fenced publication entry') END;
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM visibility_manifests vm
        JOIN d4s6_manifest_seal_completeness c ON c.manifest_id = vm.manifest_id
        WHERE vm.manifest_id = NEW.manifest_id AND vm.status = 'published'
          AND vm.parent_manifest_id = OLD.manifest_id AND c.complete = 1
    ) THEN RAISE(ABORT, 'd4s6: successor Manifest has incomplete authority seals') END;
END;

-- Transaction-local diagnostic trace proves seal inserts precede pointer CAS.
-- These rows roll back with a failed publication and are only harness evidence.
CREATE TABLE d4s6_publication_trace (
    sequence_no INTEGER PRIMARY KEY AUTOINCREMENT,
    manifest_id TEXT NOT NULL,
    event_kind TEXT NOT NULL,
    object_id TEXT
);

CREATE TRIGGER d4s6_trace_root_seal_insert
AFTER INSERT ON d4s5_manifest_root_seals
BEGIN
    INSERT INTO d4s6_publication_trace(manifest_id,event_kind,object_id)
    VALUES(NEW.manifest_id,'root_seal_insert',NEW.root_id);
END;

CREATE TRIGGER d4s6_trace_source_seal_insert
AFTER INSERT ON d4s5_manifest_source_seals
BEGIN
    INSERT INTO d4s6_publication_trace(manifest_id,event_kind,object_id)
    VALUES(NEW.manifest_id,'source_seal_insert',NEW.source_id||'/'||NEW.path_entry_id);
END;

CREATE TRIGGER d4s6_trace_initial_pointer_cas
BEFORE INSERT ON active_visibility_pointer
BEGIN
    INSERT INTO d4s6_publication_trace(manifest_id,event_kind,object_id)
    VALUES(NEW.manifest_id,'initial_pointer_cas',CAST(NEW.revision AS TEXT));
END;

CREATE TRIGGER d4s6_trace_successor_pointer_cas
BEFORE UPDATE ON active_visibility_pointer
BEGIN
    INSERT INTO d4s6_publication_trace(manifest_id,event_kind,object_id)
    VALUES(NEW.manifest_id,'successor_pointer_cas',CAST(NEW.revision AS TEXT));
END;
