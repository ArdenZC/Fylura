-- @object
CREATE TABLE IF NOT EXISTS global_entry_source_facts (
    volume_id TEXT PRIMARY KEY NOT NULL,
    active_rows INTEGER NOT NULL CHECK (active_rows >= 0),
    active_ids INTEGER NOT NULL CHECK (active_ids >= 0 AND active_ids <= active_rows),
    max_last_seen_at INTEGER,
    CHECK (active_rows > 0 OR max_last_seen_at IS NULL)
) WITHOUT ROWID;
-- @object
CREATE TRIGGER IF NOT EXISTS global_entry_source_facts_ai
AFTER INSERT ON global_entries WHEN new.is_stale = 0
BEGIN
    INSERT INTO global_entry_source_facts(volume_id, active_rows, active_ids, max_last_seen_at)
    VALUES(new.volume_id, 1, new.id IS NOT NULL, new.last_seen_at)
    ON CONFLICT(volume_id) DO UPDATE SET
        active_rows = active_rows + 1,
        active_ids = active_ids + (new.id IS NOT NULL),
        max_last_seen_at = CASE WHEN max_last_seen_at IS NULL OR new.last_seen_at > max_last_seen_at
            THEN new.last_seen_at ELSE max_last_seen_at END;
END;
-- @object
CREATE TRIGGER IF NOT EXISTS global_entry_source_facts_ad
AFTER DELETE ON global_entries WHEN old.is_stale = 0
BEGIN
    SELECT RAISE(ABORT, 'global_source_facts_missing')
    WHERE NOT EXISTS(SELECT 1 FROM global_entry_source_facts WHERE volume_id = old.volume_id);
    UPDATE global_entry_source_facts SET
        active_rows = active_rows - 1,
        active_ids = active_ids - (old.id IS NOT NULL),
        max_last_seen_at = CASE WHEN old.last_seen_at IS max_last_seen_at
            THEN (SELECT MAX(last_seen_at) FROM global_entries INDEXED BY idx_global_entries_active_source_facts
                  WHERE volume_id = old.volume_id AND is_stale = 0)
            ELSE max_last_seen_at END
    WHERE volume_id = old.volume_id;
END;
-- @object
CREATE TRIGGER IF NOT EXISTS global_entry_source_facts_au
AFTER UPDATE OF volume_id, is_stale, id, last_seen_at ON global_entries
WHEN (old.is_stale = 0 OR new.is_stale = 0) AND
    (old.volume_id IS NOT new.volume_id OR old.is_stale IS NOT new.is_stale
     OR old.id IS NOT new.id OR old.last_seen_at IS NOT new.last_seen_at)
BEGIN
    SELECT RAISE(ABORT, 'global_source_facts_missing')
    WHERE old.is_stale = 0 AND NOT EXISTS(
        SELECT 1 FROM global_entry_source_facts WHERE volume_id = old.volume_id);
    UPDATE global_entry_source_facts SET
        active_rows = active_rows - 1,
        active_ids = active_ids - (old.id IS NOT NULL),
        max_last_seen_at = CASE WHEN old.last_seen_at IS max_last_seen_at
            THEN (SELECT MAX(last_seen_at) FROM global_entries INDEXED BY idx_global_entries_active_source_facts
                  WHERE volume_id = old.volume_id AND is_stale = 0)
            ELSE max_last_seen_at END
    WHERE old.is_stale = 0 AND old.volume_id IS NOT new.volume_id AND volume_id = old.volume_id;
    INSERT INTO global_entry_source_facts(volume_id, active_rows, active_ids, max_last_seen_at)
    SELECT new.volume_id, new.is_stale = 0, new.is_stale = 0 AND new.id IS NOT NULL,
           CASE WHEN new.is_stale = 0 THEN new.last_seen_at END
    WHERE new.is_stale = 0 OR old.volume_id IS new.volume_id
    ON CONFLICT(volume_id) DO UPDATE SET
        active_rows = active_rows + (new.is_stale = 0)
            - (old.volume_id IS new.volume_id AND old.is_stale = 0),
        active_ids = active_ids + (new.is_stale = 0 AND new.id IS NOT NULL)
            - (old.volume_id IS new.volume_id AND old.is_stale = 0 AND old.id IS NOT NULL),
        max_last_seen_at = CASE
            WHEN old.volume_id IS new.volume_id AND old.is_stale = 0
                 AND old.last_seen_at IS max_last_seen_at
                 AND (new.is_stale <> 0 OR old.last_seen_at IS NOT new.last_seen_at)
            THEN (SELECT MAX(last_seen_at) FROM global_entries INDEXED BY idx_global_entries_active_source_facts
                  WHERE volume_id = new.volume_id AND is_stale = 0)
            WHEN new.is_stale = 0 AND (max_last_seen_at IS NULL OR new.last_seen_at > max_last_seen_at)
            THEN new.last_seen_at ELSE max_last_seen_at END
    WHERE old.volume_id IS NOT new.volume_id OR old.is_stale IS NOT new.is_stale
       OR ((old.id IS NULL) <> (new.id IS NULL))
       OR (old.is_stale = 0 AND old.last_seen_at IS max_last_seen_at)
       OR (new.is_stale = 0 AND (max_last_seen_at IS NULL OR new.last_seen_at > max_last_seen_at));
END;
-- @object
CREATE TRIGGER IF NOT EXISTS global_entry_source_facts_volume_ai
AFTER INSERT ON global_volumes
BEGIN
    INSERT INTO global_entry_source_facts(volume_id, active_rows, active_ids, max_last_seen_at)
    VALUES(new.id, 0, 0, NULL) ON CONFLICT(volume_id) DO NOTHING;
END;
-- @object
CREATE TRIGGER IF NOT EXISTS global_entry_source_facts_volume_au
AFTER UPDATE OF id ON global_volumes WHEN old.id IS NOT new.id
BEGIN
    INSERT INTO global_entry_source_facts(volume_id, active_rows, active_ids, max_last_seen_at)
    VALUES(new.id, 0, 0, NULL) ON CONFLICT(volume_id) DO NOTHING;
END;
