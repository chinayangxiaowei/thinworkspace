use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use tempfile::{Builder, TempDir};
use thinws_adapter_macos::{MacOsHostAdapter, MacOsPreparedDataRoot};
use thinws_core::{AbsolutePath, InstallationIdentity, InstanceId, RootMarkerState, VolumeId};
use thinws_ports::{
    BootstrapStore, DataRootLayoutEvidence, LifecycleLock, PortConflict, PortErrorKind,
    PreparedDataRootEvidence, PublishResult,
};

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

fn identity(root: &Path, instance: &str, volume_id: VolumeId) -> InstallationIdentity {
    InstallationIdentity::new(
        InstanceId::from_str(instance).unwrap(),
        AbsolutePath::try_from_bytes(root.as_os_str().as_bytes().to_vec()).unwrap(),
        volume_id,
    )
}

fn prepared_identity(
    adapter: &MacOsHostAdapter,
    root: &Path,
    instance: &str,
) -> (MacOsPreparedDataRoot, InstallationIdentity) {
    let data_root = AbsolutePath::try_from_bytes(root.as_os_str().as_bytes().to_vec()).unwrap();
    let prepared = adapter.prepare_data_root(&data_root).unwrap();
    let identity = InstallationIdentity::new(
        InstanceId::from_str(instance).unwrap(),
        data_root,
        prepared.volume_id(),
    );
    (prepared, identity)
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
fn p1_03_prepares_private_directories_and_descriptor_bound_layout() {
    let temp = controlled_tempdir();
    let bootstrap = temp.path().join("missing").join("bootstrap");
    let data_root = temp.path().join("data-parent").join("data");
    let adapter = MacOsHostAdapter::new(&bootstrap).unwrap();

    adapter.prepare_bootstrap().unwrap();
    let lock = adapter
        .acquire_bootstrap(Duration::from_millis(500))
        .unwrap();
    let (prepared, expected) = prepared_identity(&adapter, &data_root, INSTANCE_ID);
    let proof = adapter
        .create_initializing(&lock, prepared, &expected)
        .unwrap();
    let layout = adapter.initialize_layout(&lock, &proof).unwrap();

    layout.revalidate().unwrap();
    assert_eq!(
        layout.database_path().as_bytes(),
        data_root.join("metadata/state.db").as_os_str().as_bytes()
    );
    for directory in ["metadata", "logs", "workspaces", "staging", "trash"] {
        assert_eq!(
            fs::metadata(data_root.join(directory))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o700
        );
    }
    assert_eq!(
        fs::metadata(data_root.join("metadata/state.db"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o600
    );
    adapter.publish_ready(&lock, proof).unwrap();
    adapter
        .validate_layout(&expected)
        .unwrap()
        .revalidate()
        .unwrap();
}

#[test]
fn p1_03_rejects_nonempty_or_replaced_prepared_roots_without_writing_a_marker() {
    let temp = controlled_tempdir();
    let bootstrap = temp.path().join("bootstrap");
    let nonempty = temp.path().join("nonempty");
    private_dir(&nonempty);
    fs::write(nonempty.join("owned-by-user"), b"preserve").unwrap();
    let adapter = MacOsHostAdapter::new(&bootstrap).unwrap();
    adapter.prepare_bootstrap().unwrap();
    let nonempty_path =
        AbsolutePath::try_from_bytes(nonempty.as_os_str().as_bytes().to_vec()).unwrap();
    assert_eq!(
        adapter
            .prepare_data_root(&nonempty_path)
            .err()
            .unwrap()
            .kind(),
        PortErrorKind::NotEmpty
    );
    assert_eq!(
        fs::read(nonempty.join("owned-by-user")).unwrap(),
        b"preserve"
    );

    let data_root = temp.path().join("data");
    private_dir(&data_root);
    let (prepared, expected) = prepared_identity(&adapter, &data_root, INSTANCE_ID);
    let displaced = temp.path().join("displaced");
    fs::rename(&data_root, &displaced).unwrap();
    private_dir(&data_root);
    let lock = adapter
        .acquire_bootstrap(Duration::from_millis(500))
        .unwrap();
    assert!(
        adapter
            .create_initializing(&lock, prepared, &expected)
            .is_err()
    );
    assert!(!data_root.join(".thinws-root.toml").exists());
    assert!(!displaced.join(".thinws-root.toml").exists());
}

#[test]
fn p1_03_rechecks_prepared_root_emptiness_before_publishing_the_marker() {
    let temp = controlled_tempdir();
    let bootstrap = temp.path().join("bootstrap");
    let data_root = temp.path().join("data");
    let adapter = MacOsHostAdapter::new(&bootstrap).unwrap();
    adapter.prepare_bootstrap().unwrap();
    let lock = adapter
        .acquire_bootstrap(Duration::from_millis(500))
        .unwrap();
    let (prepared, expected) = prepared_identity(&adapter, &data_root, INSTANCE_ID);

    fs::write(data_root.join("arrived-after-prepare"), b"preserve").unwrap();
    let error = adapter
        .create_initializing(&lock, prepared, &expected)
        .err()
        .expect("a prepared root that became nonempty must be rejected");

    assert_eq!(error.kind(), PortErrorKind::NotEmpty);
    assert_eq!(
        fs::read(data_root.join("arrived-after-prepare")).unwrap(),
        b"preserve"
    );
    assert!(!data_root.join(".thinws-root.toml").exists());
}

#[test]
fn p1_03_layout_evidence_detects_data_root_replacement() {
    let temp = controlled_tempdir();
    let bootstrap = temp.path().join("bootstrap");
    let data_root = temp.path().join("data");
    private_dir(&bootstrap);
    private_dir(&data_root);
    let adapter = MacOsHostAdapter::new(&bootstrap).unwrap();
    let lock = adapter
        .acquire_bootstrap(Duration::from_millis(500))
        .unwrap();
    let (prepared, expected) = prepared_identity(&adapter, &data_root, INSTANCE_ID);
    let proof = adapter
        .create_initializing(&lock, prepared, &expected)
        .unwrap();
    let layout = adapter.initialize_layout(&lock, &proof).unwrap();

    let displaced = temp.path().join("displaced");
    fs::rename(&data_root, &displaced).unwrap();
    private_dir(&data_root);
    assert_eq!(
        layout.revalidate().unwrap_err().kind(),
        PortErrorKind::InvalidLayout
    );
    assert!(adapter.publish_ready(&lock, proof).is_err());
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
    let (prepared, expected) = prepared_identity(&adapter, &data_root, INSTANCE_ID);

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

    let proof = adapter
        .create_initializing(&lock, prepared, &expected)
        .unwrap();
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

    let conflicting = identity(&data_root, OTHER_INSTANCE_ID, expected.volume_id());
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

    let (first_prepared, first) = prepared_identity(&adapter, &first_root, INSTANCE_ID);
    let proof = adapter
        .create_initializing(&lock, first_prepared, &first)
        .unwrap();
    let marker_path = first_root.join(".thinws-root.toml");
    let displaced = first_root.join("displaced-marker");
    fs::rename(&marker_path, &displaced).unwrap();
    let replacement = encoded_marker(
        &identity(&first_root, OTHER_INSTANCE_ID, first.volume_id()),
        "ready",
    );
    write_private(&marker_path, &replacement);
    let error = adapter.publish_ready(&lock, proof).unwrap_err();
    assert!(matches!(
        error.kind(),
        PortErrorKind::InvalidData | PortErrorKind::Conflict
    ));
    assert_eq!(fs::read(&marker_path).unwrap(), replacement);

    let (second_prepared, second) = prepared_identity(&adapter, &second_root, INSTANCE_ID);
    let existing = encoded_marker(&second, "initializing");
    write_private(&second_root.join(".thinws-root.toml"), &existing);
    let error = adapter
        .create_initializing(&lock, second_prepared, &second)
        .err()
        .unwrap();
    assert_eq!(error.kind(), PortErrorKind::NotEmpty);
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
    let (prepared, expected) = prepared_identity(&adapter, &data_root, INSTANCE_ID);
    let proof = adapter
        .create_initializing(&lock, prepared, &expected)
        .unwrap();
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
fn an_initializing_marker_moved_into_a_replacement_data_root_cannot_be_promoted() {
    let temp = controlled_tempdir();
    let bootstrap = temp.path().join("bootstrap");
    let data_root = temp.path().join("data");
    let displaced_root = temp.path().join("displaced-data");
    private_dir(&bootstrap);
    private_dir(&data_root);
    let adapter = MacOsHostAdapter::new(&bootstrap).unwrap();
    let lock = adapter
        .acquire_bootstrap(Duration::from_millis(500))
        .unwrap();
    let (prepared, expected) = prepared_identity(&adapter, &data_root, INSTANCE_ID);
    let proof = adapter
        .create_initializing(&lock, prepared, &expected)
        .unwrap();

    fs::rename(&data_root, &displaced_root).unwrap();
    private_dir(&data_root);
    fs::rename(
        displaced_root.join(".thinws-root.toml"),
        data_root.join(".thinws-root.toml"),
    )
    .unwrap();

    let error = adapter.publish_ready(&lock, proof).unwrap_err();
    assert_eq!(error.kind(), PortErrorKind::InvalidData);
    assert_eq!(
        fs::read(data_root.join(".thinws-root.toml")).unwrap(),
        encoded_marker(&expected, "initializing")
    );
    assert!(!bootstrap.join("config.toml").exists());
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

    let (first_prepared, first) = prepared_identity(&adapter, &first_root, INSTANCE_ID);
    let first_proof = adapter
        .create_initializing(&lock, first_prepared, &first)
        .unwrap();
    adapter.publish_ready(&lock, first_proof).unwrap();
    adapter.publish_config(&lock, &first).unwrap();

    let (second_prepared, second) = prepared_identity(&adapter, &second_root, OTHER_INSTANCE_ID);
    let second_proof = adapter
        .create_initializing(&lock, second_prepared, &second)
        .unwrap();
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
    let (prepared, expected) = prepared_identity(&other, &root, INSTANCE_ID);
    let error = other
        .create_initializing(&lock, prepared, &expected)
        .err()
        .unwrap();
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
    let aliased_identity = identity(
        &alias.join("data"),
        INSTANCE_ID,
        VolumeId::from_str(VOLUME_ID).unwrap(),
    );
    assert!(
        direct_adapter
            .prepare_data_root(aliased_identity.data_root())
            .is_err()
    );
    assert!(!real_data_root.join(".thinws-root.toml").exists());
}
