-- Local-first synchronization state.
--
-- SQLite stays the operational store; these tables track how it relates to the
-- single JSON document in Google Drive. `sync_state` is a singleton row.

CREATE TABLE sync_state (
    id             INTEGER PRIMARY KEY CHECK (id = 1),
    workspace_id   TEXT,
    device_id      TEXT NOT NULL,
    drive_file_id  TEXT,
    base_revision  TEXT,
    base_etag      TEXT,
    base_document  TEXT,
    last_synced_at TEXT,
    dirty          INTEGER NOT NULL DEFAULT 0,
    status         TEXT NOT NULL DEFAULT 'disconnected',
    last_error     TEXT
);

-- A random device id, not derived from any personal information.
INSERT INTO sync_state (id, device_id)
VALUES (1, lower(hex(randomblob(16))));

-- One row per unresolved field-level conflict. `id` is deterministic so a
-- repeated sync never duplicates a conflict the user is already looking at.
CREATE TABLE sync_conflicts (
    id             TEXT PRIMARY KEY NOT NULL,
    community_id   TEXT NOT NULL,
    community_name TEXT NOT NULL,
    field          TEXT NOT NULL,
    base_value     TEXT,
    local_value    TEXT,
    remote_value   TEXT,
    local_record   TEXT,
    remote_record  TEXT,
    created_at     TEXT NOT NULL
);

CREATE INDEX ix_sync_conflicts_created ON sync_conflicts (created_at);

-- Every write path marks the workspace dirty. Triggers are used rather than
-- editing each handler, so new code cannot forget to schedule a sync.
CREATE TRIGGER trg_communities_dirty_insert AFTER INSERT ON communities
BEGIN
    UPDATE sync_state SET dirty = dirty + 1 WHERE id = 1;
END;

CREATE TRIGGER trg_communities_dirty_update AFTER UPDATE ON communities
BEGIN
    UPDATE sync_state SET dirty = dirty + 1 WHERE id = 1;
END;

CREATE TRIGGER trg_communities_dirty_delete AFTER DELETE ON communities
BEGIN
    UPDATE sync_state SET dirty = dirty + 1 WHERE id = 1;
END;

CREATE TRIGGER trg_community_sources_dirty_insert AFTER INSERT ON community_sources
BEGIN
    UPDATE sync_state SET dirty = dirty + 1 WHERE id = 1;
END;

CREATE TRIGGER trg_community_sources_dirty_update AFTER UPDATE ON community_sources
BEGIN
    UPDATE sync_state SET dirty = dirty + 1 WHERE id = 1;
END;

CREATE TRIGGER trg_settings_dirty_insert AFTER INSERT ON settings
BEGIN
    UPDATE sync_state SET dirty = dirty + 1 WHERE id = 1;
END;

CREATE TRIGGER trg_settings_dirty_update AFTER UPDATE ON settings
BEGIN
    UPDATE sync_state SET dirty = dirty + 1 WHERE id = 1;
END;

CREATE TRIGGER trg_settings_dirty_delete AFTER DELETE ON settings
BEGIN
    UPDATE sync_state SET dirty = dirty + 1 WHERE id = 1;
END;

CREATE TRIGGER trg_discovery_runs_dirty_insert AFTER INSERT ON discovery_runs
BEGIN
    UPDATE sync_state SET dirty = dirty + 1 WHERE id = 1;
END;

CREATE TRIGGER trg_discovery_runs_dirty_update AFTER UPDATE ON discovery_runs
BEGIN
    UPDATE sync_state SET dirty = dirty + 1 WHERE id = 1;
END;
