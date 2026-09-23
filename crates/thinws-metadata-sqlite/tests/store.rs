use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use rusqlite::{Connection, params};
use tempfile::{Builder, TempDir};
use thinws_core::{
    AbsolutePath, ErrorCode, InstallationIdentity, InstallationRecord, InstanceId, RemovalMode,
    UnixMillis, VolumeId, WorkspaceId, WorkspaceName, WorkspaceReservation, WorkspaceState,
};
use thinws_metadata_sqlite::{APPLICATION_ID, SCHEMA_VERSION, SqliteMetadataStore};
use thinws_ports::{MetadataStore, PortConflict, PortErrorKind};

const INSTANCE_ID: &str = "01890a5d-ac96-774b-bd5b-55c7b8d09f33";
const VOLUME_ID: &str = "550e8400-e29b-41d4-a716-446655440000";
const WORKSPACE_IDS: [&str; 8] = [
    "ws_01890a5d-ac96-774b-bd5b-55c7b8d09f40",
    "ws_01890a5d-ac96-774b-bd5b-55c7b8d09f41",
    "ws_01890a5d-ac96-774b-bd5b-55c7b8d09f42",
    "ws_01890a5d-ac96-774b-bd5b-55c7b8d09f43",
    "ws_01890a5d-ac96-774b-bd5b-55c7b8d09f44",
    "ws_01890a5d-ac96-774b-bd5b-55c7b8d09f45",
    "ws_01890a5d-ac96-774b-bd5b-55c7b8d09f46",
    "ws_01890a5d-ac96-774b-bd5b-55c7b8d09f47",
];

fn controlled_tempdir() -> TempDir {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-02-sqlite-tests");
    fs::create_dir_all(&root).expect("create controlled test root");
    Builder::new()
        .prefix("metadata-")
        .tempdir_in(root)
        .expect("create controlled temporary directory")
}

fn installation() -> InstallationRecord {
    InstallationRecord::new(
        InstallationIdentity::new(
            InstanceId::from_str(INSTANCE_ID).unwrap(),
            AbsolutePath::try_from_bytes(b"/Volumes/data/thinws".to_vec()).unwrap(),
            VolumeId::from_str(VOLUME_ID).unwrap(),
        ),
        UnixMillis::new(1_700_000_000_000).unwrap(),
    )
}

fn reservation(index: usize, name: &str, target: &str) -> WorkspaceReservation {
    WorkspaceReservation::new(
        WorkspaceId::from_str(WORKSPACE_IDS[index]).unwrap(),
        InstanceId::from_str(INSTANCE_ID).unwrap(),
        WorkspaceName::from_str(name).unwrap(),
        AbsolutePath::try_from_bytes(format!("/Volumes/data/source-{index}").into_bytes()).unwrap(),
        AbsolutePath::try_from_bytes(target.as_bytes().to_vec()).unwrap(),
        VolumeId::from_str(VOLUME_ID).unwrap(),
        VolumeId::from_str(VOLUME_ID).unwrap(),
        index.is_multiple_of(2),
        UnixMillis::new(1_700_000_000_100 + index as i64).unwrap(),
    )
}

fn open(path: &std::path::Path) -> SqliteMetadataStore {
    SqliteMetadataStore::open(path, &installation(), Duration::from_secs(2)).unwrap()
}

#[test]
fn empty_database_is_migrated_atomically_and_reopens_with_required_settings() {
    let temp = controlled_tempdir();
    let path = temp.path().join("state.db");
    let expected = installation();

    let store = SqliteMetadataStore::open(&path, &expected, Duration::from_millis(1_234))
        .expect("an empty path initializes schema v1");
    let settings = store.connection_settings().expect("settings are queryable");
    assert_eq!(settings.journal_mode, "wal");
    assert_eq!(settings.synchronous, 2);
    assert!(settings.foreign_keys);
    assert_eq!(settings.busy_timeout_ms, 1_234);
    drop(store);

    let raw = Connection::open(&path).unwrap();
    assert_eq!(
        raw.pragma_query_value(None, "application_id", |row| row.get::<_, i32>(0))
            .unwrap(),
        APPLICATION_ID
    );
    assert_eq!(
        raw.pragma_query_value(None, "user_version", |row| row.get::<_, i32>(0))
            .unwrap(),
        SCHEMA_VERSION
    );
    drop(raw);

    SqliteMetadataStore::open(&path, &expected, Duration::from_millis(500))
        .expect("the same identity reopens schema v1");
}

#[test]
fn foreign_future_unowned_and_identity_conflicting_databases_are_not_taken_over() {
    let temp = controlled_tempdir();

    let foreign_path = temp.path().join("foreign.db");
    let foreign = Connection::open(&foreign_path).unwrap();
    foreign
        .pragma_update(None, "application_id", 1_234_i32)
        .unwrap();
    drop(foreign);
    let error =
        SqliteMetadataStore::open(&foreign_path, &installation(), Duration::from_millis(10))
            .err()
            .expect("foreign database must be rejected");
    assert_eq!(error.kind(), PortErrorKind::UnsupportedVersion);
    assert_eq!(
        Connection::open(&foreign_path)
            .unwrap()
            .pragma_query_value(None, "application_id", |row| row.get::<_, i32>(0))
            .unwrap(),
        1_234
    );

    let unowned_path = temp.path().join("unowned.db");
    let unowned = Connection::open(&unowned_path).unwrap();
    unowned
        .execute("CREATE TABLE user_data(value TEXT)", [])
        .unwrap();
    drop(unowned);
    let error =
        SqliteMetadataStore::open(&unowned_path, &installation(), Duration::from_millis(10))
            .err()
            .expect("an application_id=0 database with user tables is not empty");
    assert_eq!(error.kind(), PortErrorKind::UnsupportedVersion);
    assert_eq!(
        Connection::open(&unowned_path)
            .unwrap()
            .query_row("SELECT count(*) FROM user_data", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );

    let future_path = temp.path().join("future.db");
    drop(open(&future_path));
    let future = Connection::open(&future_path).unwrap();
    future
        .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
        .unwrap();
    drop(future);
    let error = SqliteMetadataStore::open(&future_path, &installation(), Duration::from_millis(10))
        .err()
        .expect("a future schema must be rejected");
    assert_eq!(error.kind(), PortErrorKind::UnsupportedVersion);

    let identity_path = temp.path().join("identity.db");
    drop(open(&identity_path));
    let conflicting = InstallationRecord::new(
        InstallationIdentity::new(
            InstanceId::from_str("01890a5d-ac96-774b-bd5b-55c7b8d09f34").unwrap(),
            AbsolutePath::try_from_bytes(b"/Volumes/data/thinws".to_vec()).unwrap(),
            VolumeId::from_str(VOLUME_ID).unwrap(),
        ),
        UnixMillis::new(9).unwrap(),
    );
    let error = SqliteMetadataStore::open(&identity_path, &conflicting, Duration::from_millis(10))
        .err()
        .expect("identity conflict must be rejected");
    assert_eq!(error.kind(), PortErrorKind::Conflict);
    assert_eq!(
        error.conflict_kind(),
        Some(PortConflict::InstallationIdentity)
    );
}

#[test]
fn database_leaf_symlink_is_rejected_without_modifying_its_target() {
    let temp = controlled_tempdir();
    let victim = temp.path().join("victim.db");
    fs::write(&victim, b"not a database and must remain unchanged").unwrap();
    let link = temp.path().join("state.db");
    symlink(&victim, &link).unwrap();

    assert!(SqliteMetadataStore::open(&link, &installation(), Duration::from_millis(10),).is_err());
    assert_eq!(
        fs::read(victim).unwrap(),
        b"not a database and must remain unchanged"
    );
}

#[test]
fn store_transitions_preserve_unfinished_state_and_tombstone_identity() {
    let temp = controlled_tempdir();
    let path = temp.path().join("state.db");
    let mut store = open(&path);
    let alpha = reservation(0, "alpha", "/Volumes/data/thinws/workspaces/alpha");
    let beta = reservation(1, "beta", "/Volumes/data/thinws/workspaces/beta");

    assert_eq!(
        store.reserve_workspace(&beta).unwrap().state(),
        WorkspaceState::Creating
    );
    assert_eq!(
        store.reserve_workspace(&alpha).unwrap().state(),
        WorkspaceState::Creating
    );
    let listed = store.workspaces().unwrap();
    assert_eq!(listed[0].reservation().name().as_str(), "alpha");
    assert_eq!(listed[1].reservation().name().as_str(), "beta");

    let failed = store
        .record_failure(
            alpha.workspace_id(),
            WorkspaceState::Creating,
            ErrorCode::Filesystem,
            UnixMillis::new(1_700_000_000_200).unwrap(),
        )
        .unwrap();
    assert_eq!(failed.state(), WorkspaceState::Error);
    assert_eq!(failed.last_error_code(), Some(ErrorCode::Filesystem));
    let stale = store
        .record_failure(
            alpha.workspace_id(),
            WorkspaceState::Creating,
            ErrorCode::Metadata,
            UnixMillis::new(1_700_000_000_201).unwrap(),
        )
        .unwrap_err();
    assert_eq!(stale.conflict_kind(), Some(PortConflict::ExpectedState));
    assert_eq!(
        store
            .begin_removal(
                alpha.workspace_id(),
                WorkspaceState::Error,
                RemovalMode::Normal,
                UnixMillis::new(1_700_000_000_202).unwrap(),
            )
            .unwrap_err()
            .kind(),
        PortErrorKind::InvalidData
    );
    let deleting = store
        .begin_removal(
            alpha.workspace_id(),
            WorkspaceState::Error,
            RemovalMode::Force,
            UnixMillis::new(1_700_000_000_203).unwrap(),
        )
        .unwrap();
    assert_eq!(deleting.state(), WorkspaceState::Deleting);
    assert_eq!(deleting.last_error_code(), None);
    let repeated = store
        .begin_removal(
            alpha.workspace_id(),
            WorkspaceState::Deleting,
            RemovalMode::Force,
            UnixMillis::new(1_700_000_000_999).unwrap(),
        )
        .unwrap();
    assert_eq!(repeated.updated_at(), deleting.updated_at());

    store
        .begin_removal(
            beta.workspace_id(),
            WorkspaceState::Creating,
            RemovalMode::Force,
            UnixMillis::new(1_700_000_000_204).unwrap(),
        )
        .unwrap();
    drop(store);

    let mut reopened = open(&path);
    assert_eq!(
        reopened
            .workspace(alpha.workspace_id())
            .unwrap()
            .unwrap()
            .state(),
        WorkspaceState::Deleting
    );
    assert_eq!(
        reopened
            .workspace(beta.workspace_id())
            .unwrap()
            .unwrap()
            .state(),
        WorkspaceState::Deleting
    );
    let tombstone = reopened
        .complete_deletion(
            alpha.workspace_id(),
            alpha.instance_id(),
            UnixMillis::new(1_700_000_000_300).unwrap(),
        )
        .unwrap();
    assert_eq!(tombstone.workspace_id(), alpha.workspace_id());
    assert!(reopened.workspace(alpha.workspace_id()).unwrap().is_none());

    let reused = WorkspaceReservation::new(
        alpha.workspace_id(),
        alpha.instance_id(),
        WorkspaceName::from_str("alpha-reused").unwrap(),
        alpha.source_path().clone(),
        AbsolutePath::try_from_bytes(b"/Volumes/data/thinws/workspaces/reused".to_vec()).unwrap(),
        alpha.source_volume_id(),
        alpha.data_volume_id(),
        alpha.allow_full_copy(),
        UnixMillis::new(1_700_000_000_400).unwrap(),
    );
    let error = reopened.reserve_workspace(&reused).unwrap_err();
    assert_eq!(error.conflict_kind(), Some(PortConflict::WorkspaceId));
}

#[test]
fn name_and_target_uniqueness_are_decided_by_sqlite_across_connections() {
    for same_name in [true, false] {
        let temp = controlled_tempdir();
        let path = temp.path().join("race.db");
        drop(open(&path));
        let barrier = Arc::new(Barrier::new(2));
        let mut handles = Vec::new();
        for index in 0..2 {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                let mut store = open(&path);
                let name = if same_name {
                    "same"
                } else if index == 0 {
                    "left"
                } else {
                    "right"
                };
                let target = if same_name {
                    format!("/Volumes/data/thinws/workspaces/race-{index}")
                } else {
                    "/Volumes/data/thinws/workspaces/same-target".to_owned()
                };
                let value = reservation(index, name, &target);
                barrier.wait();
                store.reserve_workspace(&value)
            }));
        }
        let results: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        let error = results.into_iter().find_map(Result::err).unwrap();
        assert_eq!(
            error.conflict_kind(),
            Some(if same_name {
                PortConflict::WorkspaceName
            } else {
                PortConflict::TargetPath
            })
        );
    }
}

#[test]
fn schema_rejects_nul_error_mismatch_illegal_edges_and_unprotected_deletes() {
    let temp = controlled_tempdir();
    let identity_checks = Connection::open(temp.path().join("identity-checks.db")).unwrap();
    identity_checks
        .execute_batch(include_str!("../src/schema_v1.sql"))
        .unwrap();
    assert!(
        identity_checks
            .execute(
                "INSERT INTO installation VALUES (1, ?1, ?2, ?3, 1)",
                params![format!("{INSTANCE_ID}\0tail"), b"/data", VOLUME_ID],
            )
            .is_err()
    );
    assert!(
        identity_checks
            .execute(
                "INSERT INTO installation VALUES (1, ?1, ?2, ?3, 1)",
                params![INSTANCE_ID, b"/data", format!("{VOLUME_ID}\0tail")],
            )
            .is_err()
    );
    drop(identity_checks);

    let path = temp.path().join("constraints.db");
    let mut store = open(&path);
    let value = reservation(
        2,
        "constraints",
        "/Volumes/data/thinws/workspaces/constraints",
    );
    store.reserve_workspace(&value).unwrap();
    drop(store);

    let connection = Connection::open(&path).unwrap();
    connection
        .pragma_update(None, "foreign_keys", true)
        .unwrap();
    assert!(connection.execute(
        "UPDATE workspaces SET state='ready', updated_at_unix_ms=updated_at_unix_ms+1 WHERE workspace_id=?1",
        [value.workspace_id().to_string()],
    ).is_err());
    assert!(connection.execute(
        "UPDATE workspaces SET state='error', last_error_code=NULL, updated_at_unix_ms=updated_at_unix_ms+1 WHERE workspace_id=?1",
        [value.workspace_id().to_string()],
    ).is_err());
    assert!(
        connection
            .execute(
                "UPDATE workspaces SET last_error_code='E_FILESYSTEM' WHERE workspace_id=?1",
                [value.workspace_id().to_string()],
            )
            .is_err()
    );
    assert!(
        connection
            .execute(
                "INSERT INTO materialization_receipts VALUES (?1, 1, 'not-json', 1)",
                [value.workspace_id().to_string()],
            )
            .is_err()
    );
    connection
        .execute(
            "INSERT INTO materialization_receipts VALUES (?1, 1, '{\"mode\":\"test\"}', 1)",
            [value.workspace_id().to_string()],
        )
        .unwrap();
    assert!(
        connection
            .execute(
                "DELETE FROM materialization_receipts WHERE workspace_id=?1",
                [value.workspace_id().to_string()],
            )
            .is_err()
    );
    connection.execute(
        "UPDATE workspaces SET state='ready', updated_at_unix_ms=updated_at_unix_ms+1 WHERE workspace_id=?1",
        [value.workspace_id().to_string()],
    ).unwrap();
    assert!(connection.execute(
        "UPDATE workspaces SET state='creating', updated_at_unix_ms=updated_at_unix_ms+1 WHERE workspace_id=?1",
        [value.workspace_id().to_string()],
    ).is_err());
    connection.execute(
        "UPDATE workspaces SET state='deleting', updated_at_unix_ms=updated_at_unix_ms+1 WHERE workspace_id=?1",
        [value.workspace_id().to_string()],
    ).unwrap();
    assert!(
        connection
            .execute(
                "DELETE FROM workspaces WHERE workspace_id=?1",
                [value.workspace_id().to_string()],
            )
            .is_err()
    );
    connection
        .execute(
            "INSERT INTO deletion_tombstones VALUES (?1, ?2, 10)",
            params![
                value.workspace_id().to_string(),
                value.instance_id().to_string()
            ],
        )
        .unwrap();
    connection
        .execute(
            "DELETE FROM workspaces WHERE workspace_id=?1",
            [value.workspace_id().to_string()],
        )
        .unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM materialization_receipts WHERE workspace_id=?1",
                [value.workspace_id().to_string()],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    assert!(
        connection
            .execute(
                "DELETE FROM deletion_tombstones WHERE workspace_id=?1",
                [value.workspace_id().to_string()],
            )
            .is_err()
    );

    let insert_sql = "INSERT INTO workspaces (
        workspace_id, instance_id, name, source_path, target_path,
        source_volume_id, data_volume_id, allow_full_copy, state,
        last_error_code, created_at_unix_ms, updated_at_unix_ms
    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, 0, ?7, ?8, 20, 20)";
    let valid_id = WORKSPACE_IDS[3];
    for (workspace_id, name, volume, state, code) in [
        (
            format!("{valid_id}\0tail"),
            "valid".to_owned(),
            VOLUME_ID.to_owned(),
            "creating",
            None,
        ),
        (
            valid_id.to_owned(),
            "valid\0tail".to_owned(),
            VOLUME_ID.to_owned(),
            "creating",
            None,
        ),
        (
            valid_id.to_owned(),
            "valid".to_owned(),
            format!("{VOLUME_ID}\0tail"),
            "creating",
            None,
        ),
        (
            valid_id.to_owned(),
            "valid".to_owned(),
            VOLUME_ID.to_owned(),
            "error",
            None,
        ),
        (
            valid_id.to_owned(),
            "valid".to_owned(),
            VOLUME_ID.to_owned(),
            "creating",
            Some("E_FILESYSTEM"),
        ),
    ] {
        assert!(
            connection
                .execute(
                    insert_sql,
                    params![
                        workspace_id,
                        INSTANCE_ID,
                        name,
                        b"/source",
                        b"/target-unique",
                        volume,
                        state,
                        code
                    ],
                )
                .is_err()
        );
    }
}

#[test]
fn failed_delete_rolls_back_tombstone_and_active_row_together() {
    let temp = controlled_tempdir();
    let path = temp.path().join("rollback.db");
    let mut store = open(&path);
    let value = reservation(4, "rollback", "/Volumes/data/thinws/workspaces/rollback");
    store.reserve_workspace(&value).unwrap();
    store
        .begin_removal(
            value.workspace_id(),
            WorkspaceState::Creating,
            RemovalMode::Force,
            UnixMillis::new(1_700_000_001_000).unwrap(),
        )
        .unwrap();
    drop(store);

    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER injected_delete_failure BEFORE DELETE ON workspaces
         BEGIN SELECT RAISE(ABORT, 'injected delete failure'); END;",
        )
        .unwrap();
    drop(connection);

    let mut store = open(&path);
    assert!(
        store
            .complete_deletion(
                value.workspace_id(),
                value.instance_id(),
                UnixMillis::new(1_700_000_001_001).unwrap(),
            )
            .is_err()
    );
    drop(store);
    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM workspaces", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row("SELECT count(*) FROM deletion_tombstones", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    connection
        .execute_batch("DROP TRIGGER injected_delete_failure;")
        .unwrap();
    drop(connection);
    open(&path)
        .complete_deletion(
            value.workspace_id(),
            value.instance_id(),
            UnixMillis::new(1_700_000_001_002).unwrap(),
        )
        .unwrap();
}
