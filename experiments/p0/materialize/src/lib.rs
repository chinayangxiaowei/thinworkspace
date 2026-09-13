//! P0-02 experiment for APFS clone and explicit byte-copy materialization.
//!
//! This crate records repeatable platform evidence. It is not the Phase 1
//! materializer Port, lifecycle state machine, or fallback implementation.

#![deny(unsafe_code)]

#[allow(unsafe_code)]
mod ffi;
mod layout_policy;
#[cfg(test)]
#[path = "../tests/support/mod.rs"]
mod test_support;

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{CStr, CString, OsStr, OsString};
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::FileExt;
use std::path::{Component, Path, PathBuf};

use thinws_p0_probe::{
    AccessPreflight, Evidence, FileIdentity, MaterializationPathProbeRequest,
    MaterializationPathReport, MaterializerCandidate, PathCapabilityReport, ProbeError,
    RevalidationChange, RevalidationStatus, SupportState, inspect_materialization_paths,
    revalidate_path, validate_probe_path,
};
use thiserror::Error;

use crate::layout_policy::clone_layout_preconditions_satisfied;

/// Stable name for this non-product experiment.
pub const EXPERIMENT_NAME: &str = "p0-02-apfs-materialization";

/// Backend selected explicitly for one experimental attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Backend {
    /// APFS file clone through `fclonefileat`.
    ApfsFileClone,
    /// Explicit userspace read/write copy without transparent clone helpers.
    FullCopy,
}

/// User intent supplied to the minimal policy-ordering experiment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestedMode {
    CowClone,
    FullCopy,
}

/// Whether the local experiment may retry an unsupported clone as Full Copy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FallbackPolicy {
    Deny,
    AllowFullCopyOnCowUnsupported,
}

/// Clone preflight override used only to exercise an otherwise unavailable branch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClonePreflightOverride {
    Observed,
    InjectUnsupported,
}

/// Four already-existing roots inspected by the experiment.
#[derive(Clone, Copy, Debug)]
pub struct MaterializeRequest<'a> {
    pub source: &'a Path,
    pub target: &'a Path,
    pub staging: &'a Path,
    pub trash: &'a Path,
}

/// One lossless relative pathname used in evidence.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct EncodedRelativePath {
    pub display: String,
    pub bytes_hex: String,
}

/// Entry type covered by the deliberately limited experiment manifest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManifestEntryKind {
    Directory,
    RegularFile,
    SymbolicLink,
}

/// One filesystem modification time represented at nanosecond precision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModifiedTime {
    pub seconds: i64,
    pub nanoseconds: i64,
}

/// Manifest entry for the metadata explicitly covered by this experiment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManifestEntry {
    pub path: EncodedRelativePath,
    pub kind: ManifestEntryKind,
    pub permissions: Option<u32>,
    pub modified_time: Option<ModifiedTime>,
    pub length: u64,
    pub content_digest_hex: Option<String>,
}

/// Sorted, limited manifest of one source or target tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreeManifest {
    pub root_permissions: u32,
    pub root_modified_time: ModifiedTime,
    pub entries: Vec<ManifestEntry>,
}

/// Evidence that CoW was, was not, or could not be established.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CowEvidence {
    Confirmed,
    NotUsed,
    Unknown,
}

/// Whether one actual attempt completed or left partial state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptOutcome {
    Succeeded,
    Failed,
    Partial,
}

/// Structured syscall category retained by experimental failures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SystemOperation {
    CloneFileAt,
    Named(String),
}

impl fmt::Display for SystemOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CloneFileAt => formatter.write_str("fclonefileat"),
            Self::Named(name) => formatter.write_str(name),
        }
    }
}

impl From<&str> for SystemOperation {
    fn from(name: &str) -> Self {
        Self::Named(name.to_owned())
    }
}

/// Identity state captured immediately after one object was created.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CreatedIdentity {
    Confirmed(FileIdentity),
    Unconfirmed,
}

/// An object created by exactly one experimental attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreatedObjectEvidence {
    pub path: EncodedRelativePath,
    pub kind: ManifestEntryKind,
    pub identity: CreatedIdentity,
}

/// Source/target identities proving a successful regular-file operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrdinaryFileEvidence {
    pub path: EncodedRelativePath,
    pub source_identity: FileIdentity,
    pub target_identity: FileIdentity,
    pub real_clone_call_succeeded: bool,
}

/// Whether rollback was unnecessary, confirmed, or incomplete.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RollbackStatus {
    NotNeeded,
    ConfirmedBaseline,
    Incomplete,
}

/// One structured rollback problem.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RollbackProblem {
    pub operation: String,
    pub path: Option<EncodedRelativePath>,
    pub errno: Option<i32>,
    pub expected_identity: Option<FileIdentity>,
    pub observed_identity: Option<FileIdentity>,
    pub detail: String,
}

/// Precise result of the bounded, identity-checked rollback.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RollbackEvidence {
    pub status: RollbackStatus,
    pub removed: Vec<EncodedRelativePath>,
    pub remaining: Vec<EncodedRelativePath>,
    pub problems: Vec<RollbackProblem>,
}

/// Structured failure from the materialization experiment.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum MaterializationError {
    #[error("path probe failed during {stage}: {source}")]
    Probe {
        stage: String,
        #[source]
        source: Box<ProbeError>,
    },
    #[error("path evidence became stale for {path:?}")]
    PlanStale { path: PathBuf },
    #[error("source and target roots overlap: source={source_path:?}, target={target_path:?}")]
    OverlappingRoots {
        source_path: PathBuf,
        target_path: PathBuf,
    },
    #[error("clone preflight does not permit this policy: {reason}")]
    PreflightUnsupported { reason: String },
    #[error("target contains an entry not owned by this attempt: {path:?}")]
    UnknownTargetEntry { path: EncodedRelativePath },
    #[error("source entry {path:?} has unsupported type {kind}")]
    UnsupportedSourceEntry { path: PathBuf, kind: String },
    #[error("source entry changed while materialization was running: {path:?}")]
    SourceChanged { path: PathBuf },
    #[error("target entry changed while materialization was running: {path:?}")]
    TargetChanged { path: PathBuf },
    #[error("source and target manifests differ")]
    ManifestMismatch,
    #[error("created object identity could not be registered at {path:?}")]
    IdentityRegistrationFailed { path: EncodedRelativePath },
    #[error("system call {operation} failed for {path:?}: {message}")]
    SystemCall {
        operation: SystemOperation,
        path: PathBuf,
        errno: Option<i32>,
        message: String,
        injected: bool,
    },
}

/// Full evidence from one actual backend attempt, successful or failed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptEvidence {
    pub backend: Backend,
    pub outcome: AttemptOutcome,
    pub cow_evidence: CowEvidence,
    pub clone_calls_succeeded: usize,
    pub ordinary_files_materialized: usize,
    pub target_root_metadata_started: bool,
    pub created: Vec<CreatedObjectEvidence>,
    pub ordinary_files: Vec<OrdinaryFileEvidence>,
    pub source_manifest: Option<TreeManifest>,
    pub target_manifest: Option<TreeManifest>,
    pub failure: Option<MaterializationError>,
    pub rollback: RollbackEvidence,
}

/// Failed attempt paired with all partial and rollback evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttemptFailure {
    pub error: MaterializationError,
    pub evidence: Box<AttemptEvidence>,
}

/// Reason for a policy-authorized second backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FallbackReason {
    PreflightCloneUnsupported { injected: bool },
    RuntimeCloneUnsupported { errno: i32 },
}

/// Options for the minimal ordering experiment; not a product Plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyExperimentOptions {
    pub requested_mode: RequestedMode,
    pub fallback_policy: FallbackPolicy,
    pub clone_preflight: ClonePreflightOverride,
    pub first_attempt_faults: FaultInjection,
    pub second_attempt_faults: FaultInjection,
}

impl Default for PolicyExperimentOptions {
    fn default() -> Self {
        Self {
            requested_mode: RequestedMode::CowClone,
            fallback_policy: FallbackPolicy::Deny,
            clone_preflight: ClonePreflightOverride::Observed,
            first_attempt_faults: FaultInjection::default(),
            second_attempt_faults: FaultInjection::default(),
        }
    }
}

/// Successful result of the minimal policy-ordering experiment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyExperimentReceipt {
    pub requested_mode: RequestedMode,
    pub actual_backend: Backend,
    pub cow_evidence: CowEvidence,
    pub fallback_reason: Option<FallbackReason>,
    pub attempts: Vec<AttemptEvidence>,
}

/// Failed policy experiment with all attempts retained in order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyExperimentFailure {
    pub error: MaterializationError,
    pub attempts: Vec<AttemptEvidence>,
}

/// Copy failure after a precise number of target bytes were written.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CopyWriteFault {
    pub ordinary_file_index: usize,
    pub after_bytes: usize,
    pub errno: i32,
}

/// Fault applied after the primary attempt failure and before rollback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RollbackFault {
    None,
    AddUnknownEntry,
    ReplaceCreated { created_index: usize },
    FailDeletion { reverse_index: usize, errno: i32 },
}

/// Bounded fault injection used only by repeatable P0-02 tests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FaultInjection {
    pub clone_errno_at: Option<(usize, i32)>,
    pub copy_write: Option<CopyWriteFault>,
    pub identity_registration_at: Option<usize>,
    pub mutate_source_after_file: Option<usize>,
    pub mtime_errno_at: Option<(usize, i32)>,
    pub rollback: RollbackFault,
}

impl Default for FaultInjection {
    fn default() -> Self {
        Self {
            clone_errno_at: None,
            copy_write: None,
            identity_registration_at: None,
            mutate_source_after_file: None,
            mtime_errno_at: None,
            rollback: RollbackFault::None,
        }
    }
}

/// Read-only preparation evidence held for execution-time revalidation.
pub struct PreparedAttempt {
    request: OwnedRequest,
    backend: Backend,
    report: MaterializationPathReport,
    source: TreeSnapshot,
    target_baseline: ffi::NodeMetadata,
}

impl PreparedAttempt {
    /// Candidate state observed by the P0-01 probe for this backend.
    pub fn candidate_state(&self) -> SupportState {
        self.report
            .candidates
            .iter()
            .find(|evidence| evidence.candidate == candidate(self.backend))
            .map_or(SupportState::Unknown, |evidence| evidence.state)
    }
}

/// Inspect and snapshot all read-only facts needed by one later attempt.
pub fn prepare_attempt(
    request: &MaterializeRequest<'_>,
    backend: Backend,
) -> Result<PreparedAttempt, MaterializationError> {
    let request = OwnedRequest::from(request);
    let candidates = match backend {
        Backend::ApfsFileClone => vec![
            MaterializerCandidate::ApfsFileClone,
            MaterializerCandidate::FullCopy,
        ],
        Backend::FullCopy => vec![MaterializerCandidate::FullCopy],
    };
    let report = inspect_materialization_paths(&MaterializationPathProbeRequest {
        source: &request.source,
        target_root: &request.target,
        staging: &request.staging,
        trash: &request.trash,
        candidates: &candidates,
    })
    .map_err(|source| probe_error("initial path inspection", source))?;

    if roots_overlap(&report.source, &report.target_root) {
        return Err(MaterializationError::OverlappingRoots {
            source_path: request.source.clone(),
            target_path: request.target.clone(),
        });
    }

    let source_fd = open_verified_directory(&request.source, &report.source)?;
    let target_fd = open_verified_directory(&request.target, &report.target_root)?;
    let _staging_fd = open_verified_directory(&request.staging, &report.staging)?;
    let _trash_fd = open_verified_directory(&request.trash, &report.trash)?;
    let source = snapshot_tree(&source_fd, &request.source)?;
    verify_target_empty(&target_fd, &request.target)?;
    let target_baseline = ffi::metadata(&target_fd)
        .map_err(|error| syscall_error("fstat target baseline", &request.target, error, false))?;

    Ok(PreparedAttempt {
        request,
        backend,
        report,
        source,
        target_baseline,
    })
}

/// Execute one prepared attempt after fresh path and tree revalidation.
pub fn execute_prepared(
    prepared: &PreparedAttempt,
    faults: &FaultInjection,
) -> Result<AttemptEvidence, AttemptFailure> {
    let opened = match revalidate_and_open(prepared) {
        Ok(opened) => opened,
        Err(error) => {
            return Err(AttemptFailure {
                error: error.clone(),
                evidence: Box::new(empty_failed_attempt(prepared.backend, error)),
            });
        }
    };

    match execute_prepared_inner(prepared, faults, opened) {
        Ok(evidence) => Ok(evidence),
        Err(failure) => {
            let (error, mut context, target) = *failure;
            let mut rollback = if context.target_root_metadata_started {
                preserve_after_root_metadata_failure(prepared, &target, &context)
            } else {
                let fault_problems = apply_rollback_fault(prepared, &target, &mut context, faults);
                rollback_created(
                    prepared,
                    &target,
                    &context.created,
                    faults.rollback,
                    fault_problems,
                )
            };
            let target_manifest = match snapshot_tree(&target, &prepared.request.target) {
                Ok(snapshot) => Some(snapshot.manifest),
                Err(snapshot_error) => {
                    rollback.status = RollbackStatus::Incomplete;
                    rollback.problems.push(RollbackProblem {
                        operation: "snapshot target after failed attempt".to_owned(),
                        path: None,
                        errno: materialization_errno(&snapshot_error),
                        expected_identity: None,
                        observed_identity: None,
                        detail: snapshot_error.to_string(),
                    });
                    None
                }
            };
            let outcome = if context.created.is_empty() && !context.target_root_metadata_started {
                AttemptOutcome::Failed
            } else {
                AttemptOutcome::Partial
            };
            let evidence = context.into_evidence(
                outcome,
                CowEvidence::Unknown,
                Some(error.clone()),
                rollback,
                Some(prepared.source.manifest.clone()),
                target_manifest,
            );
            Err(AttemptFailure {
                error,
                evidence: Box::new(evidence),
            })
        }
    }
}

/// Prepare and execute one backend without applying product fallback policy.
pub fn materialize_once(
    request: &MaterializeRequest<'_>,
    backend: Backend,
) -> Result<AttemptEvidence, AttemptFailure> {
    let prepared = prepare_attempt(request, backend).map_err(|error| AttemptFailure {
        error: error.clone(),
        evidence: Box::new(empty_failed_attempt(backend, error)),
    })?;
    execute_prepared(&prepared, &FaultInjection::default())
}

/// Exercise only the ordering rules required by the P0-02 task card.
pub fn run_policy_experiment(
    request: &MaterializeRequest<'_>,
    options: &PolicyExperimentOptions,
) -> Result<PolicyExperimentReceipt, PolicyExperimentFailure> {
    match options.requested_mode {
        RequestedMode::FullCopy => {
            let attempt = run_attempt(request, Backend::FullCopy, &options.first_attempt_faults)?;
            Ok(PolicyExperimentReceipt {
                requested_mode: RequestedMode::FullCopy,
                actual_backend: Backend::FullCopy,
                cow_evidence: attempt.cow_evidence,
                fallback_reason: None,
                attempts: vec![attempt],
            })
        }
        RequestedMode::CowClone => {
            let prepared = prepare_attempt(request, Backend::ApfsFileClone).map_err(|error| {
                PolicyExperimentFailure {
                    error,
                    attempts: Vec::new(),
                }
            })?;
            run_cow_policy_experiment(request, options, prepared)
        }
    }
}

fn run_cow_policy_experiment(
    request: &MaterializeRequest<'_>,
    options: &PolicyExperimentOptions,
    prepared: PreparedAttempt,
) -> Result<PolicyExperimentReceipt, PolicyExperimentFailure> {
    let injected_unsupported = options.clone_preflight == ClonePreflightOverride::InjectUnsupported;
    let observed_unsupported = prepared.candidate_state() == SupportState::Unsupported;

    if injected_unsupported || observed_unsupported {
        let clone_capability_absent = prepared.report.candidates.iter().any(|evidence| {
            evidence.candidate == MaterializerCandidate::ApfsFileClone
                && evidence
                    .evidence
                    .iter()
                    .any(|statement| statement.code == "clone_capability_absent")
        });
        let eligible_preflight_fallback = clone_layout_preconditions_satisfied(&prepared.report)
            && (injected_unsupported || clone_capability_absent);
        if options.fallback_policy != FallbackPolicy::AllowFullCopyOnCowUnsupported
            || !eligible_preflight_fallback
        {
            return Err(PolicyExperimentFailure {
                error: MaterializationError::PreflightUnsupported {
                    reason: if injected_unsupported {
                        "injected same-volume clone unsupported evidence".to_owned()
                    } else {
                        "observed clone preflight is not an eligible same-volume fallback"
                            .to_owned()
                    },
                },
                attempts: Vec::new(),
            });
        }

        let copy_prepared =
            prepare_fresh_copy_after_clone(request, &prepared).map_err(|error| {
                PolicyExperimentFailure {
                    error,
                    attempts: Vec::new(),
                }
            })?;
        let copy =
            execute_prepared(&copy_prepared, &options.first_attempt_faults).map_err(|failure| {
                PolicyExperimentFailure {
                    error: failure.error,
                    attempts: vec![*failure.evidence],
                }
            })?;
        return Ok(PolicyExperimentReceipt {
            requested_mode: RequestedMode::CowClone,
            actual_backend: Backend::FullCopy,
            cow_evidence: CowEvidence::NotUsed,
            fallback_reason: Some(FallbackReason::PreflightCloneUnsupported {
                injected: injected_unsupported,
            }),
            attempts: vec![copy],
        });
    }

    match execute_prepared(&prepared, &options.first_attempt_faults) {
        Ok(attempt) => Ok(PolicyExperimentReceipt {
            requested_mode: RequestedMode::CowClone,
            actual_backend: Backend::ApfsFileClone,
            cow_evidence: attempt.cow_evidence,
            fallback_reason: None,
            attempts: vec![attempt],
        }),
        Err(clone_failure) => {
            let runtime_unsupported = matches!(
                clone_failure.error,
                MaterializationError::SystemCall {
                    ref operation,
                    errno: Some(libc::ENOTSUP),
                    ..
                } if *operation == SystemOperation::CloneFileAt
            );
            if !runtime_unsupported
                || options.fallback_policy != FallbackPolicy::AllowFullCopyOnCowUnsupported
                || clone_failure.evidence.rollback.status != RollbackStatus::ConfirmedBaseline
            {
                return Err(PolicyExperimentFailure {
                    error: clone_failure.error,
                    attempts: vec![*clone_failure.evidence],
                });
            }

            let mut attempts = vec![*clone_failure.evidence];
            let copy_prepared = match prepare_fresh_copy_after_clone(request, &prepared) {
                Ok(prepared) => prepared,
                Err(error) => {
                    return Err(PolicyExperimentFailure { error, attempts });
                }
            };
            match execute_prepared(&copy_prepared, &options.second_attempt_faults) {
                Ok(copy) => {
                    attempts.push(copy);
                    Ok(PolicyExperimentReceipt {
                        requested_mode: RequestedMode::CowClone,
                        actual_backend: Backend::FullCopy,
                        cow_evidence: CowEvidence::NotUsed,
                        fallback_reason: Some(FallbackReason::RuntimeCloneUnsupported {
                            errno: libc::ENOTSUP,
                        }),
                        attempts,
                    })
                }
                Err(failure) => {
                    attempts.push(*failure.evidence);
                    Err(PolicyExperimentFailure {
                        error: failure.error,
                        attempts,
                    })
                }
            }
        }
    }
}

fn run_attempt(
    request: &MaterializeRequest<'_>,
    backend: Backend,
    faults: &FaultInjection,
) -> Result<AttemptEvidence, PolicyExperimentFailure> {
    let prepared = prepare_attempt(request, backend).map_err(|error| PolicyExperimentFailure {
        error,
        attempts: Vec::new(),
    })?;
    execute_prepared(&prepared, faults).map_err(|failure| PolicyExperimentFailure {
        error: failure.error,
        attempts: vec![*failure.evidence],
    })
}

fn prepare_fresh_copy_after_clone(
    request: &MaterializeRequest<'_>,
    previous: &PreparedAttempt,
) -> Result<PreparedAttempt, MaterializationError> {
    let mut fresh = prepare_attempt(request, Backend::ApfsFileClone)?;
    for (path, previous_path, fresh_path) in [
        (
            &previous.request.source,
            &previous.report.source,
            &fresh.report.source,
        ),
        (
            &previous.request.target,
            &previous.report.target_root,
            &fresh.report.target_root,
        ),
        (
            &previous.request.staging,
            &previous.report.staging,
            &fresh.report.staging,
        ),
        (
            &previous.request.trash,
            &previous.report.trash,
            &fresh.report.trash,
        ),
    ] {
        if !fallback_path_evidence_matches(previous_path, fresh_path) {
            return Err(MaterializationError::PlanStale { path: path.clone() });
        }
    }
    if previous.source != fresh.source {
        return Err(MaterializationError::SourceChanged {
            path: previous.request.source.clone(),
        });
    }
    if !clone_layout_preconditions_satisfied(&fresh.report) {
        return Err(MaterializationError::PreflightUnsupported {
            reason: "fresh fallback probe did not confirm the original same-volume APFS layout and Full Copy permissions"
                .to_owned(),
        });
    }
    fresh.backend = Backend::FullCopy;
    Ok(fresh)
}

fn fallback_path_evidence_matches(
    previous: &PathCapabilityReport,
    fresh: &PathCapabilityReport,
) -> bool {
    matches!(previous.filesystem.volume_uuid, Evidence::Known { .. })
        && matches!(fresh.filesystem.volume_uuid, Evidence::Known { .. })
        && previous.filesystem == fresh.filesystem
        && previous.mount == fresh.mount
        && previous
            .ancestry
            .iter()
            .map(|entry| entry.identity)
            .eq(fresh.ancestry.iter().map(|entry| entry.identity))
}

fn execute_prepared_inner(
    prepared: &PreparedAttempt,
    faults: &FaultInjection,
    opened: OpenedRoots,
) -> Result<AttemptEvidence, Box<(MaterializationError, ExecutionContext, OwnedFd)>> {
    let OpenedRoots {
        source,
        target,
        staging,
        trash,
    } = opened;

    let source_now = match snapshot_tree(&source, &prepared.request.source) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return Err(Box::new((
                error,
                ExecutionContext::new(prepared.backend),
                target,
            )));
        }
    };
    if source_now != prepared.source {
        return Err(Box::new((
            MaterializationError::SourceChanged {
                path: prepared.request.source.clone(),
            },
            ExecutionContext::new(prepared.backend),
            target,
        )));
    }
    if let Err(error) = verify_target_empty(&target, &prepared.request.target) {
        return Err(Box::new((
            error,
            ExecutionContext::new(prepared.backend),
            target,
        )));
    }
    let target_now = match ffi::metadata(&target) {
        Ok(metadata) => metadata,
        Err(error) => {
            return Err(Box::new((
                syscall_error(
                    "fstat target root before materialization",
                    &prepared.request.target,
                    error,
                    false,
                ),
                ExecutionContext::new(prepared.backend),
                target,
            )));
        }
    };
    if !same_preserved_root_metadata(target_now, prepared.target_baseline) {
        return Err(Box::new((
            MaterializationError::PlanStale {
                path: prepared.request.target.clone(),
            },
            ExecutionContext::new(prepared.backend),
            target,
        )));
    }

    let mut context = ExecutionContext::new(prepared.backend);
    if let Err(error) = materialize_directory(
        &source,
        &target,
        &[],
        source_now.root_metadata.device,
        (&prepared.request.source, &prepared.request.target),
        faults,
        &mut context,
    ) {
        return Err(Box::new((error, context, target)));
    }

    let source_after = match snapshot_tree(&source, &prepared.request.source) {
        Ok(snapshot) => snapshot,
        Err(error) => return Err(Box::new((error, context, target))),
    };
    if source_after != prepared.source {
        return Err(Box::new((
            MaterializationError::SourceChanged {
                path: prepared.request.source.clone(),
            },
            context,
            target,
        )));
    }
    if let Err(error) = revalidate_held_roots(prepared, [&source, &target, &staging, &trash]) {
        return Err(Box::new((error, context, target)));
    }
    if let Err(error) = set_preserved_metadata(
        &target,
        identity(prepared.target_baseline),
        source_after.root_metadata,
        &prepared.request.target,
        faults,
        &mut context,
        true,
    ) {
        return Err(Box::new((error, context, target)));
    }
    let target_snapshot = match snapshot_tree(&target, &prepared.request.target) {
        Ok(snapshot) => snapshot,
        Err(error) => return Err(Box::new((error, context, target))),
    };
    if !context.matches_target_snapshot(&prepared.source.manifest, &target_snapshot) {
        return Err(Box::new((
            MaterializationError::ManifestMismatch,
            context,
            target,
        )));
    }
    if let Err(error) = revalidate_held_roots_after_target_metadata(
        prepared,
        [&source, &target, &staging, &trash],
        source_after.root_metadata,
    ) {
        return Err(Box::new((error, context, target)));
    }

    let regular_count = prepared
        .source
        .manifest
        .entries
        .iter()
        .filter(|entry| entry.kind == ManifestEntryKind::RegularFile)
        .count();
    let cow = match prepared.backend {
        Backend::FullCopy => CowEvidence::NotUsed,
        Backend::ApfsFileClone if regular_count == 0 => CowEvidence::NotUsed,
        Backend::ApfsFileClone if context.clone_calls_succeeded == regular_count => {
            CowEvidence::Confirmed
        }
        Backend::ApfsFileClone => CowEvidence::Unknown,
    };
    Ok(context.into_evidence(
        AttemptOutcome::Succeeded,
        cow,
        None,
        RollbackEvidence {
            status: RollbackStatus::NotNeeded,
            removed: Vec::new(),
            remaining: Vec::new(),
            problems: Vec::new(),
        },
        Some(prepared.source.manifest.clone()),
        Some(target_snapshot.manifest),
    ))
}

fn revalidate_and_open(prepared: &PreparedAttempt) -> Result<OpenedRoots, MaterializationError> {
    for (path, report) in [
        (&prepared.request.source, &prepared.report.source),
        (&prepared.request.target, &prepared.report.target_root),
        (&prepared.request.staging, &prepared.report.staging),
        (&prepared.request.trash, &prepared.report.trash),
    ] {
        let revalidation = match revalidate_path(report) {
            Ok(revalidation) => revalidation,
            Err(source) => {
                return Err(probe_error("path revalidation", source));
            }
        };
        if revalidation.status != RevalidationStatus::Unchanged {
            return Err(MaterializationError::PlanStale { path: path.clone() });
        }
    }

    Ok(OpenedRoots {
        source: open_verified_directory(&prepared.request.source, &prepared.report.source)?,
        target: open_verified_directory(&prepared.request.target, &prepared.report.target_root)?,
        staging: open_verified_directory(&prepared.request.staging, &prepared.report.staging)?,
        trash: open_verified_directory(&prepared.request.trash, &prepared.report.trash)?,
    })
}

fn revalidate_held_roots(
    prepared: &PreparedAttempt,
    held: [&OwnedFd; 4],
) -> Result<(), MaterializationError> {
    let fresh = revalidate_and_open(prepared)?;
    let fresh = [&fresh.source, &fresh.target, &fresh.staging, &fresh.trash];
    let paths = [
        &prepared.request.source,
        &prepared.request.target,
        &prepared.request.staging,
        &prepared.request.trash,
    ];
    for ((held, fresh), path) in held.into_iter().zip(fresh).zip(paths) {
        let held_metadata = ffi::metadata(held)
            .map_err(|error| syscall_error("fstat held root", path, error, false))?;
        let fresh_metadata = ffi::metadata(fresh)
            .map_err(|error| syscall_error("fstat revalidated root", path, error, false))?;
        if !same_identity_and_kind(fresh_metadata, identity(held_metadata), held_metadata.kind) {
            return Err(MaterializationError::PlanStale { path: path.clone() });
        }
    }
    Ok(())
}

fn revalidate_held_roots_after_target_metadata(
    prepared: &PreparedAttempt,
    held: [&OwnedFd; 4],
    expected_target_metadata: ffi::NodeMetadata,
) -> Result<(), MaterializationError> {
    let paths_and_reports = [
        (&prepared.request.source, &prepared.report.source),
        (&prepared.request.target, &prepared.report.target_root),
        (&prepared.request.staging, &prepared.report.staging),
        (&prepared.request.trash, &prepared.report.trash),
    ];
    let mut fresh = Vec::with_capacity(paths_and_reports.len());
    for (index, (path, report)) in paths_and_reports.into_iter().enumerate() {
        let revalidation = revalidate_path(report)
            .map_err(|source| probe_error("final path revalidation", source))?;
        let only_expected_target_writability_changed = index == 1
            && !revalidation.changes.is_empty()
            && revalidation.changes.iter().all(|change| {
                is_expected_target_writability_change(
                    change,
                    prepared.target_baseline.mode,
                    expected_target_metadata.mode,
                )
            });
        if revalidation.status != RevalidationStatus::Unchanged
            && !only_expected_target_writability_changed
        {
            return Err(MaterializationError::PlanStale { path: path.clone() });
        }
        fresh.push(open_verified_directory(path, report)?);
    }

    let paths = [
        &prepared.request.source,
        &prepared.request.target,
        &prepared.request.staging,
        &prepared.request.trash,
    ];
    for (index, ((held, fresh), path)) in held.into_iter().zip(&fresh).zip(paths).enumerate() {
        let held_metadata = ffi::metadata(held)
            .map_err(|error| syscall_error("fstat held final root", path, error, false))?;
        let fresh_metadata = ffi::metadata(fresh)
            .map_err(|error| syscall_error("fstat revalidated final root", path, error, false))?;
        if !same_identity_and_kind(fresh_metadata, identity(held_metadata), held_metadata.kind) {
            return Err(MaterializationError::PlanStale { path: path.clone() });
        }
        if index == 1
            && (!same_identity_and_kind(
                held_metadata,
                identity(prepared.target_baseline),
                ffi::NodeKind::Directory,
            ) || !has_preserved_metadata(held_metadata, expected_target_metadata))
        {
            return Err(MaterializationError::ManifestMismatch);
        }
    }
    Ok(())
}

fn is_expected_target_writability_change(
    change: &RevalidationChange,
    before_mode: u32,
    after_mode: u32,
) -> bool {
    let RevalidationChange::WritabilityChanged { before, after } = change else {
        return false;
    };
    let owner_write_removed = before_mode & 0o200 != 0 && after_mode & 0o200 == 0;
    if !owner_write_removed
        || before.mount_state != after.mount_state
        || before.execution_guarantee != after.execution_guarantee
        || before.write_attempt != after.write_attempt
    {
        return false;
    }
    matches!(
        (
            &before.effective_access_preflight,
            &after.effective_access_preflight,
        ),
        (
            AccessPreflight::Allowed {
                source: before_source,
            },
            AccessPreflight::Denied {
                errno: Some(libc::EACCES),
                source: after_source,
                ..
            }
        ) if before_source == after_source
    )
}

fn materialize_directory(
    source: &OwnedFd,
    target: &OwnedFd,
    relative: &[OsString],
    source_root_device: u64,
    roots: (&Path, &Path),
    faults: &FaultInjection,
    context: &mut ExecutionContext,
) -> Result<(), MaterializationError> {
    let (source_root, target_root) = roots;
    let mut names = read_names(source, &join_relative(source_root, relative))?;
    names.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    for name in names {
        let component = ComponentName::new(&name)?;
        let mut child_relative = relative.to_vec();
        child_relative.push(name.clone());
        let source_path = join_relative(source_root, &child_relative);
        let target_path = join_relative(target_root, &child_relative);
        let before = ffi::metadata_at(source, component.as_c_str())
            .map_err(|error| syscall_error("fstatat source entry", &source_path, error, false))?;
        if before.device != source_root_device {
            return Err(MaterializationError::UnsupportedSourceEntry {
                path: source_path,
                kind: "submount".to_owned(),
            });
        }

        match before.kind {
            ffi::NodeKind::Directory => {
                let source_child =
                    ffi::open_directory_at(source, component.as_c_str()).map_err(|error| {
                        syscall_error("openat source directory", &source_path, error, false)
                    })?;
                if ffi::metadata(&source_child).map_err(|error| {
                    syscall_error("fstat source directory", &source_path, error, false)
                })? != before
                {
                    return Err(MaterializationError::SourceChanged { path: source_path });
                }
                ffi::create_directory_at(target, component.as_c_str(), 0o700).map_err(|error| {
                    syscall_error("mkdirat target directory", &target_path, error, false)
                })?;
                let registered = register_created(
                    target,
                    &component,
                    &child_relative,
                    before.kind,
                    None,
                    faults,
                    context,
                )?;
                let target_child =
                    ffi::open_directory_at(target, component.as_c_str()).map_err(|error| {
                        syscall_error("openat target directory", &target_path, error, false)
                    })?;
                let target_held = ffi::metadata(&target_child).map_err(|error| {
                    syscall_error("fstat target directory", &target_path, error, false)
                })?;
                if !same_identity_and_kind(target_held, registered, ffi::NodeKind::Directory) {
                    return Err(MaterializationError::TargetChanged { path: target_path });
                }
                materialize_directory(
                    &source_child,
                    &target_child,
                    &child_relative,
                    source_root_device,
                    roots,
                    faults,
                    context,
                )?;
                set_preserved_metadata(
                    &target_child,
                    registered,
                    before,
                    &target_path,
                    faults,
                    context,
                    false,
                )?;
            }
            ffi::NodeKind::RegularFile => {
                let source_file =
                    ffi::open_file_read_at(source, component.as_c_str()).map_err(|error| {
                        syscall_error("openat source file", &source_path, error, false)
                    })?;
                let held_before = ffi::metadata(&source_file).map_err(|error| {
                    syscall_error("fstat source file", &source_path, error, false)
                })?;
                if held_before != before {
                    return Err(MaterializationError::SourceChanged { path: source_path });
                }
                let ordinary_index = context.ordinary_files_materialized;
                let (registered, target_file) = match context.backend {
                    Backend::ApfsFileClone => {
                        if faults
                            .clone_errno_at
                            .is_some_and(|(index, _)| index == ordinary_index)
                        {
                            let errno = faults.clone_errno_at.expect("checked above").1;
                            return Err(injected_syscall_error(
                                SystemOperation::CloneFileAt,
                                &target_path,
                                errno,
                            ));
                        }
                        ffi::clone_file_at(&source_file, target, component.as_c_str()).map_err(
                            |error| {
                                syscall_error(
                                    SystemOperation::CloneFileAt,
                                    &target_path,
                                    error,
                                    false,
                                )
                            },
                        )?;
                        context.clone_calls_succeeded += 1;
                        let registered = register_created(
                            target,
                            &component,
                            &child_relative,
                            before.kind,
                            None,
                            faults,
                            context,
                        )?;
                        let target_file = ffi::open_file_read_at(target, component.as_c_str())
                            .map_err(|error| {
                                syscall_error(
                                    "openat cloned target file",
                                    &target_path,
                                    error,
                                    false,
                                )
                            })?;
                        (registered, target_file)
                    }
                    Backend::FullCopy => {
                        let target_file = ffi::create_file_at(target, component.as_c_str(), 0o600)
                            .map_err(|error| {
                                syscall_error("openat target file", &target_path, error, false)
                            })?;
                        let registered = register_created(
                            target,
                            &component,
                            &child_relative,
                            before.kind,
                            Some(&target_file),
                            faults,
                            context,
                        )?;
                        copy_file_bytes(
                            &source_file,
                            &target_file,
                            ordinary_index,
                            &target_path,
                            faults.copy_write,
                        )?;
                        (registered, target_file)
                    }
                };
                set_preserved_metadata(
                    &target_file,
                    registered,
                    before,
                    &target_path,
                    faults,
                    context,
                    false,
                )?;

                let target_metadata =
                    ffi::metadata_at(target, component.as_c_str()).map_err(|error| {
                        syscall_error("fstatat target file", &target_path, error, false)
                    })?;
                if !same_identity_and_kind(target_metadata, registered, ffi::NodeKind::RegularFile)
                {
                    return Err(MaterializationError::TargetChanged { path: target_path });
                }
                if identity(target_metadata) == identity(held_before) {
                    return Err(MaterializationError::TargetChanged { path: target_path });
                }
                context.ordinary_files.push(OrdinaryFileEvidence {
                    path: encode_relative(&child_relative),
                    source_identity: identity(before),
                    target_identity: registered,
                    real_clone_call_succeeded: context.backend == Backend::ApfsFileClone,
                });
                context.ordinary_files_materialized += 1;

                if faults.mutate_source_after_file == Some(ordinary_index) {
                    mutate_source_same_inode(source, &component, &source_path, before)?;
                }
                let held_after = ffi::metadata(&source_file).map_err(|error| {
                    syscall_error(
                        "fstat source file after materialization",
                        &source_path,
                        error,
                        false,
                    )
                })?;
                if held_after != held_before {
                    return Err(MaterializationError::SourceChanged { path: source_path });
                }
            }
            ffi::NodeKind::SymbolicLink => {
                let link_text =
                    ffi::read_link_at(source, component.as_c_str()).map_err(|error| {
                        syscall_error("readlinkat source", &source_path, error, false)
                    })?;
                let after = ffi::metadata_at(source, component.as_c_str()).map_err(|error| {
                    syscall_error("fstatat source symlink", &source_path, error, false)
                })?;
                if after != before {
                    return Err(MaterializationError::SourceChanged { path: source_path });
                }
                let link_text = CString::new(link_text).map_err(|_| {
                    MaterializationError::UnsupportedSourceEntry {
                        path: source_path.clone(),
                        kind: "symlink-target-with-nul".to_owned(),
                    }
                })?;
                ffi::create_symlink_at(&link_text, target, component.as_c_str()).map_err(
                    |error| syscall_error("symlinkat target", &target_path, error, false),
                )?;
                let _registered = register_created(
                    target,
                    &component,
                    &child_relative,
                    before.kind,
                    None,
                    faults,
                    context,
                )?;
            }
            ffi::NodeKind::Special => {
                return Err(MaterializationError::UnsupportedSourceEntry {
                    path: source_path,
                    kind: "special".to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn set_preserved_metadata(
    target: &OwnedFd,
    expected_identity: FileIdentity,
    source: ffi::NodeMetadata,
    target_path: &Path,
    faults: &FaultInjection,
    context: &mut ExecutionContext,
    is_target_root: bool,
) -> Result<(), MaterializationError> {
    let held = ffi::metadata(target).map_err(|error| {
        syscall_error("fstat target before metadata", target_path, error, false)
    })?;
    if !same_identity_and_kind(held, expected_identity, source.kind) {
        return Err(MaterializationError::TargetChanged {
            path: target_path.to_path_buf(),
        });
    }
    if is_target_root {
        context.target_root_metadata_started = true;
    }
    let metadata_index = context.metadata_updates_started;
    context.metadata_updates_started += 1;
    if faults
        .mtime_errno_at
        .is_some_and(|(index, _)| index == metadata_index)
    {
        let errno = faults.mtime_errno_at.expect("checked above").1;
        return Err(injected_syscall_error(
            "futimens target mtime",
            target_path,
            errno,
        ));
    }
    ffi::set_modified_time(target, source.modified_seconds, source.modified_nanoseconds)
        .map_err(|error| syscall_error("futimens target mtime", target_path, error, false))?;
    ffi::set_mode(target, source.mode)
        .map_err(|error| syscall_error("fchmod target mode", target_path, error, false))
}

fn copy_file_bytes(
    source: &OwnedFd,
    target: &OwnedFd,
    ordinary_index: usize,
    target_path: &Path,
    fault: Option<CopyWriteFault>,
) -> Result<(), MaterializationError> {
    let mut source = File::from(
        source
            .try_clone()
            .map_err(|error| syscall_error("dup source file", target_path, error, false))?,
    );
    let mut target = File::from(
        target
            .try_clone()
            .map_err(|error| syscall_error("dup target file", target_path, error, false))?,
    );
    let mut buffer = [0u8; 65_536];
    let fault = fault.filter(|fault| fault.ordinary_file_index == ordinary_index);
    let mut written_total = 0usize;
    loop {
        if let Some(fault) = fault
            && written_total >= fault.after_bytes
        {
            return Err(injected_syscall_error(
                "write target file",
                target_path,
                fault.errno,
            ));
        }
        let read_limit = match fault {
            Some(fault) => buffer
                .len()
                .min(fault.after_bytes.saturating_sub(written_total).max(1)),
            _ => buffer.len(),
        };
        let read = source
            .read(&mut buffer[..read_limit])
            .map_err(|error| syscall_error("read source file", target_path, error, false))?;
        let read = match read {
            0 => break,
            read => read,
        };
        let mut remaining = &buffer[..read];
        while !remaining.is_empty() {
            let written = target
                .write(remaining)
                .map_err(|error| syscall_error("write target file", target_path, error, false))?;
            if written == 0 {
                return Err(syscall_error(
                    "write target file",
                    target_path,
                    io::Error::from(io::ErrorKind::WriteZero),
                    false,
                ));
            }
            remaining = &remaining[written..];
            written_total += written;
        }
    }
    Ok(())
}

fn mutate_source_same_inode(
    parent: &OwnedFd,
    component: &ComponentName,
    path: &Path,
    expected: ffi::NodeMetadata,
) -> Result<(), MaterializationError> {
    if expected.size == 0 {
        return Err(MaterializationError::UnsupportedSourceEntry {
            path: path.to_path_buf(),
            kind: "cannot-inject-same-inode-change-into-empty-file".to_owned(),
        });
    }
    let file = ffi::open_file_write_at(parent, component.as_c_str())
        .map_err(|error| syscall_error("openat source fault injection", path, error, false))?;
    let held = ffi::metadata(&file)
        .map_err(|error| syscall_error("fstat source fault injection", path, error, false))?;
    if !same_identity_and_kind(held, identity(expected), ffi::NodeKind::RegularFile) {
        return Err(MaterializationError::SourceChanged {
            path: path.to_path_buf(),
        });
    }
    let file = File::from(file);
    file.write_at(b"!", 0)
        .map_err(|error| syscall_error("pwrite source fault injection", path, error, true))?;
    Ok(())
}

fn register_created(
    parent: &OwnedFd,
    component: &ComponentName,
    relative: &[OsString],
    expected_kind: ffi::NodeKind,
    created_fd: Option<&OwnedFd>,
    faults: &FaultInjection,
    context: &mut ExecutionContext,
) -> Result<FileIdentity, MaterializationError> {
    let index = context.created.len();
    context.created.push(TrackedCreated {
        relative: relative.to_vec(),
        kind: expected_kind,
        identity: None,
    });
    if faults.identity_registration_at == Some(index) {
        return Err(MaterializationError::IdentityRegistrationFailed {
            path: encode_relative(relative),
        });
    }
    let metadata = ffi::metadata_at(parent, component.as_c_str()).map_err(|error| {
        syscall_error(
            "fstatat created target",
            &PathBuf::from_iter(relative),
            error,
            false,
        )
    })?;
    if metadata.kind != expected_kind {
        return Err(MaterializationError::TargetChanged {
            path: PathBuf::from_iter(relative),
        });
    }
    let registered = identity(metadata);
    if let Some(created_fd) = created_fd {
        let held = ffi::metadata(created_fd).map_err(|error| {
            syscall_error(
                "fstat held created target",
                &PathBuf::from_iter(relative),
                error,
                false,
            )
        })?;
        if !same_identity_and_kind(held, registered, expected_kind) {
            return Err(MaterializationError::TargetChanged {
                path: PathBuf::from_iter(relative),
            });
        }
    }
    context.created[index].identity = Some(registered);
    Ok(registered)
}

fn snapshot_tree(root: &OwnedFd, root_path: &Path) -> Result<TreeSnapshot, MaterializationError> {
    let root_before = ffi::metadata(root)
        .map_err(|error| syscall_error("fstat manifest root", root_path, error, false))?;
    let mut snapshot = TreeSnapshot {
        manifest: TreeManifest {
            root_permissions: root_before.mode,
            root_modified_time: modified_time(root_before),
            entries: Vec::new(),
        },
        identities: BTreeMap::new(),
        root_metadata: root_before,
    };
    snapshot_directory(root, &[], root_before.device, root_path, &mut snapshot)?;
    let root_after = ffi::metadata(root).map_err(|error| {
        syscall_error("fstat manifest root after scan", root_path, error, false)
    })?;
    if root_after != root_before {
        return Err(MaterializationError::SourceChanged {
            path: root_path.to_path_buf(),
        });
    }
    Ok(snapshot)
}

fn snapshot_directory(
    directory: &OwnedFd,
    relative: &[OsString],
    root_device: u64,
    root_path: &Path,
    snapshot: &mut TreeSnapshot,
) -> Result<(), MaterializationError> {
    let mut names = read_names(directory, &join_relative(root_path, relative))?;
    names.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    for name in names {
        let component = ComponentName::new(&name)?;
        let mut child = relative.to_vec();
        child.push(name);
        let path = join_relative(root_path, &child);
        let before = ffi::metadata_at(directory, component.as_c_str())
            .map_err(|error| syscall_error("fstatat manifest entry", &path, error, false))?;
        if before.device != root_device {
            return Err(MaterializationError::UnsupportedSourceEntry {
                path,
                kind: "submount".to_owned(),
            });
        }
        let encoded = encode_relative(&child);
        snapshot
            .identities
            .insert(encoded.clone(), identity(before));
        match before.kind {
            ffi::NodeKind::Directory => {
                snapshot.manifest.entries.push(ManifestEntry {
                    path: encoded,
                    kind: ManifestEntryKind::Directory,
                    permissions: Some(before.mode),
                    modified_time: Some(modified_time(before)),
                    length: 0,
                    content_digest_hex: None,
                });
                let child_fd =
                    ffi::open_directory_at(directory, component.as_c_str()).map_err(|error| {
                        syscall_error("openat manifest directory", &path, error, false)
                    })?;
                if ffi::metadata(&child_fd).map_err(|error| {
                    syscall_error("fstat manifest directory", &path, error, false)
                })? != before
                {
                    return Err(MaterializationError::SourceChanged { path });
                }
                snapshot_directory(&child_fd, &child, root_device, root_path, snapshot)?;
            }
            ffi::NodeKind::RegularFile => {
                let file = ffi::open_file_read_at(directory, component.as_c_str())
                    .map_err(|error| syscall_error("openat manifest file", &path, error, false))?;
                let held_before = ffi::metadata(&file)
                    .map_err(|error| syscall_error("fstat manifest file", &path, error, false))?;
                if held_before != before {
                    return Err(MaterializationError::SourceChanged { path });
                }
                let (length, digest) = digest_file(file, &path)?;
                if length != before.size {
                    return Err(MaterializationError::SourceChanged { path });
                }
                snapshot.manifest.entries.push(ManifestEntry {
                    path: encoded,
                    kind: ManifestEntryKind::RegularFile,
                    permissions: Some(before.mode),
                    modified_time: Some(modified_time(before)),
                    length,
                    content_digest_hex: Some(digest),
                });
            }
            ffi::NodeKind::SymbolicLink => {
                let link_text =
                    ffi::read_link_at(directory, component.as_c_str()).map_err(|error| {
                        syscall_error("readlinkat manifest symlink", &path, error, false)
                    })?;
                let after = ffi::metadata_at(directory, component.as_c_str()).map_err(|error| {
                    syscall_error("fstatat manifest symlink", &path, error, false)
                })?;
                if after != before {
                    return Err(MaterializationError::SourceChanged { path });
                }
                snapshot.manifest.entries.push(ManifestEntry {
                    path: encoded,
                    kind: ManifestEntryKind::SymbolicLink,
                    permissions: None,
                    modified_time: None,
                    length: link_text.len() as u64,
                    content_digest_hex: Some(blake3::hash(&link_text).to_hex().to_string()),
                });
            }
            ffi::NodeKind::Special => {
                return Err(MaterializationError::UnsupportedSourceEntry {
                    path,
                    kind: "special".to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn digest_file(file: OwnedFd, path: &Path) -> Result<(u64, String), MaterializationError> {
    let before = ffi::metadata(&file)
        .map_err(|error| syscall_error("fstat manifest file", path, error, false))?;
    let mut file = File::from(file);
    let mut hasher = blake3::Hasher::new();
    let mut length = 0u64;
    let mut buffer = [0u8; 65_536];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| syscall_error("read manifest file", path, error, false))?;
        match read {
            0 => break,
            read => {
                hasher.update(&buffer[..read]);
                length += read as u64;
            }
        }
    }
    let after = ffi::metadata(&file)
        .map_err(|error| syscall_error("fstat manifest file after read", path, error, false))?;
    if after != before {
        return Err(MaterializationError::SourceChanged {
            path: path.to_path_buf(),
        });
    }
    Ok((length, hasher.finalize().to_hex().to_string()))
}

fn verify_target_empty(target: &OwnedFd, target_path: &Path) -> Result<(), MaterializationError> {
    if let Some(name) = read_names(target, target_path)?.into_iter().next() {
        return Err(MaterializationError::UnknownTargetEntry {
            path: encode_relative(&[name]),
        });
    }
    Ok(())
}

fn apply_rollback_fault(
    prepared: &PreparedAttempt,
    target: &OwnedFd,
    context: &mut ExecutionContext,
    faults: &FaultInjection,
) -> Vec<RollbackProblem> {
    if let Err(problem) = validate_rollback_boundary(prepared, target) {
        return vec![*problem];
    }
    if let Err(problem) = validate_rollback_set(target, &context.created) {
        return vec![*problem];
    }
    let mut problems = Vec::new();
    match faults.rollback {
        RollbackFault::AddUnknownEntry => {
            let component = ComponentName::new(OsStr::new("p0-02-injected-unknown"))
                .expect("static component is valid");
            match ffi::create_file_at(target, component.as_c_str(), 0o600) {
                Ok(file) => {
                    if let Err(error) = ffi::metadata(&file) {
                        problems.push(rollback_problem(
                            "fstat injected unknown entry",
                            &[OsString::from("p0-02-injected-unknown")],
                            error,
                        ));
                    }
                }
                Err(error) => problems.push(rollback_problem(
                    "create injected unknown entry",
                    &[OsString::from("p0-02-injected-unknown")],
                    error,
                )),
            }
        }
        RollbackFault::ReplaceCreated { created_index } => {
            let Some(created) = context.created.get(created_index) else {
                problems.push(injected_rollback_problem(
                    "select created entry for replacement",
                    None,
                    "created_index is outside the attempt ledger",
                ));
                return problems;
            };
            let Some(expected) = created.identity else {
                problems.push(injected_rollback_problem(
                    "select created identity for replacement",
                    Some(encode_relative(&created.relative)),
                    "created identity was not confirmed",
                ));
                return problems;
            };
            let (parent, component) = match open_relative_parent(target, &created.relative) {
                Ok(value) => value,
                Err(error) => {
                    problems.push(rollback_problem(
                        "openat injected replacement parent",
                        &created.relative,
                        error,
                    ));
                    return problems;
                }
            };
            let current = match ffi::metadata_at(&parent, component.as_c_str()) {
                Ok(metadata) => metadata,
                Err(error) => {
                    problems.push(rollback_problem(
                        "fstatat injected replacement source",
                        &created.relative,
                        error,
                    ));
                    return problems;
                }
            };
            if let Some(problem) = validate_rollback_parent(&parent, created, &context.created) {
                problems.push(problem);
                return problems;
            }
            if !same_identity_and_kind(current, expected, created.kind) {
                problems.push(RollbackProblem {
                    operation: "revalidate injected replacement source".to_owned(),
                    path: Some(encode_relative(&created.relative)),
                    errno: None,
                    expected_identity: Some(expected),
                    observed_identity: Some(identity(current)),
                    detail: "created entry changed before replacement injection".to_owned(),
                });
                return problems;
            }
            if let Err(error) = ffi::remove_at(&parent, component.as_c_str(), current.kind) {
                problems.push(rollback_problem(
                    "unlinkat injected replacement source",
                    &created.relative,
                    error,
                ));
                return problems;
            }
            match ffi::create_file_at(&parent, component.as_c_str(), 0o600) {
                Ok(file) => {
                    if let Err(error) = ffi::metadata(&file) {
                        problems.push(rollback_problem(
                            "fstat injected replacement",
                            &created.relative,
                            error,
                        ));
                    }
                }
                Err(error) => problems.push(rollback_problem(
                    "create injected replacement",
                    &created.relative,
                    error,
                )),
            }
        }
        RollbackFault::None | RollbackFault::FailDeletion { .. } => {}
    }
    problems
}

fn preserve_after_root_metadata_failure(
    prepared: &PreparedAttempt,
    target: &OwnedFd,
    context: &ExecutionContext,
) -> RollbackEvidence {
    let observed_identity = ffi::metadata(target).ok().map(identity);
    RollbackEvidence {
        status: RollbackStatus::Incomplete,
        removed: Vec::new(),
        remaining: context
            .created
            .iter()
            .map(|entry| encode_relative(&entry.relative))
            .collect(),
        problems: vec![RollbackProblem {
            operation: "preserve target after root metadata stage failure".to_owned(),
            path: None,
            errno: None,
            expected_identity: Some(identity(prepared.target_baseline)),
            observed_identity,
            detail: "target root metadata may already have changed; automatic rollback is intentionally stopped"
                .to_owned(),
        }],
    }
}

fn rollback_created(
    prepared: &PreparedAttempt,
    target: &OwnedFd,
    created: &[TrackedCreated],
    rollback_fault: RollbackFault,
    mut problems: Vec<RollbackProblem>,
) -> RollbackEvidence {
    let mut removed = Vec::new();
    let mut remaining: BTreeSet<_> = created
        .iter()
        .map(|entry| encode_relative(&entry.relative))
        .collect();
    let mut active_count = created.len();
    for (reverse_index, entry) in created.iter().rev().enumerate() {
        if !problems.is_empty() {
            break;
        }
        if let Err(problem) = validate_rollback_boundary(prepared, target) {
            problems.push(*problem);
            break;
        }
        if let Err(problem) = validate_rollback_set(target, &created[..active_count]) {
            problems.push(*problem);
            break;
        }
        if let RollbackFault::FailDeletion {
            reverse_index: at,
            errno,
        } = rollback_fault
            && at == reverse_index
        {
            problems.push(RollbackProblem {
                operation: "unlinkat injected rollback failure".to_owned(),
                path: Some(encode_relative(&entry.relative)),
                errno: Some(errno),
                expected_identity: entry.identity,
                observed_identity: entry.identity,
                detail: "fault injection stopped rollback before deletion".to_owned(),
            });
            break;
        }
        let Some(expected) = entry.identity else {
            problems.push(RollbackProblem {
                operation: "revalidate created identity".to_owned(),
                path: Some(encode_relative(&entry.relative)),
                errno: None,
                expected_identity: None,
                observed_identity: None,
                detail: "created identity was never confirmed".to_owned(),
            });
            break;
        };
        let (parent, component) = match open_relative_parent(target, &entry.relative) {
            Ok(value) => value,
            Err(error) => {
                problems.push(rollback_problem(
                    "openat rollback parent",
                    &entry.relative,
                    error,
                ));
                break;
            }
        };
        let current = match ffi::metadata_at(&parent, component.as_c_str()) {
            Ok(metadata) => metadata,
            Err(error) => {
                problems.push(rollback_problem(
                    "fstatat rollback entry",
                    &entry.relative,
                    error,
                ));
                break;
            }
        };
        if let Some(problem) = validate_rollback_parent(&parent, entry, created) {
            problems.push(problem);
            break;
        }
        if !same_identity_and_kind(current, expected, entry.kind) {
            problems.push(RollbackProblem {
                operation: "revalidate created identity".to_owned(),
                path: Some(encode_relative(&entry.relative)),
                errno: None,
                expected_identity: Some(expected),
                observed_identity: Some(identity(current)),
                detail: "created entry identity or type changed".to_owned(),
            });
            break;
        }
        if let Err(error) = ffi::remove_at(&parent, component.as_c_str(), entry.kind) {
            problems.push(rollback_problem(
                "unlinkat rollback entry",
                &entry.relative,
                error,
            ));
            break;
        }
        let path = encode_relative(&entry.relative);
        remaining.remove(&path);
        removed.push(path);
        active_count -= 1;
    }

    if problems.is_empty() {
        if let Err(problem) = validate_rollback_boundary(prepared, target) {
            problems.push(*problem);
        } else if let Err(problem) = validate_rollback_set(target, &created[..active_count]) {
            problems.push(*problem);
        } else if let Err(problem) = restore_or_verify_target_root_after_rollback(
            target,
            prepared.target_baseline,
            !created.is_empty(),
        ) {
            problems.push(*problem);
        }
    }
    RollbackEvidence {
        status: if problems.is_empty() {
            RollbackStatus::ConfirmedBaseline
        } else {
            RollbackStatus::Incomplete
        },
        removed,
        remaining: remaining.into_iter().collect(),
        problems,
    }
}

fn restore_or_verify_target_root_after_rollback(
    target: &OwnedFd,
    baseline: ffi::NodeMetadata,
    restore_mtime: bool,
) -> Result<(), Box<RollbackProblem>> {
    let before = ffi::metadata(target).map_err(|error| {
        Box::new(RollbackProblem {
            operation: "fstat target root after rollback".to_owned(),
            path: None,
            errno: error.raw_os_error(),
            expected_identity: Some(identity(baseline)),
            observed_identity: None,
            detail: error.to_string(),
        })
    })?;
    if !same_identity_and_kind(before, identity(baseline), ffi::NodeKind::Directory)
        || before.mode != baseline.mode
    {
        return Err(Box::new(RollbackProblem {
            operation: "compare target root before mtime restoration".to_owned(),
            path: None,
            errno: None,
            expected_identity: Some(identity(baseline)),
            observed_identity: Some(identity(before)),
            detail: "target root identity, type, or permissions changed during the attempt"
                .to_owned(),
        }));
    }
    if restore_mtime {
        ffi::set_modified_time(
            target,
            baseline.modified_seconds,
            baseline.modified_nanoseconds,
        )
        .map_err(|error| {
            Box::new(RollbackProblem {
                operation: "restore target root mtime after rollback".to_owned(),
                path: None,
                errno: error.raw_os_error(),
                expected_identity: Some(identity(baseline)),
                observed_identity: Some(identity(before)),
                detail: error.to_string(),
            })
        })?;
    }
    let after = ffi::metadata(target).map_err(|error| {
        Box::new(RollbackProblem {
            operation: "fstat restored target root".to_owned(),
            path: None,
            errno: error.raw_os_error(),
            expected_identity: Some(identity(baseline)),
            observed_identity: None,
            detail: error.to_string(),
        })
    })?;
    if same_preserved_root_metadata(after, baseline) {
        Ok(())
    } else {
        Err(Box::new(RollbackProblem {
            operation: "compare restored target root metadata".to_owned(),
            path: None,
            errno: None,
            expected_identity: Some(identity(baseline)),
            observed_identity: Some(identity(after)),
            detail: "target root mtime or permissions do not match the prepared baseline"
                .to_owned(),
        }))
    }
}

fn validate_rollback_set(
    target: &OwnedFd,
    created: &[TrackedCreated],
) -> Result<(), Box<RollbackProblem>> {
    if created.iter().any(|entry| entry.identity.is_none()) {
        return Err(Box::new(RollbackProblem {
            operation: "revalidate created identity".to_owned(),
            path: created
                .iter()
                .find(|entry| entry.identity.is_none())
                .map(|entry| encode_relative(&entry.relative)),
            errno: None,
            expected_identity: None,
            observed_identity: None,
            detail: "created identity was never confirmed".to_owned(),
        }));
    }

    let target_device = ffi::metadata(target)
        .map_err(|error| {
            Box::new(RollbackProblem {
                operation: "fstat rollback target root".to_owned(),
                path: None,
                errno: error.raw_os_error(),
                expected_identity: None,
                observed_identity: None,
                detail: error.to_string(),
            })
        })?
        .device;
    let actual = observe_target_entries(target, &[], target_device).map_err(|error| {
        Box::new(RollbackProblem {
            operation: "enumerate rollback target".to_owned(),
            path: None,
            errno: materialization_errno(&error),
            expected_identity: None,
            observed_identity: None,
            detail: error.to_string(),
        })
    })?;
    let expected: BTreeMap<_, _> = created
        .iter()
        .map(|entry| {
            (
                encode_relative(&entry.relative),
                (entry.identity.expect("checked above"), entry.kind),
            )
        })
        .collect();
    if actual != expected {
        let changed = actual
            .keys()
            .chain(expected.keys())
            .find(|path| actual.get(*path) != expected.get(*path))
            .cloned();
        let expected_identity = changed
            .as_ref()
            .and_then(|path| expected.get(path).map(|(identity, _)| *identity));
        let observed_identity = changed
            .as_ref()
            .and_then(|path| actual.get(path).map(|(identity, _)| *identity));
        return Err(Box::new(RollbackProblem {
            operation: "compare rollback target set".to_owned(),
            path: changed,
            errno: None,
            expected_identity,
            observed_identity,
            detail: "target contains an unknown, missing, or identity-replaced entry".to_owned(),
        }));
    }
    Ok(())
}

fn validate_rollback_boundary(
    prepared: &PreparedAttempt,
    held_target: &OwnedFd,
) -> Result<(), Box<RollbackProblem>> {
    let fresh = revalidate_and_open(prepared).map_err(|error| {
        Box::new(RollbackProblem {
            operation: "revalidate four paths before rollback step".to_owned(),
            path: None,
            errno: materialization_errno(&error),
            expected_identity: None,
            observed_identity: None,
            detail: error.to_string(),
        })
    })?;
    let expected = ffi::metadata(held_target).map_err(|error| {
        Box::new(RollbackProblem {
            operation: "fstat held rollback target".to_owned(),
            path: None,
            errno: error.raw_os_error(),
            expected_identity: None,
            observed_identity: None,
            detail: error.to_string(),
        })
    })?;
    let observed = ffi::metadata(&fresh.target).map_err(|error| {
        Box::new(RollbackProblem {
            operation: "fstat revalidated rollback target".to_owned(),
            path: None,
            errno: error.raw_os_error(),
            expected_identity: Some(identity(expected)),
            observed_identity: None,
            detail: error.to_string(),
        })
    })?;
    if same_identity_and_kind(observed, identity(expected), expected.kind) {
        Ok(())
    } else {
        Err(Box::new(RollbackProblem {
            operation: "compare rollback target root identity".to_owned(),
            path: None,
            errno: None,
            expected_identity: Some(identity(expected)),
            observed_identity: Some(identity(observed)),
            detail: "target root changed before rollback step".to_owned(),
        }))
    }
}

fn validate_rollback_parent(
    parent: &OwnedFd,
    entry: &TrackedCreated,
    created: &[TrackedCreated],
) -> Option<RollbackProblem> {
    let parent_relative = entry
        .relative
        .get(..entry.relative.len().saturating_sub(1))?;
    if parent_relative.is_empty() {
        return None;
    }
    let expected = created
        .iter()
        .find(|candidate| candidate.relative == parent_relative)
        .and_then(|candidate| candidate.identity);
    let observed = match ffi::metadata(parent) {
        Ok(metadata) => metadata,
        Err(error) => {
            return Some(rollback_problem(
                "fstat rollback parent",
                parent_relative,
                error,
            ));
        }
    };
    match expected {
        Some(expected) if same_identity_and_kind(observed, expected, ffi::NodeKind::Directory) => {
            None
        }
        _ => Some(RollbackProblem {
            operation: "compare rollback parent identity".to_owned(),
            path: Some(encode_relative(parent_relative)),
            errno: None,
            expected_identity: expected,
            observed_identity: Some(identity(observed)),
            detail: "created parent directory changed before child deletion".to_owned(),
        }),
    }
}

fn observe_target_entries(
    directory: &OwnedFd,
    relative: &[OsString],
    root_device: u64,
) -> Result<BTreeMap<EncodedRelativePath, (FileIdentity, ffi::NodeKind)>, MaterializationError> {
    let mut observed = BTreeMap::new();
    let mut names = read_names(directory, Path::new("rollback-target"))?;
    names.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    for name in names {
        let component = ComponentName::new(&name)?;
        let mut child = relative.to_vec();
        child.push(name);
        let metadata = ffi::metadata_at(directory, component.as_c_str()).map_err(|error| {
            syscall_error(
                "fstatat rollback target",
                Path::new("rollback-target"),
                error,
                false,
            )
        })?;
        if metadata.device != root_device {
            return Err(MaterializationError::UnsupportedSourceEntry {
                path: PathBuf::from_iter(&child),
                kind: "submount".to_owned(),
            });
        }
        observed.insert(encode_relative(&child), (identity(metadata), metadata.kind));
        if metadata.kind == ffi::NodeKind::Directory {
            let child_fd =
                ffi::open_directory_at(directory, component.as_c_str()).map_err(|error| {
                    syscall_error(
                        "openat rollback directory",
                        Path::new("rollback-target"),
                        error,
                        false,
                    )
                })?;
            observed.extend(observe_target_entries(&child_fd, &child, root_device)?);
        }
    }
    Ok(observed)
}

fn open_relative_parent(
    root: &OwnedFd,
    relative: &[OsString],
) -> io::Result<(OwnedFd, ComponentName)> {
    let (name, parents) = relative
        .split_last()
        .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?;
    let mut current = root.try_clone()?;
    for parent in parents {
        let component = ComponentName::new_io(parent)?;
        current = ffi::open_directory_at(&current, component.as_c_str())?;
    }
    Ok((current, ComponentName::new_io(name)?))
}

fn open_verified_directory(
    path: &Path,
    report: &PathCapabilityReport,
) -> Result<OwnedFd, MaterializationError> {
    validate_probe_path(path).map_err(|source| probe_error("path validation", source))?;
    let mut current = ffi::open_root_directory()
        .map_err(|error| syscall_error("open root directory", Path::new("/"), error, false))?;
    for component in path.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        let component = ComponentName::new(name)?;
        current = ffi::open_directory_at(&current, component.as_c_str())
            .map_err(|error| syscall_error("openat directory", path, error, false))?;
    }

    let metadata = ffi::metadata(&current)
        .map_err(|error| syscall_error("fstat held directory", path, error, false))?;
    let expected = report
        .ancestry
        .last()
        .expect("a successful probe always includes the root directory");
    if metadata.device != expected.identity.device || metadata.inode != expected.identity.inode {
        return Err(MaterializationError::PlanStale {
            path: path.to_path_buf(),
        });
    }
    Ok(current)
}

fn read_names(directory: &OwnedFd, path: &Path) -> Result<Vec<OsString>, MaterializationError> {
    ffi::read_directory(directory)
        .map_err(|error| syscall_error("readdir held directory", path, error, false))
}

fn candidate(backend: Backend) -> MaterializerCandidate {
    match backend {
        Backend::ApfsFileClone => MaterializerCandidate::ApfsFileClone,
        Backend::FullCopy => MaterializerCandidate::FullCopy,
    }
}

fn roots_overlap(source: &PathCapabilityReport, target: &PathCapabilityReport) -> bool {
    let Some(source_root) = source.ancestry.last().map(|entry| entry.identity) else {
        return true;
    };
    let Some(target_root) = target.ancestry.last().map(|entry| entry.identity) else {
        return true;
    };
    target
        .ancestry
        .iter()
        .any(|entry| entry.identity == source_root)
        || source
            .ancestry
            .iter()
            .any(|entry| entry.identity == target_root)
}

fn manifest_kind(kind: ffi::NodeKind) -> ManifestEntryKind {
    match kind {
        ffi::NodeKind::Directory => ManifestEntryKind::Directory,
        ffi::NodeKind::RegularFile => ManifestEntryKind::RegularFile,
        ffi::NodeKind::SymbolicLink => ManifestEntryKind::SymbolicLink,
        ffi::NodeKind::Special => unreachable!("special entries are rejected before registration"),
    }
}

fn identity(metadata: ffi::NodeMetadata) -> FileIdentity {
    FileIdentity {
        device: metadata.device,
        inode: metadata.inode,
    }
}

fn modified_time(metadata: ffi::NodeMetadata) -> ModifiedTime {
    ModifiedTime {
        seconds: metadata.modified_seconds,
        nanoseconds: metadata.modified_nanoseconds,
    }
}

fn same_preserved_root_metadata(observed: ffi::NodeMetadata, expected: ffi::NodeMetadata) -> bool {
    same_identity_and_kind(observed, identity(expected), ffi::NodeKind::Directory)
        && observed.mode == expected.mode
        && modified_time(observed) == modified_time(expected)
}

fn has_preserved_metadata(observed: ffi::NodeMetadata, source: ffi::NodeMetadata) -> bool {
    observed.kind == source.kind
        && observed.mode == source.mode
        && modified_time(observed) == modified_time(source)
}

fn same_identity_and_kind(
    observed: ffi::NodeMetadata,
    expected_identity: FileIdentity,
    expected_kind: ffi::NodeKind,
) -> bool {
    identity(observed) == expected_identity && observed.kind == expected_kind
}

fn join_relative(root: &Path, relative: &[OsString]) -> PathBuf {
    relative
        .iter()
        .fold(root.to_path_buf(), |path, name| path.join(name))
}

fn encode_relative(relative: &[OsString]) -> EncodedRelativePath {
    let mut bytes = Vec::new();
    for (index, component) in relative.iter().enumerate() {
        if index > 0 {
            bytes.push(b'/');
        }
        bytes.extend_from_slice(component.as_bytes());
    }
    EncodedRelativePath {
        display: PathBuf::from(OsString::from_vec(bytes.clone()))
            .to_string_lossy()
            .into_owned(),
        bytes_hex: encode_hex(&bytes),
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn probe_error(stage: &str, source: ProbeError) -> MaterializationError {
    MaterializationError::Probe {
        stage: stage.to_owned(),
        source: Box::new(source),
    }
}

fn syscall_error(
    operation: impl Into<SystemOperation>,
    path: &Path,
    error: io::Error,
    injected: bool,
) -> MaterializationError {
    MaterializationError::SystemCall {
        operation: operation.into(),
        path: path.to_path_buf(),
        errno: error.raw_os_error(),
        message: error.to_string(),
        injected,
    }
}

fn injected_syscall_error(
    operation: impl Into<SystemOperation>,
    path: &Path,
    errno: i32,
) -> MaterializationError {
    syscall_error(operation, path, io::Error::from_raw_os_error(errno), true)
}

fn materialization_errno(error: &MaterializationError) -> Option<i32> {
    match error {
        MaterializationError::SystemCall { errno, .. } => *errno,
        MaterializationError::Probe { source, .. } => match source.as_ref() {
            ProbeError::SystemCall { errno, .. } => *errno,
            _ => None,
        },
        _ => None,
    }
}

fn rollback_problem(operation: &str, path: &[OsString], error: io::Error) -> RollbackProblem {
    RollbackProblem {
        operation: operation.to_owned(),
        path: Some(encode_relative(path)),
        errno: error.raw_os_error(),
        expected_identity: None,
        observed_identity: None,
        detail: error.to_string(),
    }
}

fn injected_rollback_problem(
    operation: &str,
    path: Option<EncodedRelativePath>,
    detail: &str,
) -> RollbackProblem {
    RollbackProblem {
        operation: operation.to_owned(),
        path,
        errno: None,
        expected_identity: None,
        observed_identity: None,
        detail: detail.to_owned(),
    }
}

fn empty_failed_attempt(backend: Backend, error: MaterializationError) -> AttemptEvidence {
    AttemptEvidence {
        backend,
        outcome: AttemptOutcome::Failed,
        cow_evidence: CowEvidence::Unknown,
        clone_calls_succeeded: 0,
        ordinary_files_materialized: 0,
        target_root_metadata_started: false,
        created: Vec::new(),
        ordinary_files: Vec::new(),
        source_manifest: None,
        target_manifest: None,
        failure: Some(error),
        rollback: RollbackEvidence {
            status: RollbackStatus::NotNeeded,
            removed: Vec::new(),
            remaining: Vec::new(),
            problems: Vec::new(),
        },
    }
}

#[derive(Clone)]
struct OwnedRequest {
    source: PathBuf,
    target: PathBuf,
    staging: PathBuf,
    trash: PathBuf,
}

struct OpenedRoots {
    source: OwnedFd,
    target: OwnedFd,
    staging: OwnedFd,
    trash: OwnedFd,
}

impl From<&MaterializeRequest<'_>> for OwnedRequest {
    fn from(request: &MaterializeRequest<'_>) -> Self {
        Self {
            source: request.source.to_path_buf(),
            target: request.target.to_path_buf(),
            staging: request.staging.to_path_buf(),
            trash: request.trash.to_path_buf(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TreeSnapshot {
    manifest: TreeManifest,
    identities: BTreeMap<EncodedRelativePath, FileIdentity>,
    root_metadata: ffi::NodeMetadata,
}

struct TrackedCreated {
    relative: Vec<OsString>,
    kind: ffi::NodeKind,
    identity: Option<FileIdentity>,
}

struct ExecutionContext {
    backend: Backend,
    clone_calls_succeeded: usize,
    ordinary_files_materialized: usize,
    metadata_updates_started: usize,
    target_root_metadata_started: bool,
    created: Vec<TrackedCreated>,
    ordinary_files: Vec<OrdinaryFileEvidence>,
}

impl ExecutionContext {
    fn new(backend: Backend) -> Self {
        Self {
            backend,
            clone_calls_succeeded: 0,
            ordinary_files_materialized: 0,
            metadata_updates_started: 0,
            target_root_metadata_started: false,
            created: Vec::new(),
            ordinary_files: Vec::new(),
        }
    }

    fn into_evidence(
        self,
        outcome: AttemptOutcome,
        cow_evidence: CowEvidence,
        failure: Option<MaterializationError>,
        rollback: RollbackEvidence,
        source_manifest: Option<TreeManifest>,
        target_manifest: Option<TreeManifest>,
    ) -> AttemptEvidence {
        AttemptEvidence {
            backend: self.backend,
            outcome,
            cow_evidence,
            clone_calls_succeeded: self.clone_calls_succeeded,
            ordinary_files_materialized: self.ordinary_files_materialized,
            target_root_metadata_started: self.target_root_metadata_started,
            created: self
                .created
                .into_iter()
                .map(|entry| CreatedObjectEvidence {
                    path: encode_relative(&entry.relative),
                    kind: manifest_kind(entry.kind),
                    identity: entry
                        .identity
                        .map_or(CreatedIdentity::Unconfirmed, CreatedIdentity::Confirmed),
                })
                .collect(),
            ordinary_files: self.ordinary_files,
            source_manifest,
            target_manifest,
            failure,
            rollback,
        }
    }

    fn matches_target_snapshot(
        &self,
        source_manifest: &TreeManifest,
        observed: &TreeSnapshot,
    ) -> bool {
        if &observed.manifest != source_manifest || observed.identities.len() != self.created.len()
        {
            return false;
        }
        self.created.iter().all(|entry| {
            entry.identity.is_some_and(|expected| {
                observed.identities.get(&encode_relative(&entry.relative)) == Some(&expected)
            })
        })
    }
}

struct ComponentName(CString);

impl ComponentName {
    fn new(name: &OsStr) -> Result<Self, MaterializationError> {
        Self::new_io(name).map_err(|error| {
            syscall_error("validate path component", Path::new(name), error, false)
        })
    }

    fn new_io(name: &OsStr) -> io::Result<Self> {
        let bytes = name.as_bytes();
        if bytes.is_empty() || bytes == b"." || bytes == b".." || bytes.contains(&b'/') {
            return Err(io::Error::from_raw_os_error(libc::EINVAL));
        }
        CString::new(bytes)
            .map(Self)
            .map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))
    }

    fn as_c_str(&self) -> &CStr {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::ffi::{OsStr, OsString};
    use std::fs;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    use std::path::{Path, PathBuf};

    use rustix::fs::{Mode, OFlags};
    use thinws_p0_probe::{
        AccessPreflight, Evidence, EvidenceSource, EvidenceStatement, FileIdentity,
        MaterializationPathProbeRequest, MaterializationPathReport, MaterializerCandidate,
        MountWriteState, ProbeError, RevalidationChange, SupportState,
        inspect_materialization_paths, inspect_path,
    };

    use super::{
        Backend, ClonePreflightOverride, ComponentName, CopyWriteFault, ExecutionContext,
        FallbackPolicy, FaultInjection, ManifestEntry, ManifestEntryKind, MaterializationError,
        MaterializeRequest, ModifiedTime, PolicyExperimentOptions, RequestedMode, RollbackFault,
        RollbackStatus, SystemOperation, TrackedCreated, TreeManifest, TreeSnapshot,
        apply_rollback_fault, clone_layout_preconditions_satisfied, copy_file_bytes, digest_file,
        encode_relative, fallback_path_evidence_matches, has_preserved_metadata, identity,
        is_expected_target_writability_change, materialization_errno, modified_time,
        mutate_source_same_inode, open_verified_directory, prepare_attempt, register_created,
        restore_or_verify_target_root_after_rollback, revalidate_and_open, revalidate_held_roots,
        revalidate_held_roots_after_target_metadata, rollback_created, run_cow_policy_experiment,
        same_identity_and_kind, same_preserved_root_metadata, set_preserved_metadata,
        validate_rollback_parent, verify_target_empty,
    };
    use crate::test_support::ControlledTree;

    fn controlled_materialization_roots(
        label: &str,
    ) -> (ControlledTree, PathBuf, PathBuf, PathBuf, PathBuf) {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("experiment crate is below repository root");
        let parent_path = repository_root.join("target");
        let mut fixture =
            ControlledTree::create_in(&parent_path, label).expect("create controlled fixture");
        for directory in ["source", "target", "staging", "trash"] {
            fixture
                .create_directory(Path::new(directory), 0o700)
                .expect("create materialization root");
        }
        let source = fixture.root_path().join("source");
        let target = fixture.root_path().join("target");
        let staging = fixture.root_path().join("staging");
        let trash = fixture.root_path().join("trash");
        (fixture, source, target, staging, trash)
    }

    fn candidate_evidence_mut(
        prepared: &mut super::PreparedAttempt,
        candidate: MaterializerCandidate,
    ) -> &mut thinws_p0_probe::CandidateEvidence {
        prepared
            .report
            .candidates
            .iter_mut()
            .find(|evidence| evidence.candidate == candidate)
            .expect("requested candidate evidence exists")
    }

    #[test]
    fn structured_private_helpers_preserve_exact_values_and_component_boundaries() {
        assert_eq!(SystemOperation::CloneFileAt.to_string(), "fclonefileat");
        assert_eq!(
            SystemOperation::Named("write target file".to_owned()).to_string(),
            "write target file"
        );

        let direct = MaterializationError::SystemCall {
            operation: SystemOperation::CloneFileAt,
            path: PathBuf::from("/direct"),
            errno: Some(libc::EXDEV),
            message: "cross-volume".to_owned(),
            injected: false,
        };
        assert_eq!(materialization_errno(&direct), Some(libc::EXDEV));
        let no_errno = MaterializationError::SystemCall {
            operation: SystemOperation::Named("write".to_owned()),
            path: PathBuf::from("/no-errno"),
            errno: None,
            message: "unknown".to_owned(),
            injected: false,
        };
        assert_eq!(materialization_errno(&no_errno), None);
        let probe = MaterializationError::Probe {
            stage: "test".to_owned(),
            source: Box::new(ProbeError::SystemCall {
                operation: "openat".to_owned(),
                path: None,
                errno: Some(libc::EACCES),
                message: "denied".to_owned(),
            }),
        };
        assert_eq!(materialization_errno(&probe), Some(libc::EACCES));
        let probe_without_errno = MaterializationError::Probe {
            stage: "test".to_owned(),
            source: Box::new(ProbeError::PathMustBeAbsolute),
        };
        assert_eq!(materialization_errno(&probe_without_errno), None);
        assert_eq!(
            materialization_errno(&MaterializationError::ManifestMismatch),
            None
        );

        for invalid in [
            b"".as_slice(),
            b".".as_slice(),
            b"..".as_slice(),
            b"left/right".as_slice(),
            b"nul\0byte".as_slice(),
        ] {
            let error = match ComponentName::new_io(OsStr::from_bytes(invalid)) {
                Ok(_) => panic!("invalid component must be rejected"),
                Err(error) => error,
            };
            assert_eq!(error.raw_os_error(), Some(libc::EINVAL));
        }
        assert_eq!(
            ComponentName::new_io(OsStr::from_bytes(b"valid"))
                .expect("ordinary component is valid")
                .as_c_str()
                .to_bytes(),
            b"valid"
        );
    }

    #[test]
    fn metadata_identity_mismatch_does_not_change_mode_or_mtime() {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("experiment crate is below repository root");
        let mut fixture = ControlledTree::create_in(
            &repository_root.join("target"),
            "metadata-held-identity-mismatch",
        )
        .expect("create controlled metadata fixture");
        fixture
            .create_file(Path::new("expected"), b"expected", 0o400)
            .expect("create registered-identity file");
        fixture
            .create_file(Path::new("wrong-held"), b"wrong", 0o640)
            .expect("create wrong held file");
        fixture
            .set_modified_time(Path::new("expected"), 1_650_000_001, 123_456_789)
            .expect("set expected metadata");
        fixture
            .set_modified_time(Path::new("wrong-held"), 1_660_000_002, 234_567_890)
            .expect("set wrong-held metadata");
        let expected = rustix::fs::openat(
            fixture.root_fd(),
            "expected",
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .expect("open expected file");
        let wrong_held = rustix::fs::openat(
            fixture.root_fd(),
            "wrong-held",
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .expect("open wrong held file");
        let source_metadata = crate::ffi::metadata(&expected).expect("observe source metadata");
        let expected_identity = identity(source_metadata);
        let before = crate::ffi::metadata(&wrong_held).expect("observe wrong held before call");
        let mut context = ExecutionContext::new(Backend::ApfsFileClone);

        let result = set_preserved_metadata(
            &wrong_held,
            expected_identity,
            source_metadata,
            &fixture.root_path().join("wrong-held"),
            &FaultInjection::default(),
            &mut context,
            false,
        );
        let after = crate::ffi::metadata(&wrong_held).expect("observe wrong held after call");

        drop((wrong_held, expected));
        fixture
            .cleanup()
            .expect("remove controlled metadata fixture");
        assert!(matches!(
            result,
            Err(MaterializationError::TargetChanged { .. })
        ));
        assert_eq!(after.mode, before.mode);
        assert_eq!(modified_time(after), modified_time(before));
        assert_eq!(context.metadata_updates_started, 0);
    }

    #[test]
    fn final_target_writability_exception_rejects_unknown_and_unexplained_mode_changes() {
        let (fixture, source, target, staging, trash) =
            controlled_materialization_roots("final-writability-exception");
        let prepared = prepare_attempt(
            &MaterializeRequest {
                source: &source,
                target: &target,
                staging: &staging,
                trash: &trash,
            },
            Backend::FullCopy,
        )
        .expect("prepare target writability evidence");
        let before = prepared.report.target_root.writability.clone();
        let source = match &before.effective_access_preflight {
            AccessPreflight::Allowed { source } => source.clone(),
            other => panic!("controlled target must initially be writable: {other:?}"),
        };
        let mut denied = before.clone();
        denied.effective_access_preflight = AccessPreflight::Denied {
            errno: Some(libc::EACCES),
            reason: "synthetic chmod result".to_owned(),
            source: source.clone(),
        };
        let denied_change = RevalidationChange::WritabilityChanged {
            before: before.clone(),
            after: denied,
        };
        let mut unknown = before.clone();
        unknown.effective_access_preflight = AccessPreflight::Unknown {
            errno: Some(libc::EIO),
            reason: "synthetic uncertainty".to_owned(),
            source,
        };
        let unknown_change = RevalidationChange::WritabilityChanged {
            before,
            after: unknown,
        };

        let expected = is_expected_target_writability_change(&denied_change, 0o700, 0o555);
        let mode_unchanged = is_expected_target_writability_change(&denied_change, 0o700, 0o700);
        let write_bits_unchanged =
            is_expected_target_writability_change(&denied_change, 0o700, 0o710);
        let only_group_write_removed =
            is_expected_target_writability_change(&denied_change, 0o770, 0o750);
        let owner_write_was_already_absent =
            is_expected_target_writability_change(&denied_change, 0o555, 0o555);
        let uncertain = is_expected_target_writability_change(&unknown_change, 0o700, 0o555);

        fixture.cleanup().expect("remove controlled probe fixture");
        assert!(expected);
        assert!(!mode_unchanged);
        assert!(!write_bits_unchanged);
        assert!(!only_group_write_removed);
        assert!(!owner_write_was_already_absent);
        assert!(!uncertain);
    }

    #[test]
    fn final_target_writability_exception_requires_each_explaining_fact() {
        let (fixture, source, target, staging, trash) =
            controlled_materialization_roots("final-writability-facts");
        let prepared = prepare_attempt(
            &MaterializeRequest {
                source: &source,
                target: &target,
                staging: &staging,
                trash: &trash,
            },
            Backend::FullCopy,
        )
        .expect("prepare target writability evidence");
        let before = prepared.report.target_root.writability.clone();
        let evidence_source = match &before.effective_access_preflight {
            AccessPreflight::Allowed { source } => source.clone(),
            other => panic!("controlled target must initially be writable: {other:?}"),
        };
        let mut denied = before.clone();
        denied.effective_access_preflight = AccessPreflight::Denied {
            errno: Some(libc::EACCES),
            reason: "synthetic chmod result".to_owned(),
            source: evidence_source.clone(),
        };
        let expected = RevalidationChange::WritabilityChanged {
            before: before.clone(),
            after: denied.clone(),
        };

        let mut different_mount = denied.clone();
        different_mount.mount_state = match before.mount_state {
            MountWriteState::WritableAtInspection => MountWriteState::ReadOnly,
            MountWriteState::ReadOnly => MountWriteState::WritableAtInspection,
        };
        let mut wrong_errno = denied.clone();
        wrong_errno.effective_access_preflight = AccessPreflight::Denied {
            errno: Some(libc::EPERM),
            reason: "different denial".to_owned(),
            source: evidence_source.clone(),
        };
        let mut missing_errno = denied.clone();
        missing_errno.effective_access_preflight = AccessPreflight::Denied {
            errno: None,
            reason: "denial without errno".to_owned(),
            source: evidence_source.clone(),
        };
        let mut different_source = denied.clone();
        different_source.effective_access_preflight = AccessPreflight::Denied {
            errno: Some(libc::EACCES),
            reason: "different evidence source".to_owned(),
            source: EvidenceSource::FstatHeldDirectoryFd,
        };
        let before_denied = RevalidationChange::WritabilityChanged {
            before: {
                let mut value = before.clone();
                value.effective_access_preflight = AccessPreflight::Denied {
                    errno: Some(libc::EACCES),
                    reason: "already denied".to_owned(),
                    source: evidence_source,
                };
                value
            },
            after: denied,
        };
        let cases = [
            RevalidationChange::PathRejected {
                error: ProbeError::PathMustBeAbsolute,
            },
            RevalidationChange::WritabilityChanged {
                before: before.clone(),
                after: before,
            },
            RevalidationChange::WritabilityChanged {
                before: match &expected {
                    RevalidationChange::WritabilityChanged { before, .. } => before.clone(),
                    _ => unreachable!(),
                },
                after: different_mount,
            },
            RevalidationChange::WritabilityChanged {
                before: match &expected {
                    RevalidationChange::WritabilityChanged { before, .. } => before.clone(),
                    _ => unreachable!(),
                },
                after: wrong_errno,
            },
            RevalidationChange::WritabilityChanged {
                before: match &expected {
                    RevalidationChange::WritabilityChanged { before, .. } => before.clone(),
                    _ => unreachable!(),
                },
                after: missing_errno,
            },
            RevalidationChange::WritabilityChanged {
                before: match &expected {
                    RevalidationChange::WritabilityChanged { before, .. } => before.clone(),
                    _ => unreachable!(),
                },
                after: different_source,
            },
            before_denied,
        ];

        let expected_result = is_expected_target_writability_change(&expected, 0o700, 0o555);
        let rejected = cases
            .iter()
            .all(|change| !is_expected_target_writability_change(change, 0o700, 0o555));

        fixture.cleanup().expect("remove controlled probe fixture");
        assert!(expected_result);
        assert!(rejected);
    }

    #[test]
    fn final_metadata_revalidation_rejects_each_changed_root() {
        for (index, label) in ["source", "target", "staging", "trash"]
            .into_iter()
            .enumerate()
        {
            let (fixture, source, target, staging, trash) =
                controlled_materialization_roots(&format!("final-root-changed-{label}"));
            let prepared = prepare_attempt(
                &MaterializeRequest {
                    source: &source,
                    target: &target,
                    staging: &staging,
                    trash: &trash,
                },
                Backend::FullCopy,
            )
            .expect("prepare unchanged roots");
            let opened = revalidate_and_open(&prepared).expect("open verified roots");
            let changed = match index {
                0 => &opened.source,
                1 => &opened.target,
                2 => &opened.staging,
                3 => &opened.trash,
                _ => unreachable!(),
            };
            let changed_mode = if index == 0 { 0o100 } else { 0o500 };
            crate::ffi::set_mode(changed, changed_mode).expect("change one held root mode");
            let result = revalidate_held_roots_after_target_metadata(
                &prepared,
                [
                    &opened.source,
                    &opened.target,
                    &opened.staging,
                    &opened.trash,
                ],
                prepared.target_baseline,
            );
            crate::ffi::set_mode(changed, 0o700).expect("restore controlled root mode");

            drop(opened);
            fixture.cleanup().expect("remove controlled root fixture");
            assert!(matches!(
                result,
                Err(MaterializationError::PlanStale { ref path })
                    if path == [&source, &target, &staging, &trash][index]
            ));
        }
    }

    #[test]
    fn final_metadata_revalidation_rejects_a_wrong_held_root_identity() {
        let (fixture, source, target, staging, trash) =
            controlled_materialization_roots("final-held-identity");
        let prepared = prepare_attempt(
            &MaterializeRequest {
                source: &source,
                target: &target,
                staging: &staging,
                trash: &trash,
            },
            Backend::FullCopy,
        )
        .expect("prepare unchanged roots");
        let opened = revalidate_and_open(&prepared).expect("open verified roots");
        let result = revalidate_held_roots_after_target_metadata(
            &prepared,
            [
                &opened.staging,
                &opened.target,
                &opened.staging,
                &opened.trash,
            ],
            prepared.target_baseline,
        );

        drop(opened);
        fixture.cleanup().expect("remove controlled root fixture");
        assert!(matches!(
            result,
            Err(MaterializationError::PlanStale { ref path }) if path == &source
        ));
    }

    #[test]
    fn final_metadata_revalidation_rejects_wrong_target_mode_and_mtime() {
        for metadata_field in ["mode", "mtime"] {
            let (fixture, source, target, staging, trash) = controlled_materialization_roots(
                &format!("final-target-metadata-{metadata_field}"),
            );
            fixture
                .set_modified_time(Path::new("source"), 1_650_000_001, 123_456_789)
                .expect("set source root mtime");
            fixture
                .set_modified_time(Path::new("target"), 1_660_000_002, 234_567_890)
                .expect("set target root mtime");
            if metadata_field == "mode" {
                fixture
                    .set_modified_time(Path::new("target"), 1_650_000_001, 123_456_789)
                    .expect("align target root mtime");
                fixture
                    .set_mode(Path::new("source"), 0o555)
                    .expect("set source root mode");
            }
            let prepared = prepare_attempt(
                &MaterializeRequest {
                    source: &source,
                    target: &target,
                    staging: &staging,
                    trash: &trash,
                },
                Backend::FullCopy,
            )
            .expect("prepare differing source and target metadata");
            let opened = revalidate_and_open(&prepared).expect("open verified roots");
            let result = revalidate_held_roots_after_target_metadata(
                &prepared,
                [
                    &opened.source,
                    &opened.target,
                    &opened.staging,
                    &opened.trash,
                ],
                prepared.source.root_metadata,
            );
            if metadata_field == "mode" {
                crate::ffi::set_mode(&opened.source, 0o700)
                    .expect("restore controlled source mode");
            }

            drop(opened);
            fixture
                .cleanup()
                .expect("remove controlled metadata fixture");
            assert!(matches!(
                result,
                Err(MaterializationError::ManifestMismatch)
            ));
        }
    }

    #[test]
    fn final_metadata_revalidation_does_not_accept_a_non_target_writability_change() {
        let (fixture, source, target, staging, trash) =
            controlled_materialization_roots("final-non-target-writability");
        let prepared = prepare_attempt(
            &MaterializeRequest {
                source: &source,
                target: &target,
                staging: &staging,
                trash: &trash,
            },
            Backend::FullCopy,
        )
        .expect("prepare writable roots");
        let opened = revalidate_and_open(&prepared).expect("open verified roots");
        crate::ffi::set_mode(&opened.target, 0o555).expect("set expected target root mode");
        crate::ffi::set_mode(&opened.staging, 0o555).expect("change non-target root mode");
        let expected_target =
            crate::ffi::metadata(&opened.target).expect("observe expected target root metadata");
        let result = revalidate_held_roots_after_target_metadata(
            &prepared,
            [
                &opened.source,
                &opened.target,
                &opened.staging,
                &opened.trash,
            ],
            expected_target,
        );
        crate::ffi::set_mode(&opened.target, 0o700).expect("restore controlled target mode");
        crate::ffi::set_mode(&opened.staging, 0o700).expect("restore controlled staging mode");

        drop(opened);
        fixture.cleanup().expect("remove controlled root fixture");
        assert!(matches!(
            result,
            Err(MaterializationError::PlanStale { ref path }) if path == &staging
        ));
    }

    #[test]
    fn root_metadata_rollback_helper_restores_or_rejects_without_side_effects() {
        let (fixture, source, target, staging, trash) =
            controlled_materialization_roots("root-metadata-rollback-helper");
        fixture
            .set_modified_time(Path::new("target"), 1_650_000_001, 123_456_789)
            .expect("set prepared target baseline mtime");
        let prepared = prepare_attempt(
            &MaterializeRequest {
                source: &source,
                target: &target,
                staging: &staging,
                trash: &trash,
            },
            Backend::FullCopy,
        )
        .expect("prepare target baseline");
        let opened = revalidate_and_open(&prepared).expect("open verified roots");

        crate::ffi::set_modified_time(&opened.target, 1_660_000_002, 234_567_890)
            .expect("change target root mtime");
        restore_or_verify_target_root_after_rollback(
            &opened.target,
            prepared.target_baseline,
            true,
        )
        .expect("restore valid target root baseline");
        let restored = crate::ffi::metadata(&opened.target).expect("observe restored target root");
        assert!(same_preserved_root_metadata(
            restored,
            prepared.target_baseline
        ));

        let before_wrong_identity = crate::ffi::metadata(&opened.target)
            .expect("observe target before wrong identity check");
        let wrong_identity =
            crate::ffi::metadata(&opened.staging).expect("observe different controlled directory");
        let wrong_identity_error =
            restore_or_verify_target_root_after_rollback(&opened.target, wrong_identity, true)
                .expect_err("wrong target root identity must be rejected");
        let after_wrong_identity = crate::ffi::metadata(&opened.target)
            .expect("observe target after wrong identity check");

        let mut wrong_mode = prepared.target_baseline;
        wrong_mode.mode = 0o555;
        let before_wrong_mode =
            crate::ffi::metadata(&opened.target).expect("observe target before wrong mode check");
        let wrong_mode_error =
            restore_or_verify_target_root_after_rollback(&opened.target, wrong_mode, true)
                .expect_err("wrong target root mode must be rejected");
        let after_wrong_mode =
            crate::ffi::metadata(&opened.target).expect("observe target after wrong mode check");

        crate::ffi::set_modified_time(&opened.target, 1_670_000_003, 345_678_901)
            .expect("change target root mtime without requesting restoration");
        let before_no_restore =
            crate::ffi::metadata(&opened.target).expect("observe target before verification only");
        let no_restore_error = restore_or_verify_target_root_after_rollback(
            &opened.target,
            prepared.target_baseline,
            false,
        )
        .expect_err("verification-only call must reject an mtime mismatch");
        let after_no_restore =
            crate::ffi::metadata(&opened.target).expect("observe target after verification only");

        drop(opened);
        fixture
            .cleanup()
            .expect("remove controlled rollback fixture");
        assert_eq!(
            wrong_identity_error.operation,
            "compare target root before mtime restoration"
        );
        assert_eq!(before_wrong_identity, after_wrong_identity);
        assert_eq!(
            wrong_mode_error.operation,
            "compare target root before mtime restoration"
        );
        assert_eq!(before_wrong_mode, after_wrong_mode);
        assert_eq!(
            no_restore_error.operation,
            "compare restored target root metadata"
        );
        assert_eq!(before_no_restore, after_no_restore);
    }

    #[test]
    fn preserved_metadata_helpers_compare_each_promised_field() {
        let base = crate::ffi::NodeMetadata {
            device: 17,
            inode: 23,
            kind: crate::ffi::NodeKind::Directory,
            mode: 0o750,
            size: 11,
            modified_seconds: 1_650_000_001,
            modified_nanoseconds: 123_456_789,
        };
        let with = |change: fn(&mut crate::ffi::NodeMetadata)| {
            let mut changed = base;
            change(&mut changed);
            changed
        };
        let changed_device = with(|metadata| metadata.device ^= 1);
        let changed_inode = with(|metadata| metadata.inode ^= 1);
        let changed_kind = with(|metadata| metadata.kind = crate::ffi::NodeKind::RegularFile);
        let changed_mode = with(|metadata| metadata.mode = 0o700);
        let changed_seconds = with(|metadata| metadata.modified_seconds += 1);
        let changed_nanoseconds = with(|metadata| metadata.modified_nanoseconds += 1);

        assert!(same_preserved_root_metadata(base, base));
        for changed in [
            changed_device,
            changed_inode,
            changed_kind,
            changed_mode,
            changed_seconds,
            changed_nanoseconds,
        ] {
            assert!(!same_preserved_root_metadata(changed, base));
        }

        assert!(has_preserved_metadata(base, base));
        assert!(has_preserved_metadata(changed_device, base));
        assert!(has_preserved_metadata(changed_inode, base));
        for changed in [
            changed_kind,
            changed_mode,
            changed_seconds,
            changed_nanoseconds,
        ] {
            assert!(!has_preserved_metadata(changed, base));
        }
    }

    #[test]
    fn candidate_state_selects_only_the_requested_backend_and_defaults_to_unknown() {
        let (fixture, source, target, staging, trash) =
            controlled_materialization_roots("candidate-state-unit");
        let request = MaterializeRequest {
            source: &source,
            target: &target,
            staging: &staging,
            trash: &trash,
        };
        let mut prepared =
            prepare_attempt(&request, Backend::ApfsFileClone).expect("prepare clone candidates");
        candidate_evidence_mut(&mut prepared, MaterializerCandidate::ApfsFileClone).state =
            SupportState::Unsupported;
        candidate_evidence_mut(&mut prepared, MaterializerCandidate::FullCopy).state =
            SupportState::Supported;
        assert_eq!(prepared.candidate_state(), SupportState::Unsupported);

        prepared.backend = Backend::FullCopy;
        assert_eq!(prepared.candidate_state(), SupportState::Supported);
        prepared
            .report
            .candidates
            .retain(|evidence| evidence.candidate != MaterializerCandidate::FullCopy);
        assert_eq!(prepared.candidate_state(), SupportState::Unknown);
        fixture.cleanup().expect("remove verified fixture");
    }

    #[test]
    fn observed_clone_absence_requires_exact_candidate_code_and_valid_copy_layout() {
        let (fixture, source, target, staging, trash) =
            controlled_materialization_roots("preflight-evidence-unit");
        let request = MaterializeRequest {
            source: &source,
            target: &target,
            staging: &staging,
            trash: &trash,
        };
        let options = PolicyExperimentOptions {
            requested_mode: RequestedMode::CowClone,
            fallback_policy: FallbackPolicy::AllowFullCopyOnCowUnsupported,
            clone_preflight: ClonePreflightOverride::Observed,
            ..PolicyExperimentOptions::default()
        };

        let mut correct =
            prepare_attempt(&request, Backend::ApfsFileClone).expect("prepare correct evidence");
        let clone = candidate_evidence_mut(&mut correct, MaterializerCandidate::ApfsFileClone);
        clone.state = SupportState::Unsupported;
        clone.evidence = vec![EvidenceStatement {
            code: "clone_capability_absent".to_owned(),
            detail: "synthetic exact clone evidence".to_owned(),
        }];
        candidate_evidence_mut(&mut correct, MaterializerCandidate::FullCopy).state =
            SupportState::Supported;
        let receipt = run_cow_policy_experiment(&request, &options, correct)
            .expect("exact observed absence with a valid layout permits explicit copy");
        assert_eq!(receipt.actual_backend, Backend::FullCopy);
        assert_eq!(receipt.attempts.len(), 1);

        let mut wrong_candidate = prepare_attempt(&request, Backend::ApfsFileClone)
            .expect("prepare wrong-candidate evidence");
        candidate_evidence_mut(&mut wrong_candidate, MaterializerCandidate::ApfsFileClone).state =
            SupportState::Unsupported;
        candidate_evidence_mut(&mut wrong_candidate, MaterializerCandidate::ApfsFileClone)
            .evidence
            .clear();
        candidate_evidence_mut(&mut wrong_candidate, MaterializerCandidate::FullCopy).evidence =
            vec![EvidenceStatement {
                code: "clone_capability_absent".to_owned(),
                detail: "must not apply to Full Copy".to_owned(),
            }];
        assert!(matches!(
            run_cow_policy_experiment(&request, &options, wrong_candidate),
            Err(super::PolicyExperimentFailure {
                error: MaterializationError::PreflightUnsupported { .. },
                attempts,
            }) if attempts.is_empty()
        ));

        let mut unrelated = prepare_attempt(&request, Backend::ApfsFileClone)
            .expect("prepare unrelated-code evidence");
        let clone = candidate_evidence_mut(&mut unrelated, MaterializerCandidate::ApfsFileClone);
        clone.state = SupportState::Unsupported;
        clone.evidence = vec![EvidenceStatement {
            code: "unrelated".to_owned(),
            detail: "not clone capability evidence".to_owned(),
        }];
        assert!(matches!(
            run_cow_policy_experiment(&request, &options, unrelated),
            Err(super::PolicyExperimentFailure {
                error: MaterializationError::PreflightUnsupported { .. },
                attempts,
            }) if attempts.is_empty()
        ));

        let mut invalid_layout = prepare_attempt(&request, Backend::ApfsFileClone)
            .expect("prepare invalid synthetic layout");
        candidate_evidence_mut(&mut invalid_layout, MaterializerCandidate::FullCopy).state =
            SupportState::Unknown;
        let injected = PolicyExperimentOptions {
            clone_preflight: ClonePreflightOverride::InjectUnsupported,
            ..options
        };
        assert!(matches!(
            run_cow_policy_experiment(&request, &injected, invalid_layout),
            Err(super::PolicyExperimentFailure {
                error: MaterializationError::PreflightUnsupported { .. },
                attempts,
            }) if attempts.is_empty()
        ));
        fixture.cleanup().expect("remove verified fixture");
    }

    #[test]
    fn target_snapshot_requires_both_manifest_and_exact_confirmed_ledger() {
        let path = encode_relative(&[OsString::from("entry")]);
        let expected = FileIdentity {
            device: 17,
            inode: 29,
        };
        let root_metadata = crate::ffi::NodeMetadata {
            device: 17,
            inode: 23,
            kind: crate::ffi::NodeKind::Directory,
            mode: 0o700,
            size: 0,
            modified_seconds: 1_650_000_000,
            modified_nanoseconds: 123_456_789,
        };
        let manifest = TreeManifest {
            root_permissions: root_metadata.mode,
            root_modified_time: modified_time(root_metadata),
            entries: vec![ManifestEntry {
                path: path.clone(),
                kind: ManifestEntryKind::RegularFile,
                permissions: Some(0o600),
                modified_time: Some(ModifiedTime {
                    seconds: 1_650_000_001,
                    nanoseconds: 234_567_890,
                }),
                length: 4,
                content_digest_hex: Some(blake3::hash(b"data").to_hex().to_string()),
            }],
        };
        let context = ExecutionContext {
            backend: Backend::FullCopy,
            clone_calls_succeeded: 0,
            ordinary_files_materialized: 1,
            metadata_updates_started: 1,
            target_root_metadata_started: true,
            created: vec![TrackedCreated {
                relative: vec![OsString::from("entry")],
                kind: crate::ffi::NodeKind::RegularFile,
                identity: Some(expected),
            }],
            ordinary_files: Vec::new(),
        };
        let exact = TreeSnapshot {
            manifest: manifest.clone(),
            identities: BTreeMap::from([(path.clone(), expected)]),
            root_metadata,
        };
        assert!(context.matches_target_snapshot(&manifest, &exact));

        let mut wrong_manifest = exact.clone();
        wrong_manifest.manifest.entries[0].length += 1;
        assert!(!context.matches_target_snapshot(&manifest, &wrong_manifest));

        let mut wrong_identity = exact.clone();
        wrong_identity.identities.insert(
            path.clone(),
            FileIdentity {
                device: expected.device,
                inode: expected.inode + 1,
            },
        );
        assert!(!context.matches_target_snapshot(&manifest, &wrong_identity));

        let mut extra = exact.clone();
        extra.identities.insert(
            encode_relative(&[OsString::from("unknown")]),
            FileIdentity {
                device: 17,
                inode: 31,
            },
        );
        assert!(!context.matches_target_snapshot(&manifest, &extra));

        let unconfirmed = ExecutionContext {
            created: vec![TrackedCreated {
                relative: vec![OsString::from("entry")],
                kind: crate::ffi::NodeKind::RegularFile,
                identity: None,
            }],
            ..ExecutionContext::new(Backend::FullCopy)
        };
        assert!(!unconfirmed.matches_target_snapshot(&manifest, &exact));
    }

    #[test]
    fn identity_and_kind_comparison_rejects_each_single_field_difference() {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("experiment crate is below repository root");
        let mut fixture =
            ControlledTree::create_in(&repository_root.join("target"), "identity-kind-comparison")
                .expect("create controlled identity fixture");
        fixture
            .create_directory(Path::new("observed"), 0o700)
            .expect("create observed directory");
        let observed_fd = rustix::fs::openat(
            fixture.root_fd(),
            "observed",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .expect("open observed directory");
        let observed = crate::ffi::metadata(&observed_fd).expect("fstat observed directory");
        let expected = FileIdentity {
            device: observed.device,
            inode: observed.inode,
        };
        assert!(same_identity_and_kind(
            observed,
            expected,
            crate::ffi::NodeKind::Directory
        ));

        let mut changed_device = expected;
        changed_device.device ^= 1;
        assert!(!same_identity_and_kind(
            observed,
            changed_device,
            crate::ffi::NodeKind::Directory
        ));

        let mut changed_inode = expected;
        changed_inode.inode ^= 1;
        assert!(!same_identity_and_kind(
            observed,
            changed_inode,
            crate::ffi::NodeKind::Directory
        ));
        assert!(!same_identity_and_kind(
            observed,
            expected,
            crate::ffi::NodeKind::RegularFile
        ));
        drop(observed_fd);
        fixture.cleanup().expect("remove verified fixture");
    }

    #[test]
    fn open_verified_directory_rejects_each_identity_field_change() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"));
        let report = inspect_path(path).expect("inspect actual experiment directory");

        let mut changed_inode = report.clone();
        changed_inode
            .ancestry
            .last_mut()
            .expect("directory report has a leaf")
            .identity
            .inode ^= 1;
        assert!(matches!(
            open_verified_directory(path, &changed_inode),
            Err(MaterializationError::PlanStale { .. })
        ));

        let mut changed_device = report;
        changed_device
            .ancestry
            .last_mut()
            .expect("directory report has a leaf")
            .identity
            .device ^= 1;
        assert!(matches!(
            open_verified_directory(path, &changed_device),
            Err(MaterializationError::PlanStale { .. })
        ));
    }

    #[test]
    fn held_root_revalidation_compares_the_open_descriptors_after_fresh_probe() {
        let (fixture, source, target, staging, trash) =
            controlled_materialization_roots("held-root-unit");
        let request = MaterializeRequest {
            source: &source,
            target: &target,
            staging: &staging,
            trash: &trash,
        };
        let prepared = prepare_attempt(&request, Backend::FullCopy).expect("prepare empty attempt");
        let opened = revalidate_and_open(&prepared).expect("open all verified roots");
        let error = revalidate_held_roots(
            &prepared,
            [
                &opened.staging,
                &opened.target,
                &opened.staging,
                &opened.trash,
            ],
        )
        .expect_err("a wrong held source descriptor must make the plan stale");
        assert!(matches!(
            error,
            MaterializationError::PlanStale { ref path } if path == &source
        ));
        drop(opened);
        fixture.cleanup().expect("remove verified fixture");
    }

    #[test]
    fn target_empty_revalidation_rejects_git_and_surfaces_syscall_error() {
        let (mut fixture, _source, target_path, _staging, _trash) =
            controlled_materialization_roots("target-empty-unit");
        let target = rustix::fs::openat(
            fixture.root_fd(),
            "target",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .expect("open controlled target");
        verify_target_empty(&target, &target_path).expect("empty target is valid");

        fixture
            .create_file(Path::new("target/.git"), b"ordinary target content", 0o600)
            .expect("create existing target entry");
        assert!(matches!(
            verify_target_empty(&target, &target_path),
            Err(MaterializationError::UnknownTargetEntry { ref path }) if path.display == ".git"
        ));

        fixture
            .create_file(Path::new("not-a-directory"), b"file", 0o600)
            .expect("create controlled regular file");
        let regular = rustix::fs::openat(
            fixture.root_fd(),
            "not-a-directory",
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .expect("open regular file as deliberately invalid parent");
        assert!(matches!(
            verify_target_empty(&regular, &target_path),
            Err(MaterializationError::SystemCall {
                errno: Some(libc::ENOTDIR),
                ..
            })
        ));
        drop(regular);
        drop(target);
        fixture.cleanup().expect("remove verified fixture");
    }

    #[test]
    fn rollback_parent_validation_rejects_a_different_directory_identity() {
        let mut fixture = ControlledTree::create_in(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(3)
                .expect("experiment crate is below repository root")
                .join("target"),
            "rollback-parent-unit",
        )
        .expect("create controlled parent fixture");
        fixture
            .create_directory(Path::new("expected-parent"), 0o700)
            .expect("create expected parent");
        fixture
            .create_directory(Path::new("observed-parent"), 0o700)
            .expect("create different observed parent");
        let expected = fixture
            .identity_at(Path::new("expected-parent"))
            .expect("observe expected parent");
        let expected = FileIdentity {
            device: expected.device,
            inode: expected.inode,
        };
        let expected_parent = rustix::fs::openat(
            fixture.root_fd(),
            "expected-parent",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .expect("open expected parent");
        let observed_parent = rustix::fs::openat(
            fixture.root_fd(),
            "observed-parent",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .expect("open different parent");
        let created = vec![
            TrackedCreated {
                relative: vec![OsString::from("parent")],
                kind: crate::ffi::NodeKind::Directory,
                identity: Some(expected),
            },
            TrackedCreated {
                relative: vec![OsString::from("parent"), OsString::from("child")],
                kind: crate::ffi::NodeKind::RegularFile,
                identity: Some(FileIdentity {
                    device: expected.device,
                    inode: expected.inode + 10,
                }),
            },
        ];
        assert!(validate_rollback_parent(&expected_parent, &created[1], &created).is_none());
        let problem = validate_rollback_parent(&observed_parent, &created[1], &created)
            .expect("different held parent identity must stop rollback");
        assert_eq!(problem.operation, "compare rollback parent identity");
        assert_eq!(problem.expected_identity, Some(expected));
        assert_ne!(problem.observed_identity, Some(expected));
        drop(expected_parent);
        drop(observed_parent);
        fixture.cleanup().expect("remove verified fixture");
    }

    #[test]
    fn source_mutation_injection_refuses_a_same_kind_path_replacement() {
        let mut fixture = ControlledTree::create_in(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .ancestors()
                .nth(3)
                .expect("experiment crate is below repository root")
                .join("target"),
            "source-replacement-unit",
        )
        .expect("create controlled source fixture");
        fixture
            .create_directory(Path::new("source"), 0o700)
            .expect("create source root");
        fixture
            .create_file(Path::new("source/item"), b"original A", 0o600)
            .expect("create source A");
        let source = rustix::fs::openat(
            fixture.root_fd(),
            "source",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .expect("open held source root");
        let expected =
            crate::ffi::metadata_at(&source, c"item").expect("observe original source A");
        fixture
            .rename_tracked_exclusive(Path::new("source/item"), Path::new("source/original-a"))
            .expect("move exact source A");
        fixture
            .create_file(Path::new("source/item"), b"replacement B", 0o600)
            .expect("create same-kind source replacement B");
        let error = mutate_source_same_inode(
            &source,
            &ComponentName::new_io(OsStr::new("item")).expect("valid item component"),
            &fixture.root_path().join("source/item"),
            expected,
        )
        .expect_err("fault injection must not write through a replaced source path");
        assert!(matches!(error, MaterializationError::SourceChanged { .. }));
        assert_eq!(
            fs::read(fixture.root_path().join("source/item")).expect("read replacement B"),
            b"replacement B"
        );
        drop(source);
        fixture.cleanup().expect("remove verified fixture");
    }

    #[test]
    fn copy_fault_is_filtered_once_by_ordinary_file_index() {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("experiment crate is below repository root");
        let mut fixture = ControlledTree::create_in(&repository_root.join("target"), "copy-filter")
            .expect("create controlled copy fixture");
        for name in ["source-a", "source-b"] {
            fixture
                .create_file(Path::new(name), b"complete bytes", 0o600)
                .expect("create source file");
        }
        for name in ["target-a", "target-b"] {
            fixture
                .create_file(Path::new(name), b"", 0o600)
                .expect("create target file");
        }
        let open = |name: &str, flags| {
            rustix::fs::openat(fixture.root_fd(), name, flags, Mode::empty())
                .expect("open controlled copy file")
        };
        let source_a = open(
            "source-a",
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        );
        let target_a = open(
            "target-a",
            OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        );
        copy_file_bytes(
            &source_a,
            &target_a,
            0,
            &fixture.root_path().join("target-a"),
            Some(CopyWriteFault {
                ordinary_file_index: 1,
                after_bytes: 0,
                errno: libc::ENOSPC,
            }),
        )
        .expect("fault for a different ordinary file must be ignored");
        assert_eq!(
            fs::read(fixture.root_path().join("target-a")).expect("read completed target"),
            b"complete bytes"
        );

        let source_b = open(
            "source-b",
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        );
        let target_b = open(
            "target-b",
            OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        );
        assert!(matches!(
            copy_file_bytes(
                &source_b,
                &target_b,
                1,
                &fixture.root_path().join("target-b"),
                Some(CopyWriteFault {
                    ordinary_file_index: 1,
                    after_bytes: 0,
                    errno: libc::ENOSPC,
                }),
            ),
            Err(MaterializationError::SystemCall {
                errno: Some(libc::ENOSPC),
                injected: true,
                ..
            })
        ));
        assert_eq!(
            fs::read(fixture.root_path().join("target-b")).expect("read untouched target"),
            b""
        );
        drop(source_a);
        drop(target_a);
        drop(source_b);
        drop(target_b);
        fixture.cleanup().expect("remove verified fixture");
    }

    #[test]
    fn copy_and_digest_cover_empty_and_multiple_buffers_with_a_tail() {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("experiment crate is below repository root");
        let mut fixture =
            ControlledTree::create_in(&repository_root.join("target"), "copy-digest-boundary")
                .expect("create controlled boundary fixture");
        let large: Vec<u8> = (0..131_089).map(|index| (index % 251) as u8).collect();
        for (name, bytes) in [("source-empty", b"".as_slice()), ("source-large", &large)] {
            fixture
                .create_file(Path::new(name), bytes, 0o600)
                .expect("create controlled source");
        }
        for name in ["target-empty", "target-large"] {
            fixture
                .create_file(Path::new(name), b"", 0o600)
                .expect("create controlled target");
        }

        for (source_name, target_name) in [
            ("source-empty", "target-empty"),
            ("source-large", "target-large"),
        ] {
            let source = rustix::fs::openat(
                fixture.root_fd(),
                source_name,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .expect("open controlled source");
            let target = rustix::fs::openat(
                fixture.root_fd(),
                target_name,
                OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .expect("open controlled target");
            copy_file_bytes(
                &source,
                &target,
                0,
                &fixture.root_path().join(target_name),
                None,
            )
            .expect("copy complete controlled source");
        }
        assert_eq!(
            fs::read(fixture.root_path().join("target-empty")).expect("read empty target"),
            b""
        );
        assert_eq!(
            fs::read(fixture.root_path().join("target-large")).expect("read large target"),
            large
        );

        for (source_name, expected) in [("source-empty", b"".as_slice()), ("source-large", &large)]
        {
            let source = rustix::fs::openat(
                fixture.root_fd(),
                source_name,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .expect("open fresh source for digest");
            let (length, digest) = digest_file(source, &fixture.root_path().join(source_name))
                .expect("digest complete controlled source");
            assert_eq!(length, expected.len() as u64);
            assert_eq!(digest, blake3::hash(expected).to_hex().to_string());
        }
        fixture.cleanup().expect("remove verified fixture");
    }

    #[test]
    fn copy_fault_after_first_buffer_leaves_the_exact_large_prefix() {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("experiment crate is below repository root");
        let mut fixture =
            ControlledTree::create_in(&repository_root.join("target"), "copy-prefix-boundary")
                .expect("create controlled prefix fixture");
        let source_bytes: Vec<u8> = (0..131_089).map(|index| (index % 239) as u8).collect();
        fixture
            .create_file(Path::new("source"), &source_bytes, 0o600)
            .expect("create large source");
        fixture
            .create_file(Path::new("target"), b"", 0o600)
            .expect("create empty target");
        let source = rustix::fs::openat(
            fixture.root_fd(),
            "source",
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .expect("open large source");
        let target = rustix::fs::openat(
            fixture.root_fd(),
            "target",
            OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .expect("open prefix target");
        let after_bytes = 65_543;
        let error = copy_file_bytes(
            &source,
            &target,
            0,
            &fixture.root_path().join("target"),
            Some(CopyWriteFault {
                ordinary_file_index: 0,
                after_bytes,
                errno: libc::ENOSPC,
            }),
        )
        .expect_err("fault after the first full buffer must stop the copy");
        assert!(matches!(
            error,
            MaterializationError::SystemCall {
                errno: Some(libc::ENOSPC),
                injected: true,
                ..
            }
        ));
        assert_eq!(
            fs::read(fixture.root_path().join("target")).expect("read partial target"),
            source_bytes[..after_bytes]
        );
        assert_eq!(
            fs::read(fixture.root_path().join("source")).expect("read unchanged source"),
            source_bytes
        );
        drop(source);
        drop(target);
        fixture.cleanup().expect("remove verified fixture");
    }

    #[test]
    fn copy_to_read_only_target_preserves_ebadf_and_original_bytes() {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("experiment crate is below repository root");
        let mut fixture =
            ControlledTree::create_in(&repository_root.join("target"), "copy-read-only")
                .expect("create controlled read-only fixture");
        fixture
            .create_file(Path::new("source"), b"new bytes", 0o600)
            .expect("create source");
        fixture
            .create_file(Path::new("target"), b"original bytes", 0o600)
            .expect("create existing target contents");
        let source = rustix::fs::openat(
            fixture.root_fd(),
            "source",
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .expect("open source read-only");
        let target = rustix::fs::openat(
            fixture.root_fd(),
            "target",
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .expect("open target read-only");
        let error = copy_file_bytes(
            &source,
            &target,
            0,
            &fixture.root_path().join("target"),
            None,
        )
        .expect_err("writing through a read-only FD must fail");
        assert!(matches!(
            error,
            MaterializationError::SystemCall {
                operation: SystemOperation::Named(ref name),
                errno: Some(libc::EBADF),
                injected: false,
                ..
            } if name == "write target file"
        ));
        assert_eq!(
            fs::read(fixture.root_path().join("target")).expect("read unchanged target"),
            b"original bytes"
        );
        assert_eq!(
            fs::read(fixture.root_path().join("source")).expect("read unchanged source"),
            b"new bytes"
        );
        drop(source);
        drop(target);
        fixture.cleanup().expect("remove verified fixture");
    }

    #[test]
    fn fresh_fallback_path_rejects_changed_or_unknown_filesystem_and_mount_evidence() {
        let current = inspect_path(Path::new(env!("CARGO_MANIFEST_DIR")))
            .expect("inspect the actual APFS experiment directory");
        let Evidence::Known { .. } = &current.filesystem.volume_uuid else {
            panic!("P0-02 requires a known APFS Volume UUID");
        };
        assert!(fallback_path_evidence_matches(&current, &current));

        let mut changed_uuid = current.clone();
        let Evidence::Known { value, .. } = &mut changed_uuid.filesystem.volume_uuid else {
            unreachable!("known UUID checked above");
        };
        value.push_str("-changed");
        assert!(!fallback_path_evidence_matches(&changed_uuid, &current));

        let mut unknown_uuid = current.clone();
        unknown_uuid.filesystem.volume_uuid = Evidence::Unknown {
            reason: "synthetic missing UUID evidence".to_owned(),
            errno: None,
        };
        assert!(!fallback_path_evidence_matches(
            &unknown_uuid,
            &unknown_uuid
        ));

        let mut changed_fsid = current.clone();
        changed_fsid.filesystem.fsid[0] ^= 1;
        assert!(!fallback_path_evidence_matches(&changed_fsid, &current));

        let mut changed_type = current.clone();
        changed_type.filesystem.type_name = "syntheticfs".to_owned();
        assert!(!fallback_path_evidence_matches(&changed_type, &current));

        let mut changed_mount = current.clone();
        changed_mount.mount.raw_flags ^= 1;
        assert!(!fallback_path_evidence_matches(&changed_mount, &current));
    }

    #[test]
    fn clone_layout_requires_full_copy_supported_independent_of_clone_state_and_order() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let candidates = [
            MaterializerCandidate::ApfsFileClone,
            MaterializerCandidate::FullCopy,
        ];
        let mut report = inspect_materialization_paths(&MaterializationPathProbeRequest {
            source: root,
            target_root: root,
            staging: root,
            trash: root,
            candidates: &candidates,
        })
        .expect("inspect a fixed actual same-volume APFS layout");

        let set_state = |report: &mut MaterializationPathReport,
                         candidate: MaterializerCandidate,
                         state: SupportState| {
            report
                .candidates
                .iter_mut()
                .find(|evidence| evidence.candidate == candidate)
                .expect("requested candidate evidence exists")
                .state = state;
        };
        set_state(
            &mut report,
            MaterializerCandidate::ApfsFileClone,
            SupportState::Supported,
        );
        set_state(
            &mut report,
            MaterializerCandidate::FullCopy,
            SupportState::Unknown,
        );
        assert!(!clone_layout_preconditions_satisfied(&report));

        set_state(
            &mut report,
            MaterializerCandidate::FullCopy,
            SupportState::Unsupported,
        );
        assert!(!clone_layout_preconditions_satisfied(&report));

        set_state(
            &mut report,
            MaterializerCandidate::ApfsFileClone,
            SupportState::Unknown,
        );
        set_state(
            &mut report,
            MaterializerCandidate::FullCopy,
            SupportState::Supported,
        );
        report.candidates.reverse();
        assert!(clone_layout_preconditions_satisfied(&report));
    }

    #[test]
    fn relative_evidence_hex_preserves_non_utf8_component_bytes_without_touching_filesystem() {
        let encoded = encode_relative(&[
            OsString::from_vec(b"raw-\xff".to_vec()),
            OsString::from("tail"),
        ]);
        assert_eq!(encoded.bytes_hex, "7261772dff2f7461696c");
    }

    #[test]
    fn copy_write_fault_leaves_exact_source_prefix_before_attempt_rollback() {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("experiment crate is below repository root");
        let parent_path = repository_root.join("target");
        let mut fixture = ControlledTree::create_in(&parent_path, "copy-unit")
            .expect("create controlled copy fixture");
        fixture
            .create_file(Path::new("source"), b"abcdefghijk", 0o600)
            .expect("create controlled source file");
        fixture
            .create_file(Path::new("target"), b"", 0o600)
            .expect("create and register controlled target file");
        let source_path = fixture.root_path().join("source");
        let target_path = fixture.root_path().join("target");
        let source = rustix::fs::openat(
            fixture.root_fd(),
            "source",
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .expect("open source FD");
        let target = rustix::fs::openat(
            fixture.root_fd(),
            "target",
            OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .expect("open target FD");

        let error = copy_file_bytes(
            &source,
            &target,
            0,
            &target_path,
            Some(CopyWriteFault {
                ordinary_file_index: 0,
                after_bytes: 5,
                errno: libc::ENOSPC,
            }),
        )
        .expect_err("copy fault must stop after the configured prefix");
        assert!(matches!(
            error,
            MaterializationError::SystemCall {
                operation: SystemOperation::Named(ref name),
                errno: Some(libc::ENOSPC),
                injected: true,
                ..
            } if name == "write target file"
        ));
        assert_eq!(
            fs::read(&target_path).expect("read partial target"),
            b"abcde"
        );
        assert_eq!(
            fs::read(&source_path).expect("read unchanged source"),
            b"abcdefghijk"
        );
        fixture
            .verify_exact_tree()
            .expect("partial copy changed no fixture identities or membership");
        fixture.cleanup().expect("remove verified copy fixture");
    }

    #[test]
    fn held_created_fd_mismatch_stays_unconfirmed_and_rollback_preserves_replacement() {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("experiment crate is below repository root");
        let parent_path = repository_root.join("target");
        let mut fixture = ControlledTree::create_in(&parent_path, "register-unit")
            .expect("create controlled registration fixture");
        for directory in ["source", "target", "staging", "trash"] {
            fixture
                .create_directory(Path::new(directory), 0o700)
                .expect("create materialization root");
        }
        let source_path = fixture.root_path().join("source");
        let target_path = fixture.root_path().join("target");
        let staging_path = fixture.root_path().join("staging");
        let trash_path = fixture.root_path().join("trash");
        let request = MaterializeRequest {
            source: &source_path,
            target: &target_path,
            staging: &staging_path,
            trash: &trash_path,
        };
        let prepared = prepare_attempt(&request, Backend::FullCopy)
            .expect("prepare an empty controlled attempt");
        fixture
            .create_file(Path::new("target/victim"), b"created A", 0o600)
            .expect("create initial target A");
        let target = open_verified_directory(&target_path, &prepared.report.target_root)
            .expect("open held target root");
        let held_a = crate::ffi::open_file_write_at(&target, c"victim")
            .expect("hold the initially created target A");
        fixture
            .rename_tracked_exclusive(Path::new("target/victim"), Path::new("target/original-a"))
            .expect("move A without replacing another entry");
        fixture
            .create_file(Path::new("target/victim"), b"replacement B", 0o600)
            .expect("create independently owned replacement B");
        let replacement = fixture
            .identity_at(Path::new("target/victim"))
            .expect("observe replacement B");

        let mut context = ExecutionContext::new(Backend::FullCopy);
        let error = register_created(
            &target,
            &ComponentName::new(OsStr::new("victim")).expect("valid component"),
            &["victim".into()],
            crate::ffi::NodeKind::RegularFile,
            Some(&held_a),
            &FaultInjection::default(),
            &mut context,
        )
        .expect_err("path replacement must not be registered as the created FD");
        assert!(matches!(error, MaterializationError::TargetChanged { .. }));
        assert_eq!(context.created.len(), 1);
        assert!(context.created[0].identity.is_none());

        let rollback = rollback_created(
            &prepared,
            &target,
            &context.created,
            RollbackFault::None,
            Vec::new(),
        );
        assert_eq!(rollback.status, RollbackStatus::Incomplete);
        assert!(rollback.removed.is_empty());
        assert_eq!(rollback.remaining.len(), 1);
        assert_eq!(
            fs::read(target_path.join("victim")).unwrap(),
            b"replacement B"
        );
        assert_eq!(
            fixture
                .identity_at(Path::new("target/victim"))
                .expect("replacement B remains"),
            replacement
        );
        fixture
            .cleanup()
            .expect("remove both independently-owned fixture objects");
    }

    #[test]
    fn stale_boundary_prevents_unknown_or_replacement_rollback_fault_injection() {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("experiment crate is below repository root");
        let parent_path = repository_root.join("target");
        let mut fixture = ControlledTree::create_in(&parent_path, "rollback-boundary-unit")
            .expect("create controlled rollback fixture");
        for directory in ["source", "target", "staging", "trash"] {
            fixture
                .create_directory(Path::new(directory), 0o700)
                .expect("create materialization root");
        }
        let source_path = fixture.root_path().join("source");
        let target_path = fixture.root_path().join("target");
        let staging_path = fixture.root_path().join("staging");
        let trash_path = fixture.root_path().join("trash");
        let request = MaterializeRequest {
            source: &source_path,
            target: &target_path,
            staging: &staging_path,
            trash: &trash_path,
        };
        let prepared = prepare_attempt(&request, Backend::FullCopy)
            .expect("prepare an empty controlled attempt");
        fixture
            .create_file(Path::new("target/victim"), b"owned victim", 0o600)
            .expect("create fixture-owned target");
        let target = open_verified_directory(&target_path, &prepared.report.target_root)
            .expect("open held target root");
        let mut context = ExecutionContext::new(Backend::FullCopy);
        let registered = register_created(
            &target,
            &ComponentName::new(OsStr::new("victim")).expect("valid component"),
            &["victim".into()],
            crate::ffi::NodeKind::RegularFile,
            None,
            &FaultInjection::default(),
            &mut context,
        )
        .expect("register fixture-owned target identity");
        let before = fixture
            .observed_tree()
            .expect("observe exact held tree before path move");
        let moved_name = fixture
            .move_fixture_parent_exclusive()
            .expect("exclusively move the fixture ancestor");

        for rollback in [
            RollbackFault::AddUnknownEntry,
            RollbackFault::ReplaceCreated { created_index: 0 },
        ] {
            let problems = apply_rollback_fault(
                &prepared,
                &target,
                &mut context,
                &FaultInjection {
                    rollback,
                    ..FaultInjection::default()
                },
            );
            assert_eq!(problems.len(), 1);
            assert_eq!(
                problems[0].operation,
                "revalidate four paths before rollback step"
            );
            assert_eq!(
                fixture
                    .observed_tree()
                    .expect("observe held tree after rejected fault injection"),
                before
            );
            assert_eq!(context.created[0].identity, Some(registered));
        }

        fixture
            .restore_fixture_parent_exclusive(&moved_name)
            .expect("restore exact fixture ancestor after refusal assertions");
        fixture.cleanup().expect("remove verified rollback fixture");
    }
}
