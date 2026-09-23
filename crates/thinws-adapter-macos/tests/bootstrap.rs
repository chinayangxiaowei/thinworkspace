use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use tempfile::{Builder, TempDir};
use thinws_adapter_macos::MacOsHostAdapter;
use thinws_core::{AbsolutePath, InstallationIdentity, InstanceId, RootMarkerState, VolumeId};
use thinws_ports::{BootstrapStore, LifecycleLock, PortConflict, PortErrorKind, PublishResult};

const INSTANCE_ID: &str = "01890a5d-ac96-774b-bd5b-55c7b8d09f33";
const OTHER_INSTANCE_ID: &str = "01890a5d-ac96-774b-bd5b-55c7b8d09f34";
const VOLUME_ID: &str = "550e8400-e29b-41d4-a716-446655440000";

fn controlled_tempdir() -> TempDir {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-02-macos-tests");
    fs::create_dir_all(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    Builder::new()
        .prefix("bootstrap-")
        .tempdir_in(fs::canonicalize(root).unwrap())
        .unwrap()
}

fn private_dir(path: &Path) {
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn identity(root: &Path, instance: &str) -> InstallationIdentity {
    InstallationIdentity::new(
        InstanceId::from_str(instance).unwrap(),
        AbsolutePath::try_from_bytes(root.as_os_str().as_bytes().to_vec()).unwrap(),
        VolumeId::from_str(VOLUME_ID).unwrap(),
    )
}

fn encoded_marker(identity: &InstallationIdentity, state: &str) -> Vec<u8> {
    let root_hex: String = identity
        .data_root()
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!(
        "schema_version = 1\ninstance_id = \"{}\"\ndata_root_hex = \"{}\"\nvolume_id = \"{}\"\nstate = \"{}\"\n",
        identity.instance_id(), root_hex, identity.volume_id(), state
    )
    .into_bytes()
}

fn write_private(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn bootstrap_documents_publish_in_one_direction_with_exact_idempotence() {
    let temp = controlled_tempdir();
    let bootstrap = temp.path().join("bootstrap");
    let data_root = temp.path().join("data");
    private_dir(&bootstrap);
    private_dir(&data_root);
    let adapter = MacOsHostAdapter::new(&bootstrap).unwrap();
    let lock = adapter
        .acquire_bootstrap(Duration::from_millis(500))
        .unwrap();
    let expected = identity(&data_root, INSTANCE_ID);

    assert!(adapter.read_config().unwrap().is_none());
    assert!(
        adapter
            .read_root_marker(expected.data_root())
            .unwrap()
            .is_none()
    );
    let premature = adapter.publish_config(&lock, &expected).unwrap_err();
    assert_eq!(
        premature.conflict_kind(),
        Some(PortConflict::InstallationIdentity)
    );

    let proof = adapter.create_initializing(&lock, &expected).unwrap();
    let marker = adapter
        .read_root_marker(expected.data_root())
        .unwrap()
        .unwrap();
    assert_eq!(marker.state(), RootMarkerState::Initializing);
    assert_eq!(marker.identity(), &expected);
    assert_eq!(
        fs::metadata(data_root.join(".thinws-root.toml"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o600
    );

    let ready = adapter.publish_ready(&lock, proof).unwrap();
    assert_eq!(ready.state(), RootMarkerState::Ready);
    assert_eq!(
        adapter.publish_config(&lock, &expected).unwrap(),
        PublishResult::Published
    );
    assert_eq!(
        adapter.publish_config(&lock, &expected).unwrap(),
        PublishResult::AlreadyCurrent
    );
    assert_eq!(adapter.read_config().unwrap().as_ref(), Some(&expected));
    assert_eq!(
        fs::metadata(bootstrap.join("config.toml"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o600
    );

    let conflicting = identity(&data_root, OTHER_INSTANCE_ID);
    let bytes_before = fs::read(bootstrap.join("config.toml")).unwrap();
    let error = adapter.publish_config(&lock, &conflicting).unwrap_err();
    assert_eq!(
        error.conflict_kind(),
        Some(PortConflict::InstallationIdentity)
    );
    assert_eq!(
        fs::read(bootstrap.join("config.toml")).unwrap(),
        bytes_before
    );
}

#[test]
fn existing_or_replaced_marker_is_never_adopted_or_overwritten() {
    let temp = controlled_tempdir();
    let bootstrap = temp.path().join("bootstrap");
    let first_root = temp.path().join("first");
    let second_root = temp.path().join("second");
    private_dir(&bootstrap);
    private_dir(&first_root);
    private_dir(&second_root);
    let adapter = MacOsHostAdapter::new(&bootstrap).unwrap();
    let lock = adapter
        .acquire_bootstrap(Duration::from_millis(500))
        .unwrap();

    let first = identity(&first_root, INSTANCE_ID);
    let proof = adapter.create_initializing(&lock, &first).unwrap();
    let marker_path = first_root.join(".thinws-root.toml");
    let displaced = first_root.join("displaced-marker");
    fs::rename(&marker_path, &displaced).unwrap();
    let replacement = encoded_marker(&identity(&first_root, OTHER_INSTANCE_ID), "ready");
    write_private(&marker_path, &replacement);
    let error = adapter.publish_ready(&lock, proof).unwrap_err();
    assert!(matches!(
        error.kind(),
        PortErrorKind::InvalidData | PortErrorKind::Conflict
    ));
    assert_eq!(fs::read(&marker_path).unwrap(), replacement);

    let second = identity(&second_root, INSTANCE_ID);
    let existing = encoded_marker(&second, "initializing");
    write_private(&second_root.join(".thinws-root.toml"), &existing);
    let error = adapter.create_initializing(&lock, &second).err().unwrap();
    assert_eq!(
        error.conflict_kind(),
        Some(PortConflict::InstallationIdentity)
    );
    assert_eq!(
        fs::read(second_root.join(".thinws-root.toml")).unwrap(),
        existing
    );
}

#[test]
fn an_in_place_modified_initializing_marker_cannot_be_promoted() {
    let temp = controlled_tempdir();
    let bootstrap = temp.path().join("bootstrap");
    let data_root = temp.path().join("data");
    private_dir(&bootstrap);
    private_dir(&data_root);
    let adapter = MacOsHostAdapter::new(&bootstrap).unwrap();
    let lock = adapter
        .acquire_bootstrap(Duration::from_millis(500))
        .unwrap();
    let expected = identity(&data_root, INSTANCE_ID);
    let proof = adapter.create_initializing(&lock, &expected).unwrap();
    let marker_path = data_root.join(".thinws-root.toml");
    let replacement = encoded_marker(&expected, "ready");
    write_private(&marker_path, &replacement);

    let error = adapter.publish_ready(&lock, proof).unwrap_err();
    assert_eq!(
        error.conflict_kind(),
        Some(PortConflict::InstallationIdentity)
    );
    assert_eq!(fs::read(marker_path).unwrap(), replacement);
}

#[test]
fn an_existing_config_conflicts_even_when_the_requested_root_marker_is_ready() {
    let temp = controlled_tempdir();
    let bootstrap = temp.path().join("bootstrap");
    let first_root = temp.path().join("first");
    let second_root = temp.path().join("second");
    private_dir(&bootstrap);
    private_dir(&first_root);
    private_dir(&second_root);
    let adapter = MacOsHostAdapter::new(&bootstrap).unwrap();
    let lock = adapter
        .acquire_bootstrap(Duration::from_millis(500))
        .unwrap();

    let first = identity(&first_root, INSTANCE_ID);
    let first_proof = adapter.create_initializing(&lock, &first).unwrap();
    adapter.publish_ready(&lock, first_proof).unwrap();
    adapter.publish_config(&lock, &first).unwrap();

    let second = identity(&second_root, OTHER_INSTANCE_ID);
    let second_proof = adapter.create_initializing(&lock, &second).unwrap();
    adapter.publish_ready(&lock, second_proof).unwrap();
    let first_config = fs::read(bootstrap.join("config.toml")).unwrap();
    let error = adapter.publish_config(&lock, &second).unwrap_err();
    assert_eq!(
        error.conflict_kind(),
        Some(PortConflict::InstallationIdentity)
    );
    assert_eq!(
        fs::read(bootstrap.join("config.toml")).unwrap(),
        first_config
    );
}

#[test]
fn leaf_symlinks_and_a_guard_from_another_adapter_fail_without_touching_targets() {
    let temp = controlled_tempdir();
    let bootstrap = temp.path().join("bootstrap");
    let root = temp.path().join("data");
    private_dir(&bootstrap);
    private_dir(&root);
    let adapter = MacOsHostAdapter::new(&bootstrap).unwrap();
    let other = MacOsHostAdapter::new(&bootstrap).unwrap();
    let lock = adapter
        .acquire_bootstrap(Duration::from_millis(500))
        .unwrap();
    let expected = identity(&root, INSTANCE_ID);
    let error = other.create_initializing(&lock, &expected).err().unwrap();
    assert_eq!(error.kind(), PortErrorKind::InvalidData);
    assert!(!root.join(".thinws-root.toml").exists());
    drop(lock);

    let victim = temp.path().join("victim");
    write_private(&victim, b"do-not-read-or-change");
    symlink(&victim, bootstrap.join("config.toml")).unwrap();
    assert!(adapter.read_config().is_err());
    assert_eq!(fs::read(victim).unwrap(), b"do-not-read-or-change");

    fs::remove_file(bootstrap.join("config.toml")).unwrap();
    let unsafe_config = format!(
        "schema_version = 1\ninstance_id = \"{INSTANCE_ID}\"\ndata_root_hex = \"2f746d70\"\nvolume_id = \"{VOLUME_ID}\"\n"
    );
    fs::write(bootstrap.join("config.toml"), unsafe_config).unwrap();
    fs::set_permissions(
        bootstrap.join("config.toml"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert_eq!(
        adapter.read_config().unwrap_err().kind(),
        PortErrorKind::InvalidData
    );

    fs::remove_file(bootstrap.join("config.toml")).unwrap();
    private_dir(&bootstrap.join("config.toml"));
    assert!(adapter.read_config().is_err());
}

#[test]
fn intermediate_directory_symlinks_are_not_followed() {
    let temp = controlled_tempdir();
    let real_parent = temp.path().join("real-parent");
    let real_bootstrap = real_parent.join("bootstrap");
    let real_data_root = real_parent.join("data");
    private_dir(&real_parent);
    private_dir(&real_bootstrap);
    private_dir(&real_data_root);
    let alias = temp.path().join("alias");
    symlink(&real_parent, &alias).unwrap();

    let aliased_adapter = MacOsHostAdapter::new(alias.join("bootstrap")).unwrap();
    assert!(aliased_adapter.read_config().is_err());

    let direct_adapter = MacOsHostAdapter::new(&real_bootstrap).unwrap();
    let lock = direct_adapter
        .acquire_bootstrap(Duration::from_millis(500))
        .unwrap();
    let aliased_identity = identity(&alias.join("data"), INSTANCE_ID);
    assert!(
        direct_adapter
            .create_initializing(&lock, &aliased_identity)
            .is_err()
    );
    assert!(!real_data_root.join(".thinws-root.toml").exists());
}
