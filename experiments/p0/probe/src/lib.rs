//! Repeatable, read-only macOS host and path evidence for the P0-01 experiment.
//!
//! This crate reports preflight observations only. It neither materializes data
//! nor establishes that a later operation will succeed or preserve CoW.

#![deny(unsafe_code)]

#[allow(unsafe_code)]
mod ffi;

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Stable experiment identifier embedded in every report.
pub const EXPERIMENT_NAME: &str = "p0-01-host-path-probe";
/// Schema version understood by this experimental crate.
pub const REPORT_SCHEMA_VERSION: u32 = 1;

/// Evidence about CoW behavior gathered by this read-only probe.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CowEvidence {
    NotExecutedByProbe,
    NotApplicable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
/// Measured host identity and the narrow scope of this experiment.
pub struct HostCapabilityReport {
    pub schema_version: u32,
    pub experiment: String,
    pub observed_at_unix_ms: u64,
    pub operating_system: String,
    pub product_version: String,
    pub kernel_release: String,
    pub architecture: String,
    pub adapter: String,
    pub capability_scope: CapabilityScope,
    pub cow_evidence: CowEvidence,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Platform boundary covered by the compiled experiment.
pub enum CapabilityScope {
    CompiledMacosProbeOnly,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
/// A path rendered for humans and encoded losslessly as hexadecimal bytes.
pub struct EncodedPath {
    pub display: String,
    pub bytes_hex: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Parsed absolute path whose components are safe for component-wise traversal.
pub struct ValidatedProbePath {
    pub normalized: EncodedPath,
    pub components: Vec<EncodedPath>,
    raw_components: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Whether the requested path exists or ends in an absent suffix.
pub enum PathResolution {
    ExistingDirectory,
    MissingTarget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// File type observed without following the directory entry.
pub enum FileType {
    Directory,
    RegularFile,
    SymbolicLink,
    Fifo,
    Socket,
    CharacterDevice,
    BlockDevice,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
/// Device/inode identity measured with `fstat` on a held descriptor.
pub struct FileIdentity {
    pub device: u64,
    pub inode: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
/// Identity of one directory in the component-wise ancestry chain.
pub struct DirectoryIdentityEvidence {
    pub path: EncodedPath,
    pub identity: FileIdentity,
    pub source: EvidenceSource,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// System interface and held descriptor used to obtain an observation.
pub enum EvidenceSource {
    FstatHeldDirectoryFd,
    FstatfsHeldDirectoryFd,
    FgetattrlistHeldDirectoryFd,
    FaccessatHeldDirectoryFd,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
/// Known evidence or a structured reason that it could not be obtained.
pub enum Evidence<T> {
    Known { value: T, source: EvidenceSource },
    Unknown { reason: String, errno: Option<i32> },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
/// Filesystem and volume evidence for the held existing directory.
pub struct FileSystemIdentity {
    pub type_name: String,
    pub fsid: [i32; 2],
    pub volume_uuid: Evidence<String>,
    pub clone_capability: CloneCapabilityEvidence,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
/// `VOL_CAP_INT_CLONE` preflight evidence; this is not clone execution proof.
pub struct CloneCapabilityEvidence {
    pub state: SupportState,
    pub interface_capabilities: Option<u32>,
    pub interface_valid: Option<u32>,
    pub errno: Option<i32>,
    pub source: EvidenceSource,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Mount flags relevant to materialization preflight.
pub enum MountFlag {
    ReadOnly,
    NoExec,
    NoSuid,
    NoDevice,
    Local,
    Journaled,
    Automounted,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
/// Raw and recognized mount flags returned for a held descriptor.
pub struct MountEvidence {
    pub raw_flags: u32,
    pub recognized_flags: BTreeSet<MountFlag>,
    pub source: EvidenceSource,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Mount-level write state at inspection time.
pub enum MountWriteState {
    WritableAtInspection,
    ReadOnly,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
/// Effective-ID access preflight, including denial or uncertainty details.
pub enum AccessPreflight {
    Allowed {
        source: EvidenceSource,
    },
    Denied {
        errno: Option<i32>,
        reason: String,
        source: EvidenceSource,
    },
    Unknown {
        errno: Option<i32>,
        reason: String,
        source: EvidenceSource,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Explicit boundary between preflight evidence and execution guarantees.
pub enum ExecutionGuarantee {
    NotEstablishedByReadOnlyProbe,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
/// Destination write/search preflight obtained without creating anything.
pub struct WritabilityEvidence {
    pub mount_state: MountWriteState,
    pub effective_access_preflight: AccessPreflight,
    pub execution_guarantee: ExecutionGuarantee,
    pub write_attempt: WriteAttempt,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
/// Source read/search preflight obtained without reading source contents.
pub struct ReadabilityEvidence {
    pub effective_access_preflight: AccessPreflight,
    pub execution_guarantee: ExecutionGuarantee,
    pub read_attempt: ReadAttempt,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Whether source contents were read by the probe.
pub enum ReadAttempt {
    NotPerformedReadOnlyProbe,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Whether a write was performed by the probe.
pub enum WriteAttempt {
    NotPerformedReadOnlyProbe,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Three-state capability result that preserves uncertainty.
pub enum SupportState {
    Supported,
    Unsupported,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Strength of a reported candidate result.
pub enum SupportAssurance {
    PreflightOnly,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
/// Machine-oriented evidence code with a human-readable detail.
pub struct EvidenceStatement {
    pub code: String,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
/// Candidate evidence available from one inspected path alone.
pub struct PathCandidateEvidence {
    pub candidate: MaterializerCandidate,
    pub state: SupportState,
    pub assurance: SupportAssurance,
    pub evidence: Vec<EvidenceStatement>,
    pub cow_evidence: CowEvidence,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
/// Read-only path report anchored to held directory descriptors.
pub struct PathCapabilityReport {
    pub schema_version: u32,
    pub experiment: String,
    pub observed_at_unix_ms: u64,
    pub requested_path: EncodedPath,
    pub resolution: PathResolution,
    pub nearest_existing_ancestor: EncodedPath,
    pub missing_components: Vec<EncodedPath>,
    pub ancestry: Vec<DirectoryIdentityEvidence>,
    pub filesystem: FileSystemIdentity,
    pub mount: MountEvidence,
    pub readability: ReadabilityEvidence,
    pub writability: WritabilityEvidence,
    pub candidate_path_evidence: Vec<PathCandidateEvidence>,
    pub cow_evidence: CowEvidence,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Materialization mechanism evaluated by the experiment.
pub enum MaterializerCandidate {
    ApfsFileClone,
    FullCopy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
/// Four path roles and candidates required for combined preflight.
pub struct MaterializationPathProbeRequest<'a> {
    pub source: &'a Path,
    pub target_root: &'a Path,
    pub staging: &'a Path,
    pub trash: &'a Path,
    pub candidates: &'a [MaterializerCandidate],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Role of a path in a materialization request.
pub enum PathRole {
    Source,
    TargetRoot,
    Staging,
    Trash,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Relationship inferred from UUID, filesystem type, and fsid evidence.
pub enum VolumeRelation {
    SameVolume,
    DifferentVolume,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
/// Volume relationship evidence for a pair of materialization paths.
pub struct PathPairEvidence {
    pub left: PathRole,
    pub right: PathRole,
    pub relation: VolumeRelation,
    pub evidence: Vec<EvidenceStatement>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Explicitly records that platform evidence does not choose product policy.
pub enum ProductPolicyDecision {
    NotEvaluatedByPlatformProbe,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Whether the candidate was actually executed by the probe.
pub enum CandidateExecution {
    NotAttemptedByProbe,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
/// Combined preflight evidence for one requested materializer candidate.
pub struct CandidateEvidence {
    pub candidate: MaterializerCandidate,
    pub state: SupportState,
    pub assurance: SupportAssurance,
    pub execution: CandidateExecution,
    pub policy_decision: ProductPolicyDecision,
    pub evidence: Vec<EvidenceStatement>,
    pub cow_evidence: CowEvidence,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
/// Combined source, target, staging, trash, pair, and candidate evidence.
pub struct MaterializationPathReport {
    pub schema_version: u32,
    pub experiment: String,
    pub observed_at_unix_ms: u64,
    pub source: PathCapabilityReport,
    pub target_root: PathCapabilityReport,
    pub staging: PathCapabilityReport,
    pub trash: PathCapabilityReport,
    pub pairs: Vec<PathPairEvidence>,
    pub candidates: Vec<CandidateEvidence>,
    pub cow_evidence: CowEvidence,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Result of comparing fresh observations with previous evidence.
pub enum RevalidationStatus {
    Unchanged,
    Stale,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
/// Specific observation that changed during revalidation.
pub enum RevalidationChange {
    ResolutionChanged {
        before: PathResolution,
        after: PathResolution,
    },
    ExistingPrefixChanged {
        before: EncodedPath,
        after: EncodedPath,
    },
    MissingSuffixChanged {
        before: Vec<EncodedPath>,
        after: Vec<EncodedPath>,
    },
    AncestryIdentityChanged {
        path: EncodedPath,
        before: Option<FileIdentity>,
        after: Option<FileIdentity>,
    },
    FileSystemChanged {
        before: FileSystemIdentity,
        after: FileSystemIdentity,
    },
    MountChanged {
        before: MountEvidence,
        after: MountEvidence,
    },
    WritabilityChanged {
        before: WritabilityEvidence,
        after: WritabilityEvidence,
    },
    ReadabilityChanged {
        before: ReadabilityEvidence,
        after: ReadabilityEvidence,
    },
    PathRejected {
        error: ProbeError,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
/// Fresh report and structured differences from prior path evidence.
pub struct PathRevalidationReport {
    pub schema_version: u32,
    pub experiment: String,
    pub observed_at_unix_ms: u64,
    pub requested_path: EncodedPath,
    pub status: RevalidationStatus,
    pub changes: Vec<RevalidationChange>,
    pub current: Option<PathCapabilityReport>,
}

#[derive(Clone, Debug, Eq, PartialEq, Error, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
/// Structured validation, traversal, and syscall failures from the experiment.
pub enum ProbeError {
    #[error("the probe path must be absolute")]
    PathMustBeAbsolute,
    #[error("parent traversal is not allowed")]
    ParentTraversal,
    #[error("a path component contains an embedded NUL byte")]
    EmbeddedNul { component_index: usize },
    #[error("a symbolic link was encountered at {path:?}")]
    SymbolicLinkEncountered { path: EncodedPath },
    #[error("the probe requires directories but found {file_type:?} at {path:?}")]
    UnsupportedFileType {
        path: EncodedPath,
        file_type: FileType,
    },
    #[error("the path changed while it was being inspected at {path:?}")]
    PathChangedDuringInspection { path: EncodedPath },
    #[error("system call {operation} failed for {path:?}: {message}")]
    SystemCall {
        operation: String,
        path: Option<EncodedPath>,
        errno: Option<i32>,
        message: String,
    },
    #[error("the serialized path evidence is corrupt: {reason}")]
    CorruptPathEvidence { reason: String },
}

/// Inspect the current macOS host without mutating host or filesystem state.
pub fn inspect_host() -> Result<HostCapabilityReport, ProbeError> {
    let identity = ffi::host_identity().map_err(|error| system_call_error("uname", None, error))?;
    let product_version =
        ffi::product_version().map_err(|error| system_call_error("sysctlbyname", None, error))?;

    Ok(HostCapabilityReport {
        schema_version: REPORT_SCHEMA_VERSION,
        experiment: EXPERIMENT_NAME.to_owned(),
        observed_at_unix_ms: observed_at_unix_ms(),
        operating_system: if identity.system_name == "Darwin" {
            "macos".to_owned()
        } else {
            identity.system_name
        },
        product_version,
        kernel_release: identity.kernel_release,
        architecture: identity.architecture,
        adapter: "macos-held-directory-fd".to_owned(),
        capability_scope: CapabilityScope::CompiledMacosProbeOnly,
        cow_evidence: CowEvidence::NotExecutedByProbe,
    })
}

/// Parse an absolute path into normal, non-NUL components.
///
/// Parent traversal is rejected. The returned components are suitable for the
/// crate's single-component `openat` traversal; this function does no I/O.
pub fn validate_probe_path(path: &Path) -> Result<ValidatedProbePath, ProbeError> {
    if !path.is_absolute() {
        return Err(ProbeError::PathMustBeAbsolute);
    }

    let mut raw_components = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                let bytes = name.as_bytes();
                if bytes.contains(&0) {
                    return Err(ProbeError::EmbeddedNul {
                        component_index: raw_components.len(),
                    });
                }
                raw_components.push(bytes.to_vec());
            }
            Component::ParentDir => return Err(ProbeError::ParentTraversal),
            Component::CurDir => {}
            Component::Prefix(_) => return Err(ProbeError::PathMustBeAbsolute),
        }
    }

    let normalized = normalized_path(&raw_components);
    Ok(ValidatedProbePath {
        normalized: encode_path(&normalized),
        components: raw_components
            .iter()
            .map(|component| encode_os_str(OsStr::from_bytes(component)))
            .collect(),
        raw_components,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FailedLookupClassification {
    MissingTarget,
    PathChanged,
    OpenError,
    MetadataError,
}

fn classify_failed_lookup(
    open_errno: Option<i32>,
    metadata_errno: Option<i32>,
) -> FailedLookupClassification {
    match metadata_errno {
        Some(metadata_errno)
            if open_errno == Some(libc::ENOENT) && metadata_errno == libc::ENOENT =>
        {
            FailedLookupClassification::MissingTarget
        }
        Some(_) if open_errno == Some(libc::ENOENT) => FailedLookupClassification::MetadataError,
        Some(_) => FailedLookupClassification::OpenError,
        None if matches!(
            open_errno,
            Some(libc::ENOENT) | Some(libc::ELOOP) | Some(libc::ENOTDIR)
        ) =>
        {
            FailedLookupClassification::PathChanged
        }
        None => FailedLookupClassification::OpenError,
    }
}

fn existing_entry_error(open_error: io::Error, observed_type: FileType, path: &Path) -> ProbeError {
    match observed_type {
        FileType::SymbolicLink => ProbeError::SymbolicLinkEncountered {
            path: encode_path(path),
        },
        FileType::Directory
            if classify_failed_lookup(open_error.raw_os_error(), None)
                == FailedLookupClassification::PathChanged =>
        {
            ProbeError::PathChangedDuringInspection {
                path: encode_path(path),
            }
        }
        FileType::Directory => {
            system_call_error("openat directory with O_NOFOLLOW", Some(path), open_error)
        }
        other => ProbeError::UnsupportedFileType {
            path: encode_path(path),
            file_type: other,
        },
    }
}

/// Inspect a path through component-wise, no-follow directory traversal.
///
/// Missing suffixes are reported from the nearest held existing ancestor. No
/// target is created and no symlink is followed.
pub fn inspect_path(path: &Path) -> Result<PathCapabilityReport, ProbeError> {
    let validated = validate_probe_path(path)?;
    let mut current_fd = ffi::open_root_directory()
        .map_err(|error| system_call_error("open root directory", Some(Path::new("/")), error))?;
    let mut current_path = PathBuf::from("/");
    let mut ancestry = vec![directory_evidence(&current_fd, &current_path)?];
    let mut missing_components = Vec::new();

    for (index, raw_component) in validated.raw_components.iter().enumerate() {
        let name = ffi::c_string(raw_component).map_err(|_| ProbeError::EmbeddedNul {
            component_index: index,
        })?;
        let candidate_path = current_path.join(OsStr::from_bytes(raw_component));

        match ffi::open_directory_at(&current_fd, &name) {
            Ok(next_fd) => {
                let evidence = directory_evidence(&next_fd, &candidate_path)?;
                current_fd = next_fd;
                current_path = candidate_path;
                ancestry.push(evidence);
            }
            Err(open_error) => match ffi::metadata_at(&current_fd, &name) {
                Ok(metadata) => {
                    return Err(existing_entry_error(
                        open_error,
                        file_type(metadata.kind),
                        &candidate_path,
                    ));
                }
                Err(metadata_error) => match classify_failed_lookup(
                    open_error.raw_os_error(),
                    metadata_error.raw_os_error(),
                ) {
                    FailedLookupClassification::MissingTarget => {
                        missing_components = validated.components[index..].to_vec();
                        break;
                    }
                    FailedLookupClassification::PathChanged => {
                        return Err(ProbeError::PathChangedDuringInspection {
                            path: encode_path(&candidate_path),
                        });
                    }
                    FailedLookupClassification::OpenError => {
                        return Err(system_call_error(
                            "openat directory with O_NOFOLLOW",
                            Some(&candidate_path),
                            open_error,
                        ));
                    }
                    FailedLookupClassification::MetadataError => {
                        return Err(system_call_error(
                            "fstatat missing-target verification",
                            Some(&candidate_path),
                            metadata_error,
                        ));
                    }
                },
            },
        }
    }

    let file_system_raw = ffi::file_system_metadata(&current_fd)
        .map_err(|error| system_call_error("fstatfs held directory", Some(&current_path), error))?;
    let volume_uuid = volume_uuid_evidence(ffi::volume_uuid(&current_fd));
    let clone_capability = clone_capability_evidence(&current_fd);
    let filesystem = FileSystemIdentity {
        type_name: file_system_raw.type_name,
        fsid: file_system_raw.fsid,
        volume_uuid,
        clone_capability,
    };
    let mount = mount_evidence(file_system_raw.mount_flags);
    let readability = readability_evidence(&current_fd);
    let writability = writability_evidence(&current_fd, &mount);
    let resolution = if missing_components.is_empty() {
        PathResolution::ExistingDirectory
    } else {
        PathResolution::MissingTarget
    };
    let candidate_path_evidence = path_candidate_evidence(&filesystem);

    Ok(PathCapabilityReport {
        schema_version: REPORT_SCHEMA_VERSION,
        experiment: EXPERIMENT_NAME.to_owned(),
        observed_at_unix_ms: observed_at_unix_ms(),
        requested_path: validated.normalized,
        resolution,
        nearest_existing_ancestor: encode_path(&current_path),
        missing_components,
        ancestry,
        filesystem,
        mount,
        readability,
        writability,
        candidate_path_evidence,
        cow_evidence: CowEvidence::NotExecutedByProbe,
    })
}

fn volume_uuid_evidence(result: io::Result<Option<[u8; 16]>>) -> Evidence<String> {
    match result {
        Ok(Some(uuid)) if uuid != [0; 16] => Evidence::Known {
            value: format_uuid(uuid),
            source: EvidenceSource::FgetattrlistHeldDirectoryFd,
        },
        Ok(Some(_)) => Evidence::Unknown {
            reason: "ATTR_VOL_UUID returned the all-zero sentinel".to_owned(),
            errno: None,
        },
        Ok(None) => Evidence::Unknown {
            reason: "ATTR_VOL_UUID was not returned by the filesystem".to_owned(),
            errno: None,
        },
        Err(error) => Evidence::Unknown {
            reason: format!("fgetattrlist ATTR_VOL_UUID failed: {error}"),
            errno: error.raw_os_error(),
        },
    }
}

/// Inspect all materialization roles and derive pair/candidate preflight evidence.
pub fn inspect_materialization_paths(
    request: &MaterializationPathProbeRequest<'_>,
) -> Result<MaterializationPathReport, ProbeError> {
    let source = inspect_path(request.source)?;
    let target_root = inspect_path(request.target_root)?;
    let staging = inspect_path(request.staging)?;
    let trash = inspect_path(request.trash)?;
    let paths = [
        (PathRole::Source, &source),
        (PathRole::TargetRoot, &target_root),
        (PathRole::Staging, &staging),
        (PathRole::Trash, &trash),
    ];
    let mut pairs = Vec::new();
    for left in 0..paths.len() {
        for right in (left + 1)..paths.len() {
            pairs.push(pair_evidence(
                paths[left].0,
                paths[left].1,
                paths[right].0,
                paths[right].1,
            ));
        }
    }

    let candidates = request
        .candidates
        .iter()
        .copied()
        .map(|candidate| candidate_evidence(candidate, &paths))
        .collect();

    Ok(MaterializationPathReport {
        schema_version: REPORT_SCHEMA_VERSION,
        experiment: EXPERIMENT_NAME.to_owned(),
        observed_at_unix_ms: observed_at_unix_ms(),
        source,
        target_root,
        staging,
        trash,
        pairs,
        candidates,
        cow_evidence: CowEvidence::NotExecutedByProbe,
    })
}

/// Re-inspect a prior path report and identify stale safety evidence.
///
/// Evidence from another schema or experiment is rejected before decoding its
/// path bytes.
pub fn revalidate_path(
    previous: &PathCapabilityReport,
) -> Result<PathRevalidationReport, ProbeError> {
    if previous.schema_version != REPORT_SCHEMA_VERSION {
        return Err(ProbeError::CorruptPathEvidence {
            reason: format!(
                "unsupported schema_version {}; expected {}",
                previous.schema_version, REPORT_SCHEMA_VERSION
            ),
        });
    }
    if previous.experiment != EXPERIMENT_NAME {
        return Err(ProbeError::CorruptPathEvidence {
            reason: format!(
                "unexpected experiment {:?}; expected {:?}",
                previous.experiment, EXPERIMENT_NAME
            ),
        });
    }

    let raw_path = decode_hex(&previous.requested_path.bytes_hex).ok_or_else(|| {
        ProbeError::CorruptPathEvidence {
            reason: "requested_path.bytes_hex is not valid even-length hexadecimal".to_owned(),
        }
    })?;
    let path = PathBuf::from(OsString::from_vec(raw_path));

    let current = match inspect_path(&path) {
        Ok(current) => current,
        Err(error) => {
            return Ok(PathRevalidationReport {
                schema_version: REPORT_SCHEMA_VERSION,
                experiment: EXPERIMENT_NAME.to_owned(),
                observed_at_unix_ms: observed_at_unix_ms(),
                requested_path: previous.requested_path.clone(),
                status: RevalidationStatus::Stale,
                changes: vec![RevalidationChange::PathRejected { error }],
                current: None,
            });
        }
    };

    let mut changes = Vec::new();
    if previous.resolution != current.resolution {
        changes.push(RevalidationChange::ResolutionChanged {
            before: previous.resolution.clone(),
            after: current.resolution.clone(),
        });
    }
    if previous.nearest_existing_ancestor != current.nearest_existing_ancestor {
        changes.push(RevalidationChange::ExistingPrefixChanged {
            before: previous.nearest_existing_ancestor.clone(),
            after: current.nearest_existing_ancestor.clone(),
        });
    }
    if previous.missing_components != current.missing_components {
        changes.push(RevalidationChange::MissingSuffixChanged {
            before: previous.missing_components.clone(),
            after: current.missing_components.clone(),
        });
    }
    compare_ancestry(&previous.ancestry, &current.ancestry, &mut changes);
    if previous.filesystem != current.filesystem {
        changes.push(RevalidationChange::FileSystemChanged {
            before: previous.filesystem.clone(),
            after: current.filesystem.clone(),
        });
    }
    if previous.mount != current.mount {
        changes.push(RevalidationChange::MountChanged {
            before: previous.mount.clone(),
            after: current.mount.clone(),
        });
    }
    if previous.writability != current.writability {
        changes.push(RevalidationChange::WritabilityChanged {
            before: previous.writability.clone(),
            after: current.writability.clone(),
        });
    }
    if previous.readability != current.readability {
        changes.push(RevalidationChange::ReadabilityChanged {
            before: previous.readability.clone(),
            after: current.readability.clone(),
        });
    }

    Ok(PathRevalidationReport {
        schema_version: REPORT_SCHEMA_VERSION,
        experiment: EXPERIMENT_NAME.to_owned(),
        observed_at_unix_ms: observed_at_unix_ms(),
        requested_path: previous.requested_path.clone(),
        status: if changes.is_empty() {
            RevalidationStatus::Unchanged
        } else {
            RevalidationStatus::Stale
        },
        changes,
        current: Some(current),
    })
}

fn directory_evidence(
    fd: &std::os::fd::OwnedFd,
    path: &Path,
) -> Result<DirectoryIdentityEvidence, ProbeError> {
    let metadata = ffi::metadata(fd)
        .map_err(|error| system_call_error("fstat held directory", Some(path), error))?;
    if metadata.kind != ffi::RawFileKind::Directory {
        return Err(ProbeError::UnsupportedFileType {
            path: encode_path(path),
            file_type: file_type(metadata.kind),
        });
    }
    Ok(DirectoryIdentityEvidence {
        path: encode_path(path),
        identity: FileIdentity {
            device: metadata.device,
            inode: metadata.inode,
        },
        source: EvidenceSource::FstatHeldDirectoryFd,
    })
}

fn path_candidate_evidence(filesystem: &FileSystemIdentity) -> Vec<PathCandidateEvidence> {
    let (state, evidence) = if filesystem.type_name != "apfs" {
        (
            SupportState::Unsupported,
            vec![statement(
                "filesystem_not_apfs",
                format!("filesystem type is {}", filesystem.type_name),
            )],
        )
    } else if matches!(filesystem.volume_uuid, Evidence::Unknown { .. }) {
        (
            SupportState::Unknown,
            vec![statement(
                "volume_uuid_unknown",
                "APFS Volume UUID was not available from the held directory FD",
            )],
        )
    } else if filesystem.clone_capability.state != SupportState::Supported {
        (
            filesystem.clone_capability.state,
            vec![statement(
                "clone_capability_not_supported",
                filesystem.clone_capability.reason.clone(),
            )],
        )
    } else {
        (
            SupportState::Supported,
            vec![statement(
                "apfs_path_preconditions_observed",
                "APFS type and a Volume UUID were observed; no clone was attempted",
            )],
        )
    };

    vec![
        PathCandidateEvidence {
            candidate: MaterializerCandidate::ApfsFileClone,
            state,
            assurance: SupportAssurance::PreflightOnly,
            evidence,
            cow_evidence: CowEvidence::NotExecutedByProbe,
        },
        PathCandidateEvidence {
            candidate: MaterializerCandidate::FullCopy,
            state: SupportState::Supported,
            assurance: SupportAssurance::PreflightOnly,
            evidence: vec![statement(
                "path_identity_observed",
                "directory or nearest existing ancestor was opened without following symlinks",
            )],
            cow_evidence: CowEvidence::NotApplicable,
        },
    ]
}

fn clone_capability_evidence(fd: &std::os::fd::OwnedFd) -> CloneCapabilityEvidence {
    clone_capability_evidence_from_result(ffi::clone_capability(fd))
}

fn clone_capability_evidence_from_result(
    result: io::Result<Option<ffi::RawCloneCapability>>,
) -> CloneCapabilityEvidence {
    match result {
        Ok(Some(capability))
            if capability.interface_valid & libc::VOL_CAP_INT_CLONE == 0 =>
        {
            CloneCapabilityEvidence {
                state: SupportState::Unknown,
                interface_capabilities: Some(capability.interface_capabilities),
                interface_valid: Some(capability.interface_valid),
                errno: None,
                source: EvidenceSource::FgetattrlistHeldDirectoryFd,
                reason: "the volume did not mark VOL_CAP_INT_CLONE as a valid capability bit"
                    .to_owned(),
            }
        }
        Ok(Some(capability))
            if capability.interface_capabilities & libc::VOL_CAP_INT_CLONE != 0 =>
        {
            CloneCapabilityEvidence {
                state: SupportState::Supported,
                interface_capabilities: Some(capability.interface_capabilities),
                interface_valid: Some(capability.interface_valid),
                errno: None,
                source: EvidenceSource::FgetattrlistHeldDirectoryFd,
                reason: "the volume reports valid VOL_CAP_INT_CLONE preflight capability; no clone was attempted"
                    .to_owned(),
            }
        }
        Ok(Some(capability)) => CloneCapabilityEvidence {
            state: SupportState::Unsupported,
            interface_capabilities: Some(capability.interface_capabilities),
            interface_valid: Some(capability.interface_valid),
            errno: None,
            source: EvidenceSource::FgetattrlistHeldDirectoryFd,
            reason: "the volume reports valid VOL_CAP_INT_CLONE as absent".to_owned(),
        },
        Ok(None) => CloneCapabilityEvidence {
            state: SupportState::Unknown,
            interface_capabilities: None,
            interface_valid: None,
            errno: None,
            source: EvidenceSource::FgetattrlistHeldDirectoryFd,
            reason: "ATTR_VOL_CAPABILITIES was not returned by the filesystem".to_owned(),
        },
        Err(error) => CloneCapabilityEvidence {
            state: SupportState::Unknown,
            interface_capabilities: None,
            interface_valid: None,
            errno: error.raw_os_error(),
            source: EvidenceSource::FgetattrlistHeldDirectoryFd,
            reason: format!("fgetattrlist ATTR_VOL_CAPABILITIES failed: {error}"),
        },
    }
}

fn pair_evidence(
    left_role: PathRole,
    left: &PathCapabilityReport,
    right_role: PathRole,
    right: &PathCapabilityReport,
) -> PathPairEvidence {
    let (relation, evidence) = volume_relation(&left.filesystem, &right.filesystem);
    PathPairEvidence {
        left: left_role,
        right: right_role,
        relation,
        evidence,
    }
}

fn volume_relation(
    left: &FileSystemIdentity,
    right: &FileSystemIdentity,
) -> (VolumeRelation, Vec<EvidenceStatement>) {
    match (&left.volume_uuid, &right.volume_uuid) {
        (
            Evidence::Known {
                value: left_uuid, ..
            },
            Evidence::Known {
                value: right_uuid, ..
            },
        ) if left_uuid == right_uuid => {
            if left.type_name == right.type_name && left.fsid == right.fsid {
                (
                    VolumeRelation::SameVolume,
                    vec![statement(
                        "matching_volume_identity",
                        format!(
                            "both held FDs reported Volume UUID {left_uuid}, filesystem type {}, and fsid {:?}",
                            left.type_name, left.fsid
                        ),
                    )],
                )
            } else {
                (
                    VolumeRelation::Unknown,
                    vec![statement(
                        "contradictory_volume_identity",
                        format!(
                            "matching Volume UUID {left_uuid} conflicts with filesystem type/fsid evidence: {}/{:?} versus {}/{:?}",
                            left.type_name, left.fsid, right.type_name, right.fsid
                        ),
                    )],
                )
            }
        }
        (
            Evidence::Known {
                value: left_uuid, ..
            },
            Evidence::Known {
                value: right_uuid, ..
            },
        ) => (
            VolumeRelation::DifferentVolume,
            vec![statement(
                "different_volume_uuid",
                format!("held FDs reported Volume UUIDs {left_uuid} and {right_uuid}"),
            )],
        ),
        _ if left.type_name != right.type_name => (
            VolumeRelation::DifferentVolume,
            vec![statement(
                "different_filesystem_type",
                format!(
                    "held FDs reported filesystem types {} and {}",
                    left.type_name, right.type_name
                ),
            )],
        ),
        _ => (
            VolumeRelation::Unknown,
            vec![statement(
                "volume_uuid_unavailable",
                "a stable Volume UUID was unavailable for at least one held FD",
            )],
        ),
    }
}

fn candidate_evidence(
    candidate: MaterializerCandidate,
    paths: &[(PathRole, &PathCapabilityReport); 4],
) -> CandidateEvidence {
    match candidate {
        MaterializerCandidate::ApfsFileClone => apfs_clone_candidate_evidence(paths),
        MaterializerCandidate::FullCopy => full_copy_candidate_evidence(paths),
    }
}

fn apfs_clone_candidate_evidence(
    paths: &[(PathRole, &PathCapabilityReport); 4],
) -> CandidateEvidence {
    let mut evidence = Vec::new();
    let mut state = SupportState::Supported;

    if paths[0].1.resolution != PathResolution::ExistingDirectory {
        state = SupportState::Unsupported;
        evidence.push(statement(
            "source_missing",
            "the APFS clone source directory does not exist",
        ));
    }
    if paths
        .iter()
        .any(|(_, path)| path.filesystem.type_name != "apfs")
    {
        state = SupportState::Unsupported;
        evidence.push(statement(
            "path_not_apfs",
            "at least one held path FD is not on APFS",
        ));
    }
    if paths
        .iter()
        .any(|(_, path)| path.filesystem.clone_capability.state == SupportState::Unsupported)
    {
        state = SupportState::Unsupported;
        evidence.push(statement(
            "clone_capability_absent",
            "at least one held path FD reports valid VOL_CAP_INT_CLONE as absent",
        ));
    } else if paths
        .iter()
        .any(|(_, path)| path.filesystem.clone_capability.state == SupportState::Unknown)
        && state == SupportState::Supported
    {
        state = SupportState::Unknown;
        evidence.push(statement(
            "clone_capability_unknown",
            "at least one held path FD lacks valid VOL_CAP_INT_CLONE evidence",
        ));
    }

    let relations = [
        volume_relation(&paths[0].1.filesystem, &paths[1].1.filesystem).0,
        volume_relation(&paths[0].1.filesystem, &paths[2].1.filesystem).0,
        volume_relation(&paths[0].1.filesystem, &paths[3].1.filesystem).0,
        volume_relation(&paths[1].1.filesystem, &paths[2].1.filesystem).0,
        volume_relation(&paths[1].1.filesystem, &paths[3].1.filesystem).0,
        volume_relation(&paths[2].1.filesystem, &paths[3].1.filesystem).0,
    ];
    let has_different_volume = relations.contains(&VolumeRelation::DifferentVolume);
    let has_unknown_volume = relations.contains(&VolumeRelation::Unknown);
    if has_different_volume {
        state = SupportState::Unsupported;
        evidence.push(statement(
            "cross_volume",
            "at least one source, target, staging, or trash pair is on a different volume",
        ));
    } else if has_unknown_volume && state == SupportState::Supported {
        state = SupportState::Unknown;
        evidence.push(statement(
            "volume_relation_unknown",
            "at least one source, target, staging, or trash pair lacks consistent volume identity evidence",
        ));
    }

    incorporate_source_readability(paths, &mut state, &mut evidence);
    incorporate_destination_writability(paths, &mut state, &mut evidence);
    if state == SupportState::Supported {
        evidence.push(statement(
            "apfs_clone_preconditions_satisfied",
            "read-only preflight found one APFS Volume and writable destination ancestors; clone execution remains untested",
        ));
    }

    CandidateEvidence {
        candidate: MaterializerCandidate::ApfsFileClone,
        state,
        assurance: SupportAssurance::PreflightOnly,
        execution: CandidateExecution::NotAttemptedByProbe,
        policy_decision: ProductPolicyDecision::NotEvaluatedByPlatformProbe,
        evidence,
        cow_evidence: CowEvidence::NotExecutedByProbe,
    }
}

fn full_copy_candidate_evidence(
    paths: &[(PathRole, &PathCapabilityReport); 4],
) -> CandidateEvidence {
    let mut state = SupportState::Supported;
    let mut evidence = Vec::new();
    if paths[0].1.resolution != PathResolution::ExistingDirectory {
        state = SupportState::Unsupported;
        evidence.push(statement(
            "source_missing",
            "the full-copy source directory does not exist",
        ));
    }
    incorporate_source_readability(paths, &mut state, &mut evidence);
    incorporate_destination_writability(paths, &mut state, &mut evidence);
    if state == SupportState::Supported {
        evidence.push(statement(
            "full_copy_underlying_preconditions_satisfied",
            "read-only preflight found usable destination ancestors; cross-volume product policy was not evaluated",
        ));
    }

    CandidateEvidence {
        candidate: MaterializerCandidate::FullCopy,
        state,
        assurance: SupportAssurance::PreflightOnly,
        execution: CandidateExecution::NotAttemptedByProbe,
        policy_decision: ProductPolicyDecision::NotEvaluatedByPlatformProbe,
        evidence,
        cow_evidence: CowEvidence::NotApplicable,
    }
}

fn incorporate_source_readability(
    paths: &[(PathRole, &PathCapabilityReport); 4],
    state: &mut SupportState,
    evidence: &mut Vec<EvidenceStatement>,
) {
    match &paths[0].1.readability.effective_access_preflight {
        AccessPreflight::Allowed { .. } => {}
        AccessPreflight::Denied { .. } => {
            *state = SupportState::Unsupported;
            evidence.push(statement(
                "source_not_readable_at_inspection",
                "the source failed the effective read-and-search access preflight",
            ));
        }
        AccessPreflight::Unknown { .. } if *state == SupportState::Supported => {
            *state = SupportState::Unknown;
            evidence.push(statement(
                "source_readability_unknown",
                "source read-and-search access could not be preflighted",
            ));
        }
        AccessPreflight::Unknown { .. } => {}
    }
}

fn incorporate_destination_writability(
    paths: &[(PathRole, &PathCapabilityReport); 4],
    state: &mut SupportState,
    evidence: &mut Vec<EvidenceStatement>,
) {
    for (role, path) in &paths[1..] {
        match (
            &path.writability.mount_state,
            &path.writability.effective_access_preflight,
        ) {
            (MountWriteState::ReadOnly, _) | (_, AccessPreflight::Denied { .. }) => {
                *state = SupportState::Unsupported;
                evidence.push(statement(
                    "destination_not_writable_at_inspection",
                    format!("{role:?} failed a mount or effective-access preflight"),
                ));
            }
            (_, AccessPreflight::Unknown { .. }) if *state == SupportState::Supported => {
                *state = SupportState::Unknown;
                evidence.push(statement(
                    "destination_writability_unknown",
                    format!("{role:?} writability could not be preflighted"),
                ));
            }
            _ => {}
        }
    }
}

fn mount_evidence(raw_flags: u32) -> MountEvidence {
    let known = [
        (libc::MNT_RDONLY as u32, MountFlag::ReadOnly),
        (libc::MNT_NOEXEC as u32, MountFlag::NoExec),
        (libc::MNT_NOSUID as u32, MountFlag::NoSuid),
        (libc::MNT_NODEV as u32, MountFlag::NoDevice),
        (libc::MNT_LOCAL as u32, MountFlag::Local),
        (libc::MNT_JOURNALED as u32, MountFlag::Journaled),
        (libc::MNT_AUTOMOUNTED as u32, MountFlag::Automounted),
    ];
    let recognized_flags = known
        .into_iter()
        .filter_map(|(raw, flag)| (raw_flags & raw != 0).then_some(flag))
        .collect();
    MountEvidence {
        raw_flags,
        recognized_flags,
        source: EvidenceSource::FstatfsHeldDirectoryFd,
    }
}

fn writability_evidence(fd: &std::os::fd::OwnedFd, mount: &MountEvidence) -> WritabilityEvidence {
    let mount_state = if mount.recognized_flags.contains(&MountFlag::ReadOnly) {
        MountWriteState::ReadOnly
    } else {
        MountWriteState::WritableAtInspection
    };
    let effective_access_preflight = write_access_preflight(ffi::effective_write_search_access(fd));
    WritabilityEvidence {
        mount_state,
        effective_access_preflight,
        execution_guarantee: ExecutionGuarantee::NotEstablishedByReadOnlyProbe,
        write_attempt: WriteAttempt::NotPerformedReadOnlyProbe,
    }
}

fn write_access_preflight(result: io::Result<()>) -> AccessPreflight {
    match result {
        Ok(()) => AccessPreflight::Allowed {
            source: EvidenceSource::FaccessatHeldDirectoryFd,
        },
        Err(error) if matches!(error.raw_os_error(), Some(libc::EACCES) | Some(libc::EROFS)) => {
            AccessPreflight::Denied {
                errno: error.raw_os_error(),
                reason: error.to_string(),
                source: EvidenceSource::FaccessatHeldDirectoryFd,
            }
        }
        Err(error) => AccessPreflight::Unknown {
            errno: error.raw_os_error(),
            reason: error.to_string(),
            source: EvidenceSource::FaccessatHeldDirectoryFd,
        },
    }
}

fn readability_evidence(fd: &std::os::fd::OwnedFd) -> ReadabilityEvidence {
    let effective_access_preflight = read_access_preflight(ffi::effective_read_search_access(fd));
    ReadabilityEvidence {
        effective_access_preflight,
        execution_guarantee: ExecutionGuarantee::NotEstablishedByReadOnlyProbe,
        read_attempt: ReadAttempt::NotPerformedReadOnlyProbe,
    }
}

fn read_access_preflight(result: io::Result<()>) -> AccessPreflight {
    match result {
        Ok(()) => AccessPreflight::Allowed {
            source: EvidenceSource::FaccessatHeldDirectoryFd,
        },
        Err(error) if error.raw_os_error() == Some(libc::EACCES) => AccessPreflight::Denied {
            errno: error.raw_os_error(),
            reason: error.to_string(),
            source: EvidenceSource::FaccessatHeldDirectoryFd,
        },
        Err(error) => AccessPreflight::Unknown {
            errno: error.raw_os_error(),
            reason: error.to_string(),
            source: EvidenceSource::FaccessatHeldDirectoryFd,
        },
    }
}

fn compare_ancestry(
    before: &[DirectoryIdentityEvidence],
    after: &[DirectoryIdentityEvidence],
    changes: &mut Vec<RevalidationChange>,
) {
    let count = before.len().max(after.len());
    for index in 0..count {
        let before_item = before.get(index);
        let after_item = after.get(index);
        if before_item.map(|item| item.identity) != after_item.map(|item| item.identity)
            && let Some(path) = after_item
                .map(|item| item.path.clone())
                .or_else(|| before_item.map(|item| item.path.clone()))
        {
            changes.push(RevalidationChange::AncestryIdentityChanged {
                path,
                before: before_item.map(|item| item.identity),
                after: after_item.map(|item| item.identity),
            });
        }
    }
}

fn normalized_path(raw_components: &[Vec<u8>]) -> PathBuf {
    let mut path = PathBuf::from("/");
    for component in raw_components {
        path.push(OsStr::from_bytes(component));
    }
    path
}

fn encode_path(path: &Path) -> EncodedPath {
    encode_os_str(path.as_os_str())
}

fn encode_os_str(value: &OsStr) -> EncodedPath {
    EncodedPath {
        display: value.to_string_lossy().into_owned(),
        bytes_hex: encode_hex(value.as_bytes()),
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| Some((decode_nibble(pair[0])? << 4) | decode_nibble(pair[1])?))
        .collect()
}

fn decode_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn file_type(kind: ffi::RawFileKind) -> FileType {
    match kind {
        ffi::RawFileKind::Directory => FileType::Directory,
        ffi::RawFileKind::RegularFile => FileType::RegularFile,
        ffi::RawFileKind::SymbolicLink => FileType::SymbolicLink,
        ffi::RawFileKind::Fifo => FileType::Fifo,
        ffi::RawFileKind::Socket => FileType::Socket,
        ffi::RawFileKind::CharacterDevice => FileType::CharacterDevice,
        ffi::RawFileKind::BlockDevice => FileType::BlockDevice,
        ffi::RawFileKind::Unknown => FileType::Unknown,
    }
}

fn format_uuid(uuid: [u8; 16]) -> String {
    format!(
        "{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        uuid[0],
        uuid[1],
        uuid[2],
        uuid[3],
        uuid[4],
        uuid[5],
        uuid[6],
        uuid[7],
        uuid[8],
        uuid[9],
        uuid[10],
        uuid[11],
        uuid[12],
        uuid[13],
        uuid[14],
        uuid[15]
    )
}

fn statement(code: impl Into<String>, detail: impl Into<String>) -> EvidenceStatement {
    EvidenceStatement {
        code: code.into(),
        detail: detail.into(),
    }
}

fn system_call_error(operation: &str, path: Option<&Path>, error: io::Error) -> ProbeError {
    ProbeError::SystemCall {
        operation: operation.to_owned(),
        path: path.map(encode_path),
        errno: error.raw_os_error(),
        message: error.to_string(),
    }
}

fn observed_at_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        AccessPreflight, Evidence, EvidenceSource, FailedLookupClassification, FileType,
        MaterializerCandidate, PathResolution, PathRole, ProbeError, SupportState, VolumeRelation,
        apfs_clone_candidate_evidence, classify_failed_lookup,
        clone_capability_evidence_from_result, decode_hex, encode_hex, existing_entry_error,
        full_copy_candidate_evidence, inspect_path, path_candidate_evidence, read_access_preflight,
        volume_relation, volume_uuid_evidence, write_access_preflight,
    };
    use crate::ffi::RawCloneCapability;

    #[test]
    fn hex_round_trip_is_lossless() {
        let bytes = b"/path/\x00/\xff";
        assert_eq!(
            decode_hex(&encode_hex(bytes)).as_deref(),
            Some(bytes.as_slice())
        );
    }

    #[test]
    fn hex_decoder_rejects_malformed_evidence() {
        assert_eq!(decode_hex("0"), None);
        assert_eq!(decode_hex("gg"), None);
        assert_eq!(decode_hex("AF"), Some(vec![0xaf]));
    }

    #[test]
    fn contradictory_same_uuid_filesystem_evidence_is_unknown_everywhere() {
        let source = inspect_path(Path::new(env!("CARGO_MANIFEST_DIR")))
            .expect("inspect actual APFS path before synthesizing contradiction");
        let mut contradictory = source.clone();
        contradictory.filesystem.fsid[0] ^= 1;

        assert_eq!(
            volume_relation(&source.filesystem, &contradictory.filesystem).0,
            VolumeRelation::Unknown
        );

        let paths = [
            (PathRole::Source, &source),
            (PathRole::TargetRoot, &contradictory),
            (PathRole::Staging, &source),
            (PathRole::Trash, &source),
        ];
        let candidate = apfs_clone_candidate_evidence(&paths);
        assert_eq!(candidate.candidate, MaterializerCandidate::ApfsFileClone);
        assert_eq!(candidate.state, SupportState::Unknown);
    }

    #[test]
    fn clone_capability_failure_preserves_errno() {
        let evidence = clone_capability_evidence_from_result(Err(
            std::io::Error::from_raw_os_error(libc::ENOTSUP),
        ));

        assert_eq!(evidence.state, SupportState::Unknown);
        assert_eq!(evidence.errno, Some(libc::ENOTSUP));
    }

    #[test]
    fn clone_capability_requires_a_valid_present_interface_bit() {
        let clone_bit = libc::VOL_CAP_INT_CLONE;
        let cases = [
            (clone_bit, 0, SupportState::Unknown),
            (clone_bit, clone_bit, SupportState::Supported),
            (0, clone_bit, SupportState::Unsupported),
        ];

        for (interface_capabilities, interface_valid, expected) in cases {
            let evidence = clone_capability_evidence_from_result(Ok(Some(RawCloneCapability {
                interface_capabilities,
                interface_valid,
            })));
            assert_eq!(evidence.state, expected);
            assert_eq!(
                evidence.interface_capabilities,
                Some(interface_capabilities)
            );
            assert_eq!(evidence.interface_valid, Some(interface_valid));
            assert_eq!(evidence.errno, None);
        }

        let unavailable = clone_capability_evidence_from_result(Ok(None));
        assert_eq!(unavailable.state, SupportState::Unknown);
        assert_eq!(unavailable.interface_capabilities, None);
        assert_eq!(unavailable.interface_valid, None);
        assert_eq!(unavailable.errno, None);
    }

    #[test]
    fn volume_uuid_mapping_distinguishes_value_zero_absent_and_error() {
        assert!(matches!(
            volume_uuid_evidence(Ok(Some([1; 16]))),
            Evidence::Known {
                source: EvidenceSource::FgetattrlistHeldDirectoryFd,
                ..
            }
        ));
        assert!(matches!(
            volume_uuid_evidence(Ok(Some([0; 16]))),
            Evidence::Unknown {
                errno: None,
                reason
            } if reason.contains("all-zero")
        ));
        assert!(matches!(
            volume_uuid_evidence(Ok(None)),
            Evidence::Unknown {
                errno: None,
                reason
            } if reason.contains("not returned")
        ));
        assert!(matches!(
            volume_uuid_evidence(Err(std::io::Error::from_raw_os_error(libc::EIO))),
            Evidence::Unknown {
                errno: Some(libc::EIO),
                ..
            }
        ));
    }

    #[test]
    fn access_preflight_mapping_preserves_denial_and_uncertainty() {
        assert!(matches!(
            write_access_preflight(Ok(())),
            AccessPreflight::Allowed { .. }
        ));
        for errno in [libc::EACCES, libc::EROFS] {
            assert!(matches!(
                write_access_preflight(Err(std::io::Error::from_raw_os_error(errno))),
                AccessPreflight::Denied {
                    errno: Some(observed),
                    ..
                } if observed == errno
            ));
        }
        assert!(matches!(
            write_access_preflight(Err(std::io::Error::from_raw_os_error(libc::EIO))),
            AccessPreflight::Unknown {
                errno: Some(libc::EIO),
                ..
            }
        ));

        assert!(matches!(
            read_access_preflight(Ok(())),
            AccessPreflight::Allowed { .. }
        ));
        assert!(matches!(
            read_access_preflight(Err(std::io::Error::from_raw_os_error(libc::EACCES))),
            AccessPreflight::Denied {
                errno: Some(libc::EACCES),
                ..
            }
        ));
        for errno in [libc::EROFS, libc::EIO] {
            assert!(matches!(
                read_access_preflight(Err(std::io::Error::from_raw_os_error(errno))),
                AccessPreflight::Unknown {
                    errno: Some(observed),
                    ..
                } if observed == errno
            ));
        }
    }

    #[test]
    fn missing_target_metadata_failures_preserve_the_metadata_error() {
        for errno in [libc::EIO, libc::EACCES, libc::ESTALE] {
            assert_eq!(
                classify_failed_lookup(Some(libc::ENOENT), Some(errno)),
                FailedLookupClassification::MetadataError
            );
        }
    }

    #[test]
    fn failed_lookup_classifier_only_accepts_two_enoent_observations_as_missing() {
        let cases = [
            (
                Some(libc::ENOENT),
                Some(libc::ENOENT),
                FailedLookupClassification::MissingTarget,
            ),
            (
                Some(libc::ENOENT),
                Some(libc::EIO),
                FailedLookupClassification::MetadataError,
            ),
            (
                Some(libc::EACCES),
                Some(libc::EIO),
                FailedLookupClassification::OpenError,
            ),
            (
                Some(libc::ENOENT),
                None,
                FailedLookupClassification::PathChanged,
            ),
            (
                Some(libc::ELOOP),
                None,
                FailedLookupClassification::PathChanged,
            ),
            (
                Some(libc::EACCES),
                None,
                FailedLookupClassification::OpenError,
            ),
        ];

        for (open_errno, metadata_errno, expected) in cases {
            assert_eq!(classify_failed_lookup(open_errno, metadata_errno), expected);
        }
    }

    #[test]
    fn existing_entry_errors_preserve_final_type_path_and_errno() {
        let path = Path::new("/controlled/entry");
        for errno in [libc::ENOENT, libc::ELOOP, libc::ENOTDIR] {
            assert!(matches!(
                existing_entry_error(
                    std::io::Error::from_raw_os_error(errno),
                    FileType::Directory,
                    path
                ),
                ProbeError::PathChangedDuringInspection { path: observed }
                    if observed.display == path.display().to_string()
            ));
        }
        for errno in [libc::EACCES, libc::EIO] {
            assert!(matches!(
                existing_entry_error(
                    std::io::Error::from_raw_os_error(errno),
                    FileType::Directory,
                    path
                ),
                ProbeError::SystemCall {
                    operation,
                    errno: Some(observed),
                    ..
                } if operation == "openat directory with O_NOFOLLOW" && observed == errno
            ));
        }
        assert!(matches!(
            existing_entry_error(
                std::io::Error::from_raw_os_error(libc::ELOOP),
                FileType::SymbolicLink,
                path
            ),
            ProbeError::SymbolicLinkEncountered { path: observed }
                if observed.display == path.display().to_string()
        ));
        assert!(matches!(
            existing_entry_error(
                std::io::Error::from_raw_os_error(libc::EACCES),
                FileType::RegularFile,
                path
            ),
            ProbeError::UnsupportedFileType {
                path: observed,
                file_type: FileType::RegularFile
            } if observed.display == path.display().to_string()
        ));
    }

    #[test]
    fn volume_relation_requires_consistent_identity_evidence() {
        let left = inspect_path(Path::new(env!("CARGO_MANIFEST_DIR")))
            .expect("inspect actual APFS path")
            .filesystem;

        assert_eq!(volume_relation(&left, &left).0, VolumeRelation::SameVolume);

        let mut different_uuid = left.clone();
        different_uuid.volume_uuid = Evidence::Known {
            value: "00000000-0000-0000-0000-000000000001".to_owned(),
            source: EvidenceSource::FgetattrlistHeldDirectoryFd,
        };
        assert_eq!(
            volume_relation(&left, &different_uuid).0,
            VolumeRelation::DifferentVolume
        );

        let mut unavailable = left.clone();
        unavailable.volume_uuid = Evidence::Unknown {
            reason: "synthetic unavailable UUID".to_owned(),
            errno: None,
        };
        assert_eq!(
            volume_relation(&left, &unavailable).0,
            VolumeRelation::Unknown
        );

        unavailable.type_name = "syntheticfs".to_owned();
        assert_eq!(
            volume_relation(&left, &unavailable).0,
            VolumeRelation::DifferentVolume
        );

        let mut contradictory_type = left.clone();
        contradictory_type.type_name = "syntheticfs".to_owned();
        assert_eq!(
            volume_relation(&left, &contradictory_type).0,
            VolumeRelation::Unknown
        );
    }

    #[test]
    fn path_candidate_requires_apfs_uuid_and_supported_clone_capability() {
        let base = inspect_path(Path::new(env!("CARGO_MANIFEST_DIR")))
            .expect("inspect actual APFS path before synthesizing evidence")
            .filesystem;
        let clone_state = |filesystem| {
            path_candidate_evidence(filesystem)
                .into_iter()
                .find(|candidate| candidate.candidate == MaterializerCandidate::ApfsFileClone)
                .expect("APFS clone path candidate exists")
                .state
        };

        assert_eq!(clone_state(&base), SupportState::Supported);

        let mut unknown_uuid = base.clone();
        unknown_uuid.volume_uuid = Evidence::Unknown {
            reason: "synthetic unavailable UUID".to_owned(),
            errno: None,
        };
        assert_eq!(clone_state(&unknown_uuid), SupportState::Unknown);

        let mut unknown_capability = base.clone();
        unknown_capability.clone_capability.state = SupportState::Unknown;
        assert_eq!(clone_state(&unknown_capability), SupportState::Unknown);

        let mut absent_capability = base.clone();
        absent_capability.clone_capability.state = SupportState::Unsupported;
        assert_eq!(clone_state(&absent_capability), SupportState::Unsupported);

        let mut other_filesystem = base.clone();
        other_filesystem.type_name = "syntheticfs".to_owned();
        assert_eq!(clone_state(&other_filesystem), SupportState::Unsupported);
    }

    #[test]
    fn combined_candidate_state_preserves_unknown_and_unsupported_priority() {
        let base = inspect_path(Path::new(env!("CARGO_MANIFEST_DIR")))
            .expect("inspect actual APFS path before synthesizing evidence");
        let source = base.clone();
        let mut target = base.clone();
        let staging = base.clone();
        let trash = base.clone();

        let supported_paths = [
            (PathRole::Source, &source),
            (PathRole::TargetRoot, &source),
            (PathRole::Staging, &staging),
            (PathRole::Trash, &trash),
        ];
        let supported_clone = apfs_clone_candidate_evidence(&supported_paths);
        assert_eq!(supported_clone.state, SupportState::Supported);
        assert!(
            supported_clone
                .evidence
                .iter()
                .any(|item| item.code == "apfs_clone_preconditions_satisfied")
        );
        let supported_copy = full_copy_candidate_evidence(&supported_paths);
        assert_eq!(supported_copy.state, SupportState::Supported);
        assert!(
            supported_copy
                .evidence
                .iter()
                .any(|item| item.code == "full_copy_underlying_preconditions_satisfied")
        );

        target.filesystem.clone_capability.state = SupportState::Unknown;
        let paths = [
            (PathRole::Source, &source),
            (PathRole::TargetRoot, &target),
            (PathRole::Staging, &staging),
            (PathRole::Trash, &trash),
        ];
        let unknown_clone = apfs_clone_candidate_evidence(&paths);
        assert_eq!(unknown_clone.state, SupportState::Unknown);
        assert!(
            unknown_clone
                .evidence
                .iter()
                .all(|item| item.code != "apfs_clone_preconditions_satisfied")
        );

        let mut missing_source = source.clone();
        missing_source.resolution = PathResolution::MissingTarget;
        missing_source.readability.effective_access_preflight = AccessPreflight::Unknown {
            errno: Some(libc::EIO),
            reason: "synthetic source access uncertainty".to_owned(),
            source: EvidenceSource::FaccessatHeldDirectoryFd,
        };
        let paths = [
            (PathRole::Source, &missing_source),
            (PathRole::TargetRoot, &target),
            (PathRole::Staging, &staging),
            (PathRole::Trash, &trash),
        ];
        let unsupported_clone = apfs_clone_candidate_evidence(&paths);
        assert_eq!(unsupported_clone.state, SupportState::Unsupported);
        assert!(
            unsupported_clone
                .evidence
                .iter()
                .all(|item| item.code != "apfs_clone_preconditions_satisfied")
        );
        let unsupported_copy = full_copy_candidate_evidence(&paths);
        assert_eq!(unsupported_copy.state, SupportState::Unsupported);
        assert!(
            unsupported_copy
                .evidence
                .iter()
                .all(|item| item.code != "full_copy_underlying_preconditions_satisfied")
        );

        let mut unknown_source = source.clone();
        unknown_source.readability.effective_access_preflight = AccessPreflight::Unknown {
            errno: Some(libc::EIO),
            reason: "synthetic source access uncertainty".to_owned(),
            source: EvidenceSource::FaccessatHeldDirectoryFd,
        };
        let paths = [
            (PathRole::Source, &unknown_source),
            (PathRole::TargetRoot, &source),
            (PathRole::Staging, &staging),
            (PathRole::Trash, &trash),
        ];
        let unknown_copy = full_copy_candidate_evidence(&paths);
        assert_eq!(unknown_copy.state, SupportState::Unknown);
        assert!(
            unknown_copy
                .evidence
                .iter()
                .all(|item| item.code != "full_copy_underlying_preconditions_satisfied")
        );

        let mut unknown_destination = source.clone();
        unknown_destination.writability.effective_access_preflight = AccessPreflight::Unknown {
            errno: Some(libc::EIO),
            reason: "synthetic destination access uncertainty".to_owned(),
            source: EvidenceSource::FaccessatHeldDirectoryFd,
        };
        let paths = [
            (PathRole::Source, &source),
            (PathRole::TargetRoot, &unknown_destination),
            (PathRole::Staging, &staging),
            (PathRole::Trash, &trash),
        ];
        assert_eq!(
            full_copy_candidate_evidence(&paths).state,
            SupportState::Unknown
        );

        let paths = [
            (PathRole::Source, &missing_source),
            (PathRole::TargetRoot, &unknown_destination),
            (PathRole::Staging, &staging),
            (PathRole::Trash, &trash),
        ];
        assert_eq!(
            full_copy_candidate_evidence(&paths).state,
            SupportState::Unsupported
        );
    }
}
