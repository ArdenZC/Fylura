-- Additive, dormant physical versions. Logical IDs still belong to the native
-- identity contract; no FK from versions to mutable legacy global_entries.
CREATE TABLE global_index_generations (
    generation_id TEXT PRIMARY KEY NOT NULL CHECK (length(generation_id) > 0),
    volume_id TEXT NOT NULL REFERENCES global_volumes(id) ON DELETE RESTRICT,
    stable_volume_id TEXT NOT NULL,
    source_provider TEXT NOT NULL,
    operation_revision INTEGER NOT NULL CHECK (operation_revision > 0),
    operation_owner_id TEXT NOT NULL,
    rebuild_intent_revision INTEGER NOT NULL CHECK (rebuild_intent_revision >= 0),
    capture_kind TEXT NOT NULL CHECK (capture_kind IN ('initial','rebuild','reconcile')),
    state TEXT NOT NULL DEFAULT 'staging' CHECK (state IN ('staging','sealed','aborted')),
    materialization_revision INTEGER NOT NULL DEFAULT 0 CHECK (materialization_revision >= 0),
    entry_count INTEGER NOT NULL DEFAULT 0 CHECK (entry_count >= 0),
    manifest_digest TEXT,
    journal_id TEXT,
    journal_cursor TEXT,
    capture_complete INTEGER NOT NULL DEFAULT 0 CHECK (capture_complete IN (0,1)),
    abort_reason TEXT,
    created_at INTEGER NOT NULL,
    sealed_at INTEGER,
    CHECK ((state = 'sealed' AND capture_complete = 1 AND manifest_digest IS NOT NULL
            AND length(manifest_digest) = 64 AND sealed_at IS NOT NULL)
        OR (state IN ('staging','aborted') AND capture_complete = 0
            AND manifest_digest IS NULL AND sealed_at IS NULL)),
    CHECK ((state = 'aborted' AND abort_reason IS NOT NULL) OR
           (state <> 'aborted' AND abort_reason IS NULL)),
    UNIQUE(volume_id, generation_id),
    UNIQUE(volume_id, operation_revision)
);
CREATE TABLE global_index_generation_heads (
    volume_id TEXT PRIMARY KEY NOT NULL REFERENCES global_volumes(id) ON DELETE RESTRICT,
    legacy_state TEXT NOT NULL DEFAULT 'LEGACY_UNVERIFIED'
        CHECK (legacy_state IN ('LEGACY_UNVERIFIED','VERSIONED')),
    published_generation_id TEXT,
    candidate_generation_id TEXT,
    CHECK ((legacy_state = 'LEGACY_UNVERIFIED' AND published_generation_id IS NULL)
        OR (legacy_state = 'VERSIONED' AND published_generation_id IS NOT NULL)),
    CHECK (candidate_generation_id IS NULL OR published_generation_id IS NULL
           OR candidate_generation_id <> published_generation_id),
    FOREIGN KEY(volume_id, published_generation_id)
        REFERENCES global_index_generations(volume_id, generation_id) ON DELETE RESTRICT,
    FOREIGN KEY(volume_id, candidate_generation_id)
        REFERENCES global_index_generations(volume_id, generation_id) ON DELETE RESTRICT
);
CREATE TABLE global_index_generation_entries (
    version_rowid INTEGER PRIMARY KEY AUTOINCREMENT,
    generation_id TEXT NOT NULL REFERENCES global_index_generations(generation_id) ON DELETE RESTRICT,
    logical_entry_id TEXT NOT NULL,
    name TEXT NOT NULL,
    name_normalized TEXT NOT NULL,
    path TEXT NOT NULL,
    path_normalized TEXT NOT NULL,
    extension TEXT NOT NULL,
    -- Canonical serde encoding of the complete GlobalEntryInput, including
    -- native/parent identity, provider, timestamps, attributes and size.
    metadata_json TEXT NOT NULL CHECK (json_valid(metadata_json)),
    UNIQUE(generation_id, logical_entry_id)
);
CREATE VIRTUAL TABLE global_index_generation_fts USING fts5(
    name, path, extension,
    content='global_index_generation_entries', content_rowid='version_rowid',
    tokenize='trigram'
);
CREATE TRIGGER global_index_generation_entries_insert_guard
BEFORE INSERT ON global_index_generation_entries BEGIN
    SELECT RAISE(ABORT, 'generation_not_staging') WHERE NOT EXISTS (
        SELECT 1 FROM global_index_generations g
        WHERE g.generation_id = new.generation_id AND g.state = 'staging'
    );
END;
CREATE TRIGGER global_index_generation_entries_no_update
BEFORE UPDATE ON global_index_generation_entries BEGIN
    SELECT RAISE(ABORT, 'generation_entries_append_only');
END;
CREATE TRIGGER global_index_generation_entries_no_delete
BEFORE DELETE ON global_index_generation_entries BEGIN
    SELECT RAISE(ABORT, 'generation_gc_not_activated');
END;
CREATE TRIGGER global_index_generation_entries_ai
AFTER INSERT ON global_index_generation_entries BEGIN
    INSERT INTO global_index_generation_fts(rowid, name, path, extension)
        VALUES(new.version_rowid, new.name, new.path, new.extension);
END;
