use std::str::FromStr;

use thinws_core::{
    AbsolutePath, CandidateEvidence, CowEvidence, DirectoryIdentityEvidence, Evidence,
    FallbackPolicy, FallbackReason, FileIdentity, FileSystemIdentity,
    MaterializationAttemptEvidence, MaterializationFailureKind, MaterializationMode,
    MaterializationOutcome, MaterializationPathReport, MaterializationPlan,
    MaterializationPlanError, MaterializationReceipt, MaterializationReceiptError,
    MaterializerKind, MountEvidence, PathCapabilityReport, PathResolution, ProbeEvidenceDigest,
    RelativePath, RollbackEvidence, RollbackStatus, SupportState, TreeDigest, VolumeId,
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
    report_for_with_inode(requested, volume_id, clone_support, 2)
}

fn report_for_with_inode(
    requested: &str,
    volume_id: VolumeId,
    clone_support: SupportState,
    inode: u64,
) -> PathCapabilityReport {
    let requested = path(requested);
    PathCapabilityReport::new(
        requested.clone(),
        PathResolution::ExistingDirectory,
        requested.clone(),
        Vec::new(),
        vec![DirectoryIdentityEvidence::new(
            requested,
            FileIdentity::new(1, inode),
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
        CandidateEvidence::new(
            MaterializerKind::FullCopy,
            SupportState::Supported,
            Vec::new(),
        ),
        ProbeEvidenceDigest::new([9; 32]),
    )
}

fn combined_candidates(
    source_volume: VolumeId,
    target_volume: VolumeId,
    clone_state: SupportState,
    clone_reasons: &[&str],
    copy_state: SupportState,
) -> MaterializationPathReport {
    MaterializationPathReport::new(
        report_for("/Volumes/data/source", source_volume, clone_state),
        report_for(
            "/Volumes/data/workspaces/ws/root",
            target_volume,
            clone_state,
        ),
        report_for("/Volumes/data/staging", target_volume, clone_state),
        report_for("/Volumes/data/trash", target_volume, clone_state),
        CandidateEvidence::new(
            MaterializerKind::ApfsFileClone,
            clone_state,
            clone_reasons.iter().map(ToString::to_string).collect(),
        ),
        CandidateEvidence::new(MaterializerKind::FullCopy, copy_state, Vec::new()),
        ProbeEvidenceDigest::new([10; 32]),
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
    assert_eq!(
        MaterializationPlan::for_apfs_clone(
            &combined(registered, other, SupportState::Unsupported),
            FallbackPolicy::Deny,
        ),
        Err(MaterializationPlanError::DifferentVolume)
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
    assert_eq!(empty.source_volume_id(), Some(registered));
    assert_eq!(empty.target_volume_id(), Some(registered));
    assert_eq!(empty.elapsed_millis(), 4);

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
    assert_eq!(cloned.source_volume_id(), Some(registered));
    assert_eq!(cloned.target_volume_id(), Some(registered));
    assert_eq!(cloned.elapsed_millis(), 5);

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

    let mismatched_manifest = MaterializationReceipt::successful_apfs_clone(
        &plan,
        1,
        1,
        Vec::new(),
        TreeDigest::new([3; 32]),
        TreeDigest::new([4; 32]),
        5,
        12,
        None,
    );
    assert_eq!(
        mismatched_manifest,
        Err(MaterializationReceiptError::ManifestMismatch)
    );
}

#[test]
fn preflight_full_copy_requires_explicit_policy_and_only_clone_unsupported_evidence() {
    let registered = volume("1a42c888-32e3-489c-9bfa-67fd640a94e8");
    let eligible = combined_candidates(
        registered,
        registered,
        SupportState::Unsupported,
        &["clone_capability_unsupported"],
        SupportState::Supported,
    );

    let plan = MaterializationPlan::for_full_copy_after_preflight(
        &eligible,
        FallbackPolicy::AllowFullCopyOnCowUnsupported,
    )
    .unwrap();
    assert_eq!(plan.requested_mode(), MaterializationMode::CowClone);
    assert_eq!(plan.effective_mode(), MaterializationMode::FullCopy);
    assert_eq!(plan.selected_adapter(), MaterializerKind::FullCopy);
    assert_eq!(
        plan.fallback_reason(),
        Some(FallbackReason::CloneUnsupportedAtPreflight)
    );
    assert!(plan.failed_attempts().is_empty());

    assert_eq!(
        MaterializationPlan::for_full_copy_after_preflight(&eligible, FallbackPolicy::Deny),
        Err(MaterializationPlanError::FallbackDenied)
    );
    let unknown = combined_candidates(
        registered,
        registered,
        SupportState::Unknown,
        &["clone_capability_unknown"],
        SupportState::Supported,
    );
    assert_eq!(
        MaterializationPlan::for_full_copy_after_preflight(
            &unknown,
            FallbackPolicy::AllowFullCopyOnCowUnsupported,
        ),
        Err(MaterializationPlanError::FallbackNotEligible)
    );
    let wrong_copy_backend = MaterializationPathReport::new(
        eligible.source().clone(),
        eligible.target_root().clone(),
        eligible.staging().clone(),
        eligible.trash().clone(),
        eligible.apfs_clone().clone(),
        CandidateEvidence::new(
            MaterializerKind::ApfsFileClone,
            SupportState::Supported,
            Vec::new(),
        ),
        eligible.evidence_digest(),
    );
    assert_eq!(
        MaterializationPlan::for_full_copy_after_preflight(
            &wrong_copy_backend,
            FallbackPolicy::AllowFullCopyOnCowUnsupported,
        ),
        Err(MaterializationPlanError::WrongFullCopyCandidate)
    );
    let unsafe_reason = combined_candidates(
        registered,
        registered,
        SupportState::Unsupported,
        &["clone_capability_unsupported", "target_write_unsupported"],
        SupportState::Supported,
    );
    assert_eq!(
        MaterializationPlan::for_full_copy_after_preflight(
            &unsafe_reason,
            FallbackPolicy::AllowFullCopyOnCowUnsupported,
        ),
        Err(MaterializationPlanError::FallbackNotEligible)
    );
    let other = volume("c25d1051-142f-423e-bcd2-1e07daa4246e");
    assert_eq!(
        MaterializationPlan::for_full_copy_after_preflight(
            &combined_candidates(
                registered,
                other,
                SupportState::Unsupported,
                &["clone_capability_unsupported", "different_volume"],
                SupportState::Supported,
            ),
            FallbackPolicy::AllowFullCopyOnCowUnsupported,
        ),
        Err(MaterializationPlanError::DifferentVolume)
    );
}

#[test]
fn runtime_full_copy_requires_cow_unavailable_and_a_confirmed_clean_baseline() {
    let registered = volume("1a42c888-32e3-489c-9bfa-67fd640a94e8");
    let initial = combined_candidates(
        registered,
        registered,
        SupportState::Supported,
        &[],
        SupportState::Supported,
    );
    let prior_plan = MaterializationPlan::for_apfs_clone(
        &initial,
        FallbackPolicy::AllowFullCopyOnCowUnsupported,
    )
    .unwrap();
    let clean_failure = MaterializationReceipt::failed_apfs_clone(
        &prior_plan,
        MaterializationFailureKind::CowUnavailable,
        Vec::new(),
        false,
        RollbackEvidence::new(RollbackStatus::NotNeeded, Vec::new(), Vec::new()),
        MaterializationAttemptEvidence::new(
            None,
            None,
            None,
            0,
            Some(TreeDigest::new([5; 32])),
            None,
        ),
        7,
    );
    let fresh = combined_candidates(
        registered,
        registered,
        SupportState::Unknown,
        &["clone_capability_unknown"],
        SupportState::Supported,
    );

    let unconfirmed_staging =
        clean_failure
            .clone()
            .with_unconfirmed_staging(thinws_core::CreatedObjectEvidence::new(
                RelativePath::try_from_bytes(b".thinws-materialize-123-1".to_vec()).unwrap(),
                thinws_core::MaterializedEntryKind::RegularFile,
                None,
            ));
    assert_eq!(
        unconfirmed_staging.outcome(),
        MaterializationOutcome::Partial
    );
    assert_eq!(
        MaterializationPlan::for_full_copy_after_cow_unavailable(
            &fresh,
            &prior_plan,
            &unconfirmed_staging,
        ),
        Err(MaterializationPlanError::PreviousAttemptNotClean)
    );

    let fallback = MaterializationPlan::for_full_copy_after_cow_unavailable(
        &fresh,
        &prior_plan,
        &clean_failure,
    )
    .unwrap();
    assert_eq!(fallback.effective_mode(), MaterializationMode::FullCopy);
    assert_eq!(
        fallback.fallback_reason(),
        Some(FallbackReason::CloneUnavailableAtRuntime)
    );
    assert_eq!(fallback.failed_attempts().len(), 1);
    assert_eq!(
        fallback.failed_attempts()[0].failure_kind(),
        MaterializationFailureKind::CowUnavailable
    );
    assert_eq!(fallback.failed_attempts()[0].elapsed_millis(), 7);
    assert_eq!(
        fallback.failed_attempts()[0].probe_evidence_digest(),
        prior_plan.probe_evidence_digest()
    );

    let unverified_source = MaterializationReceipt::failed_apfs_clone(
        &prior_plan,
        MaterializationFailureKind::CowUnavailable,
        Vec::new(),
        false,
        RollbackEvidence::new(RollbackStatus::NotNeeded, Vec::new(), Vec::new()),
        MaterializationAttemptEvidence::default(),
        1,
    );
    assert_eq!(
        MaterializationPlan::for_full_copy_after_cow_unavailable(
            &fresh,
            &prior_plan,
            &unverified_source,
        ),
        Err(MaterializationPlanError::FallbackNotEligible)
    );

    for failure_kind in [
        MaterializationFailureKind::NoSpace,
        MaterializationFailureKind::Filesystem,
        MaterializationFailureKind::PlanStale,
        MaterializationFailureKind::SourceChanged,
        MaterializationFailureKind::TargetChanged,
    ] {
        let failed_clone = MaterializationReceipt::failed_apfs_clone(
            &prior_plan,
            failure_kind,
            Vec::new(),
            false,
            RollbackEvidence::new(RollbackStatus::NotNeeded, Vec::new(), Vec::new()),
            MaterializationAttemptEvidence::default(),
            1,
        );
        assert_eq!(
            MaterializationPlan::for_full_copy_after_cow_unavailable(
                &fresh,
                &prior_plan,
                &failed_clone,
            ),
            Err(MaterializationPlanError::FallbackNotEligible),
            "failure kind {failure_kind:?}"
        );
    }

    let incomplete = MaterializationReceipt::failed_apfs_clone(
        &prior_plan,
        MaterializationFailureKind::CowUnavailable,
        vec![thinws_core::CreatedObjectEvidence::new(
            RelativePath::try_from_bytes(b"partial".to_vec()).unwrap(),
            thinws_core::MaterializedEntryKind::RegularFile,
            Some(FileIdentity::new(1, 99)),
        )],
        true,
        RollbackEvidence::new(RollbackStatus::Incomplete, Vec::new(), Vec::new()),
        MaterializationAttemptEvidence::default(),
        2,
    );
    assert_eq!(
        MaterializationPlan::for_full_copy_after_cow_unavailable(&fresh, &prior_plan, &incomplete,),
        Err(MaterializationPlanError::PreviousAttemptNotClean)
    );

    let retained = thinws_core::CreatedObjectEvidence::new(
        RelativePath::try_from_bytes(b"retained".to_vec()).unwrap(),
        thinws_core::MaterializedEntryKind::RegularFile,
        Some(FileIdentity::new(1, 100)),
    );
    let retained_path = RelativePath::try_from_bytes(b"retained".to_vec()).unwrap();
    let bad_rollbacks = [
        RollbackEvidence::new(
            RollbackStatus::ConfirmedBaseline,
            Vec::new(),
            vec![retained.clone()],
        ),
        RollbackEvidence::new(RollbackStatus::ConfirmedBaseline, Vec::new(), Vec::new())
            .with_unconfirmed_quarantined(vec![retained_path.clone()]),
        RollbackEvidence::new(RollbackStatus::NotNeeded, Vec::new(), vec![retained]),
        RollbackEvidence::new(RollbackStatus::NotNeeded, Vec::new(), Vec::new())
            .with_unconfirmed_quarantined(vec![retained_path.clone()]),
        RollbackEvidence::new(
            RollbackStatus::NotNeeded,
            vec![retained_path.clone()],
            Vec::new(),
        ),
        RollbackEvidence::new(RollbackStatus::NotNeeded, Vec::new(), Vec::new())
            .with_quarantined(vec![retained_path]),
    ];
    for rollback in bad_rollbacks {
        let failed_clone = MaterializationReceipt::failed_apfs_clone(
            &prior_plan,
            MaterializationFailureKind::CowUnavailable,
            Vec::new(),
            false,
            rollback,
            MaterializationAttemptEvidence::new(
                None,
                None,
                None,
                0,
                Some(TreeDigest::new([5; 32])),
                None,
            ),
            1,
        );
        assert_eq!(
            MaterializationPlan::for_full_copy_after_cow_unavailable(
                &fresh,
                &prior_plan,
                &failed_clone,
            ),
            Err(MaterializationPlanError::PreviousAttemptNotClean)
        );
    }
    for (target_modified, clone_calls) in [(true, 0), (false, 1)] {
        let failed_clone = MaterializationReceipt::failed_apfs_clone(
            &prior_plan,
            MaterializationFailureKind::CowUnavailable,
            Vec::new(),
            target_modified,
            RollbackEvidence::new(RollbackStatus::NotNeeded, Vec::new(), Vec::new()),
            MaterializationAttemptEvidence::new(
                None,
                None,
                None,
                clone_calls,
                Some(TreeDigest::new([5; 32])),
                None,
            ),
            1,
        );
        assert_eq!(
            MaterializationPlan::for_full_copy_after_cow_unavailable(
                &fresh,
                &prior_plan,
                &failed_clone,
            ),
            Err(MaterializationPlanError::PreviousAttemptNotClean)
        );
    }

    let created = thinws_core::CreatedObjectEvidence::new(
        RelativePath::try_from_bytes(b"file.txt".to_vec()).unwrap(),
        thinws_core::MaterializedEntryKind::RegularFile,
        Some(FileIdentity::new(1, 88)),
    );
    let rolled_back = MaterializationReceipt::failed_apfs_clone(
        &prior_plan,
        MaterializationFailureKind::CowUnavailable,
        vec![created.clone()],
        true,
        RollbackEvidence::new(
            RollbackStatus::ConfirmedBaseline,
            vec![RelativePath::try_from_bytes(b"file.txt".to_vec()).unwrap()],
            Vec::new(),
        ),
        MaterializationAttemptEvidence::new(
            Some(12),
            None,
            Some(1),
            1,
            Some(TreeDigest::new([5; 32])),
            None,
        ),
        9,
    );
    let fallback =
        MaterializationPlan::for_full_copy_after_cow_unavailable(&fresh, &prior_plan, &rolled_back)
            .unwrap();
    assert_eq!(
        fallback.failed_attempts()[0].outcome(),
        MaterializationOutcome::Partial
    );
    assert_eq!(fallback.failed_attempts()[0].created(), &[created]);
    assert_eq!(fallback.failed_attempts()[0].clone_calls_succeeded(), 1);
    assert_eq!(
        fallback.failed_attempts()[0].source_volume_id(),
        Some(registered)
    );
    assert_eq!(
        fallback.failed_attempts()[0].target_volume_id(),
        Some(registered)
    );
    assert_eq!(
        fallback.failed_attempts()[0].evidence().logical_bytes(),
        Some(12)
    );
    assert_eq!(
        fallback.failed_attempts()[0]
            .evidence()
            .regular_file_count(),
        Some(1)
    );
    let failed_copy = MaterializationReceipt::failed_full_copy(
        &fallback,
        MaterializationFailureKind::NoSpace,
        Vec::new(),
        false,
        RollbackEvidence::new(RollbackStatus::NotNeeded, Vec::new(), Vec::new()),
        MaterializationAttemptEvidence::default(),
        3,
    );
    assert_eq!(
        failed_copy.failure_kind(),
        Some(MaterializationFailureKind::NoSpace)
    );
    assert_eq!(failed_copy.actual_adapter(), MaterializerKind::FullCopy);
    assert_eq!(failed_copy.cow_evidence(), CowEvidence::NotUsed);
    assert_eq!(failed_copy.failed_attempts(), fallback.failed_attempts());

    let denied_plan = MaterializationPlan::for_apfs_clone(&initial, FallbackPolicy::Deny).unwrap();
    let denied_receipt = MaterializationReceipt::failed_apfs_clone(
        &denied_plan,
        MaterializationFailureKind::CowUnavailable,
        Vec::new(),
        false,
        RollbackEvidence::new(RollbackStatus::NotNeeded, Vec::new(), Vec::new()),
        MaterializationAttemptEvidence::default(),
        1,
    );
    assert_eq!(
        MaterializationPlan::for_full_copy_after_cow_unavailable(
            &fresh,
            &denied_plan,
            &denied_receipt,
        ),
        Err(MaterializationPlanError::FallbackDenied)
    );

    let unavailable_copy = combined_candidates(
        registered,
        registered,
        SupportState::Unknown,
        &["clone_capability_unknown"],
        SupportState::Unknown,
    );
    assert_eq!(
        MaterializationPlan::for_full_copy_after_cow_unavailable(
            &unavailable_copy,
            &prior_plan,
            &clean_failure,
        ),
        Err(MaterializationPlanError::FullCopyUnavailable)
    );

    let changed_path = MaterializationPathReport::new(
        report_for(
            "/Volumes/data/other-source",
            registered,
            SupportState::Unknown,
        ),
        fresh.target_root().clone(),
        fresh.staging().clone(),
        fresh.trash().clone(),
        fresh.apfs_clone().clone(),
        fresh.full_copy().clone(),
        ProbeEvidenceDigest::new([11; 32]),
    );
    assert_eq!(
        MaterializationPlan::for_full_copy_after_cow_unavailable(
            &changed_path,
            &prior_plan,
            &clean_failure,
        ),
        Err(MaterializationPlanError::PreviousAttemptMismatch)
    );

    let replaced_target = MaterializationPathReport::new(
        fresh.source().clone(),
        report_for_with_inode(
            "/Volumes/data/workspaces/ws/root",
            registered,
            SupportState::Unknown,
            999,
        ),
        fresh.staging().clone(),
        fresh.trash().clone(),
        fresh.apfs_clone().clone(),
        fresh.full_copy().clone(),
        ProbeEvidenceDigest::new([12; 32]),
    );
    assert_eq!(
        MaterializationPlan::for_full_copy_after_cow_unavailable(
            &replaced_target,
            &prior_plan,
            &clean_failure,
        ),
        Err(MaterializationPlanError::PreviousAttemptMismatch)
    );

    let other_report = MaterializationPathReport::new(
        report_for(
            "/Volumes/data/other-source",
            registered,
            SupportState::Supported,
        ),
        fresh.target_root().clone(),
        fresh.staging().clone(),
        fresh.trash().clone(),
        fresh.apfs_clone().clone(),
        fresh.full_copy().clone(),
        ProbeEvidenceDigest::new([13; 32]),
    );
    let other_plan = MaterializationPlan::for_apfs_clone(
        &other_report,
        FallbackPolicy::AllowFullCopyOnCowUnsupported,
    )
    .unwrap();
    assert_eq!(
        MaterializationPlan::for_full_copy_after_cow_unavailable(
            &other_report,
            &other_plan,
            &clean_failure,
        ),
        Err(MaterializationPlanError::PreviousAttemptMismatch)
    );
}

#[test]
fn full_copy_receipt_preserves_fallback_facts_without_claiming_cow() {
    let registered = volume("1a42c888-32e3-489c-9bfa-67fd640a94e8");
    let report = combined_candidates(
        registered,
        registered,
        SupportState::Unsupported,
        &["clone_capability_unsupported"],
        SupportState::Supported,
    );
    let plan = MaterializationPlan::for_full_copy_after_preflight(
        &report,
        FallbackPolicy::AllowFullCopyOnCowUnsupported,
    )
    .unwrap();
    let digest = TreeDigest::new([7; 32]);

    assert_eq!(
        MaterializationReceipt::successful_apfs_clone(
            &plan,
            1,
            1,
            Vec::new(),
            digest,
            digest,
            0,
            0,
            None,
        ),
        Err(MaterializationReceiptError::AdapterMismatch)
    );

    let receipt = MaterializationReceipt::successful_full_copy(
        &plan,
        2,
        Vec::new(),
        digest,
        digest,
        8,
        16,
        Some(16),
    )
    .unwrap();
    assert_eq!(receipt.requested_mode(), MaterializationMode::CowClone);
    assert_eq!(receipt.effective_mode(), MaterializationMode::FullCopy);
    assert_eq!(receipt.actual_mode(), MaterializationMode::FullCopy);
    assert_eq!(receipt.actual_adapter(), MaterializerKind::FullCopy);
    assert_eq!(receipt.cow_evidence(), CowEvidence::NotUsed);
    assert_eq!(receipt.clone_calls_succeeded(), 0);
    assert_eq!(
        receipt.probe_evidence_digest(),
        plan.probe_evidence_digest()
    );
    assert_eq!(
        receipt.fallback_reason(),
        Some(FallbackReason::CloneUnsupportedAtPreflight)
    );
    assert!(receipt.failed_attempts().is_empty());

    for (created, target_modified) in [
        (Vec::new(), true),
        (
            vec![thinws_core::CreatedObjectEvidence::new(
                RelativePath::try_from_bytes(b"created".to_vec()).unwrap(),
                thinws_core::MaterializedEntryKind::RegularFile,
                Some(FileIdentity::new(1, 101)),
            )],
            false,
        ),
    ] {
        let failed = MaterializationReceipt::failed_full_copy(
            &plan,
            MaterializationFailureKind::Filesystem,
            created,
            target_modified,
            RollbackEvidence::new(RollbackStatus::Incomplete, Vec::new(), Vec::new()),
            MaterializationAttemptEvidence::default(),
            1,
        );
        assert_eq!(failed.outcome(), MaterializationOutcome::Partial);
    }

    assert_eq!(
        MaterializationReceipt::successful_full_copy(
            &plan,
            2,
            Vec::new(),
            TreeDigest::new([7; 32]),
            TreeDigest::new([8; 32]),
            8,
            16,
            Some(16),
        ),
        Err(MaterializationReceiptError::ManifestMismatch)
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
