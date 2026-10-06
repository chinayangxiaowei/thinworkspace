#![cfg(target_os = "linux")]

use std::env;
use std::os::unix::fs::symlink;
use std::path::Path;

use thinws_adapter_linux::LinuxPlatformProbe;
use thinws_core::{
    AbsolutePath, FallbackPolicy, MaterializationPlan, MaterializerKind, PathResolution,
    SupportState,
};
use thinws_ports::{MaterializationPathProbeRequest, PlatformProbe};

fn absolute(path: &Path) -> AbsolutePath {
    AbsolutePath::try_from_bytes(path.as_os_str().as_encoded_bytes().to_vec()).unwrap()
}

#[test]
fn actual_ext4_control_root_has_a_stable_non_btrfs_identity() {
    let root = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT")
        .expect("set THINWS_LINUX_EXT4_TEST_ROOT to a readable ext4 directory");
    let path = absolute(Path::new(&root));
    let first = LinuxPlatformProbe.inspect_path(&path).unwrap();
    let second = LinuxPlatformProbe.inspect_path(&path).unwrap();
    assert_eq!(first.filesystem().type_name(), "ext4");
    assert_eq!(
        first.filesystem().volume_id().known(),
        second.filesystem().volume_id().known()
    );
    assert!(first.filesystem().volume_id().known().is_some());
    assert_eq!(first.cow_clone(), SupportState::Unsupported);
}

#[test]
fn real_shared_source_is_not_a_btrfs_clone_candidate() {
    let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
        .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to the dedicated Btrfs test mount");
    let shared_file = env::var_os("THINWS_LINUX_OTHER_TEST_FILE")
        .expect("set THINWS_LINUX_OTHER_TEST_FILE to a readable file on the shared mount");
    let fixture = tempfile::Builder::new()
        .prefix("thinws-linux-crossfs-")
        .tempdir_in(root)
        .unwrap();
    let source = Path::new(&shared_file).parent().unwrap();
    let request = MaterializationPathProbeRequest::new(
        absolute(source),
        absolute(&fixture.path().join("target")),
        absolute(&fixture.path().join("staging")),
        absolute(&fixture.path().join("trash")),
    );
    let report = LinuxPlatformProbe
        .inspect_materialization_paths(&request)
        .unwrap();
    assert_eq!(report.cow_clone().state(), SupportState::Unsupported);
    assert_eq!(report.full_copy().state(), SupportState::Unsupported);
    assert_ne!(report.source().filesystem().type_name(), "btrfs");
}

#[test]
fn real_ext4_source_to_btrfs_target_is_not_a_clone_candidate() {
    let ext4 = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT")
        .expect("set THINWS_LINUX_EXT4_TEST_ROOT to a writable ext4 test root");
    let btrfs = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
        .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs test root");
    let source_fixture = tempfile::Builder::new()
        .prefix("thinws-linux-ext4-source-")
        .tempdir_in(ext4)
        .unwrap();
    let target_fixture = tempfile::Builder::new()
        .prefix("thinws-linux-btrfs-target-")
        .tempdir_in(btrfs)
        .unwrap();
    let source = source_fixture.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let request = MaterializationPathProbeRequest::new(
        absolute(&source),
        absolute(&target_fixture.path().join("target")),
        absolute(&target_fixture.path().join("staging")),
        absolute(&target_fixture.path().join("trash")),
    );
    let report = LinuxPlatformProbe
        .inspect_materialization_paths(&request)
        .unwrap();
    assert_eq!(report.source().filesystem().type_name(), "ext4");
    assert_eq!(report.target_root().filesystem().type_name(), "btrfs");
    assert_eq!(report.cow_clone().state(), SupportState::Unsupported);
    assert_eq!(report.full_copy().state(), SupportState::Unsupported);
}

#[test]
fn path_probe_refuses_symlinked_components() {
    let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
        .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to the dedicated Btrfs test mount");
    let fixture = tempfile::Builder::new()
        .prefix("thinws-linux-symlink-")
        .tempdir_in(root)
        .unwrap();
    let genuine = fixture.path().join("genuine");
    std::fs::create_dir(&genuine).unwrap();
    let alias = fixture.path().join("alias");
    symlink(&genuine, &alias).unwrap();
    let error = LinuxPlatformProbe
        .inspect_path(&absolute(&alias))
        .unwrap_err();
    assert_eq!(error.kind(), thinws_ports::PortErrorKind::InvalidLayout);
}

#[test]
fn actual_btrfs_paths_have_one_known_mount_and_a_btrfs_candidate() {
    let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
        .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to the dedicated Btrfs test mount");
    let fixture = tempfile::Builder::new()
        .prefix("thinws-linux-probe-")
        .tempdir_in(root)
        .unwrap();
    let source = fixture.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let request = MaterializationPathProbeRequest::new(
        absolute(&source),
        absolute(&fixture.path().join("target")),
        absolute(&fixture.path().join("staging")),
        absolute(&fixture.path().join("trash")),
    );
    let probe = LinuxPlatformProbe;
    let host = probe.inspect_host().unwrap();
    assert_eq!(host.platform(), "Linux");
    assert_eq!(host.adapter(), "linux-btrfs");
    let report = probe.inspect_materialization_paths(&request).unwrap();
    assert_eq!(report.source().writability(), SupportState::Supported);
    assert_eq!(report.cow_clone().kind(), MaterializerKind::BtrfsReflink);
    assert_ne!(report.cow_clone().state(), SupportState::Unsupported);
    assert_eq!(report.full_copy().state(), SupportState::Unknown);
    let volume = report
        .source()
        .filesystem()
        .volume_id()
        .known()
        .copied()
        .unwrap();
    let mount = report.source().mount().mount_id().unwrap();
    for path in [report.target_root(), report.staging(), report.trash()] {
        assert_eq!(path.filesystem().type_name(), "btrfs");
        assert_eq!(path.filesystem().volume_id().known().copied(), Some(volume));
        assert_eq!(path.mount().mount_id(), Some(mount));
        assert_eq!(path.resolution(), PathResolution::MissingTarget);
    }
    let plan = MaterializationPlan::for_cow_clone(&report, FallbackPolicy::Deny).unwrap();
    assert_eq!(plan.selected_adapter(), MaterializerKind::BtrfsReflink);
}

#[test]
#[ignore = "requires THINWS_LINUX_SECOND_BTRFS_MOUNT_ROOT on a distinct mount of the same Btrfs filesystem"]
fn same_btrfs_filesystem_on_a_different_mount_is_rejected() {
    let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
        .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to the primary Btrfs mount");
    let second = env::var_os("THINWS_LINUX_SECOND_BTRFS_MOUNT_ROOT")
        .expect("set THINWS_LINUX_SECOND_BTRFS_MOUNT_ROOT to a distinct Btrfs mount");
    let source_fixture = tempfile::Builder::new()
        .prefix("thinws-linux-primary-mount-")
        .tempdir_in(root)
        .unwrap();
    let target_fixture = tempfile::Builder::new()
        .prefix("thinws-linux-second-mount-")
        .tempdir_in(second)
        .unwrap();
    let source = source_fixture.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let request = MaterializationPathProbeRequest::new(
        absolute(&source),
        absolute(&target_fixture.path().join("target")),
        absolute(&target_fixture.path().join("staging")),
        absolute(&target_fixture.path().join("trash")),
    );
    let report = LinuxPlatformProbe
        .inspect_materialization_paths(&request)
        .unwrap();
    assert_eq!(report.source().filesystem().type_name(), "btrfs");
    assert_eq!(report.target_root().filesystem().type_name(), "btrfs");
    assert_eq!(
        report.source().filesystem().volume_id().known(),
        report.target_root().filesystem().volume_id().known()
    );
    assert_ne!(
        report.source().mount().mount_id(),
        report.target_root().mount().mount_id()
    );
    assert_eq!(report.cow_clone().state(), SupportState::Unsupported);
    assert_eq!(report.full_copy().state(), SupportState::Unsupported);
    assert!(
        report
            .cow_clone()
            .reasons()
            .iter()
            .any(|reason| reason == "different_mount")
    );
}
