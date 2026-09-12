use std::env;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Write;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::symlink;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use tempfile::{TempDir, tempdir_in};
use thinws_p0_probe::{
    AccessPreflight, CandidateExecution, CowEvidence, Evidence, ExecutionGuarantee, FileType,
    MaterializationPathProbeRequest, MaterializerCandidate, MountFlag, MountWriteState,
    PathResolution, ProbeError, ProductPolicyDecision, SupportAssurance, SupportState,
    VolumeRelation, WriteAttempt, inspect_host, inspect_materialization_paths, inspect_path,
    validate_probe_path,
};

fn worktree_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("the experiment crate is nested under the worktree")
        .canonicalize()
        .expect("the current worktree root exists")
}

fn apfs_tempdir() -> TempDir {
    tempdir_in(worktree_root()).expect("the controlled APFS test root is writable")
}

fn known_volume_uuid(path: &Path) -> String {
    match inspect_path(path)
        .expect("path probe should succeed")
        .filesystem
        .volume_uuid
    {
        Evidence::Known { value, .. } => value,
        Evidence::Unknown { reason, errno } => {
            panic!("expected Volume UUID, got unknown: {reason}; errno={errno:?}")
        }
    }
}

fn diskutil_volume_uuid(path: &Path) -> String {
    let stat = Command::new("/usr/bin/stat")
        .args(["-f", "%Sd"])
        .arg(path)
        .output()
        .expect("query BSD device for actual test path");
    assert!(
        stat.status.success(),
        "stat failed: {}",
        String::from_utf8_lossy(&stat.stderr)
    );
    let device = String::from_utf8(stat.stdout)
        .expect("BSD device is UTF-8")
        .trim()
        .to_owned();
    let diskutil = Command::new("/usr/sbin/diskutil")
        .args(["info", "-plist"])
        .arg(device)
        .output()
        .expect("run independent diskutil query");
    assert!(
        diskutil.status.success(),
        "diskutil failed: {}",
        String::from_utf8_lossy(&diskutil.stderr)
    );

    let mut plutil = Command::new("/usr/bin/plutil")
        .args(["-extract", "VolumeUUID", "raw", "-o", "-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start plutil");
    plutil
        .stdin
        .take()
        .expect("plutil stdin is piped")
        .write_all(&diskutil.stdout)
        .expect("send plist to plutil");
    let output = plutil.wait_with_output().expect("wait for plutil");
    assert!(
        output.status.success(),
        "plutil failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("diskutil UUID is UTF-8")
        .trim()
        .to_owned()
}

fn candidate(
    report: &thinws_p0_probe::MaterializationPathReport,
    requested: MaterializerCandidate,
) -> &thinws_p0_probe::CandidateEvidence {
    report
        .candidates
        .iter()
        .find(|evidence| evidence.candidate == requested)
        .expect("requested candidate is present")
}

#[test]
fn host_report_is_measured_and_never_confirms_cow() {
    let report = inspect_host().expect("host inspection should succeed on macOS");
    let sw_vers = Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .expect("query the platform version independently");
    assert!(sw_vers.status.success());
    let expected_product_version = String::from_utf8(sw_vers.stdout)
        .expect("sw_vers product version is UTF-8")
        .trim()
        .to_owned();

    assert_eq!(report.operating_system, "macos");
    assert_eq!(report.product_version, expected_product_version);
    assert!(!report.kernel_release.is_empty());
    assert!(!report.architecture.is_empty());
    assert_eq!(report.cow_evidence, CowEvidence::NotExecutedByProbe);
}

#[test]
fn apfs_volume_uuid_is_stable_and_matches_independent_system_query() {
    let first = apfs_tempdir();
    let second = apfs_tempdir();

    let first_report = inspect_path(first.path()).expect("inspect first APFS directory");
    let second_report = inspect_path(second.path()).expect("inspect second APFS directory");
    assert_eq!(first_report.filesystem.type_name, "apfs");
    assert_eq!(second_report.filesystem.type_name, "apfs");
    assert_eq!(
        known_volume_uuid(first.path()),
        known_volume_uuid(second.path())
    );
    assert_eq!(
        known_volume_uuid(first.path()),
        diskutil_volume_uuid(first.path())
    );
    assert_eq!(
        first_report.filesystem.clone_capability.state,
        SupportState::Supported
    );
}

#[test]
fn path_identity_comes_from_existing_directory_and_is_stable() {
    let temp = apfs_tempdir();
    let directory = temp.path().join("identity");
    fs::create_dir(&directory).expect("create controlled directory");

    let first = inspect_path(&directory).expect("inspect directory");
    let second = inspect_path(&directory).expect("inspect directory again");

    assert_eq!(first.resolution, PathResolution::ExistingDirectory);
    assert_eq!(first.ancestry.last(), second.ancestry.last());
    assert_ne!(
        first.ancestry.last().expect("leaf evidence").identity.inode,
        0
    );
}

#[test]
fn missing_target_uses_nearest_existing_ancestor_without_creation() {
    let temp = apfs_tempdir();
    let existing = temp.path().join("existing");
    fs::create_dir(&existing).expect("create controlled ancestor");
    let missing = existing.join("first").join("second");

    let report = inspect_path(&missing).expect("missing target inspection should succeed");

    assert_eq!(report.resolution, PathResolution::MissingTarget);
    assert_eq!(
        report.nearest_existing_ancestor.display,
        existing.display().to_string()
    );
    assert_eq!(
        report
            .missing_components
            .iter()
            .map(|component| component.display.as_str())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
    assert_eq!(report.cow_evidence, CowEvidence::NotExecutedByProbe);
    assert!(
        !missing.exists(),
        "a read-only probe must not create the target"
    );
}

#[test]
fn path_validation_rejects_relative_parent_and_embedded_nul() {
    assert_eq!(
        validate_probe_path(Path::new("relative")),
        Err(ProbeError::PathMustBeAbsolute)
    );
    assert_eq!(
        validate_probe_path(Path::new("/safe/../escape")),
        Err(ProbeError::ParentTraversal)
    );

    let nul_path = PathBuf::from(OsString::from_vec(b"/safe/nul\0byte".to_vec()));
    assert_eq!(
        validate_probe_path(&nul_path),
        Err(ProbeError::EmbeddedNul { component_index: 1 })
    );
}

#[test]
fn inspect_path_rejects_leaf_intermediate_and_dangling_symlinks() {
    let temp = apfs_tempdir();
    let real = temp.path().join("real");
    fs::create_dir(&real).expect("create symlink target");

    let leaf_link = temp.path().join("leaf-link");
    symlink(&real, &leaf_link).expect("create leaf symlink");
    assert!(matches!(
        inspect_path(&leaf_link),
        Err(ProbeError::SymbolicLinkEncountered { .. })
    ));

    let intermediate = temp.path().join("intermediate");
    symlink(&real, &intermediate).expect("create intermediate symlink");
    assert!(matches!(
        inspect_path(&intermediate.join("child")),
        Err(ProbeError::SymbolicLinkEncountered { .. })
    ));

    let dangling = temp.path().join("dangling");
    symlink(temp.path().join("absent"), &dangling).expect("create dangling symlink");
    assert!(matches!(
        inspect_path(&dangling.join("child")),
        Err(ProbeError::SymbolicLinkEncountered { .. })
    ));
}

#[test]
fn inspect_path_rejects_regular_files_and_special_files() {
    let temp = apfs_tempdir();
    let regular = temp.path().join("regular");
    File::create(&regular).expect("create controlled regular file");
    assert!(matches!(
        inspect_path(&regular),
        Err(ProbeError::UnsupportedFileType {
            file_type: FileType::RegularFile,
            ..
        })
    ));

    let socket_temp = tempdir_in(Path::new("/private/tmp"))
        .expect("create a controlled short root for the Unix socket path");
    let socket = socket_temp.path().join("socket");
    let _listener = UnixListener::bind(&socket).expect("create controlled Unix socket");
    assert!(matches!(
        inspect_path(&socket),
        Err(ProbeError::UnsupportedFileType {
            file_type: FileType::Socket,
            ..
        })
    ));
}

#[test]
fn writability_is_preflight_evidence_not_an_execution_guarantee() {
    let temp = apfs_tempdir();
    let report = inspect_path(temp.path()).expect("inspect writable controlled root");

    assert_eq!(
        report.writability.mount_state,
        MountWriteState::WritableAtInspection
    );
    assert!(matches!(
        report.writability.effective_access_preflight,
        AccessPreflight::Allowed { .. }
    ));
    assert_eq!(
        report.writability.execution_guarantee,
        ExecutionGuarantee::NotEstablishedByReadOnlyProbe
    );
    assert_eq!(
        report.writability.write_attempt,
        WriteAttempt::NotPerformedReadOnlyProbe
    );

    let root = inspect_path(Path::new("/")).expect("inspect sealed system root");
    assert_eq!(root.writability.mount_state, MountWriteState::ReadOnly);
    assert!(root.mount.recognized_flags.contains(&MountFlag::ReadOnly));
}

#[test]
fn same_volume_materialization_report_is_structured_but_only_preflight() {
    let temp = apfs_tempdir();
    let source = temp.path().join("source");
    let staging = temp.path().join("staging");
    let trash = temp.path().join("trash");
    fs::create_dir(&source).expect("create source");
    fs::create_dir(&staging).expect("create staging");
    fs::create_dir(&trash).expect("create trash");
    let target = temp.path().join("target");
    let candidates = [
        MaterializerCandidate::ApfsFileClone,
        MaterializerCandidate::FullCopy,
    ];

    let report = inspect_materialization_paths(&MaterializationPathProbeRequest {
        source: &source,
        target_root: &target,
        staging: &staging,
        trash: &trash,
        candidates: &candidates,
    })
    .expect("inspect same-volume materialization paths");

    assert_eq!(report.pairs.len(), 6);
    assert!(
        report
            .pairs
            .iter()
            .all(|pair| pair.relation == VolumeRelation::SameVolume)
    );
    let clone = candidate(&report, MaterializerCandidate::ApfsFileClone);
    assert_eq!(clone.state, SupportState::Supported);
    assert_eq!(clone.assurance, SupportAssurance::PreflightOnly);
    assert_eq!(clone.execution, CandidateExecution::NotAttemptedByProbe);
    assert_eq!(
        clone.policy_decision,
        ProductPolicyDecision::NotEvaluatedByPlatformProbe
    );
    assert_eq!(clone.cow_evidence, CowEvidence::NotExecutedByProbe);
    assert_eq!(report.target_root.resolution, PathResolution::MissingTarget);
    assert!(!target.exists());

    let json = serde_json::to_string_pretty(&report).expect("serialize experiment JSON");
    assert!(json.contains("not_executed_by_probe"));
    assert!(!json.contains("confirmed"));
}

#[test]
fn non_apfs_path_does_not_claim_clone_support() {
    let report = inspect_path(Path::new("/dev")).expect("inspect devfs directory");
    let clone = report
        .candidate_path_evidence
        .iter()
        .find(|evidence| evidence.candidate == MaterializerCandidate::ApfsFileClone)
        .expect("APFS path evidence exists");

    assert_ne!(report.filesystem.type_name, "apfs");
    assert_eq!(clone.state, SupportState::Unsupported);
    assert_eq!(clone.assurance, SupportAssurance::PreflightOnly);
    assert_eq!(clone.cow_evidence, CowEvidence::NotExecutedByProbe);
    assert_eq!(
        report.filesystem.clone_capability.state,
        SupportState::Unknown
    );
}

#[test]
#[ignore = "set THINWS_P0_CROSS_VOLUME_ROOT to a writable directory on a distinct real volume"]
fn configured_real_cross_volume_distinguishes_clone_from_full_copy() {
    let other_root = env::var_os("THINWS_P0_CROSS_VOLUME_ROOT")
        .expect("THINWS_P0_CROSS_VOLUME_ROOT is required for the ignored test");
    let other_root = PathBuf::from(other_root)
        .canonicalize()
        .expect("cross-volume root must already exist");
    let source_root = apfs_tempdir();
    let destination_root =
        tempdir_in(&other_root).expect("create controlled cross-volume temp root");
    let source = source_root.path().join("source");
    fs::create_dir(&source).expect("create source");
    let staging = destination_root.path().join("staging");
    let trash = destination_root.path().join("trash");
    fs::create_dir(&staging).expect("create staging");
    fs::create_dir(&trash).expect("create trash");
    let target = destination_root.path().join("target");
    let candidates = [
        MaterializerCandidate::ApfsFileClone,
        MaterializerCandidate::FullCopy,
    ];

    let report = inspect_materialization_paths(&MaterializationPathProbeRequest {
        source: &source,
        target_root: &target,
        staging: &staging,
        trash: &trash,
        candidates: &candidates,
    })
    .expect("inspect configured real cross-volume paths");

    assert_ne!(
        known_volume_uuid(&source),
        known_volume_uuid(destination_root.path()),
        "test setup must use distinct volumes"
    );
    assert_eq!(
        candidate(&report, MaterializerCandidate::ApfsFileClone).state,
        SupportState::Unsupported
    );
    let full_copy = candidate(&report, MaterializerCandidate::FullCopy);
    assert_eq!(full_copy.state, SupportState::Supported);
    assert_eq!(full_copy.assurance, SupportAssurance::PreflightOnly);
    assert_eq!(
        full_copy.policy_decision,
        ProductPolicyDecision::NotEvaluatedByPlatformProbe
    );
    assert_eq!(full_copy.cow_evidence, CowEvidence::NotApplicable);
    assert!(
        report
            .pairs
            .iter()
            .any(|pair| pair.relation == VolumeRelation::DifferentVolume)
    );
}
