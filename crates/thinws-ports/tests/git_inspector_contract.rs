use std::io;
use std::num::NonZeroUsize;
use std::path::PathBuf;

use thinws_core::{DiscoveryCompleteness, GitState, RepositoryState};
use thinws_ports::{
    CleanupIoFailure, CleanupOperation, DirectChildExit, GitInspection, GitInspectionIssue,
    GitQueryFailureKind, RepositoryInspection,
};

#[test]
fn complete_empty_discovery_is_not_applicable() {
    let report = GitInspection::new(DiscoveryCompleteness::Complete, vec![], vec![]);
    assert_eq!(report.aggregate(), GitState::NotApplicable);
    assert!(report.repositories().is_empty());
}

#[test]
fn incomplete_discovery_cannot_look_clean() {
    let report = GitInspection::new(
        DiscoveryCompleteness::Incomplete,
        vec![RepositoryInspection::new(
            PathBuf::new(),
            RepositoryState::Clean,
            vec![],
        )],
        vec![GitInspectionIssue::ScanFailed],
    );
    assert_eq!(report.aggregate(), GitState::Unknown);
    assert_eq!(report.issues(), &[GitInspectionIssue::ScanFailed]);
}

#[test]
fn unknown_repository_preserves_exact_query_and_cleanup_evidence() {
    let issue = GitInspectionIssue::QueryFailed {
        kind: GitQueryFailureKind::TimedOut,
        direct_child_exit: DirectChildExit::Unconfirmed,
        cleanup_io: Some(CleanupIoFailure {
            operation: CleanupOperation::ConfirmPoll,
            error_kind: io::ErrorKind::Interrupted,
            raw_os_error: Some(4),
        }),
    };
    let repository = RepositoryInspection::new(
        PathBuf::from("nested"),
        RepositoryState::Dirty {
            tracked_changes: NonZeroUsize::new(1).expect("static nonzero count"),
        },
        vec![issue],
    );
    assert_eq!(repository.state(), RepositoryState::Unknown);
    assert_eq!(repository.issues(), &[issue]);
    let report = GitInspection::new(DiscoveryCompleteness::Complete, vec![repository], vec![]);
    assert_eq!(report.aggregate(), GitState::Unknown);
    assert_eq!(
        report.repositories()[0].relative_path(),
        &PathBuf::from("nested")
    );
}
