#![cfg(target_os = "linux")]

use std::env;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use tempfile::{Builder, TempDir};
use thinws_adapter_linux::{BtrfsReflinkMaterializer, LinuxHostAdapter};
use thinws_application::{CreateRequest, InitRequest, ThinWorkspaceService};
use thinws_core::{
    AbsolutePath, ErrorCode, MaterializationPlan, MaterializationReceipt, MaterializeRequest,
    MaterializerKind, UnixMillis, WorkspaceName,
};
use thinws_metadata_sqlite::SqliteMetadataStoreFactory;
use thinws_ports::{MaterializationFailure, WorkspaceMaterializer};

fn absolute(path: &Path) -> AbsolutePath {
    AbsolutePath::try_from_bytes(path.as_os_str().as_bytes().to_vec()).unwrap()
}

fn fixture() -> (
    TempDir,
    TempDir,
    ThinWorkspaceService<LinuxHostAdapter, SqliteMetadataStoreFactory>,
    AbsolutePath,
    AbsolutePath,
) {
    let ext4 = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT")
        .expect("set THINWS_LINUX_EXT4_TEST_ROOT to a writable ext4 test root");
    let btrfs = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
        .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs test root");
    let control_fixture = Builder::new()
        .prefix("thinws-app-linux-control-")
        .tempdir_in(ext4)
        .unwrap();
    let data_fixture = Builder::new()
        .prefix("thinws-app-linux-data-")
        .tempdir_in(btrfs)
        .unwrap();
    let control = control_fixture.path().join(".thinws");
    let source = data_fixture.path().join("source");
    let target = data_fixture.path().join("target");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("file"), b"source bytes").unwrap();
    let service = ThinWorkspaceService::new(
        LinuxHostAdapter::new(&control).unwrap(),
        SqliteMetadataStoreFactory,
        Duration::from_secs(1),
        Duration::from_secs(1),
    );
    service
        .init(InitRequest::new(
            absolute(&control),
            UnixMillis::new(1_700_000_000_000).unwrap(),
        ))
        .unwrap();
    (
        control_fixture,
        data_fixture,
        service,
        absolute(&source),
        absolute(&target),
    )
}

fn request(source: AbsolutePath, target: AbsolutePath, allow_copy: bool) -> CreateRequest {
    CreateRequest::new(
        source,
        target,
        WorkspaceName::from_str("linux-create").unwrap(),
        allow_copy,
        UnixMillis::new(1_700_000_000_100).unwrap(),
    )
}

#[test]
fn same_name_reuse_requires_the_same_copy_policy() {
    let (_control, _data, service, source, target) = fixture();
    let clone = BtrfsReflinkMaterializer::new();
    let original = request(source.clone(), target.clone(), false);
    assert!(
        service
            .create_cow_only(original.clone(), &clone)
            .unwrap()
            .created()
    );
    assert!(!service.create_cow_only(original, &clone).unwrap().created());

    let error = service
        .create_cow_only(request(source, target, true), &clone)
        .unwrap_err();
    assert_eq!(error.diagnostic().code(), ErrorCode::NameConflict);
}

struct WrongPlatformClone;

impl WorkspaceMaterializer for WrongPlatformClone {
    fn kind(&self) -> MaterializerKind {
        MaterializerKind::ApfsFileClone
    }

    fn materialize(
        &self,
        _request: &MaterializeRequest,
        _plan: &MaterializationPlan,
    ) -> Result<MaterializationReceipt, MaterializationFailure> {
        panic!("a mismatched materializer must not be called");
    }
}

#[test]
fn selected_btrfs_backend_rejects_an_injected_apfs_executor() {
    let (_control, _data, service, source, target) = fixture();
    let error = service
        .create_cow_only(request(source, target.clone(), false), &WrongPlatformClone)
        .unwrap_err();
    assert_eq!(error.diagnostic().code(), ErrorCode::CapabilityUnavailable);
    let target = Path::new(std::ffi::OsStr::from_bytes(target.as_bytes()));
    assert!(target.is_dir());
    assert!(fs::read_dir(target).unwrap().next().is_none());
}
