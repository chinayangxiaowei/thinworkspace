#![cfg(target_os = "linux")]

use std::env;
use std::path::Path;
use std::time::Duration;

use thinws_adapter_linux::LinuxHostAdapter;
use thinws_core::{
    AbsolutePath, InstallationIdentity, InstallationRecord, InstanceId, RootMarkerState, UnixMillis,
};
use thinws_metadata_sqlite::SqliteMetadataStoreFactory;
use thinws_ports::{
    LifecycleLock, MetadataStoreFactory, PortErrorKind, PreparedDataRootEvidence, PublishResult,
};

fn absolute(path: &Path) -> AbsolutePath {
    AbsolutePath::try_from_bytes(path.as_os_str().as_encoded_bytes().to_vec()).unwrap()
}

#[test]
fn ext4_control_root_initializes_in_one_direction_and_reopens_read_only() {
    let root = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT")
        .expect("set THINWS_LINUX_EXT4_TEST_ROOT to a writable ext4 test directory");
    let fixture = tempfile::Builder::new()
        .prefix("thinws-linux-init-")
        .tempdir_in(root)
        .unwrap();
    let control = fixture.path().join("control");
    let adapter = LinuxHostAdapter::new(&control).unwrap();
    adapter.prepare_bootstrap().unwrap();
    let lock = adapter
        .acquire_bootstrap(Duration::from_millis(100))
        .unwrap();
    let prepared = adapter.prepare_data_root(&absolute(&control)).unwrap();
    let identity = InstallationIdentity::new(
        "01890a5d-ac96-774b-bd5b-55c7b8d09f33"
            .parse::<InstanceId>()
            .unwrap(),
        absolute(&control),
        prepared.volume_id(),
    );
    let proof = adapter
        .create_initializing(&lock, prepared, &identity)
        .unwrap();
    assert_eq!(adapter.read_config().unwrap(), None);
    assert_eq!(
        adapter
            .read_root_marker(identity.data_root())
            .unwrap()
            .unwrap()
            .state(),
        RootMarkerState::Initializing
    );
    assert_eq!(
        adapter.publish_config(&lock, &identity).unwrap_err().kind(),
        PortErrorKind::Conflict
    );
    let layout = adapter.initialize_layout(&lock, &proof).unwrap();
    let record = InstallationRecord::new(
        identity.clone(),
        UnixMillis::new(1_700_000_000_000).unwrap(),
    );
    SqliteMetadataStoreFactory
        .initialize(&layout, &record, Duration::from_millis(200))
        .unwrap();
    adapter.publish_ready(&lock, proof).unwrap();
    assert_eq!(
        adapter.publish_config(&lock, &identity).unwrap(),
        PublishResult::Published
    );
    assert_eq!(
        adapter.publish_config(&lock, &identity).unwrap(),
        PublishResult::AlreadyCurrent
    );
    drop(lock);
    assert_eq!(adapter.read_config().unwrap(), Some(identity.clone()));
    assert!(std::fs::read_dir(&control).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".root-marker-ready.tmp-")
    }));
    assert_eq!(
        adapter
            .read_root_marker(identity.data_root())
            .unwrap()
            .unwrap()
            .state(),
        RootMarkerState::Ready
    );
    let reopened = adapter.validate_layout(&identity).unwrap();
    assert_eq!(
        SqliteMetadataStoreFactory
            .inspect(&reopened, &record, Duration::from_millis(200))
            .unwrap()
            .installation(),
        &record
    );
}

#[test]
fn unknown_entry_added_after_preparation_prevents_marker_publication() {
    let root = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT")
        .expect("set THINWS_LINUX_EXT4_TEST_ROOT to a writable ext4 test directory");
    let fixture = tempfile::Builder::new()
        .prefix("thinws-linux-init-foreign-")
        .tempdir_in(root)
        .unwrap();
    let control = fixture.path().join("control");
    let adapter = LinuxHostAdapter::new(&control).unwrap();
    adapter.prepare_bootstrap().unwrap();
    let lock = adapter
        .acquire_bootstrap(Duration::from_millis(100))
        .unwrap();
    let prepared = adapter.prepare_data_root(&absolute(&control)).unwrap();
    let identity = InstallationIdentity::new(
        "01890a5d-ac96-774b-bd5b-55c7b8d09f33"
            .parse::<InstanceId>()
            .unwrap(),
        absolute(&control),
        prepared.volume_id(),
    );
    std::fs::write(control.join("foreign"), b"keep").unwrap();
    assert_eq!(
        adapter
            .create_initializing(&lock, prepared, &identity)
            .err()
            .unwrap()
            .kind(),
        PortErrorKind::NotEmpty
    );
    assert_eq!(std::fs::read(control.join("foreign")).unwrap(), b"keep");
    assert_eq!(
        adapter.read_root_marker(identity.data_root()).unwrap(),
        None
    );
}

#[test]
fn changed_initializing_marker_is_not_promoted_or_overwritten() {
    let root = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT")
        .expect("set THINWS_LINUX_EXT4_TEST_ROOT to a writable ext4 test directory");
    let fixture = tempfile::Builder::new()
        .prefix("thinws-linux-init-marker-change-")
        .tempdir_in(root)
        .unwrap();
    let control = fixture.path().join("control");
    let adapter = LinuxHostAdapter::new(&control).unwrap();
    adapter.prepare_bootstrap().unwrap();
    let lock = adapter
        .acquire_bootstrap(Duration::from_millis(100))
        .unwrap();
    let prepared = adapter.prepare_data_root(&absolute(&control)).unwrap();
    let identity = InstallationIdentity::new(
        "01890a5d-ac96-774b-bd5b-55c7b8d09f33"
            .parse::<InstanceId>()
            .unwrap(),
        absolute(&control),
        prepared.volume_id(),
    );
    let proof = adapter
        .create_initializing(&lock, prepared, &identity)
        .unwrap();
    let marker_path = control.join(".thinws-control.toml");
    std::fs::write(&marker_path, b"foreign content").unwrap();
    assert!(adapter.publish_ready(&lock, proof).is_err());
    assert_eq!(std::fs::read(&marker_path).unwrap(), b"foreign content");
    assert_eq!(adapter.read_config().unwrap(), None);
}

#[test]
fn marker_cannot_claim_another_path_on_the_same_control_filesystem() {
    let root = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT")
        .expect("set THINWS_LINUX_EXT4_TEST_ROOT to a writable ext4 test directory");
    let fixture = tempfile::Builder::new()
        .prefix("thinws-linux-init-wrong-root-")
        .tempdir_in(root)
        .unwrap();
    let control = fixture.path().join("control");
    let claimed = fixture.path().join("claimed");
    let adapter = LinuxHostAdapter::new(&control).unwrap();
    adapter.prepare_bootstrap().unwrap();
    let lock = adapter
        .acquire_bootstrap(Duration::from_millis(100))
        .unwrap();
    let prepared = adapter.prepare_data_root(&absolute(&control)).unwrap();
    let wrong = InstallationIdentity::new(
        "01890a5d-ac96-774b-bd5b-55c7b8d09f33"
            .parse::<InstanceId>()
            .unwrap(),
        absolute(&claimed),
        prepared.volume_id(),
    );
    assert_eq!(
        adapter
            .create_initializing(&lock, prepared, &wrong)
            .err()
            .unwrap()
            .kind(),
        PortErrorKind::Conflict
    );
    assert!(!control.join(".thinws-control.toml").exists());
    assert!(!claimed.exists());
}
