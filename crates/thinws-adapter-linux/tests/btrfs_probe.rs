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
    assert_ne!(report.source().filesystem().type_name(), "btrfs");
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
    assert_eq!(report.cow_clone().kind(), MaterializerKind::BtrfsReflink);
    assert_ne!(report.cow_clone().state(), SupportState::Unsupported);
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
