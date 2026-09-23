#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! SQLite implementation of the Phase 1 MetadataStore boundary.

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params,
};
use thinws_core::{
    AbsolutePath, DeletionTombstone, ErrorCode, InstallationIdentity, InstallationRecord,
    InstanceId, RemovalMode, UnixMillis, VolumeId, WorkspaceEvent, WorkspaceId, WorkspaceName,
    WorkspaceRecord, WorkspaceReservation, WorkspaceState,
};
use thinws_ports::{MetadataStore, PortConflict, PortError, PortErrorKind};

/// SQLite application ID for ASCII `THWS`.
pub const APPLICATION_ID: i32 = 1_414_027_091;
/// Current durable metadata schema version.
pub const SCHEMA_VERSION: i32 = 1;

const SCHEMA_V1: &str = include_str!("schema_v1.sql");
const DATABASE_OPEN_FLAGS: OpenFlags = OpenFlags::SQLITE_OPEN_READ_WRITE
    .union(OpenFlags::SQLITE_OPEN_CREATE)
    .union(OpenFlags::SQLITE_OPEN_NO_MUTEX)
    .union(OpenFlags::SQLITE_OPEN_NOFOLLOW);

/// Verified settings on the concrete SQLite connection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConnectionSettings {
    /// Effective journal mode, normalized to lowercase.
    pub journal_mode: String,
    /// SQLite numeric synchronous level (`2` means FULL).
    pub synchronous: i64,
    /// Whether foreign key enforcement is enabled on this connection.
    pub foreign_keys: bool,
    /// Effective SQLite busy timeout in milliseconds.
    pub busy_timeout_ms: i64,
}

/// One open, identity-validated metadata database.
pub struct SqliteMetadataStore {
    connection: Connection,
    installation: InstallationRecord,
}

impl SqliteMetadataStore {
    /// Opens or transactionally initializes a v1 database.
    ///
    /// `installation.created_at` is used only for a truly empty v0 database;
    /// reopening validates identity and returns the timestamp already stored.
    pub fn open(
        path: &Path,
        installation: &InstallationRecord,
        busy_timeout: Duration,
    ) -> Result<Self, PortError> {
        let mut connection = Connection::open_with_flags(path, DATABASE_OPEN_FLAGS)
            .map_err(|error| storage_error("open metadata database", error))?;
        let application_id = pragma_i32(&connection, "application_id")?;
        let user_version = pragma_i32(&connection, "user_version")?;
        let user_table_count = user_table_count(&connection)?;

        let initialize = match (application_id, user_version, user_table_count) {
            (0, 0, 0) => true,
            (APPLICATION_ID, SCHEMA_VERSION, _) => false,
            (APPLICATION_ID, version, _) if version > SCHEMA_VERSION => {
                return Err(PortError::new(
                    PortErrorKind::UnsupportedVersion,
                    "open future metadata schema",
                ));
            }
            (0, _, _) => {
                return Err(PortError::new(
                    PortErrorKind::UnsupportedVersion,
                    "refuse unowned metadata database",
                ));
            }
            (APPLICATION_ID, _, _) => {
                return Err(PortError::new(
                    PortErrorKind::InvalidData,
                    "validate metadata schema version",
                ));
            }
            _ => {
                return Err(PortError::new(
                    PortErrorKind::UnsupportedVersion,
                    "refuse foreign metadata database",
                ));
            }
        };

        let actual = if initialize {
            configure_connection(&connection, busy_timeout)?;
            migrate_v0(&mut connection, installation, SCHEMA_V1)?;
            read_installation(&connection)?
        } else {
            let actual = read_installation(&connection)?;
            if actual.identity() != installation.identity() {
                return Err(PortError::conflict(
                    "validate installation identity",
                    PortConflict::InstallationIdentity,
                ));
            }
            configure_connection(&connection, busy_timeout)?;
            actual
        };

        Ok(Self {
            connection,
            installation: actual,
        })
    }

    /// Returns settings verified on the store's own connection.
    pub fn connection_settings(&self) -> Result<ConnectionSettings, PortError> {
        read_connection_settings(&self.connection)
    }
}

impl MetadataStore for SqliteMetadataStore {
    fn installation(&self) -> &InstallationRecord {
        &self.installation
    }

    fn reserve_workspace(
        &mut self,
        reservation: &WorkspaceReservation,
    ) -> Result<WorkspaceRecord, PortError> {
        if (
            reservation.instance_id(),
            reservation.source_volume_id(),
            reservation.data_volume_id(),
        ) != (
            self.installation.identity().instance_id(),
            self.installation.identity().volume_id(),
            self.installation.identity().volume_id(),
        ) {
            return Err(PortError::conflict(
                "reserve workspace",
                PortConflict::InstallationIdentity,
            ));
        }

        let result = self.connection.execute(
            "INSERT INTO workspaces (
                workspace_id, instance_id, name, source_path, target_path,
                source_volume_id, data_volume_id, allow_full_copy, state,
                last_error_code, created_at_unix_ms, updated_at_unix_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'creating', NULL, ?9, ?9)",
            params![
                reservation.workspace_id().to_string(),
                reservation.instance_id().to_string(),
                reservation.name().as_str(),
                reservation.source_path().as_bytes(),
                reservation.target_path().as_bytes(),
                reservation.source_volume_id().to_string(),
                reservation.data_volume_id().to_string(),
                i64::from(reservation.allow_full_copy()),
                reservation.created_at().get(),
            ],
        );
        if let Err(error) = result {
            return Err(self.classify_reservation_error(reservation, error));
        }
        WorkspaceRecord::new(
            reservation.clone(),
            WorkspaceState::Creating,
            None,
            reservation.created_at(),
        )
        .map_err(|error| invalid_data("build reserved workspace", error))
    }

    fn workspace(&self, workspace_id: WorkspaceId) -> Result<Option<WorkspaceRecord>, PortError> {
        read_workspace(&self.connection, workspace_id)
    }

    fn workspaces(&self) -> Result<Vec<WorkspaceRecord>, PortError> {
        let sql = format!(
            "SELECT {WORKSPACE_COLUMNS} FROM workspaces \
             ORDER BY name COLLATE BINARY, workspace_id"
        );
        let mut statement = self
            .connection
            .prepare(&sql)
            .map_err(|error| storage_error("prepare Workspace list", error))?;
        let rows = statement
            .query_map([], RawWorkspace::from_row)
            .map_err(|error| storage_error("query Workspace list", error))?;
        rows.map(|row| {
            row.map_err(|error| storage_error("read Workspace row", error))?
                .into_record()
        })
        .collect()
    }

    fn record_failure(
        &mut self,
        workspace_id: WorkspaceId,
        expected: WorkspaceState,
        error_code: ErrorCode,
        updated_at: UnixMillis,
    ) -> Result<WorkspaceRecord, PortError> {
        expected
            .transition(WorkspaceEvent::Failed)
            .map_err(|error| invalid_data("validate failure transition", error))?;
        let changed = self.connection.execute(
            "UPDATE workspaces
             SET state = 'error', last_error_code = ?1, updated_at_unix_ms = ?2
             WHERE workspace_id = ?3 AND state = ?4",
            params![
                error_code.as_str(),
                updated_at.get(),
                workspace_id.to_string(),
                expected.as_str(),
            ],
        );
        self.finish_expected_update(workspace_id, expected, changed, "record Workspace failure")
    }

    fn begin_removal(
        &mut self,
        workspace_id: WorkspaceId,
        expected: WorkspaceState,
        mode: RemovalMode,
        updated_at: UnixMillis,
    ) -> Result<WorkspaceRecord, PortError> {
        let next = expected
            .transition(WorkspaceEvent::BeginRemoval(mode))
            .map_err(|error| invalid_data("validate removal transition", error))?;
        if next == expected {
            return self.require_expected(workspace_id, expected, "reaffirm Workspace removal");
        }
        let changed = self.connection.execute(
            "UPDATE workspaces
             SET state = 'deleting', last_error_code = NULL, updated_at_unix_ms = ?1
             WHERE workspace_id = ?2 AND state = ?3",
            params![
                updated_at.get(),
                workspace_id.to_string(),
                expected.as_str()
            ],
        );
        self.finish_expected_update(workspace_id, expected, changed, "begin Workspace removal")
    }

    fn complete_deletion(
        &mut self,
        workspace_id: WorkspaceId,
        instance_id: InstanceId,
        deleted_at: UnixMillis,
    ) -> Result<DeletionTombstone, PortError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| storage_error("begin deletion transaction", error))?;
        let found: Option<(String, String)> = transaction
            .query_row(
                "SELECT instance_id, state FROM workspaces WHERE workspace_id = ?1",
                [workspace_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| storage_error("read deleting Workspace", error))?;
        let Some((actual_instance, actual_state)) = found else {
            return Err(PortError::new(
                PortErrorKind::NotFound,
                "complete Workspace deletion",
            ));
        };
        if actual_instance != instance_id.to_string() {
            return Err(PortError::conflict(
                "complete Workspace deletion",
                PortConflict::InstallationIdentity,
            ));
        }
        if actual_state != WorkspaceState::Deleting.as_str() {
            return Err(PortError::conflict(
                "complete Workspace deletion",
                PortConflict::ExpectedState,
            ));
        }
        transaction
            .execute(
                "INSERT INTO deletion_tombstones (workspace_id, instance_id, deleted_at_unix_ms)
                 VALUES (?1, ?2, ?3)",
                params![
                    workspace_id.to_string(),
                    instance_id.to_string(),
                    deleted_at.get()
                ],
            )
            .map_err(|error| storage_error("write deletion tombstone", error))?;
        let deleted = transaction
            .execute(
                "DELETE FROM workspaces WHERE workspace_id = ?1 AND state = 'deleting'",
                [workspace_id.to_string()],
            )
            .map_err(|error| storage_error("delete active Workspace row", error))?;
        if deleted != 1 {
            return Err(PortError::conflict(
                "delete active Workspace row",
                PortConflict::ExpectedState,
            ));
        }
        transaction
            .commit()
            .map_err(|error| storage_error("commit deletion transaction", error))?;
        Ok(DeletionTombstone::new(
            workspace_id,
            instance_id,
            deleted_at,
        ))
    }
}

impl SqliteMetadataStore {
    fn finish_expected_update(
        &self,
        workspace_id: WorkspaceId,
        expected: WorkspaceState,
        result: rusqlite::Result<usize>,
        operation: &'static str,
    ) -> Result<WorkspaceRecord, PortError> {
        let changed = result.map_err(|error| update_error(operation, error))?;
        if changed == 0 {
            return match self.workspace(workspace_id)? {
                None => Err(PortError::new(PortErrorKind::NotFound, operation)),
                Some(_) => Err(PortError::conflict(operation, PortConflict::ExpectedState)),
            };
        }
        let record = self
            .workspace(workspace_id)?
            .ok_or_else(|| PortError::conflict(operation, PortConflict::ExpectedState))?;
        if record.state() == expected {
            return Err(PortError::conflict(operation, PortConflict::ExpectedState));
        }
        Ok(record)
    }

    fn require_expected(
        &self,
        workspace_id: WorkspaceId,
        expected: WorkspaceState,
        operation: &'static str,
    ) -> Result<WorkspaceRecord, PortError> {
        match self.workspace(workspace_id)? {
            None => Err(PortError::new(PortErrorKind::NotFound, operation)),
            Some(record) if record.state() != expected => {
                Err(PortError::conflict(operation, PortConflict::ExpectedState))
            }
            Some(record) => Ok(record),
        }
    }

    fn classify_reservation_error(
        &self,
        reservation: &WorkspaceReservation,
        error: rusqlite::Error,
    ) -> PortError {
        if !is_constraint(&error) {
            return storage_error("reserve workspace", error);
        }
        let conflict = self
            .connection
            .query_row(
                "SELECT CASE
                    WHEN EXISTS (SELECT 1 FROM workspaces WHERE workspace_id = ?1)
                      OR EXISTS (SELECT 1 FROM deletion_tombstones WHERE workspace_id = ?1)
                        THEN 1
                    WHEN EXISTS (SELECT 1 FROM workspaces WHERE name = ?2) THEN 2
                    WHEN EXISTS (SELECT 1 FROM workspaces WHERE target_path = ?3) THEN 3
                    ELSE 0 END",
                params![
                    reservation.workspace_id().to_string(),
                    reservation.name().as_str(),
                    reservation.target_path().as_bytes(),
                ],
                |row| row.get::<_, i64>(0),
            )
            .ok()
            .and_then(|value| match value {
                1 => Some(PortConflict::WorkspaceId),
                2 => Some(PortConflict::WorkspaceName),
                3 => Some(PortConflict::TargetPath),
                _ => None,
            });
        match conflict {
            Some(conflict) => PortError::conflict("reserve workspace", conflict).with_source(error),
            None => invalid_data("reserve workspace", error),
        }
    }
}

const WORKSPACE_COLUMNS: &str = "workspace_id, instance_id, name, source_path, target_path, \
    source_volume_id, data_volume_id, allow_full_copy, state, last_error_code, \
    created_at_unix_ms, updated_at_unix_ms";

struct RawWorkspace {
    workspace_id: String,
    instance_id: String,
    name: String,
    source_path: Vec<u8>,
    target_path: Vec<u8>,
    source_volume_id: String,
    data_volume_id: String,
    allow_full_copy: i64,
    state: String,
    last_error_code: Option<String>,
    created_at: i64,
    updated_at: i64,
}

impl RawWorkspace {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            workspace_id: row.get(0)?,
            instance_id: row.get(1)?,
            name: row.get(2)?,
            source_path: row.get(3)?,
            target_path: row.get(4)?,
            source_volume_id: row.get(5)?,
            data_volume_id: row.get(6)?,
            allow_full_copy: row.get(7)?,
            state: row.get(8)?,
            last_error_code: row.get(9)?,
            created_at: row.get(10)?,
            updated_at: row.get(11)?,
        })
    }

    fn into_record(self) -> Result<WorkspaceRecord, PortError> {
        let reservation = WorkspaceReservation::new(
            WorkspaceId::from_str(&self.workspace_id)
                .map_err(|error| invalid_data("decode Workspace ID", error))?,
            InstanceId::from_str(&self.instance_id)
                .map_err(|error| invalid_data("decode instance ID", error))?,
            WorkspaceName::from_str(&self.name)
                .map_err(|error| invalid_data("decode Workspace name", error))?,
            AbsolutePath::try_from_bytes(self.source_path)
                .map_err(|error| invalid_data("decode source path", error))?,
            AbsolutePath::try_from_bytes(self.target_path)
                .map_err(|error| invalid_data("decode target path", error))?,
            VolumeId::from_str(&self.source_volume_id)
                .map_err(|error| invalid_data("decode source volume", error))?,
            VolumeId::from_str(&self.data_volume_id)
                .map_err(|error| invalid_data("decode data volume", error))?,
            match self.allow_full_copy {
                0 => false,
                1 => true,
                _ => {
                    return Err(PortError::new(
                        PortErrorKind::InvalidData,
                        "decode full-copy permission",
                    ));
                }
            },
            UnixMillis::new(self.created_at)
                .map_err(|error| invalid_data("decode creation timestamp", error))?,
        );
        let state = WorkspaceState::from_str(&self.state)
            .map_err(|error| invalid_data("decode Workspace state", error))?;
        let last_error_code = self
            .last_error_code
            .map(|value| {
                ErrorCode::from_str(&value)
                    .map_err(|error| invalid_data("decode Workspace error code", error))
            })
            .transpose()?;
        WorkspaceRecord::new(
            reservation,
            state,
            last_error_code,
            UnixMillis::new(self.updated_at)
                .map_err(|error| invalid_data("decode update timestamp", error))?,
        )
        .map_err(|error| invalid_data("validate Workspace row", error))
    }
}

fn read_workspace(
    connection: &Connection,
    workspace_id: WorkspaceId,
) -> Result<Option<WorkspaceRecord>, PortError> {
    let sql = format!("SELECT {WORKSPACE_COLUMNS} FROM workspaces WHERE workspace_id = ?1");
    connection
        .query_row(&sql, [workspace_id.to_string()], RawWorkspace::from_row)
        .optional()
        .map_err(|error| storage_error("read Workspace", error))?
        .map(RawWorkspace::into_record)
        .transpose()
}

fn configure_connection(connection: &Connection, timeout: Duration) -> Result<(), PortError> {
    let timeout_ms = i64::try_from(timeout.as_millis())
        .map_err(|error| invalid_data("validate SQLite busy timeout", error))?;
    connection
        .busy_timeout(timeout)
        .map_err(|error| storage_error("set SQLite busy timeout", error))?;
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(|error| storage_error("set SQLite journal mode", error))?;
    connection
        .pragma_update(None, "synchronous", "FULL")
        .map_err(|error| storage_error("set SQLite synchronous mode", error))?;
    connection
        .pragma_update(None, "foreign_keys", true)
        .map_err(|error| storage_error("enable SQLite foreign keys", error))?;
    let settings = read_connection_settings(connection)?;
    verify_connection_settings(settings, timeout_ms)
}

fn verify_connection_settings(
    settings: ConnectionSettings,
    timeout_ms: i64,
) -> Result<(), PortError> {
    let required = ConnectionSettings {
        journal_mode: "wal".to_owned(),
        synchronous: 2,
        foreign_keys: true,
        busy_timeout_ms: timeout_ms,
    };
    if settings != required {
        return Err(PortError::new(
            PortErrorKind::Storage,
            "verify SQLite connection settings",
        ));
    }
    Ok(())
}

fn read_connection_settings(connection: &Connection) -> Result<ConnectionSettings, PortError> {
    Ok(ConnectionSettings {
        journal_mode: connection
            .pragma_query_value(None, "journal_mode", |row| row.get::<_, String>(0))
            .map_err(|error| storage_error("read SQLite journal mode", error))?
            .to_ascii_lowercase(),
        synchronous: connection
            .pragma_query_value(None, "synchronous", |row| row.get(0))
            .map_err(|error| storage_error("read SQLite synchronous mode", error))?,
        foreign_keys: connection
            .pragma_query_value(None, "foreign_keys", |row| row.get::<_, i64>(0))
            .map_err(|error| storage_error("read SQLite foreign keys", error))?
            == 1,
        busy_timeout_ms: connection
            .pragma_query_value(None, "busy_timeout", |row| row.get(0))
            .map_err(|error| storage_error("read SQLite busy timeout", error))?,
    })
}

fn migrate_v0(
    connection: &mut Connection,
    installation: &InstallationRecord,
    schema: &str,
) -> Result<(), PortError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| storage_error("begin metadata migration", error))?;
    apply_v1(&transaction, installation, schema)?;
    transaction
        .commit()
        .map_err(|error| storage_error("commit metadata migration", error))
}

fn apply_v1(
    transaction: &Transaction<'_>,
    installation: &InstallationRecord,
    schema: &str,
) -> Result<(), PortError> {
    transaction
        .execute_batch(schema)
        .map_err(|error| storage_error("create metadata schema", error))?;
    transaction
        .execute(
            "INSERT INTO installation (
                singleton, instance_id, data_root, volume_id, created_at_unix_ms
             ) VALUES (1, ?1, ?2, ?3, ?4)",
            params![
                installation.identity().instance_id().to_string(),
                installation.identity().data_root().as_bytes(),
                installation.identity().volume_id().to_string(),
                installation.created_at().get(),
            ],
        )
        .map_err(|error| storage_error("write installation row", error))?;
    transaction
        .pragma_update(None, "application_id", APPLICATION_ID)
        .map_err(|error| storage_error("set SQLite application ID", error))?;
    transaction
        .pragma_update(None, "user_version", SCHEMA_VERSION)
        .map_err(|error| storage_error("set SQLite schema version", error))?;
    Ok(())
}

fn read_installation(connection: &Connection) -> Result<InstallationRecord, PortError> {
    let row: Option<(String, Vec<u8>, String, i64)> = connection
        .query_row(
            "SELECT instance_id, data_root, volume_id, created_at_unix_ms
             FROM installation WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()
        .map_err(|error| storage_error("read installation row", error))?;
    let Some((instance_id, data_root, volume_id, created_at)) = row else {
        return Err(PortError::new(
            PortErrorKind::InvalidData,
            "read installation row",
        ));
    };
    Ok(InstallationRecord::new(
        InstallationIdentity::new(
            InstanceId::from_str(&instance_id)
                .map_err(|error| invalid_data("decode installation ID", error))?,
            AbsolutePath::try_from_bytes(data_root)
                .map_err(|error| invalid_data("decode data-root path", error))?,
            VolumeId::from_str(&volume_id)
                .map_err(|error| invalid_data("decode installation volume", error))?,
        ),
        UnixMillis::new(created_at)
            .map_err(|error| invalid_data("decode installation timestamp", error))?,
    ))
}

fn pragma_i32(connection: &Connection, name: &str) -> Result<i32, PortError> {
    connection
        .pragma_query_value(None, name, |row| row.get(0))
        .map_err(|error| storage_error("read SQLite schema identity", error))
}

fn user_table_count(connection: &Connection) -> Result<i64, PortError> {
    connection
        .query_row(
            "SELECT count(*) FROM sqlite_schema
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| storage_error("inspect SQLite user tables", error))
}

fn is_constraint(error: &rusqlite::Error) -> bool {
    error.sqlite_error_code() == Some(rusqlite::ErrorCode::ConstraintViolation)
}

fn storage_error(
    operation: &'static str,
    error: impl std::error::Error + Send + Sync + 'static,
) -> PortError {
    PortError::new(PortErrorKind::Storage, operation).with_source(error)
}

fn invalid_data(
    operation: &'static str,
    error: impl std::error::Error + Send + Sync + 'static,
) -> PortError {
    PortError::new(PortErrorKind::InvalidData, operation).with_source(error)
}

fn update_error(operation: &'static str, error: rusqlite::Error) -> PortError {
    if is_constraint(&error) {
        invalid_data(operation, error)
    } else {
        storage_error(operation, error)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use tempfile::Builder;

    use super::*;

    #[test]
    fn failed_schema_batch_rolls_back_every_migration_fact() {
        let root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-02-sqlite-tests");
        fs::create_dir_all(&root).unwrap();
        let temp = Builder::new()
            .prefix("migration-")
            .tempdir_in(root)
            .unwrap();
        let mut connection = Connection::open(temp.path().join("state.db")).unwrap();
        configure_connection(&connection, Duration::from_millis(10)).unwrap();
        let installation = InstallationRecord::new(
            InstallationIdentity::new(
                InstanceId::from_str("01890a5d-ac96-774b-bd5b-55c7b8d09f33").unwrap(),
                AbsolutePath::try_from_bytes(b"/tmp/thinws".to_vec()).unwrap(),
                VolumeId::from_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
            ),
            UnixMillis::new(1).unwrap(),
        );
        let broken = "CREATE TABLE partial(value INTEGER) STRICT; THIS IS NOT SQL;";

        assert!(migrate_v0(&mut connection, &installation, broken).is_err());
        assert_eq!(user_table_count(&connection).unwrap(), 0);
        assert_eq!(pragma_i32(&connection, "application_id").unwrap(), 0);
        assert_eq!(pragma_i32(&connection, "user_version").unwrap(), 0);
    }

    #[test]
    fn database_flags_and_each_required_connection_setting_are_exact() {
        assert_eq!(
            DATABASE_OPEN_FLAGS,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                .union(OpenFlags::SQLITE_OPEN_CREATE)
                .union(OpenFlags::SQLITE_OPEN_NO_MUTEX)
                .union(OpenFlags::SQLITE_OPEN_NOFOLLOW)
        );
        let expected = ConnectionSettings {
            journal_mode: "wal".to_owned(),
            synchronous: 2,
            foreign_keys: true,
            busy_timeout_ms: 50,
        };
        verify_connection_settings(expected.clone(), 50).unwrap();
        for changed in [
            ConnectionSettings {
                journal_mode: "delete".to_owned(),
                ..expected.clone()
            },
            ConnectionSettings {
                synchronous: 1,
                ..expected.clone()
            },
            ConnectionSettings {
                foreign_keys: false,
                ..expected.clone()
            },
            ConnectionSettings {
                busy_timeout_ms: 49,
                ..expected.clone()
            },
        ] {
            assert_eq!(
                verify_connection_settings(changed, 50).unwrap_err().kind(),
                PortErrorKind::Storage
            );
        }
    }
}
