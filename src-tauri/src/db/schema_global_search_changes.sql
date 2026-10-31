-- The search change feed is an additive, self-versioned subsystem because it
-- may ship beside an independent Global Index schema migration. Its state and
-- events are derived invalidation facts; SQLite Global Index rows stay the
-- only durable file/source authority.
-- @object
CREATE TABLE IF NOT EXISTS global_search_change_state (
    singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
    schema_version INTEGER NOT NULL CHECK (schema_version = 1),
    generation INTEGER NOT NULL DEFAULT 0 CHECK (generation >= 0),
    feed_floor_generation INTEGER NOT NULL DEFAULT 0 CHECK (feed_floor_generation >= 0),
    event_count INTEGER NOT NULL DEFAULT 0 CHECK (event_count >= 0),
    feed_saturated INTEGER NOT NULL DEFAULT 0 CHECK (feed_saturated IN (0, 1))
);
-- @object
CREATE TABLE IF NOT EXISTS global_search_changes (
    event_id INTEGER PRIMARY KEY,
    generation INTEGER NOT NULL UNIQUE CHECK (generation > 0),
    change_kind TEXT NOT NULL CHECK (change_kind IN ('entry', 'invalidate')),
    entity_id TEXT NOT NULL,
    source_id TEXT
);
-- @object
CREATE INDEX IF NOT EXISTS idx_global_search_changes_generation
    ON global_search_changes(generation, change_kind, entity_id);
-- @object
CREATE TRIGGER IF NOT EXISTS global_search_changes_entry_ai
AFTER INSERT ON global_entries
BEGIN
    UPDATE global_search_change_state
    SET generation = generation + 1,
        event_count = event_count + CASE
            WHEN feed_saturated = 0 AND event_count < 100000 THEN 1 ELSE 0 END,
        feed_saturated = CASE
            WHEN feed_saturated = 1 THEN 1
            WHEN event_count >= 100000 THEN 1 ELSE 0 END
    WHERE singleton_id = 1;
    INSERT INTO global_search_changes(change_kind, entity_id, source_id, generation)
    SELECT 'entry', NEW.id, NEW.volume_id, generation
    FROM global_search_change_state
    WHERE singleton_id = 1 AND feed_saturated = 0
    ;
END;
-- @object
CREATE TRIGGER IF NOT EXISTS global_search_changes_entry_ad
AFTER DELETE ON global_entries
BEGIN
    UPDATE global_search_change_state
    SET generation = generation + 1,
        event_count = event_count + CASE
            WHEN feed_saturated = 0 AND event_count < 100000 THEN 1 ELSE 0 END,
        feed_saturated = CASE
            WHEN feed_saturated = 1 THEN 1
            WHEN event_count >= 100000 THEN 1 ELSE 0 END
    WHERE singleton_id = 1;
    INSERT INTO global_search_changes(change_kind, entity_id, source_id, generation)
    SELECT 'entry', OLD.id, OLD.volume_id, generation
    FROM global_search_change_state
    WHERE singleton_id = 1 AND feed_saturated = 0
    ;
END;
-- @object
CREATE TRIGGER IF NOT EXISTS global_search_changes_entry_au
AFTER UPDATE ON global_entries
WHEN OLD.id IS NOT NEW.id OR OLD.volume_id IS NOT NEW.volume_id
  OR OLD.platform_file_id IS NOT NEW.platform_file_id
  OR OLD.parent_platform_file_id IS NOT NEW.parent_platform_file_id
  OR OLD.name IS NOT NEW.name OR OLD.name_normalized IS NOT NEW.name_normalized
  OR OLD.path IS NOT NEW.path OR OLD.path_normalized IS NOT NEW.path_normalized
  OR OLD.extension IS NOT NEW.extension OR OLD.is_directory IS NOT NEW.is_directory
  OR OLD.size IS NOT NEW.size OR OLD.created_at_fs IS NOT NEW.created_at_fs
  OR OLD.modified_at_fs IS NOT NEW.modified_at_fs
  OR OLD.file_attributes IS NOT NEW.file_attributes
  OR OLD.is_hidden IS NOT NEW.is_hidden OR OLD.is_system IS NOT NEW.is_system
  OR OLD.is_stale IS NOT NEW.is_stale
  OR OLD.source_provider IS NOT NEW.source_provider
BEGIN
    UPDATE global_search_change_state
    SET generation = generation + 1,
        event_count = event_count + CASE
            WHEN feed_saturated = 0 AND event_count < 100000 THEN 1 ELSE 0 END,
        feed_saturated = CASE
            WHEN feed_saturated = 1 THEN 1
            WHEN event_count >= 100000 THEN 1 ELSE 0 END
    WHERE singleton_id = 1;
    INSERT INTO global_search_changes(change_kind, entity_id, source_id, generation)
    SELECT CASE WHEN OLD.id IS NOT NEW.id OR OLD.volume_id IS NOT NEW.volume_id
                THEN 'invalidate' ELSE 'entry' END,
           CASE WHEN OLD.id IS NOT NEW.id OR OLD.volume_id IS NOT NEW.volume_id
                THEN 'all' ELSE NEW.id END,
           NEW.volume_id,
           generation
    FROM global_search_change_state
    WHERE singleton_id = 1 AND feed_saturated = 0
    ;
END;
-- @object
CREATE TRIGGER IF NOT EXISTS global_search_changes_source_ai
AFTER INSERT ON global_volumes
BEGIN
    UPDATE global_search_change_state
    SET generation = generation + 1,
        event_count = event_count + CASE
            WHEN feed_saturated = 0 AND event_count < 100000 THEN 1 ELSE 0 END,
        feed_saturated = CASE
            WHEN feed_saturated = 1 THEN 1
            WHEN event_count >= 100000 THEN 1 ELSE 0 END
    WHERE singleton_id = 1;
    INSERT INTO global_search_changes(change_kind, entity_id, source_id, generation)
    SELECT 'invalidate', 'all', NEW.id, generation
    FROM global_search_change_state
    WHERE singleton_id = 1 AND feed_saturated = 0
    ;
END;
-- @object
CREATE TRIGGER IF NOT EXISTS global_search_changes_source_ad
AFTER DELETE ON global_volumes
BEGIN
    UPDATE global_search_change_state
    SET generation = generation + 1,
        event_count = event_count + CASE
            WHEN feed_saturated = 0 AND event_count < 100000 THEN 1 ELSE 0 END,
        feed_saturated = CASE
            WHEN feed_saturated = 1 THEN 1
            WHEN event_count >= 100000 THEN 1 ELSE 0 END
    WHERE singleton_id = 1;
    INSERT INTO global_search_changes(change_kind, entity_id, source_id, generation)
    SELECT 'invalidate', 'all', OLD.id, generation
    FROM global_search_change_state
    WHERE singleton_id = 1 AND feed_saturated = 0
    ;
END;
-- @object
CREATE TRIGGER IF NOT EXISTS global_search_changes_source_au
AFTER UPDATE ON global_volumes
WHEN OLD.id IS NOT NEW.id OR OLD.stable_volume_id IS NOT NEW.stable_volume_id
  OR OLD.mount_path IS NOT NEW.mount_path OR OLD.enabled IS NOT NEW.enabled
  OR OLD.provider IS NOT NEW.provider OR OLD.index_status IS NOT NEW.index_status
  OR OLD.last_error IS NOT NEW.last_error
BEGIN
    UPDATE global_search_change_state
    SET generation = generation + 1,
        event_count = event_count + CASE
            WHEN feed_saturated = 0 AND event_count < 100000 THEN 1 ELSE 0 END,
        feed_saturated = CASE
            WHEN feed_saturated = 1 THEN 1
            WHEN event_count >= 100000 THEN 1 ELSE 0 END
    WHERE singleton_id = 1;
    INSERT INTO global_search_changes(change_kind, entity_id, source_id, generation)
    SELECT 'invalidate', 'all', NEW.id, generation
    FROM global_search_change_state
    WHERE singleton_id = 1 AND feed_saturated = 0
    ;
END;
