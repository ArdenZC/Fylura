-- TEST-ONLY physical model for #381-D4-S2. No production schema version is
-- assigned. `files` is rebuilt from Schema 37 with `path UNIQUE` removed and
-- an opaque, immutable `source_id` column added; all legacy rowids and IDs stay.

CREATE TABLE file_sources (
    source_id TEXT PRIMARY KEY,
    file_id TEXT NOT NULL UNIQUE REFERENCES files(id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK (status IN (
        'legacy_unverified', 'observed', 'retired', 'conflicted'
    )),
    identity_confidence TEXT NOT NULL CHECK (identity_confidence IN (
        'none', 'path_only', 'native_verified', 'user_claimed'
    )),
    source_revision INTEGER NOT NULL DEFAULT 1 CHECK (source_revision >= 1),
    created_at INTEGER NOT NULL,
    retired_at INTEGER
);

CREATE TABLE file_physical_identity_epochs (
    physical_identity_id TEXT PRIMARY KEY,
    platform_kind TEXT NOT NULL,
    volume_identity TEXT,
    platform_file_identity TEXT,
    identity_epoch TEXT NOT NULL,
    first_verified_at INTEGER NOT NULL,
    last_verified_at INTEGER NOT NULL,
    evidence_digest TEXT NOT NULL,
    UNIQUE (physical_identity_id, identity_epoch)
);
CREATE INDEX idx_physical_identity_tuple_nonunique
    ON file_physical_identity_epochs(
        platform_kind, volume_identity, platform_file_identity
    );

CREATE TABLE file_path_entries (
    path_entry_id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES file_sources(source_id) ON DELETE RESTRICT,
    path_text TEXT NOT NULL,
    path_key TEXT NOT NULL,
    path_key_version INTEGER NOT NULL,
    identity_status TEXT NOT NULL CHECK (identity_status IN (
        'legacy_unverified', 'path_only', 'verified', 'conflicted'
    )),
    physical_identity_id TEXT REFERENCES file_physical_identity_epochs(physical_identity_id)
        ON DELETE RESTRICT,
    evidence_digest TEXT,
    observed_at INTEGER NOT NULL,
    UNIQUE (path_entry_id, source_id)
);
CREATE INDEX idx_file_path_entries_source_observed
    ON file_path_entries(source_id, observed_at DESC, path_entry_id);
CREATE INDEX idx_file_path_entries_path_key
    ON file_path_entries(path_key, observed_at DESC);

CREATE TABLE source_identity_claims (
    claim_id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES file_sources(source_id) ON DELETE RESTRICT,
    path_entry_id TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    claim_kind TEXT NOT NULL CHECK (claim_kind IN (
        'native_match', 'explicit_legacy_claim'
    )),
    authorization_digest TEXT NOT NULL,
    claimed_at INTEGER NOT NULL,
    FOREIGN KEY (path_entry_id, source_id)
        REFERENCES file_path_entries(path_entry_id, source_id) ON DELETE RESTRICT
);

-- Independent, evidence-bound bridge. No path/basename join may create a row.
CREATE TABLE source_global_index_links (
    link_id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES file_sources(source_id) ON DELETE RESTRICT,
    global_entry_id TEXT NOT NULL REFERENCES global_entries(id) ON DELETE RESTRICT,
    link_kind TEXT NOT NULL CHECK (link_kind IN (
        'verified_physical_identity', 'explicit_user_link'
    )),
    source_identity_digest TEXT NOT NULL,
    global_identity_digest TEXT NOT NULL,
    source_revision INTEGER NOT NULL,
    global_revision INTEGER NOT NULL,
    database_generation TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    invalidated_at INTEGER,
    UNIQUE (source_id, global_entry_id, created_at)
);
CREATE INDEX idx_source_global_index_links_source
    ON source_global_index_links(source_id, invalidated_at, global_entry_id);
CREATE INDEX idx_source_global_index_links_global
    ON source_global_index_links(global_entry_id, invalidated_at, source_id);

-- Audit authorization survives deletion of the source row. It is a prerequisite
-- for retention-aware GC; ordinary stale/root removal never creates a row here.
CREATE TABLE source_gc_authorizations (
    source_id TEXT NOT NULL,
    approved_by TEXT NOT NULL,
    reason TEXT NOT NULL,
    approved_at INTEGER NOT NULL,
    PRIMARY KEY (source_id, approved_at)
);

CREATE TABLE source_generations (
    generation_id TEXT PRIMARY KEY,
    root_id TEXT NOT NULL REFERENCES scan_roots(id) ON DELETE RESTRICT,
    run_id TEXT NOT NULL UNIQUE REFERENCES scan_runs(id) ON DELETE RESTRICT,
    status TEXT NOT NULL CHECK (status IN (
        'staging', 'complete', 'published', 'rejected', 'abandoned'
    )),
    root_revision INTEGER NOT NULL,
    watcher_revision INTEGER NOT NULL,
    member_count INTEGER NOT NULL DEFAULT 0,
    member_digest TEXT NOT NULL DEFAULT '',
    completed_at INTEGER,
    UNIQUE (generation_id, root_id),
    FOREIGN KEY (run_id, root_id)
        REFERENCES scan_runs(id, scan_root_id) ON DELETE RESTRICT
);
CREATE UNIQUE INDEX idx_scan_runs_id_root
    ON scan_runs(id, scan_root_id);
CREATE UNIQUE INDEX idx_source_generations_run_root
    ON source_generations(run_id, root_id);
CREATE TABLE source_generation_members (
    generation_id TEXT NOT NULL REFERENCES source_generations(generation_id)
        ON DELETE RESTRICT,
    source_id TEXT NOT NULL REFERENCES file_sources(source_id) ON DELETE RESTRICT,
    path_entry_id TEXT NOT NULL,
    identity_digest TEXT NOT NULL,
    PRIMARY KEY (generation_id, source_id, path_entry_id),
    FOREIGN KEY (path_entry_id, source_id)
        REFERENCES file_path_entries(path_entry_id, source_id) ON DELETE RESTRICT
);
CREATE INDEX idx_source_generation_members_source
    ON source_generation_members(source_id, generation_id);

CREATE TABLE visibility_manifests (
    manifest_id TEXT PRIMARY KEY,
    parent_manifest_id TEXT REFERENCES visibility_manifests(manifest_id)
        ON DELETE RESTRICT,
    status TEXT NOT NULL CHECK (status IN ('staging', 'ready', 'published', 'abandoned')),
    created_at INTEGER NOT NULL,
    root_map_digest TEXT NOT NULL
);
CREATE TABLE manifest_root_heads (
    manifest_id TEXT NOT NULL REFERENCES visibility_manifests(manifest_id)
        ON DELETE RESTRICT,
    root_id TEXT NOT NULL REFERENCES scan_roots(id) ON DELETE RESTRICT,
    generation_id TEXT NOT NULL REFERENCES source_generations(generation_id)
        ON DELETE RESTRICT,
    PRIMARY KEY (manifest_id, root_id),
    UNIQUE (manifest_id, generation_id),
    FOREIGN KEY (generation_id, root_id)
        REFERENCES source_generations(generation_id, root_id) ON DELETE RESTRICT
);

-- Derived immutable path projection for each manifest. Its uniqueness
-- constraints reject conflicting source/path claims before pointer publication.
CREATE TABLE manifest_file_paths (
    manifest_id TEXT NOT NULL REFERENCES visibility_manifests(manifest_id)
        ON DELETE RESTRICT,
    source_id TEXT NOT NULL REFERENCES file_sources(source_id) ON DELETE RESTRICT,
    path_entry_id TEXT NOT NULL,
    path_key TEXT NOT NULL,
    projected_path TEXT NOT NULL,
    PRIMARY KEY (manifest_id, source_id),
    UNIQUE (manifest_id, path_key),
    FOREIGN KEY (path_entry_id, source_id)
        REFERENCES file_path_entries(path_entry_id, source_id) ON DELETE RESTRICT
);
CREATE INDEX idx_manifest_file_paths_source
    ON manifest_file_paths(source_id, manifest_id);

CREATE TABLE active_visibility_pointer (
    singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
    manifest_id TEXT NOT NULL REFERENCES visibility_manifests(manifest_id)
        ON DELETE RESTRICT,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    updated_at INTEGER NOT NULL
);

CREATE TABLE source_reader_leases (
    lease_id TEXT PRIMARY KEY,
    manifest_id TEXT NOT NULL REFERENCES visibility_manifests(manifest_id)
        ON DELETE RESTRICT,
    owner_kind TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    expires_at INTEGER,
    created_at INTEGER NOT NULL,
    released_at INTEGER
);
CREATE INDEX idx_source_reader_leases_manifest
    ON source_reader_leases(manifest_id, released_at, expires_at);

CREATE TABLE d4s2_candidate_migration_journal (
    migration_key TEXT PRIMARY KEY,
    input_schema_sha256 TEXT NOT NULL,
    candidate_ddl_sha256 TEXT NOT NULL,
    applied_at INTEGER NOT NULL
);

CREATE TRIGGER file_sources_delete_guard
BEFORE DELETE ON files
WHEN EXISTS (
    SELECT 1 FROM file_sources AS fs
    WHERE fs.file_id = OLD.id
      AND NOT EXISTS (
          SELECT 1 FROM source_gc_authorizations AS gc
          WHERE gc.source_id = fs.source_id
      )
)
BEGIN
    SELECT RAISE(ABORT, 'source rows require retention-aware GC');
END;

CREATE TRIGGER file_sources_mapping_insert
AFTER INSERT ON files
BEGIN
    INSERT INTO file_sources
        (source_id,file_id,status,identity_confidence,source_revision,created_at)
    VALUES
        (NEW.source_id,NEW.id,'observed','path_only',1,NEW.last_seen_at);
END;

CREATE TRIGGER file_sources_binding_insert_guard
BEFORE INSERT ON file_sources
WHEN NOT EXISTS (
    SELECT 1 FROM files AS f
    WHERE f.id = NEW.file_id AND f.source_id = NEW.source_id
)
BEGIN
    SELECT RAISE(ABORT, 'source mapping must match the file row');
END;

CREATE TRIGGER files_source_id_immutable_update
BEFORE UPDATE OF source_id ON files
WHEN NEW.source_id <> OLD.source_id
BEGIN
    SELECT RAISE(ABORT, 'source_id is immutable');
END;

CREATE TRIGGER file_sources_binding_immutable_update
BEFORE UPDATE OF source_id,file_id ON file_sources
WHEN NEW.source_id <> OLD.source_id OR NEW.file_id <> OLD.file_id
BEGIN
    SELECT RAISE(ABORT, 'source/file binding is immutable');
END;

CREATE TRIGGER file_sources_delete_requires_authorization
BEFORE DELETE ON file_sources
WHEN NOT EXISTS (
    SELECT 1 FROM source_gc_authorizations AS gc
    WHERE gc.source_id = OLD.source_id
)
BEGIN
    SELECT RAISE(ABORT, 'source rows require retention-aware GC');
END;

CREATE TRIGGER file_sources_delete_requires_file_row_removal
BEFORE DELETE ON file_sources
WHEN EXISTS (SELECT 1 FROM files WHERE id = OLD.file_id)
BEGIN
    SELECT RAISE(ABORT, 'delete the authorized file row with its source mapping');
END;

CREATE TRIGGER file_path_entries_immutable_update
BEFORE UPDATE ON file_path_entries
BEGIN
    SELECT RAISE(ABORT, 'path entries are immutable observations');
END;
CREATE TRIGGER file_path_entries_immutable_delete
BEFORE DELETE ON file_path_entries
WHEN NOT EXISTS (
    SELECT 1 FROM source_gc_authorizations AS gc
    WHERE gc.source_id = OLD.source_id
)
BEGIN
    SELECT RAISE(ABORT, 'path entries require retention-aware GC');
END;

CREATE VIEW effective_file_sources AS
SELECT DISTINCT mfp.source_id, mfp.path_entry_id, mfp.path_key, mfp.projected_path,
       fs.file_id, p.manifest_id, p.revision AS manifest_revision
FROM active_visibility_pointer AS p
JOIN manifest_file_paths AS mfp ON mfp.manifest_id = p.manifest_id
JOIN file_sources AS fs ON fs.source_id = mfp.source_id
JOIN manifest_root_heads AS mr ON mr.manifest_id = p.manifest_id
JOIN source_generation_members AS gm
  ON gm.generation_id = mr.generation_id
 AND gm.source_id = mfp.source_id
 AND gm.path_entry_id = mfp.path_entry_id;

CREATE VIEW legacy_unverified_compatibility AS
SELECT fs.source_id, fs.file_id, f.path, f.name, f.is_stale
FROM file_sources AS fs
JOIN files AS f ON f.id = fs.file_id
WHERE fs.status = 'legacy_unverified';
