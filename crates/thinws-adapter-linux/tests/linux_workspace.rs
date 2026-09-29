#![cfg(target_os = "linux")]

use std::env;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::time::Duration;

use thinws_adapter_linux::{BtrfsReflinkMaterializer, LinuxHostAdapter, LinuxPlatformProbe};
use thinws_core::{
    AbsolutePath, CowEvidence, FallbackPolicy, InstallationIdentity, InstanceId,
    MaterializationOutcome, MaterializationPlan, MaterializeRequest, UnixMillis, WorkspaceId,
    WorkspaceName, WorkspaceReservation,
};
use thinws_ports::{
    LifecycleLock, MaterializationPathProbeRequest, PlatformProbe, PortErrorKind,
    PreparedWorkspaceEvidence, WorkspaceMaterializer, WorkspaceSpace,
};

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

fn reservation(
    identity: &InstallationIdentity,
    workspace_id: WorkspaceId,
    source: &Path,
    target: &Path,
) -> WorkspaceReservation {
    let volume = LinuxPlatformProbe
        .inspect_path(&absolute(source))
        .unwrap()
        .filesystem()
        .volume_id()
        .known()
        .copied()
        .unwrap();
    WorkspaceReservation::new(
        workspace_id,
        identity.instance_id(),
        "test-copy".parse::<WorkspaceName>().unwrap(),
        absolute(source),
        absolute(target),
        volume,
        volume,
        false,
        UnixMillis::new(1_700_000_000_000).unwrap(),
    )
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

#[test]
fn completed_workspace_drops_only_empty_operation_directories_and_reopens_by_registration() {
    let (control, target_fixture, adapter, identity) = fixture();
    let layout = adapter.validate_layout(&identity).unwrap();
    let lock = adapter
        .acquire_data_root(identity.data_root(), Duration::from_millis(200))
        .unwrap();
    let workspace_id = "ws_01890a5d-ac96-774b-bd5b-55c7b8d09f34"
        .parse::<WorkspaceId>()
        .unwrap();
    let target = target_fixture.path().join("working-copy");
    let registered = reservation(&identity, workspace_id, target_fixture.path(), &target);
    let prepared = adapter
        .prepare_workspace(&lock, &layout, workspace_id, &absolute(&target))
        .unwrap();
    assert_eq!(
        adapter
            .validate_ready_workspace(&layout, &registered)
            .unwrap_err()
            .kind(),
        PortErrorKind::InvalidLayout
    );
    let staging = target_fixture
        .path()
        .join(format!(".thinws-staging-{workspace_id}"));
    let trash = target_fixture
        .path()
        .join(format!(".thinws-trash-{workspace_id}"));
    adapter
        .clear_workspace_incomplete(&lock, &layout, prepared)
        .unwrap();
    assert!(!staging.exists());
    assert!(!trash.exists());
    assert_eq!(
        adapter
            .validate_ready_workspace(&layout, &registered)
            .unwrap(),
        absolute(&target)
    );
    assert!(
        control
            .path()
            .join(format!("control/metadata/ownership-{workspace_id}.toml"))
            .exists()
    );
    std::fs::rename(&target, target_fixture.path().join("displaced")).unwrap();
    std::fs::create_dir(&target).unwrap();
    assert_eq!(
        adapter
            .validate_ready_workspace(&layout, &registered)
            .unwrap_err()
            .kind(),
        PortErrorKind::InvalidLayout
    );
}

#[test]
fn nonempty_staging_cannot_be_cleared() {
    let (_control, target_fixture, adapter, identity) = fixture();
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
    let staging = target_fixture
        .path()
        .join(format!(".thinws-staging-{workspace_id}"));
    std::fs::write(staging.join("keep"), b"not empty").unwrap();
    assert_eq!(
        adapter
            .clear_workspace_incomplete(&lock, &layout, prepared)
            .unwrap_err()
            .kind(),
        PortErrorKind::NotEmpty
    );
    assert_eq!(std::fs::read(staging.join("keep")).unwrap(), b"not empty");
    assert!(target.exists());
}

#[test]
fn nonempty_trash_refuses_completion_without_removing_empty_staging() {
    let (_control, target_fixture, adapter, identity) = fixture();
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
    let staging = target_fixture
        .path()
        .join(format!(".thinws-staging-{workspace_id}"));
    let trash = target_fixture
        .path()
        .join(format!(".thinws-trash-{workspace_id}"));
    std::fs::write(trash.join("keep"), b"not empty").unwrap();
    assert_eq!(
        adapter
            .clear_workspace_incomplete(&lock, &layout, prepared)
            .unwrap_err()
            .kind(),
        PortErrorKind::NotEmpty
    );
    assert!(staging.is_dir());
    assert_eq!(std::fs::read(trash.join("keep")).unwrap(), b"not empty");
}

#[test]
fn ready_query_rejects_missing_target_and_replaced_parent() {
    let (_control, target_fixture, adapter, identity) = fixture();
    let layout = adapter.validate_layout(&identity).unwrap();
    let lock = adapter
        .acquire_data_root(identity.data_root(), Duration::from_millis(200))
        .unwrap();
    let workspace_id = "ws_01890a5d-ac96-774b-bd5b-55c7b8d09f34"
        .parse::<WorkspaceId>()
        .unwrap();
    let parent = target_fixture.path().join("parent");
    std::fs::create_dir(&parent).unwrap();
    let target = parent.join("working-copy");
    let registered = reservation(&identity, workspace_id, target_fixture.path(), &target);
    let prepared = adapter
        .prepare_workspace(&lock, &layout, workspace_id, &absolute(&target))
        .unwrap();
    adapter
        .clear_workspace_incomplete(&lock, &layout, prepared)
        .unwrap();
    std::fs::rename(&target, parent.join("displaced")).unwrap();
    assert_eq!(
        adapter
            .validate_ready_workspace(&layout, &registered)
            .unwrap_err()
            .kind(),
        PortErrorKind::NotFound
    );
    std::fs::rename(&parent, target_fixture.path().join("old-parent")).unwrap();
    std::fs::create_dir(&parent).unwrap();
    std::fs::create_dir(&target).unwrap();
    assert_eq!(
        adapter
            .validate_ready_workspace(&layout, &registered)
            .unwrap_err()
            .kind(),
        PortErrorKind::InvalidLayout
    );
}

#[test]
fn ready_query_rejects_a_registration_with_a_different_target_volume() {
    let (_control, target_fixture, adapter, identity) = fixture();
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
    adapter
        .clear_workspace_incomplete(&lock, &layout, prepared)
        .unwrap();
    let forged = WorkspaceReservation::new(
        workspace_id,
        identity.instance_id(),
        "test-copy".parse::<WorkspaceName>().unwrap(),
        absolute(target_fixture.path()),
        absolute(&target),
        identity.volume_id(),
        identity.volume_id(),
        false,
        UnixMillis::new(1_700_000_000_000).unwrap(),
    );
    assert_eq!(
        adapter
            .validate_ready_workspace(&layout, &forged)
            .unwrap_err()
            .kind(),
        PortErrorKind::InvalidLayout
    );
    assert!(target.is_dir());
}

#[test]
fn actual_btrfs_materialization_can_be_completed_and_reopened_as_an_ordinary_path() {
    let (_control, target_fixture, adapter, identity) = fixture();
    let layout = adapter.validate_layout(&identity).unwrap();
    let lock = adapter
        .acquire_data_root(identity.data_root(), Duration::from_millis(200))
        .unwrap();
    let workspace_id = "ws_01890a5d-ac96-774b-bd5b-55c7b8d09f34"
        .parse::<WorkspaceId>()
        .unwrap();
    let source = target_fixture.path().join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("file"), b"source bytes").unwrap();
    let target = target_fixture.path().join("working-copy");
    let registered = reservation(&identity, workspace_id, &source, &target);
    let prepared = adapter
        .prepare_workspace(&lock, &layout, workspace_id, &absolute(&target))
        .unwrap();
    let request = MaterializeRequest::new(
        absolute(&source),
        prepared.target_root().clone(),
        prepared.staging_root().clone(),
        prepared.trash_root().clone(),
    );
    let report = adapter
        .inspect_materialization_paths(&MaterializationPathProbeRequest::from(&request))
        .unwrap();
    let plan = MaterializationPlan::for_cow_clone(&report, FallbackPolicy::Deny).unwrap();
    let receipt = BtrfsReflinkMaterializer::new()
        .materialize(&request, &plan)
        .unwrap();
    assert_eq!(receipt.outcome(), MaterializationOutcome::Succeeded);
    assert_eq!(receipt.cow_evidence(), CowEvidence::Confirmed);
    prepared.revalidate().unwrap();
    adapter
        .clear_workspace_incomplete(&lock, &layout, prepared)
        .unwrap();
    assert_eq!(
        adapter
            .validate_ready_workspace(&layout, &registered)
            .unwrap(),
        absolute(&target)
    );
    assert_eq!(std::fs::read(target.join("file")).unwrap(), b"source bytes");
    std::fs::write(target.join("file"), b"changed").unwrap();
    assert_eq!(std::fs::read(source.join("file")).unwrap(), b"source bytes");
}

#[test]
fn ready_space_counts_files_links_and_unique_allocated_inodes_on_real_btrfs() {
    let (_control, target_fixture, adapter, identity) = fixture();
    let layout = adapter.validate_layout(&identity).unwrap();
    let lock = adapter
        .acquire_data_root(identity.data_root(), Duration::from_millis(200))
        .unwrap();
    let workspace_id = "ws_01890a5d-ac96-774b-bd5b-55c7b8d09f34"
        .parse::<WorkspaceId>()
        .unwrap();
    let target = target_fixture.path().join("working-copy");
    let registered = reservation(&identity, workspace_id, target_fixture.path(), &target);
    let prepared = adapter
        .prepare_workspace(&lock, &layout, workspace_id, &absolute(&target))
        .unwrap();
    adapter
        .clear_workspace_incomplete(&lock, &layout, prepared)
        .unwrap();
    std::fs::write(target.join("file"), b"hello").unwrap();
    std::fs::hard_link(target.join("file"), target.join("alias")).unwrap();
    symlink("file", target.join("link")).unwrap();
    std::fs::create_dir(target.join("nested")).unwrap();
    std::fs::write(target.join("nested/other"), b"abc").unwrap();
    let allocated = [
        target.clone(),
        target.join("file"),
        target.join("link"),
        target.join("nested"),
        target.join("nested/other"),
    ]
    .iter()
    .map(|path| std::fs::symlink_metadata(path).unwrap().blocks() * 512)
    .sum();
    assert_eq!(
        adapter
            .measure_ready_workspace_space(&layout, &registered)
            .unwrap(),
        WorkspaceSpace::Complete {
            logical_bytes: 17,
            allocated_bytes_estimate: allocated,
        }
    );
    let _socket = UnixListener::bind(target.join("socket")).unwrap();
    assert_eq!(
        adapter
            .measure_ready_workspace_space(&layout, &registered)
            .unwrap(),
        WorkspaceSpace::Unknown
    );
}

#[test]
fn invalid_ready_target_is_an_error_not_an_unknown_space_estimate() {
    let (_control, target_fixture, adapter, identity) = fixture();
    let layout = adapter.validate_layout(&identity).unwrap();
    let lock = adapter
        .acquire_data_root(identity.data_root(), Duration::from_millis(200))
        .unwrap();
    let workspace_id = "ws_01890a5d-ac96-774b-bd5b-55c7b8d09f34"
        .parse::<WorkspaceId>()
        .unwrap();
    let target = target_fixture.path().join("working-copy");
    let registered = reservation(&identity, workspace_id, target_fixture.path(), &target);
    let prepared = adapter
        .prepare_workspace(&lock, &layout, workspace_id, &absolute(&target))
        .unwrap();
    adapter
        .clear_workspace_incomplete(&lock, &layout, prepared)
        .unwrap();
    std::fs::rename(&target, target_fixture.path().join("displaced")).unwrap();
    std::fs::create_dir(&target).unwrap();
    assert_eq!(
        adapter
            .measure_ready_workspace_space(&layout, &registered)
            .unwrap_err()
            .kind(),
        PortErrorKind::InvalidLayout
    );
}
