use std::str::FromStr;

use thinws_core::{
    AbsolutePath, CandidateEvidence, CowEvidence, DirectoryIdentityEvidence, Evidence,
    FallbackPolicy, FileIdentity, FileSystemIdentity, MaterializationAttemptEvidence,
    MaterializationFailureKind, MaterializationMode, MaterializationOutcome,
    MaterializationPathReport, MaterializationPlan, MaterializationPlanError,
    MaterializationReceipt, MaterializerKind, MountEvidence, PathCapabilityReport, PathResolution,
    ProbeEvidenceDigest, RelativePath, RollbackEvidence, RollbackStatus, SupportState, TreeDigest,
    VolumeId,
};

fn path(value: &str) -> AbsolutePath {
    AbsolutePath::try_from_bytes(value.as_bytes().to_vec()).unwrap()
}

fn volume(value: &str) -> VolumeId {
    VolumeId::from_str(value).unwrap()
}

fn report_for(
    requested: &str,
    volume_id: VolumeId,
    clone_support: SupportState,
) -> PathCapabilityReport {
    let requested = path(requested);
    PathCapabilityReport::new(
        requested.clone(),
        PathResolution::ExistingDirectory,
        requested.clone(),
        Vec::new(),
        vec![DirectoryIdentityEvidence::new(
            requested,
            FileIdentity::new(1, 2),
        )],
        FileSystemIdentity::new("apfs", [7, 8], Evidence::Known(volume_id)),
        MountEvidence::new(0, true),
        SupportState::Supported,
        SupportState::Supported,
        clone_support,
    )
    .unwrap()
}

fn combined(
    source_volume: VolumeId,
    target_volume: VolumeId,
    candidate_state: SupportState,
) -> MaterializationPathReport {
    MaterializationPathReport::new(
        report_for("/Volumes/data/source", source_volume, candidate_state),
        report_for(
            "/Volumes/data/workspaces/ws/root",
            target_volume,
            candidate_state,
        ),
        report_for("/Volumes/data/staging", target_volume, candidate_state),
        report_for("/Volumes/data/trash", target_volume, candidate_state),
        CandidateEvidence::new(MaterializerKind::ApfsFileClone, candidate_state, Vec::new()),
        ProbeEvidenceDigest::new([9; 32]),
    )
}

#[test]
fn apfs_plan_requires_one_same_known_volume_and_keeps_the_probe_digest() {
    let registered = volume("1a42c888-32e3-489c-9bfa-67fd640a94e8");
    let report = combined(registered, registered, SupportState::Supported);

    let plan = MaterializationPlan::for_apfs_clone(&report, FallbackPolicy::Deny).unwrap();
    assert_eq!(plan.requested_mode(), MaterializationMode::CowClone);
    assert_eq!(plan.effective_mode(), MaterializationMode::CowClone);
    assert_eq!(plan.selected_adapter(), MaterializerKind::ApfsFileClone);
    assert_eq!(
        plan.probe_evidence_digest(),
        ProbeEvidenceDigest::new([9; 32])
    );
    assert_eq!(plan.source_volume_id(), registered);
    assert_eq!(plan.target_volume_id(), registered);

    let other = volume("c25d1051-142f-423e-bcd2-1e07daa4246e");
    assert_eq!(
        MaterializationPlan::for_apfs_clone(
            &combined(registered, other, SupportState::Supported),
            FallbackPolicy::Deny,
        ),
        Err(MaterializationPlanError::DifferentVolume)
    );
    assert_eq!(
        MaterializationPlan::for_apfs_clone(
            &combined(registered, registered, SupportState::Unsupported),
            FallbackPolicy::Deny,
        ),
        Err(MaterializationPlanError::CandidateUnsupported)
    );

    let unknown = MaterializationPlan::for_apfs_clone(
        &combined(registered, registered, SupportState::Unknown),
        FallbackPolicy::Deny,
    )
    .unwrap();
    assert_eq!(unknown.selected_adapter(), MaterializerKind::ApfsFileClone);
}

#[test]
fn successful_clone_receipt_cannot_claim_cow_without_a_real_regular_file_clone() {
    let registered = volume("1a42c888-32e3-489c-9bfa-67fd640a94e8");
    let report = combined(registered, registered, SupportState::Supported);
    let plan = MaterializationPlan::for_apfs_clone(&report, FallbackPolicy::Deny).unwrap();
    let digest = TreeDigest::new([3; 32]);

    let empty = MaterializationReceipt::successful_apfs_clone(
        &plan,
        0,
        0,
        Vec::new(),
        digest,
        digest,
        4,
        0,
        Some(0),
    )
    .unwrap();
    assert_eq!(empty.cow_evidence(), CowEvidence::NotUsed);

    let cloned = MaterializationReceipt::successful_apfs_clone(
        &plan,
        2,
        2,
        Vec::new(),
        digest,
        digest,
        5,
        12,
        None,
    )
    .unwrap();
    assert_eq!(cloned.cow_evidence(), CowEvidence::Confirmed);

    assert!(
        MaterializationReceipt::successful_apfs_clone(
            &plan,
            2,
            1,
            Vec::new(),
            digest,
            digest,
            5,
            12,
            None,
        )
        .is_err()
    );
}

#[test]
fn relative_path_preserves_arbitrary_non_utf8_bytes_without_normalizing_them() {
    let raw = vec![b'd', b'i', b'r', b'/', 0xff, b'-', 0xfe];
    let path = RelativePath::try_from_bytes(raw.clone()).unwrap();

    assert_eq!(path.as_bytes(), raw);
}

#[test]
fn failed_receipt_distinguishes_a_modified_target_root_from_a_prewrite_failure() {
    let registered = volume("1a42c888-32e3-489c-9bfa-67fd640a94e8");
    let report = combined(registered, registered, SupportState::Supported);
    let plan = MaterializationPlan::for_apfs_clone(&report, FallbackPolicy::Deny).unwrap();
    let rollback = RollbackEvidence::new(RollbackStatus::ConfirmedBaseline, Vec::new(), Vec::new());

    let receipt = MaterializationReceipt::failed_apfs_clone(
        &plan,
        MaterializationFailureKind::Filesystem,
        Vec::new(),
        true,
        rollback,
        MaterializationAttemptEvidence::new(
            Some(12),
            Some(8),
            Some(1),
            1,
            Some(TreeDigest::new([3; 32])),
            Some(TreeDigest::new([4; 32])),
        ),
        1,
    );

    assert_eq!(receipt.outcome(), MaterializationOutcome::Partial);
    assert_eq!(receipt.logical_bytes(), Some(12));
    assert_eq!(receipt.physical_bytes(), Some(8));
    assert_eq!(receipt.regular_file_count(), Some(1));
    assert_eq!(receipt.clone_calls_succeeded(), 1);
    assert_eq!(
        receipt.source_manifest_digest(),
        Some(TreeDigest::new([3; 32]))
    );
    assert_eq!(
        receipt.target_manifest_digest(),
        Some(TreeDigest::new([4; 32]))
    );
}
