#![cfg(target_os = "linux")]

use std::env;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
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

struct WrongCloneKind;

impl WorkspaceMaterializer for WrongCloneKind {
    fn kind(&self) -> MaterializerKind {
        MaterializerKind::FullCopy
    }

    fn materialize(
        &self,
        _request: &MaterializeRequest,
        _plan: &MaterializationPlan,
    ) -> Result<MaterializationReceipt, MaterializationFailure> {
        panic!("a Full Copy executor cannot be used as the CoW backend");
    }
}

#[test]
fn cow_only_rejects_a_full_copy_executor_before_reserving_a_workspace() {
    let (_control, _data, service, source, target) = fixture();
    let error = service
        .create_cow_only(request(source, target.clone(), false), &WrongCloneKind)
        .unwrap_err();
    assert_eq!(error.diagnostic().code(), ErrorCode::CapabilityUnavailable);
    assert!(!Path::new(std::ffi::OsStr::from_bytes(target.as_bytes())).exists());
    assert!(service.list_workspaces().unwrap().is_empty());
}

#[test]
#[ignore = "requires a real Btrfs bind mount with both source and child test roots"]
fn bind_alias_into_an_active_workspace_is_rejected_before_creation() {
    let bind_source = env::var_os("THINWS_LINUX_BIND_MOUNT_SOURCE").unwrap();
    let bind_child = env::var_os("THINWS_LINUX_BIND_MOUNT_CHILD").unwrap();
    let ext4 = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT").unwrap();
    let control_fixture = Builder::new()
        .prefix("thinws-app-linux-alias-control-")
        .tempdir_in(ext4)
        .unwrap();
    let data_fixture = Builder::new()
        .prefix("thinws-app-linux-alias-data-")
        .tempdir_in(bind_source)
        .unwrap();
    let alias_root = Path::new(&bind_child).join(data_fixture.path().file_name().unwrap());
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
    service
        .create_cow_only(
            request(absolute(&source), absolute(&target), false),
            &BtrfsReflinkMaterializer::new(),
        )
        .unwrap();

    let alias_target = alias_root.join("target");
    assert_eq!(
        fs::metadata(&target).unwrap().dev(),
        fs::metadata(&alias_target).unwrap().dev()
    );
    assert_eq!(
        fs::metadata(&target).unwrap().ino(),
        fs::metadata(&alias_target).unwrap().ino()
    );
    assert!(!alias_target.starts_with(&target));
    let second_source = alias_root.join("second-source");
    fs::create_dir(&second_source).unwrap();
    fs::write(second_source.join("file"), b"second source").unwrap();
    let nested_target = alias_target.join("nested");
    let nested = CreateRequest::new(
        absolute(&second_source),
        absolute(&nested_target),
        WorkspaceName::from_str("nested-alias").unwrap(),
        false,
        UnixMillis::new(1_700_000_000_200).unwrap(),
    );
    assert_eq!(
        service
            .preview_create(&nested)
            .unwrap_err()
            .diagnostic()
            .code(),
        ErrorCode::TargetConflict
    );
    assert_eq!(
        service
            .create_cow_only(nested, &BtrfsReflinkMaterializer::new())
            .unwrap_err()
            .diagnostic()
            .code(),
        ErrorCode::TargetConflict
    );
    assert!(!nested_target.exists());
    assert_eq!(service.list_workspaces().unwrap().len(), 1);
}

#[test]
#[ignore = "requires a real Btrfs bind mount with both source and child test roots"]
fn bind_alias_to_control_directory_is_not_a_valid_source() {
    let bind_source = env::var_os("THINWS_LINUX_BIND_MOUNT_SOURCE").unwrap();
    let bind_child = env::var_os("THINWS_LINUX_BIND_MOUNT_CHILD").unwrap();
    let data_fixture = Builder::new()
        .prefix("thinws-app-linux-control-alias-")
        .tempdir_in(bind_source)
        .unwrap();
    let alias_root = Path::new(&bind_child).join(data_fixture.path().file_name().unwrap());
    let control = data_fixture.path().join(".thinws");
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
    let source = alias_root.join(".thinws");
    assert_eq!(
        fs::metadata(&control).unwrap().ino(),
        fs::metadata(&source).unwrap().ino()
    );
    let target = alias_root.join("target");
    let request = CreateRequest::new(
        absolute(&source),
        absolute(&target),
        WorkspaceName::from_str("control-alias").unwrap(),
        false,
        UnixMillis::new(1_700_000_000_100).unwrap(),
    );
    assert_eq!(
        service
            .preview_create(&request)
            .unwrap_err()
            .diagnostic()
            .code(),
        ErrorCode::TargetLayout
    );
    assert_eq!(
        service
            .create_cow_only(request, &BtrfsReflinkMaterializer::new())
            .unwrap_err()
            .diagnostic()
            .code(),
        ErrorCode::TargetLayout
    );
    assert!(!target.exists());
    assert!(service.list_workspaces().unwrap().is_empty());
}
