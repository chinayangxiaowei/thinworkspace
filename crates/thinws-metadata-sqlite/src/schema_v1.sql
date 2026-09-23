CREATE TABLE installation (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    instance_id TEXT NOT NULL UNIQUE
        CHECK (
            length(instance_id) = 36
            AND substr(instance_id, 9, 1) = '-'
            AND substr(instance_id, 14, 1) = '-'
            AND substr(instance_id, 15, 1) = '7'
            AND substr(instance_id, 19, 1) = '-'
            AND substr(instance_id, 20, 1) IN ('8', '9', 'a', 'b')
            AND substr(instance_id, 24, 1) = '-'
            AND instr(instance_id, char(0)) = 0
            AND length(replace(instance_id, '-', '')) = 32
            AND replace(instance_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        ),
    data_root BLOB NOT NULL UNIQUE CHECK (length(data_root) > 0),
    volume_id TEXT NOT NULL
        CHECK (
            length(volume_id) = 36
            AND substr(volume_id, 9, 1) = '-'
            AND substr(volume_id, 14, 1) = '-'
            AND substr(volume_id, 19, 1) = '-'
            AND substr(volume_id, 24, 1) = '-'
            AND instr(volume_id, char(0)) = 0
            AND length(replace(volume_id, '-', '')) = 32
            AND replace(volume_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        ),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0)
) STRICT, WITHOUT ROWID;

CREATE TABLE workspaces (
    workspace_id TEXT PRIMARY KEY
        CHECK (
            length(workspace_id) = 39
            AND substr(workspace_id, 1, 3) = 'ws_'
            AND substr(workspace_id, 12, 1) = '-'
            AND substr(workspace_id, 17, 1) = '-'
            AND substr(workspace_id, 18, 1) = '7'
            AND substr(workspace_id, 22, 1) = '-'
            AND substr(workspace_id, 23, 1) IN ('8', '9', 'a', 'b')
            AND substr(workspace_id, 27, 1) = '-'
            AND instr(workspace_id, char(0)) = 0
            AND length(replace(substr(workspace_id, 4), '-', '')) = 32
            AND replace(substr(workspace_id, 4), '-', '') NOT GLOB '*[^0-9a-f]*'
        ),
    instance_id TEXT NOT NULL,
    name TEXT NOT NULL COLLATE BINARY UNIQUE
        CHECK (
            length(name) BETWEEN 1 AND 63
            AND instr(name, char(0)) = 0
            AND name NOT GLOB '*[^a-z0-9._-]*'
            AND substr(name, 1, 1) GLOB '[a-z0-9]'
            AND substr(name, -1, 1) GLOB '[a-z0-9]'
            AND instr(name, '..') = 0
        ),
    source_path BLOB NOT NULL CHECK (length(source_path) > 0),
    target_path BLOB NOT NULL UNIQUE CHECK (length(target_path) > 0),
    source_volume_id TEXT NOT NULL,
    data_volume_id TEXT NOT NULL,
    allow_full_copy INTEGER NOT NULL CHECK (allow_full_copy IN (0, 1)),
    state TEXT NOT NULL CHECK (state IN ('creating', 'ready', 'deleting', 'error')),
    last_error_code TEXT,
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0),
    updated_at_unix_ms INTEGER NOT NULL CHECK (updated_at_unix_ms >= created_at_unix_ms),
    FOREIGN KEY (instance_id) REFERENCES installation(instance_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    CHECK (
        (
            state = 'error'
            AND last_error_code IS NOT NULL
            AND last_error_code GLOB 'E_*'
        )
        OR (state <> 'error' AND last_error_code IS NULL)
    )
) STRICT, WITHOUT ROWID;

CREATE INDEX workspaces_state_name ON workspaces(state, name);

CREATE TABLE materialization_receipts (
    workspace_id TEXT PRIMARY KEY,
    receipt_schema_version INTEGER NOT NULL CHECK (receipt_schema_version > 0),
    receipt_json TEXT NOT NULL CHECK (json_valid(receipt_json)),
    recorded_at_unix_ms INTEGER NOT NULL CHECK (recorded_at_unix_ms >= 0),
    FOREIGN KEY (workspace_id) REFERENCES workspaces(workspace_id)
        ON UPDATE RESTRICT ON DELETE CASCADE
) STRICT, WITHOUT ROWID;

CREATE TABLE deletion_tombstones (
    workspace_id TEXT PRIMARY KEY,
    instance_id TEXT NOT NULL,
    deleted_at_unix_ms INTEGER NOT NULL CHECK (deleted_at_unix_ms >= 0),
    FOREIGN KEY (instance_id) REFERENCES installation(instance_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
) STRICT, WITHOUT ROWID;

CREATE TRIGGER installation_insert_once
BEFORE INSERT ON installation
WHEN EXISTS (SELECT 1 FROM installation)
BEGIN
    SELECT RAISE(ABORT, 'installation is insert-once');
END;

CREATE TRIGGER installation_no_update
BEFORE UPDATE ON installation
BEGIN
    SELECT RAISE(ABORT, 'installation is immutable');
END;

CREATE TRIGGER installation_no_delete
BEFORE DELETE ON installation
BEGIN
    SELECT RAISE(ABORT, 'installation is immutable');
END;

CREATE TRIGGER workspaces_insert_guard
BEFORE INSERT ON workspaces
BEGIN
    SELECT CASE WHEN EXISTS (
        SELECT 1 FROM workspaces
        WHERE workspace_id = NEW.workspace_id
           OR name = NEW.name
           OR target_path = NEW.target_path
    ) THEN RAISE(ABORT, 'workspace conflicts cannot be replaced') END;
    SELECT CASE WHEN NEW.state <> 'creating' OR NEW.last_error_code IS NOT NULL
        THEN RAISE(ABORT, 'workspace must be inserted as creating') END;
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM installation
        WHERE instance_id = NEW.instance_id
          AND volume_id = NEW.source_volume_id
          AND volume_id = NEW.data_volume_id
    ) THEN RAISE(ABORT, 'workspace installation or volume mismatch') END;
    SELECT CASE WHEN EXISTS (
        SELECT 1 FROM deletion_tombstones WHERE workspace_id = NEW.workspace_id
    ) THEN RAISE(ABORT, 'workspace ID is tombstoned') END;
END;

CREATE TRIGGER workspaces_immutable_fields
BEFORE UPDATE ON workspaces
WHEN OLD.workspace_id IS NOT NEW.workspace_id
  OR OLD.instance_id IS NOT NEW.instance_id
  OR OLD.name IS NOT NEW.name
  OR OLD.source_path IS NOT NEW.source_path
  OR OLD.target_path IS NOT NEW.target_path
  OR OLD.source_volume_id IS NOT NEW.source_volume_id
  OR OLD.data_volume_id IS NOT NEW.data_volume_id
  OR OLD.allow_full_copy IS NOT NEW.allow_full_copy
  OR OLD.created_at_unix_ms IS NOT NEW.created_at_unix_ms
BEGIN
    SELECT RAISE(ABORT, 'workspace identity fields are immutable');
END;

CREATE TRIGGER workspaces_state_transition
BEFORE UPDATE OF state ON workspaces
WHEN OLD.state <> NEW.state
 AND NOT (
    (OLD.state = 'creating' AND NEW.state IN ('ready', 'error', 'deleting'))
    OR (OLD.state = 'ready' AND NEW.state IN ('deleting', 'error'))
    OR (OLD.state = 'deleting' AND NEW.state = 'error')
    OR (OLD.state = 'error' AND NEW.state = 'deleting')
 )
BEGIN
    SELECT RAISE(ABORT, 'illegal workspace state transition');
END;

CREATE TRIGGER workspaces_timestamp_monotonic
BEFORE UPDATE ON workspaces
WHEN NEW.updated_at_unix_ms < OLD.updated_at_unix_ms
BEGIN
    SELECT RAISE(ABORT, 'workspace timestamp regressed');
END;

CREATE TRIGGER workspaces_ready_requires_receipt
BEFORE UPDATE OF state ON workspaces
WHEN OLD.state <> 'ready'
 AND NEW.state = 'ready'
 AND NOT EXISTS (
    SELECT 1 FROM materialization_receipts WHERE workspace_id = NEW.workspace_id
 )
BEGIN
    SELECT RAISE(ABORT, 'ready workspace requires final receipt');
END;

CREATE TRIGGER receipts_insert_once
BEFORE INSERT ON materialization_receipts
WHEN EXISTS (
    SELECT 1 FROM materialization_receipts WHERE workspace_id = NEW.workspace_id
)
BEGIN
    SELECT RAISE(ABORT, 'receipt is insert-once');
END;

CREATE TRIGGER receipts_insert_only_creating
BEFORE INSERT ON materialization_receipts
WHEN (SELECT state FROM workspaces WHERE workspace_id = NEW.workspace_id) IS NOT 'creating'
BEGIN
    SELECT RAISE(ABORT, 'receipt requires creating workspace');
END;

CREATE TRIGGER receipts_no_update
BEFORE UPDATE ON materialization_receipts
BEGIN
    SELECT RAISE(ABORT, 'receipt is immutable');
END;

CREATE TRIGGER receipts_no_direct_delete
BEFORE DELETE ON materialization_receipts
WHEN EXISTS (SELECT 1 FROM workspaces WHERE workspace_id = OLD.workspace_id)
BEGIN
    SELECT RAISE(ABORT, 'receipt can only be cascade deleted');
END;

CREATE TRIGGER tombstones_insert_once
BEFORE INSERT ON deletion_tombstones
WHEN EXISTS (
    SELECT 1 FROM deletion_tombstones WHERE workspace_id = NEW.workspace_id
)
BEGIN
    SELECT RAISE(ABORT, 'tombstone is insert-once');
END;

CREATE TRIGGER tombstones_insert_only_deleting
BEFORE INSERT ON deletion_tombstones
WHEN NOT EXISTS (
    SELECT 1 FROM workspaces
    WHERE workspace_id = NEW.workspace_id
      AND instance_id = NEW.instance_id
      AND state = 'deleting'
)
BEGIN
    SELECT RAISE(ABORT, 'tombstone requires matching deleting workspace');
END;

CREATE TRIGGER tombstones_no_update
BEFORE UPDATE ON deletion_tombstones
BEGIN
    SELECT RAISE(ABORT, 'tombstone is immutable');
END;

CREATE TRIGGER tombstones_no_delete
BEFORE DELETE ON deletion_tombstones
BEGIN
    SELECT RAISE(ABORT, 'tombstone is immutable');
END;

CREATE TRIGGER workspaces_delete_requires_tombstone
BEFORE DELETE ON workspaces
WHEN NOT EXISTS (
    SELECT 1 FROM deletion_tombstones
    WHERE workspace_id = OLD.workspace_id
      AND instance_id = OLD.instance_id
)
BEGIN
    SELECT RAISE(ABORT, 'workspace delete requires tombstone');
END;
