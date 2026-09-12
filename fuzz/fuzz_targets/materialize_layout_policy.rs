#![no_main]

use std::collections::BTreeSet;

use libfuzzer_sys::fuzz_target;
use thinws_p0_probe::{
    AccessPreflight, CandidateEvidence, CandidateExecution, CloneCapabilityEvidence, CowEvidence,
    DirectoryIdentityEvidence, EncodedPath, Evidence, EvidenceSource, ExecutionGuarantee,
    FileIdentity, FileSystemIdentity, MaterializationPathReport, MaterializerCandidate,
    MountEvidence, MountWriteState, PathCapabilityReport, PathPairEvidence, PathResolution,
    PathRole, ProductPolicyDecision, ReadAttempt, ReadabilityEvidence, SupportAssurance,
    SupportState, VolumeRelation, WritabilityEvidence, WriteAttempt,
};

#[path = "../../experiments/p0/materialize/src/layout_policy.rs"]
mod layout_policy;

fuzz_target!(|input: &[u8]| {
    let filesystem_selectors = [
        selector(input, 0, 3),
        selector(input, 1, 3),
        selector(input, 2, 3),
        selector(input, 3, 3),
    ];
    let relation_selectors = [
        selector(input, 4, 3),
        selector(input, 5, 3),
        selector(input, 6, 3),
        selector(input, 7, 3),
        selector(input, 8, 3),
        selector(input, 9, 3),
    ];
    let candidate_selector = selector(input, 10, 15);

    let expected = filesystem_selectors.iter().all(|selector| *selector == 0)
        && relation_selectors.iter().all(|selector| *selector == 0)
        && candidate_shape_has_supported_full_copy(candidate_selector);
    let report =
        materialization_report(filesystem_selectors, relation_selectors, candidate_selector);

    assert_eq!(
        layout_policy::clone_layout_preconditions_satisfied(&report),
        expected
    );
});

fn selector(input: &[u8], index: usize, variants: u8) -> u8 {
    input.get(index).copied().unwrap_or_default() % variants
}

fn materialization_report(
    filesystem_selectors: [u8; 4],
    relation_selectors: [u8; 6],
    candidate_selector: u8,
) -> MaterializationPathReport {
    let mut filesystems = filesystem_selectors.into_iter().map(filesystem_type);
    let roles = [
        PathRole::Source,
        PathRole::TargetRoot,
        PathRole::Staging,
        PathRole::Trash,
    ];
    let role_pairs = [
        (PathRole::Source, PathRole::TargetRoot),
        (PathRole::Source, PathRole::Staging),
        (PathRole::Source, PathRole::Trash),
        (PathRole::TargetRoot, PathRole::Staging),
        (PathRole::TargetRoot, PathRole::Trash),
        (PathRole::Staging, PathRole::Trash),
    ];
    let mut paths = roles
        .into_iter()
        .map(|role| path_report(role, filesystems.next().expect("four filesystem selectors")));
    let pairs = role_pairs
        .into_iter()
        .zip(relation_selectors)
        .map(|((left, right), selector)| PathPairEvidence {
            left,
            right,
            relation: volume_relation(selector),
            evidence: Vec::new(),
        })
        .collect();

    MaterializationPathReport {
        schema_version: 1,
        experiment: "synthetic-layout-policy".to_owned(),
        observed_at_unix_ms: 0,
        source: paths.next().expect("source report"),
        target_root: paths.next().expect("target report"),
        staging: paths.next().expect("staging report"),
        trash: paths.next().expect("trash report"),
        pairs,
        candidates: candidate_evidence(candidate_selector),
        cow_evidence: CowEvidence::NotExecutedByProbe,
    }
}

fn filesystem_type(selector: u8) -> &'static str {
    match selector {
        0 => "apfs",
        1 => "syntheticfs",
        _ => "APFS",
    }
}

fn volume_relation(selector: u8) -> VolumeRelation {
    match selector {
        0 => VolumeRelation::SameVolume,
        1 => VolumeRelation::DifferentVolume,
        _ => VolumeRelation::Unknown,
    }
}

fn candidate_evidence(selector: u8) -> Vec<CandidateEvidence> {
    match selector {
        0 => vec![candidate(
            MaterializerCandidate::FullCopy,
            SupportState::Supported,
        )],
        1 => vec![candidate(
            MaterializerCandidate::FullCopy,
            SupportState::Unsupported,
        )],
        2 => vec![candidate(
            MaterializerCandidate::FullCopy,
            SupportState::Unknown,
        )],
        3 => Vec::new(),
        4 => vec![candidate(
            MaterializerCandidate::ApfsFileClone,
            SupportState::Supported,
        )],
        5 => vec![
            candidate(
                MaterializerCandidate::ApfsFileClone,
                SupportState::Supported,
            ),
            candidate(MaterializerCandidate::FullCopy, SupportState::Supported),
        ],
        6 => vec![
            candidate(
                MaterializerCandidate::ApfsFileClone,
                SupportState::Unsupported,
            ),
            candidate(MaterializerCandidate::FullCopy, SupportState::Supported),
        ],
        7 => vec![
            candidate(MaterializerCandidate::ApfsFileClone, SupportState::Unknown),
            candidate(MaterializerCandidate::FullCopy, SupportState::Supported),
        ],
        8 => vec![
            candidate(MaterializerCandidate::FullCopy, SupportState::Supported),
            candidate(
                MaterializerCandidate::ApfsFileClone,
                SupportState::Supported,
            ),
        ],
        9 => vec![
            candidate(MaterializerCandidate::FullCopy, SupportState::Supported),
            candidate(
                MaterializerCandidate::ApfsFileClone,
                SupportState::Unsupported,
            ),
        ],
        10 => vec![
            candidate(MaterializerCandidate::FullCopy, SupportState::Supported),
            candidate(MaterializerCandidate::ApfsFileClone, SupportState::Unknown),
        ],
        11 => vec![
            candidate(
                MaterializerCandidate::ApfsFileClone,
                SupportState::Supported,
            ),
            candidate(MaterializerCandidate::FullCopy, SupportState::Unknown),
        ],
        12 => vec![
            candidate(MaterializerCandidate::FullCopy, SupportState::Unknown),
            candidate(
                MaterializerCandidate::ApfsFileClone,
                SupportState::Supported,
            ),
        ],
        13 => vec![
            candidate(
                MaterializerCandidate::ApfsFileClone,
                SupportState::Supported,
            ),
            candidate(MaterializerCandidate::FullCopy, SupportState::Unsupported),
        ],
        _ => vec![
            candidate(MaterializerCandidate::FullCopy, SupportState::Unsupported),
            candidate(
                MaterializerCandidate::ApfsFileClone,
                SupportState::Supported,
            ),
        ],
    }
}

fn candidate_shape_has_supported_full_copy(selector: u8) -> bool {
    matches!(selector, 0 | 5..=10)
}

fn candidate(candidate: MaterializerCandidate, state: SupportState) -> CandidateEvidence {
    CandidateEvidence {
        candidate,
        state,
        assurance: SupportAssurance::PreflightOnly,
        execution: CandidateExecution::NotAttemptedByProbe,
        policy_decision: ProductPolicyDecision::NotEvaluatedByPlatformProbe,
        evidence: Vec::new(),
        cow_evidence: CowEvidence::NotExecutedByProbe,
    }
}

fn path_report(role: PathRole, filesystem_type: &str) -> PathCapabilityReport {
    let synthetic_path = encoded_path(role);
    let evidence_source = EvidenceSource::FstatHeldDirectoryFd;
    PathCapabilityReport {
        schema_version: 1,
        experiment: "synthetic-layout-policy".to_owned(),
        observed_at_unix_ms: 0,
        requested_path: synthetic_path.clone(),
        resolution: PathResolution::ExistingDirectory,
        nearest_existing_ancestor: synthetic_path.clone(),
        missing_components: Vec::new(),
        ancestry: vec![DirectoryIdentityEvidence {
            path: synthetic_path,
            identity: FileIdentity {
                device: 1,
                inode: role_identity(role),
            },
            source: evidence_source.clone(),
        }],
        filesystem: FileSystemIdentity {
            type_name: filesystem_type.to_owned(),
            fsid: [1, 1],
            volume_uuid: Evidence::Known {
                value: "00000000-0000-0000-0000-000000000001".to_owned(),
                source: EvidenceSource::FgetattrlistHeldDirectoryFd,
            },
            clone_capability: CloneCapabilityEvidence {
                state: SupportState::Supported,
                interface_capabilities: Some(1),
                interface_valid: Some(1),
                errno: None,
                source: EvidenceSource::FgetattrlistHeldDirectoryFd,
                reason: "synthetic in-memory evidence".to_owned(),
            },
        },
        mount: MountEvidence {
            raw_flags: 0,
            recognized_flags: BTreeSet::new(),
            source: EvidenceSource::FstatfsHeldDirectoryFd,
        },
        readability: ReadabilityEvidence {
            effective_access_preflight: AccessPreflight::Allowed {
                source: EvidenceSource::FaccessatHeldDirectoryFd,
            },
            execution_guarantee: ExecutionGuarantee::NotEstablishedByReadOnlyProbe,
            read_attempt: ReadAttempt::NotPerformedReadOnlyProbe,
        },
        writability: WritabilityEvidence {
            mount_state: MountWriteState::WritableAtInspection,
            effective_access_preflight: AccessPreflight::Allowed {
                source: EvidenceSource::FaccessatHeldDirectoryFd,
            },
            execution_guarantee: ExecutionGuarantee::NotEstablishedByReadOnlyProbe,
            write_attempt: WriteAttempt::NotPerformedReadOnlyProbe,
        },
        candidate_path_evidence: Vec::new(),
        cow_evidence: CowEvidence::NotExecutedByProbe,
    }
}

fn encoded_path(role: PathRole) -> EncodedPath {
    let (display, bytes_hex) = match role {
        PathRole::Source => ("synthetic-source", "73796e7468657469632d736f75726365"),
        PathRole::TargetRoot => ("synthetic-target", "73796e7468657469632d746172676574"),
        PathRole::Staging => ("synthetic-staging", "73796e7468657469632d73746167696e67"),
        PathRole::Trash => ("synthetic-trash", "73796e7468657469632d7472617368"),
    };
    EncodedPath {
        display: display.to_owned(),
        bytes_hex: bytes_hex.to_owned(),
    }
}

fn role_identity(role: PathRole) -> u64 {
    match role {
        PathRole::Source => 1,
        PathRole::TargetRoot => 2,
        PathRole::Staging => 3,
        PathRole::Trash => 4,
    }
}
