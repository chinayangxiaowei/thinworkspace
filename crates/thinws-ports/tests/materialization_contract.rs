use std::error::Error;

use thinws_core::{
    AbsolutePath, CandidateEvidence, CowEvidence, DirectoryIdentityEvidence, Evidence,
    FallbackPolicy, FileIdentity, FileSystemIdentity, MaterializationFailureKind,
    MaterializationOutcome, MaterializationPathReport, MaterializationPlan, MaterializationReceipt,
    MaterializerKind, MountEvidence, PathCapabilityReport, PathResolution, ProbeEvidenceDigest,
    RollbackEvidence, RollbackStatus, SupportState, VolumeId,
};
use thinws_ports::{MaterializationFailure, PortError, PortErrorKind};

fn plan() -> MaterializationPlan {
    let volume: VolumeId = "1a42c888-32e3-489c-9bfa-67fd640a94e8".parse().unwrap();
    let path = AbsolutePath::try_from_bytes(b"/Volumes/data/value".to_vec()).unwrap();
    let evidence = PathCapabilityReport::new(
        path.clone(),
        PathResolution::ExistingDirectory,
        path.clone(),
        Vec::new(),
        vec![DirectoryIdentityEvidence::new(
            path,
            FileIdentity::new(1, 2),
        )],
        FileSystemIdentity::new("apfs", [3, 4], Evidence::Known(volume)),
        MountEvidence::new(0, true),
        SupportState::Supported,
        SupportState::Supported,
        SupportState::Supported,
    )
    .unwrap();
    MaterializationPlan::for_apfs_clone(
        &MaterializationPathReport::new(
            evidence.clone(),
            evidence.clone(),
            evidence.clone(),
            evidence,
            CandidateEvidence::new(
                MaterializerKind::ApfsFileClone,
                SupportState::Supported,
                Vec::new(),
            ),
            CandidateEvidence::new(
                MaterializerKind::FullCopy,
                SupportState::Supported,
                Vec::new(),
            ),
            ProbeEvidenceDigest::new([1; 32]),
        ),
        FallbackPolicy::Deny,
    )
    .unwrap()
}

#[test]
fn materialization_failure_preserves_typed_error_and_partial_receipt() {
    let receipt = MaterializationReceipt::failed_apfs_clone(
        &plan(),
        MaterializationFailureKind::SourceChanged,
        Vec::new(),
        false,
        RollbackEvidence::new(RollbackStatus::ConfirmedBaseline, Vec::new(), Vec::new()),
        Default::default(),
        7,
    );
    let failure = MaterializationFailure::new(
        PortError::new(PortErrorKind::Io, "revalidate source"),
        receipt,
    );

    assert_eq!(failure.error().kind(), PortErrorKind::Io);
    assert_eq!(failure.receipt().outcome(), MaterializationOutcome::Failed);
    assert_eq!(failure.receipt().cow_evidence(), CowEvidence::Unknown);
    assert_eq!(
        failure.receipt().failure_kind(),
        Some(MaterializationFailureKind::SourceChanged)
    );
    assert_eq!(
        failure.receipt().rollback().status(),
        RollbackStatus::ConfirmedBaseline
    );
    assert_eq!(failure.to_string(), "revalidate source failed (Io)");
    let debug = format!("{failure:?}");
    assert!(debug.contains("MaterializationFailure"));
    assert!(debug.contains("revalidate source"));
    assert!(debug.contains("receipt"));
    assert!(failure.source().is_some());
}
