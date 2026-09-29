#![cfg(target_os = "linux")]

use std::env;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::time::Duration;

use thinws_adapter_linux::{LinuxHostAdapter, LinuxPlatformProbe};
use thinws_core::{AbsolutePath, InstallationIdentity, InstallationRecord, InstanceId, UnixMillis};
use thinws_metadata_sqlite::SqliteMetadataStoreFactory;
use thinws_ports::{DataRootLayoutEvidence, MetadataStoreFactory, PlatformProbe, PortErrorKind};

fn absolute(path: &Path) -> AbsolutePath {
    AbsolutePath::try_from_bytes(path.as_os_str().as_encoded_bytes().to_vec()).unwrap()
}

fn private_dir(path: &Path) {
    std::fs::create_dir(path).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

fn fixture() -> (
    tempfile::TempDir,
    PathBuf,
    LinuxHostAdapter,
    InstallationIdentity,
) {
    let root = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT")
        .expect("set THINWS_LINUX_EXT4_TEST_ROOT to a writable ext4 test directory");
    let fixture = tempfile::Builder::new()
        .prefix("thinws-linux-layout-")
        .tempdir_in(root)
        .unwrap();
    let control = fixture.path().join("control");
    private_dir(&control);
    private_dir(&control.join("metadata"));
    private_dir(&control.join("logs"));
    let database = control.join("metadata/state.db");
    std::fs::write(&database, b"").unwrap();
    std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o600)).unwrap();
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
    (fixture, control, adapter, identity)
}

#[test]
fn ext4_layout_can_back_the_real_sqlite_factory_without_moving_workspace_data() {
    let (_fixture, control, adapter, identity) = fixture();
    let layout = adapter.validate_layout(&identity).unwrap();
    assert_eq!(
        layout.database_path(),
        &absolute(&control.join("metadata/state.db"))
    );
    let record = InstallationRecord::new(identity, UnixMillis::new(1_700_000_000_000).unwrap());
    assert_eq!(
        SqliteMetadataStoreFactory
            .initialize(&layout, &record, Duration::from_millis(200))
            .unwrap(),
        record
    );
    layout.revalidate().unwrap();
    let snapshot = SqliteMetadataStoreFactory
        .inspect(&layout, &record, Duration::from_millis(200))
        .unwrap();
    assert_eq!(snapshot.installation(), &record);
}

#[test]
fn layout_rejects_replaced_database_and_linked_metadata() {
    let (_fixture, control, adapter, identity) = fixture();
    let layout = adapter.validate_layout(&identity).unwrap();
    let database = control.join("metadata/state.db");
    std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        layout.revalidate().unwrap_err().kind(),
        PortErrorKind::InvalidLayout
    );
    std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o600)).unwrap();
    layout.revalidate().unwrap();
    std::fs::rename(&database, control.join("metadata/displaced.db")).unwrap();
    std::fs::write(&database, b"").unwrap();
    std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        layout.revalidate().unwrap_err().kind(),
        PortErrorKind::InvalidLayout
    );

    std::fs::rename(control.join("metadata"), control.join("displaced")).unwrap();
    symlink("displaced", control.join("metadata")).unwrap();
    assert_eq!(
        adapter.validate_layout(&identity).unwrap_err().kind(),
        PortErrorKind::InvalidLayout
    );
}
