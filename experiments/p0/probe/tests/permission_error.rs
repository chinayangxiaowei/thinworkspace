//! Permission errors must remain distinguishable from observed path replacement.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use thinws_p0_probe::{
    MaterializationPathProbeRequest, MaterializerCandidate, ProbeError, RevalidationChange,
    RevalidationStatus, SupportState, inspect_materialization_paths, inspect_path, revalidate_path,
};

#[test]
fn stable_unsearchable_directory_preserves_permission_errno() {
    // The system data volume honors ownership; the development volume may use noowners.
    let root = tempfile::tempdir_in("/private/tmp").expect("controlled temporary directory");
    let child = root.path().join("no-search");
    fs::create_dir(&child).expect("test child directory");
    fs::set_permissions(&child, fs::Permissions::from_mode(0o600))
        .expect("remove directory search permission");

    let result = inspect_path(&child);

    // Restore permissions before asserting so an expected RED still permits RAII cleanup.
    fs::set_permissions(&child, fs::Permissions::from_mode(0o700))
        .expect("restore test permissions");
    assert!(
        matches!(
            result,
            Err(ProbeError::SystemCall {
                errno: Some(libc::EACCES),
                ..
            })
        ),
        "a permission failure is not proof of a path race: {result:?}"
    );
}

#[test]
fn each_unwritable_destination_blocks_both_backend_preflights() {
    let root = tempfile::tempdir_in("/private/tmp").expect("ownership-enabled test root");
    let paths = ["source", "target", "staging", "trash"].map(|name| root.path().join(name));
    for path in &paths {
        fs::create_dir(path).expect("create controlled directory");
    }
    let candidates = [
        MaterializerCandidate::ApfsFileClone,
        MaterializerCandidate::FullCopy,
    ];
    for denied in &paths[1..] {
        fs::set_permissions(denied, fs::Permissions::from_mode(0o500))
            .expect("remove write access only");
        let result = inspect_materialization_paths(&MaterializationPathProbeRequest {
            source: &paths[0],
            target_root: &paths[1],
            staging: &paths[2],
            trash: &paths[3],
            candidates: &candidates,
        });
        fs::set_permissions(denied, fs::Permissions::from_mode(0o700))
            .expect("restore permissions before assertions");

        let report = result.expect("the directory remains searchable");
        assert_eq!(report.candidates.len(), 2);
        for candidate in report.candidates {
            assert_eq!(candidate.state, SupportState::Unsupported, "{denied:?}");
            assert!(
                candidate
                    .evidence
                    .iter()
                    .any(|fact| { fact.code == "destination_not_writable_at_inspection" })
            );
        }
    }
}

#[test]
fn access_changes_invalidate_evidence_without_replacing_the_directory() {
    let root = tempfile::tempdir_in("/private/tmp").expect("ownership-enabled test root");
    let path = root.path().join("target");
    fs::create_dir(&path).expect("create test directory");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("initial access");
    let before = inspect_path(&path).expect("initial observation");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o111))
        .expect("retain search but deny read and write");
    let result = revalidate_path(&before);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
        .expect("restore permissions before assertions");

    let checked = result.expect("searchable directory can be revalidated");
    assert_eq!(checked.status, RevalidationStatus::Stale);
    assert_eq!(
        checked.current.expect("current facts").ancestry,
        before.ancestry
    );
    assert!(
        checked
            .changes
            .iter()
            .any(|change| { matches!(change, RevalidationChange::ReadabilityChanged { .. }) })
    );
    assert!(
        checked
            .changes
            .iter()
            .any(|change| { matches!(change, RevalidationChange::WritabilityChanged { .. }) })
    );
}
