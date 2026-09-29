#![cfg(target_os = "linux")]

use std::env;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use thinws_adapter_linux::{LinuxHostAdapter, LinuxPlatformProbe};
use thinws_core::{AbsolutePath, InstallationIdentity, InstanceId, RootMarker, RootMarkerState};
use thinws_ports::{LifecycleLock, PlatformProbe, PortErrorKind, PreparedDataRootEvidence};

fn absolute(path: &Path) -> AbsolutePath {
    AbsolutePath::try_from_bytes(path.as_os_str().as_encoded_bytes().to_vec()).unwrap()
}

#[test]
fn ext4_control_root_is_prepared_without_binding_workspace_to_ext4() {
    let root = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT")
        .expect("set THINWS_LINUX_EXT4_TEST_ROOT to a writable ext4 test directory");
    let fixture = tempfile::Builder::new()
        .prefix("thinws-linux-ext4-control-")
        .tempdir_in(root)
        .unwrap();
    let control = fixture.path().join("control");
    let adapter = LinuxHostAdapter::new(&control).unwrap();
    adapter.prepare_bootstrap().unwrap();
    let _lock = adapter
        .acquire_bootstrap(Duration::from_millis(100))
        .unwrap();
    let prepared = adapter.prepare_data_root(&absolute(&control)).unwrap();
    assert_eq!(prepared.data_root(), &absolute(&control));
    let report = LinuxPlatformProbe
        .inspect_path(prepared.data_root())
        .unwrap();
    assert_eq!(report.filesystem().type_name(), "ext4");
    assert_eq!(
        report.filesystem().volume_id().known().copied(),
        Some(prepared.volume_id())
    );
}

#[test]
fn refuses_foreign_control_root_entries_and_another_control_path() {
    let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
        .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to the dedicated Btrfs test mount");
    let fixture = tempfile::Builder::new()
        .prefix("thinws-linux-control-entries-")
        .tempdir_in(root)
        .unwrap();
    let control = fixture.path().join("control");
    let adapter = LinuxHostAdapter::new(&control).unwrap();
    adapter.prepare_bootstrap().unwrap();
    let _lock = adapter
        .acquire_bootstrap(Duration::from_millis(100))
        .unwrap();
    let prepared = adapter.prepare_data_root(&absolute(&control)).unwrap();
    assert_eq!(prepared.data_root(), &absolute(&control));
    prepared.revalidate().unwrap();
    assert_eq!(
        adapter
            .prepare_data_root(&absolute(fixture.path()))
            .unwrap_err()
            .kind(),
        PortErrorKind::Conflict
    );
    std::fs::write(control.join("foreign"), b"keep").unwrap();
    assert_eq!(
        adapter
            .prepare_data_root(&absolute(&control))
            .unwrap_err()
            .kind(),
        PortErrorKind::NotEmpty
    );
    assert_eq!(std::fs::read(control.join("foreign")).unwrap(), b"keep");
}

#[test]
fn prepared_control_root_detects_replacement() {
    let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
        .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to the dedicated Btrfs test mount");
    let fixture = tempfile::Builder::new()
        .prefix("thinws-linux-control-replace-")
        .tempdir_in(root)
        .unwrap();
    let control = fixture.path().join("control");
    let adapter = LinuxHostAdapter::new(&control).unwrap();
    adapter.prepare_bootstrap().unwrap();
    let _lock = adapter
        .acquire_bootstrap(Duration::from_millis(100))
        .unwrap();
    let prepared = adapter.prepare_data_root(&absolute(&control)).unwrap();
    std::fs::rename(&control, fixture.path().join("displaced")).unwrap();
    std::fs::create_dir(&control).unwrap();
    assert_eq!(
        prepared.revalidate().unwrap_err().kind(),
        PortErrorKind::InvalidLayout
    );
}

#[test]
fn unclaimed_control_root_does_not_accept_a_forged_lock_entry() {
    let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
        .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to the dedicated Btrfs test mount");
    let fixture = tempfile::Builder::new()
        .prefix("thinws-linux-control-lock-entry-")
        .tempdir_in(root)
        .unwrap();
    let control = fixture.path().join("control");
    let adapter = LinuxHostAdapter::new(&control).unwrap();
    adapter.prepare_bootstrap().unwrap();
    std::os::unix::fs::symlink("outside", control.join("lifecycle.lock")).unwrap();
    assert_eq!(
        adapter
            .prepare_data_root(&absolute(&control))
            .unwrap_err()
            .kind(),
        PortErrorKind::InvalidLayout
    );
}

#[test]
fn read_only_bootstrap_documents_reject_links_and_preserve_missing_state() {
    let root = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT")
        .expect("set THINWS_LINUX_EXT4_TEST_ROOT to a writable ext4 test directory");
    let fixture = tempfile::Builder::new()
        .prefix("thinws-linux-documents-")
        .tempdir_in(root)
        .unwrap();
    let control = fixture.path().join("control");
    let adapter = LinuxHostAdapter::new(&control).unwrap();
    assert_eq!(adapter.read_config().unwrap(), None);
    assert_eq!(adapter.read_root_marker(&absolute(&control)).unwrap(), None);
    assert!(!control.exists());

    adapter.prepare_bootstrap().unwrap();
    let identity = InstallationIdentity::new(
        "01890a5d-ac96-774b-bd5b-55c7b8d09f33"
            .parse::<InstanceId>()
            .unwrap(),
        absolute(&control),
        LinuxPlatformProbe
            .inspect_path(&absolute(&control))
            .unwrap()
            .filesystem()
            .volume_id()
            .known()
            .copied()
            .unwrap(),
    );
    let root_hex = control
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let fields = format!(
        "schema_version = 2\ninstance_id = \"{}\"\ncontrol_root_hex = \"{root_hex}\"\ncontrol_volume_id = \"{}\"\n",
        identity.instance_id(),
        identity.volume_id()
    );
    let config = control.join("config.toml");
    std::fs::write(&config, &fields).unwrap();
    std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o600)).unwrap();
    let marker = control.join(".thinws-control.toml");
    std::fs::write(&marker, format!("{fields}state = \"initializing\"\n")).unwrap();
    std::fs::set_permissions(&marker, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(adapter.read_config().unwrap(), Some(identity.clone()));
    assert_eq!(
        adapter.read_root_marker(&absolute(&control)).unwrap(),
        Some(RootMarker::new(identity, RootMarkerState::Initializing))
    );

    std::fs::rename(&config, control.join("displaced.toml")).unwrap();
    std::os::unix::fs::symlink("displaced.toml", &config).unwrap();
    assert_eq!(
        adapter.read_config().unwrap_err().kind(),
        PortErrorKind::InvalidLayout
    );
}
