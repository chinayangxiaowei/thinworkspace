#![cfg(target_os = "linux")]

use std::env;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use thinws_adapter_linux::{LinuxHostAdapter, LinuxPlatformProbe};
use thinws_core::{AbsolutePath, InstallationIdentity, InstanceId, WorkspaceId};
use thinws_ports::{LifecycleLock, PlatformProbe, PortErrorKind, PreparedWorkspaceEvidence};

fn absolute(path: &Path) -> AbsolutePath {
    AbsolutePath::try_from_bytes(path.as_os_str().as_encoded_bytes().to_vec()).unwrap()
}

fn fixture() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    LinuxHostAdapter,
    InstallationIdentity,
) {
    let control_root = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT")
        .expect("set THINWS_LINUX_EXT4_TEST_ROOT to a writable ext4 test directory");
    let target_root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
        .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs test directory");
    let control_fixture = tempfile::Builder::new()
        .prefix("thinws-linux-workspace-control-")
        .tempdir_in(control_root)
        .unwrap();
    let target_fixture = tempfile::Builder::new()
        .prefix("thinws-linux-workspace-target-")
        .tempdir_in(target_root)
        .unwrap();
    let control = control_fixture.path().join("control");
    for path in [&control, &control.join("metadata"), &control.join("logs")] {
        std::fs::create_dir(path).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::fs::write(control.join("metadata/state.db"), b"").unwrap();
    std::fs::set_permissions(
        control.join("metadata/state.db"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let adapter = LinuxHostAdapter::new(&control).unwrap();
    let volume = LinuxPlatformProbe
        .inspect_path(&absolute(&control))
        .unwrap()
        .filesystem()
        .volume_id()
        .known()
        .copied()
        .unwrap();
    let identity = InstallationIdentity::new(
        "01890a5d-ac96-774b-bd5b-55c7b8d09f33"
            .parse::<InstanceId>()
            .unwrap(),
        absolute(&control),
        volume,
    );
    (control_fixture, target_fixture, adapter, identity)
}

#[test]
fn creates_plain_btrfs_target_with_durable_ownership_only_in_ext4_control_root() {
    let (control, target_fixture, adapter, identity) = fixture();
    let layout = adapter.validate_layout(&identity).unwrap();
    let lock = adapter
        .acquire_data_root(identity.data_root(), Duration::from_millis(200))
        .unwrap();
    let workspace_id = "ws_01890a5d-ac96-774b-bd5b-55c7b8d09f34"
        .parse::<WorkspaceId>()
        .unwrap();
    let target = target_fixture.path().join("working-copy");
    let prepared = adapter
        .prepare_workspace(&lock, &layout, workspace_id, &absolute(&target))
        .unwrap();
    prepared.revalidate().unwrap();
    assert_eq!(prepared.target_root(), &absolute(&target));
    assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
    assert!(
        prepared
            .staging_root()
            .as_bytes()
            .starts_with(target_fixture.path().as_os_str().as_encoded_bytes())
    );
    assert!(
        prepared
            .trash_root()
            .as_bytes()
            .starts_with(target_fixture.path().as_os_str().as_encoded_bytes())
    );
    assert!(
        control
            .path()
            .join(format!("control/metadata/ownership-{workspace_id}.toml"))
            .exists()
    );
    assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
    let second_target = target_fixture.path().join("another-copy");
    assert_eq!(
        adapter
            .prepare_workspace(&lock, &layout, workspace_id, &absolute(&second_target))
            .unwrap_err()
            .kind(),
        PortErrorKind::Conflict
    );
    assert!(!second_target.exists());
    std::fs::rename(&target, target_fixture.path().join("replaced-old")).unwrap();
    std::fs::create_dir(&target).unwrap();
    assert_eq!(
        prepared.revalidate().unwrap_err().kind(),
        PortErrorKind::InvalidLayout
    );
}

#[test]
fn existing_target_is_never_adopted_or_overwritten() {
    let (_control, target_fixture, adapter, identity) = fixture();
    let layout = adapter.validate_layout(&identity).unwrap();
    let lock = adapter
        .acquire_data_root(identity.data_root(), Duration::from_millis(200))
        .unwrap();
    let target = target_fixture.path().join("occupied");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("keep"), b"foreign").unwrap();
    let workspace_id = "ws_01890a5d-ac96-774b-bd5b-55c7b8d09f34"
        .parse::<WorkspaceId>()
        .unwrap();
    assert_eq!(
        adapter
            .prepare_workspace(&lock, &layout, workspace_id, &absolute(&target))
            .unwrap_err()
            .kind(),
        PortErrorKind::NotEmpty
    );
    assert_eq!(std::fs::read(target.join("keep")).unwrap(), b"foreign");
}

#[test]
fn occupied_operation_sibling_prevents_target_creation() {
    let (_control, target_fixture, adapter, identity) = fixture();
    let layout = adapter.validate_layout(&identity).unwrap();
    let lock = adapter
        .acquire_data_root(identity.data_root(), Duration::from_millis(200))
        .unwrap();
    let workspace_id = "ws_01890a5d-ac96-774b-bd5b-55c7b8d09f34"
        .parse::<WorkspaceId>()
        .unwrap();
    let staging = target_fixture
        .path()
        .join(format!(".thinws-staging-{workspace_id}"));
    std::fs::create_dir(&staging).unwrap();
    std::fs::write(staging.join("keep"), b"foreign").unwrap();
    let target = target_fixture.path().join("working-copy");
    assert_eq!(
        adapter
            .prepare_workspace(&lock, &layout, workspace_id, &absolute(&target))
            .unwrap_err()
            .kind(),
        PortErrorKind::NotEmpty
    );
    assert!(!target.exists());
    assert_eq!(std::fs::read(staging.join("keep")).unwrap(), b"foreign");
}

#[test]
fn ext4_target_is_rejected_before_creating_any_workspace_directories() {
    let (control, _target_fixture, adapter, identity) = fixture();
    let layout = adapter.validate_layout(&identity).unwrap();
    let lock = adapter
        .acquire_data_root(identity.data_root(), Duration::from_millis(200))
        .unwrap();
    let workspace_id = "ws_01890a5d-ac96-774b-bd5b-55c7b8d09f34"
        .parse::<WorkspaceId>()
        .unwrap();
    let target = control.path().join("outside-control");
    assert_eq!(
        adapter
            .prepare_workspace(&lock, &layout, workspace_id, &absolute(&target))
            .unwrap_err()
            .kind(),
        PortErrorKind::CapabilityUnavailable
    );
    assert!(!target.exists());
}

#[test]
fn changed_durable_ownership_invalidates_the_prepared_workspace() {
    let (control, target_fixture, adapter, identity) = fixture();
    let layout = adapter.validate_layout(&identity).unwrap();
    let lock = adapter
        .acquire_data_root(identity.data_root(), Duration::from_millis(200))
        .unwrap();
    let workspace_id = "ws_01890a5d-ac96-774b-bd5b-55c7b8d09f34"
        .parse::<WorkspaceId>()
        .unwrap();
    let target = target_fixture.path().join("working-copy");
    let prepared = adapter
        .prepare_workspace(&lock, &layout, workspace_id, &absolute(&target))
        .unwrap();
    let document = control
        .path()
        .join(format!("control/metadata/ownership-{workspace_id}.toml"));
    let bytes = std::fs::read_to_string(&document).unwrap();
    let changed = bytes.replace("schema_version = 3", "schema_version = 2");
    assert_ne!(changed, bytes);
    std::fs::write(&document, changed).unwrap();
    assert_eq!(
        prepared.revalidate().unwrap_err().kind(),
        PortErrorKind::InvalidLayout
    );
}
