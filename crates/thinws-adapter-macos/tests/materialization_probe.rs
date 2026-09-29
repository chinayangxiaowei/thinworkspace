#![cfg(target_os = "macos")]

use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};

use tempfile::{Builder, TempDir};
use thinws_adapter_macos::MacOsHostAdapter;
use thinws_core::{
    AbsolutePath, FallbackPolicy, MaterializationPlan, MaterializationPlanError, PathResolution,
    SupportState,
};
use thinws_ports::{
    MaterializationPathProbeRequest, MaterializationPathRole, PlatformProbe, PortErrorKind,
};

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
fn combined_probe_proves_a_same_volume_cow_clone_plan_without_executing_it() {
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

    assert_eq!(report.cow_clone().state(), SupportState::Supported);
    assert_eq!(report.full_copy().state(), SupportState::Supported);
    assert_ne!(report.evidence_digest().as_bytes(), [0; 32]);
    let plan = MaterializationPlan::for_cow_clone(&report, FallbackPolicy::Deny).unwrap();
    assert_eq!(plan.source_volume_id(), plan.target_volume_id());
}

#[test]
fn combined_probe_identifies_source_that_lost_read_access_after_first_probe() {
    let temp = controlled_root("source-access-race-");
    let source = temp.path().join("source");
    let target = temp.path().join("target");
    let staging = temp.path().join("staging");
    let trash = temp.path().join("trash");
    for directory in [&source, &target, &staging, &trash] {
        fs::create_dir(directory).unwrap();
    }
    let adapter = probe();
    let first = adapter.inspect_path(&absolute(&source)).unwrap();
    assert_eq!(first.readability(), SupportState::Supported);
    fs::set_permissions(&source, fs::Permissions::from_mode(0o400)).unwrap();
    let result = adapter.inspect_materialization_paths(&MaterializationPathProbeRequest::new(
        absolute(&source),
        absolute(&target),
        absolute(&staging),
        absolute(&trash),
    ));
    let error = match result {
        Ok(_) => panic!("source without search permission must fail the combined probe"),
        Err(error) => error,
    };
    fs::set_permissions(&source, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(error.kind(), PortErrorKind::Unavailable);
    assert_eq!(
        error.materialization_path_role(),
        Some(MaterializationPathRole::Source)
    );
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
    assert_eq!(report.cow_clone().state(), SupportState::Supported);
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
fn path_probe_distinguishes_occupied_final_leaf_from_invalid_intermediate_component() {
    let temp = controlled_root("occupied-leaf-");
    let file = temp.path().join("file");
    fs::write(&file, b"keep").unwrap();
    let link = temp.path().join("link");
    symlink("file", &link).unwrap();
    let adapter = probe();

    for leaf in [&file, &link] {
        assert_eq!(
            adapter.inspect_path(&absolute(leaf)).unwrap_err().kind(),
            PortErrorKind::NotEmpty
        );
        assert_eq!(
            adapter
                .inspect_path(&absolute(&leaf.join("child")))
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );
    }
    assert_eq!(fs::read(&file).unwrap(), b"keep");
    assert_eq!(fs::read_link(&link).unwrap(), PathBuf::from("file"));
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

    assert_eq!(report.cow_clone().state(), SupportState::Unsupported);
    assert_eq!(report.full_copy().state(), SupportState::Unsupported);
    assert_eq!(
        MaterializationPlan::for_cow_clone(&report, FallbackPolicy::Deny),
        Err(MaterializationPlanError::CandidateUnsupported)
    );
}

#[test]
fn every_pair_of_materialization_roles_rejects_overlap() {
    for (left, right, expected_reason) in [
        (0, 1, "source_target_overlap"),
        (0, 2, "source_staging_overlap"),
        (0, 3, "source_trash_overlap"),
        (1, 2, "target_staging_overlap"),
        (1, 3, "target_trash_overlap"),
        (2, 3, "staging_trash_overlap"),
    ] {
        let temp = controlled_root("pairwise-overlap-");
        let mut paths = [
            temp.path().join("source"),
            temp.path().join("target"),
            temp.path().join("staging"),
            temp.path().join("trash"),
        ];
        for path in &paths {
            fs::create_dir(path).unwrap();
        }
        paths[right] = paths[left].clone();

        let report = probe()
            .inspect_materialization_paths(&MaterializationPathProbeRequest::new(
                absolute(&paths[0]),
                absolute(&paths[1]),
                absolute(&paths[2]),
                absolute(&paths[3]),
            ))
            .unwrap();

        assert_eq!(report.cow_clone().state(), SupportState::Unsupported);
        assert!(
            report
                .cow_clone()
                .reasons()
                .iter()
                .any(|reason| reason == expected_reason),
            "missing overlap reason {expected_reason}"
        );
    }
}

#[test]
#[ignore = "requires THINWS_P1_CROSS_VOLUME_ROOT on an APFS volume distinct from system temp"]
fn configured_real_cross_volume_report_is_unsupported_not_same_volume() {
    let cross_root = std::env::var_os("THINWS_P1_CROSS_VOLUME_ROOT")
        .expect("THINWS_P1_CROSS_VOLUME_ROOT must name the prepared APFS mount");
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
    assert_eq!(report.cow_clone().state(), SupportState::Unsupported);
    assert_eq!(report.full_copy().state(), SupportState::Supported);
    assert_eq!(
        MaterializationPlan::for_cow_clone(&report, FallbackPolicy::Deny),
        Err(MaterializationPlanError::DifferentVolume)
    );
    assert_eq!(
        MaterializationPlan::for_full_copy_after_preflight(
            &report,
            FallbackPolicy::AllowFullCopyOnCowUnsupported,
        ),
        Err(MaterializationPlanError::DifferentVolume)
    );
}
