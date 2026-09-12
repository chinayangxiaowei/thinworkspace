use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use tempfile::{TempDir, tempdir_in};
use thinws_p0_probe::{
    MaterializationPathProbeRequest, MaterializerCandidate, ProbeError, SupportState,
    inspect_materialization_paths, inspect_path, revalidate_path,
};

fn private_tmpdir() -> TempDir {
    tempdir_in(Path::new("/private/tmp")).expect("create controlled local-volume temp root")
}

fn materialization_report(source_mode: u32) -> thinws_p0_probe::MaterializationPathReport {
    let temp = private_tmpdir();
    let source = temp.path().join("source");
    let staging = temp.path().join("staging");
    let trash = temp.path().join("trash");
    fs::create_dir(&source).expect("create source");
    fs::create_dir(&staging).expect("create staging");
    fs::create_dir(&trash).expect("create trash");
    let target = temp.path().join("target");
    fs::set_permissions(&source, fs::Permissions::from_mode(source_mode))
        .expect("set controlled source permissions");
    let candidates = [
        MaterializerCandidate::ApfsFileClone,
        MaterializerCandidate::FullCopy,
    ];

    let result = inspect_materialization_paths(&MaterializationPathProbeRequest {
        source: &source,
        target_root: &target,
        staging: &staging,
        trash: &trash,
        candidates: &candidates,
    });
    fs::set_permissions(&source, fs::Permissions::from_mode(0o700))
        .expect("restore source permissions before assertions and cleanup");
    result.expect("materialization inspection should itself succeed")
}

fn candidate_state(
    report: &thinws_p0_probe::MaterializationPathReport,
    candidate: MaterializerCandidate,
) -> SupportState {
    report
        .candidates
        .iter()
        .find(|evidence| evidence.candidate == candidate)
        .expect("candidate evidence exists")
        .state
}

#[test]
fn revalidation_rejects_schema_or_experiment_mismatch() {
    let temp = private_tmpdir();
    let path = temp.path().join("directory");
    fs::create_dir(&path).expect("create controlled path");
    let report = inspect_path(&path).expect("inspect path");

    let mut wrong_schema = report.clone();
    wrong_schema.schema_version += 1;
    assert!(matches!(
        revalidate_path(&wrong_schema),
        Err(ProbeError::CorruptPathEvidence { .. })
    ));

    let mut wrong_experiment = report;
    wrong_experiment.experiment = "different-experiment".to_owned();
    assert!(matches!(
        revalidate_path(&wrong_experiment),
        Err(ProbeError::CorruptPathEvidence { .. })
    ));
}

#[test]
fn source_requires_read_and_search_but_not_write_preflight() {
    let unreadable = materialization_report(0o111);
    assert_ne!(
        candidate_state(&unreadable, MaterializerCandidate::ApfsFileClone),
        SupportState::Supported
    );
    assert_ne!(
        candidate_state(&unreadable, MaterializerCandidate::FullCopy),
        SupportState::Supported
    );

    let read_only = materialization_report(0o500);
    assert_eq!(
        candidate_state(&read_only, MaterializerCandidate::ApfsFileClone),
        SupportState::Supported
    );
    assert_eq!(
        candidate_state(&read_only, MaterializerCandidate::FullCopy),
        SupportState::Supported
    );
}
