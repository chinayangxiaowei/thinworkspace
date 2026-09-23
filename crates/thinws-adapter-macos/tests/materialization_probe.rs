use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use tempfile::{Builder, TempDir};
use thinws_adapter_macos::MacOsHostAdapter;
use thinws_core::{
    AbsolutePath, FallbackPolicy, MaterializationPlan, MaterializationPlanError, PathResolution,
    SupportState,
};
use thinws_ports::{MaterializationPathProbeRequest, PlatformProbe, PortErrorKind};

fn absolute(path: &Path) -> AbsolutePath {
    AbsolutePath::try_from_bytes(path.as_os_str().as_bytes().to_vec()).unwrap()
}

fn controlled_root(prefix: &str) -> TempDir {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-06-probe-tests");
    fs::create_dir_all(&root).unwrap();
    Builder::new()
        .prefix(prefix)
        .tempdir_in(fs::canonicalize(root).unwrap())
        .unwrap()
}

fn probe() -> MacOsHostAdapter {
    let bootstrap =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-06-probe-bootstrap");
    MacOsHostAdapter::new(
        fs::canonicalize(bootstrap.parent().unwrap())
            .unwrap()
            .join("p1-06-probe-bootstrap"),
    )
    .unwrap()
}

#[test]
fn combined_probe_proves_a_same_volume_apfs_clone_plan_without_executing_it() {
    let temp = controlled_root("same-volume-");
    let source = temp.path().join("source");
    let target = temp.path().join("target");
    let staging = temp.path().join("staging");
    let trash = temp.path().join("trash");
    for directory in [&source, &target, &staging, &trash] {
        fs::create_dir(directory).unwrap();
    }

    let report = probe()
        .inspect_materialization_paths(&MaterializationPathProbeRequest::new(
            absolute(&source),
            absolute(&target),
            absolute(&staging),
            absolute(&trash),
        ))
        .unwrap();

    assert_eq!(report.apfs_clone().state(), SupportState::Supported);
    assert_ne!(report.evidence_digest().as_bytes(), [0; 32]);
    let plan = MaterializationPlan::for_apfs_clone(&report, FallbackPolicy::Deny).unwrap();
    assert_eq!(plan.source_volume_id(), plan.target_volume_id());
}

#[test]
fn filesystem_root_is_a_valid_existing_probe_path() {
    let report = probe()
        .inspect_path(&AbsolutePath::try_from_bytes(b"/".to_vec()).unwrap())
        .unwrap();

    assert_eq!(report.resolution(), PathResolution::ExistingDirectory);
    assert_eq!(report.nearest_existing_ancestor().as_bytes(), b"/");
    assert!(report.missing_components().is_empty());
}

#[test]
fn missing_target_is_anchored_to_the_nearest_existing_parent_without_creation() {
    let temp = controlled_root("missing-target-");
    let requested = temp.path().join("absent").join("nested");

    let report = probe().inspect_path(&absolute(&requested)).unwrap();

    assert_eq!(report.resolution(), PathResolution::MissingTarget);
    assert_eq!(report.nearest_existing_ancestor(), &absolute(temp.path()));
    assert_eq!(
        report.missing_components(),
        &[b"absent".to_vec(), b"nested".to_vec()]
    );
    assert!(!requested.exists());
}

#[test]
fn missing_target_sibling_does_not_falsely_overlap_staging_or_trash() {
    let temp = controlled_root("missing-sibling-");
    let source = temp.path().join("source");
    let target = temp.path().join("future-target");
    let staging = temp.path().join("staging");
    let trash = temp.path().join("trash");
    for directory in [&source, &staging, &trash] {
        fs::create_dir(directory).unwrap();
    }

    let report = probe()
        .inspect_materialization_paths(&MaterializationPathProbeRequest::new(
            absolute(&source),
            absolute(&target),
            absolute(&staging),
            absolute(&trash),
        ))
        .unwrap();

    assert_eq!(
        report.target_root().resolution(),
        PathResolution::MissingTarget
    );
    assert_eq!(report.apfs_clone().state(), SupportState::Supported);
    assert!(!target.exists());
}

#[test]
fn path_probe_never_follows_an_intermediate_symlink() {
    let temp = controlled_root("symlink-");
    let real = temp.path().join("real");
    fs::create_dir(&real).unwrap();
    let link = temp.path().join("link");
    symlink(&real, &link).unwrap();

    let error = probe()
        .inspect_path(&absolute(&link.join("child")))
        .unwrap_err();
    assert_eq!(error.kind(), PortErrorKind::InvalidLayout);
}

#[test]
fn source_and_target_containment_disables_the_clone_candidate() {
    let temp = controlled_root("overlap-");
    let source = temp.path().join("source");
    let target = source.join("nested-target");
    let staging = temp.path().join("staging");
    let trash = temp.path().join("trash");
    for directory in [&source, &target, &staging, &trash] {
        fs::create_dir(directory).unwrap();
    }

    let report = probe()
        .inspect_materialization_paths(&MaterializationPathProbeRequest::new(
            absolute(&source),
            absolute(&target),
            absolute(&staging),
            absolute(&trash),
        ))
        .unwrap();

    assert_eq!(report.apfs_clone().state(), SupportState::Unsupported);
    assert_eq!(
        MaterializationPlan::for_apfs_clone(&report, FallbackPolicy::Deny),
        Err(MaterializationPlanError::CandidateUnsupported)
    );
}

#[test]
fn configured_real_cross_volume_report_is_unsupported_not_same_volume() {
    let Some(cross_root) = std::env::var_os("THINWS_P1_CROSS_VOLUME_ROOT") else {
        eprintln!("skipping: THINWS_P1_CROSS_VOLUME_ROOT is not set");
        return;
    };
    let source_parent = Builder::new()
        .prefix("p1-06-system-volume-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let source = source_parent.path().join("source");
    fs::create_dir(&source).unwrap();
    let target_parent = Builder::new()
        .prefix("p1-06-cross-")
        .tempdir_in(PathBuf::from(cross_root))
        .unwrap();
    let target = target_parent.path().join("target");
    let staging = target_parent.path().join("staging");
    let trash = target_parent.path().join("trash");
    for directory in [&target, &staging, &trash] {
        fs::create_dir(directory).unwrap();
    }

    let report = probe()
        .inspect_materialization_paths(&MaterializationPathProbeRequest::new(
            absolute(&source),
            absolute(&target),
            absolute(&staging),
            absolute(&trash),
        ))
        .unwrap();

    assert_ne!(
        report.source().filesystem().volume_id().known(),
        report.target_root().filesystem().volume_id().known()
    );
    assert_eq!(report.apfs_clone().state(), SupportState::Unsupported);
}
