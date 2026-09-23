use thiserror::Error;

use crate::{AbsolutePath, VolumeId};

/// Three-state result of a read-only capability probe.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupportState {
    /// The observed facts satisfy the candidate's preconditions.
    Supported,
    /// The observed facts prove that the candidate cannot be used.
    Unsupported,
    /// The probe could not establish either support or lack of support.
    Unknown,
}

/// Measured host facts reported without claiming an executed clone.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostCapabilityReport {
    platform: String,
    product_version: String,
    kernel_release: String,
    architecture: String,
    adapter: String,
    apfs_clone: SupportState,
}

impl HostCapabilityReport {
    /// Creates a host report from measured platform values.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        platform: impl Into<String>,
        product_version: impl Into<String>,
        kernel_release: impl Into<String>,
        architecture: impl Into<String>,
        adapter: impl Into<String>,
        apfs_clone: SupportState,
    ) -> Self {
        Self {
            platform: platform.into(),
            product_version: product_version.into(),
            kernel_release: kernel_release.into(),
            architecture: architecture.into(),
            adapter: adapter.into(),
            apfs_clone,
        }
    }

    /// Returns the operating-system family.
    #[must_use]
    pub fn platform(&self) -> &str {
        &self.platform
    }

    /// Returns the measured product version.
    #[must_use]
    pub fn product_version(&self) -> &str {
        &self.product_version
    }

    /// Returns the measured kernel release.
    #[must_use]
    pub fn kernel_release(&self) -> &str {
        &self.kernel_release
    }

    /// Returns the measured machine architecture.
    #[must_use]
    pub fn architecture(&self) -> &str {
        &self.architecture
    }

    /// Returns the compiled Adapter name.
    #[must_use]
    pub fn adapter(&self) -> &str {
        &self.adapter
    }

    /// Returns host-level APFS clone-interface evidence.
    #[must_use]
    pub const fn apfs_clone(&self) -> SupportState {
        self.apfs_clone
    }
}

/// Materialization backend selected by a plan or reported by a receipt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaterializerKind {
    /// macOS APFS `fclonefileat` backend.
    ApfsFileClone,
    /// Explicit byte-copy backend implemented by P1-07.
    FullCopy,
}

/// Stable requested, effective, or actual materialization mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaterializationMode {
    /// Require copy-on-write file clones.
    CowClone,
    /// Copy every regular-file byte into independent storage.
    FullCopy,
}

/// Product policy governing a possible second backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FallbackPolicy {
    /// Never substitute another backend.
    Deny,
    /// Permit P1-07 to use Full Copy only for clone-unavailable evidence.
    AllowFullCopyOnCowUnsupported,
}

/// Observable outcome of one materialization attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaterializationOutcome {
    /// The requested tree and its promised manifest were completed.
    Succeeded,
    /// The attempt failed after it had observable filesystem effects.
    Partial,
    /// The attempt failed before creating or modifying a target object.
    Failed,
}

/// Evidence that ordinary files did or did not use CoW.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CowEvidence {
    /// Every promised ordinary file completed a real clone call.
    Confirmed,
    /// The successful tree contained no ordinary file to clone.
    NotUsed,
    /// A failed or incomplete attempt cannot claim CoW completion.
    Unknown,
}

/// Result of returning the target to its pre-attempt baseline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RollbackStatus {
    /// The successful attempt did not require rollback.
    NotNeeded,
    /// Every created object was identity-checked and removed from the target tree.
    ConfirmedBaseline,
    /// At least one created or modified object could not be safely restored.
    Incomplete,
}

/// How a requested path resolved during read-only inspection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathResolution {
    /// The complete path named a directory.
    ExistingDirectory,
    /// A suffix was absent and evidence is anchored to its nearest parent.
    MissingTarget,
}

/// Type of one source or created tree entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaterializedEntryKind {
    /// Directory entry.
    Directory,
    /// Ordinary file entry.
    RegularFile,
    /// Symbolic link entry whose text is copied without following it.
    SymbolicLink,
}

/// Stable reason why an APFS attempt did not complete.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaterializationFailureKind {
    /// The frozen path or capability evidence changed before or during execution.
    PlanStale,
    /// Source and controlled target roots overlap or violate layout policy.
    InvalidLayout,
    /// A source entry is outside the Phase 1 supported type set.
    UnsupportedSourceEntry,
    /// Source contents or promised metadata changed during the attempt.
    SourceChanged,
    /// A target entry no longer has the identity created by this attempt.
    TargetChanged,
    /// The real clone syscall reported that CoW is unsupported.
    CowUnavailable,
    /// The filesystem reported insufficient space.
    NoSpace,
    /// The final source and target manifests did not match.
    ManifestMismatch,
    /// A filesystem operation failed without a narrower classification.
    Filesystem,
}

/// Known evidence or a structured reason that it was unavailable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Evidence<T> {
    /// A typed value was observed.
    Known(T),
    /// The platform could not produce a trustworthy value.
    Unknown {
        /// Safe implementation-owned explanation.
        reason: String,
        /// Platform errno when one exists.
        errno: Option<i32>,
    },
}

impl<T> Evidence<T> {
    /// Returns known evidence without erasing the unknown state.
    #[must_use]
    pub const fn known(&self) -> Option<&T> {
        match self {
            Self::Known(value) => Some(value),
            Self::Unknown { .. } => None,
        }
    }

    /// Returns the safe explanation attached to unknown evidence.
    #[must_use]
    pub fn unknown_reason(&self) -> Option<&str> {
        match self {
            Self::Known(_) => None,
            Self::Unknown { reason, .. } => Some(reason),
        }
    }

    /// Returns the platform errno attached to unknown evidence.
    #[must_use]
    pub const fn unknown_errno(&self) -> Option<i32> {
        match self {
            Self::Known(_) => None,
            Self::Unknown { errno, .. } => *errno,
        }
    }
}

/// Device/inode identity observed from a held descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileIdentity {
    device: u64,
    inode: u64,
}

impl FileIdentity {
    /// Creates one descriptor-derived identity.
    #[must_use]
    pub const fn new(device: u64, inode: u64) -> Self {
        Self { device, inode }
    }

    /// Returns the platform device number.
    #[must_use]
    pub const fn device(self) -> u64 {
        self.device
    }

    /// Returns the inode number.
    #[must_use]
    pub const fn inode(self) -> u64 {
        self.inode
    }
}

/// Identity of one directory in an inspected absolute path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryIdentityEvidence {
    path: AbsolutePath,
    identity: FileIdentity,
}

impl DirectoryIdentityEvidence {
    /// Binds a canonical directory path to its held-descriptor identity.
    #[must_use]
    pub const fn new(path: AbsolutePath, identity: FileIdentity) -> Self {
        Self { path, identity }
    }

    /// Returns the canonical directory path.
    #[must_use]
    pub const fn path(&self) -> &AbsolutePath {
        &self.path
    }

    /// Returns the descriptor-derived identity.
    #[must_use]
    pub const fn identity(&self) -> FileIdentity {
        self.identity
    }
}

/// Filesystem and stable Volume evidence for one held directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileSystemIdentity {
    type_name: String,
    fsid: [i32; 2],
    volume_id: Evidence<VolumeId>,
}

impl FileSystemIdentity {
    /// Creates filesystem evidence returned by a platform Adapter.
    #[must_use]
    pub fn new(
        type_name: impl Into<String>,
        fsid: [i32; 2],
        volume_id: Evidence<VolumeId>,
    ) -> Self {
        Self {
            type_name: type_name.into(),
            fsid,
            volume_id,
        }
    }

    /// Returns the filesystem type reported by the kernel.
    #[must_use]
    pub fn type_name(&self) -> &str {
        &self.type_name
    }

    /// Returns both platform fsid words.
    #[must_use]
    pub const fn fsid(&self) -> [i32; 2] {
        self.fsid
    }

    /// Returns the APFS Volume UUID evidence.
    #[must_use]
    pub const fn volume_id(&self) -> &Evidence<VolumeId> {
        &self.volume_id
    }
}

/// Mount facts relevant to safe materialization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MountEvidence {
    raw_flags: u32,
    writable: bool,
}

impl MountEvidence {
    /// Creates mount evidence from one held directory.
    #[must_use]
    pub const fn new(raw_flags: u32, writable: bool) -> Self {
        Self {
            raw_flags,
            writable,
        }
    }

    /// Returns unmodified platform mount flags.
    #[must_use]
    pub const fn raw_flags(self) -> u32 {
        self.raw_flags
    }

    /// Returns whether the mount was writable at inspection time.
    #[must_use]
    pub const fn writable(self) -> bool {
        self.writable
    }
}

/// Combined support fact for one candidate backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidateEvidence {
    kind: MaterializerKind,
    state: SupportState,
    reasons: Vec<String>,
}

impl CandidateEvidence {
    /// Creates candidate evidence without making a product fallback decision.
    #[must_use]
    pub const fn new(kind: MaterializerKind, state: SupportState, reasons: Vec<String>) -> Self {
        Self {
            kind,
            state,
            reasons,
        }
    }

    /// Returns the evaluated backend.
    #[must_use]
    pub const fn kind(&self) -> MaterializerKind {
        self.kind
    }

    /// Returns its three-state preflight result.
    #[must_use]
    pub const fn state(&self) -> SupportState {
        self.state
    }

    /// Returns stable evidence reason codes.
    #[must_use]
    pub fn reasons(&self) -> &[String] {
        &self.reasons
    }
}

/// Digest of all stable path evidence used to build a Plan.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ProbeEvidenceDigest([u8; 32]);

impl ProbeEvidenceDigest {
    /// Wraps a BLAKE3-sized digest produced by an Adapter.
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the digest bytes.
    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

/// Digest of one promised source or target tree manifest.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TreeDigest([u8; 32]);

impl TreeDigest {
    /// Wraps a BLAKE3-sized tree digest.
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the digest bytes.
    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

/// Read-only evidence for one actual or not-yet-existing path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathCapabilityReport {
    requested_path: AbsolutePath,
    resolution: PathResolution,
    nearest_existing_ancestor: AbsolutePath,
    missing_components: Vec<Vec<u8>>,
    ancestry: Vec<DirectoryIdentityEvidence>,
    filesystem: FileSystemIdentity,
    mount: MountEvidence,
    readability: SupportState,
    writability: SupportState,
    apfs_clone: SupportState,
}

impl PathCapabilityReport {
    /// Creates one internally consistent path report.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        requested_path: AbsolutePath,
        resolution: PathResolution,
        nearest_existing_ancestor: AbsolutePath,
        missing_components: Vec<Vec<u8>>,
        ancestry: Vec<DirectoryIdentityEvidence>,
        filesystem: FileSystemIdentity,
        mount: MountEvidence,
        readability: SupportState,
        writability: SupportState,
        apfs_clone: SupportState,
    ) -> Result<Self, PathCapabilityReportError> {
        if ancestry.is_empty() {
            return Err(PathCapabilityReportError::MissingAncestry);
        }
        if (resolution == PathResolution::ExistingDirectory) != missing_components.is_empty() {
            return Err(PathCapabilityReportError::ResolutionMismatch);
        }
        if ancestry.last().map(DirectoryIdentityEvidence::path) != Some(&nearest_existing_ancestor)
        {
            return Err(PathCapabilityReportError::AncestorMismatch);
        }
        Ok(Self {
            requested_path,
            resolution,
            nearest_existing_ancestor,
            missing_components,
            ancestry,
            filesystem,
            mount,
            readability,
            writability,
            apfs_clone,
        })
    }

    /// Returns the caller-supplied canonical path.
    #[must_use]
    pub const fn requested_path(&self) -> &AbsolutePath {
        &self.requested_path
    }

    /// Returns how much of the requested path existed.
    #[must_use]
    pub const fn resolution(&self) -> PathResolution {
        self.resolution
    }

    /// Returns the path that anchors filesystem evidence.
    #[must_use]
    pub const fn nearest_existing_ancestor(&self) -> &AbsolutePath {
        &self.nearest_existing_ancestor
    }

    /// Returns absent suffix components as lossless bytes.
    #[must_use]
    pub fn missing_components(&self) -> &[Vec<u8>] {
        &self.missing_components
    }

    /// Returns held-descriptor identities from root to the evidence anchor.
    #[must_use]
    pub fn ancestry(&self) -> &[DirectoryIdentityEvidence] {
        &self.ancestry
    }

    /// Returns filesystem identity at the evidence anchor.
    #[must_use]
    pub const fn filesystem(&self) -> &FileSystemIdentity {
        &self.filesystem
    }

    /// Returns mount evidence at the evidence anchor.
    #[must_use]
    pub const fn mount(&self) -> MountEvidence {
        self.mount
    }

    /// Returns source-read preflight evidence.
    #[must_use]
    pub const fn readability(&self) -> SupportState {
        self.readability
    }

    /// Returns destination-write preflight evidence.
    #[must_use]
    pub const fn writability(&self) -> SupportState {
        self.writability
    }

    /// Returns per-path APFS clone preflight evidence.
    #[must_use]
    pub const fn apfs_clone(&self) -> SupportState {
        self.apfs_clone
    }
}

/// Inconsistent fields were supplied for a path report.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum PathCapabilityReportError {
    /// No held-directory ancestry was supplied.
    #[error("path report requires ancestry evidence")]
    MissingAncestry,
    /// Resolution and missing suffix disagree.
    #[error("path resolution does not match its missing suffix")]
    ResolutionMismatch,
    /// The final ancestry item is not the named evidence anchor.
    #[error("path ancestry does not end at its evidence anchor")]
    AncestorMismatch,
}

/// Four-role path report used to choose and later revalidate a backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializationPathReport {
    source: PathCapabilityReport,
    target_root: PathCapabilityReport,
    staging: PathCapabilityReport,
    trash: PathCapabilityReport,
    apfs_clone: CandidateEvidence,
    evidence_digest: ProbeEvidenceDigest,
}

impl MaterializationPathReport {
    /// Creates a combined report from one platform probe invocation.
    #[must_use]
    pub const fn new(
        source: PathCapabilityReport,
        target_root: PathCapabilityReport,
        staging: PathCapabilityReport,
        trash: PathCapabilityReport,
        apfs_clone: CandidateEvidence,
        evidence_digest: ProbeEvidenceDigest,
    ) -> Self {
        Self {
            source,
            target_root,
            staging,
            trash,
            apfs_clone,
            evidence_digest,
        }
    }

    /// Returns source-root evidence.
    #[must_use]
    pub const fn source(&self) -> &PathCapabilityReport {
        &self.source
    }

    /// Returns target-root evidence.
    #[must_use]
    pub const fn target_root(&self) -> &PathCapabilityReport {
        &self.target_root
    }

    /// Returns staging-root evidence.
    #[must_use]
    pub const fn staging(&self) -> &PathCapabilityReport {
        &self.staging
    }

    /// Returns trash-root evidence.
    #[must_use]
    pub const fn trash(&self) -> &PathCapabilityReport {
        &self.trash
    }

    /// Returns APFS candidate evidence.
    #[must_use]
    pub const fn apfs_clone(&self) -> &CandidateEvidence {
        &self.apfs_clone
    }

    /// Returns the digest binding all stable probe evidence.
    #[must_use]
    pub const fn evidence_digest(&self) -> ProbeEvidenceDigest {
        self.evidence_digest
    }
}

/// An APFS plan could not be derived from the supplied facts.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum MaterializationPlanError {
    /// Candidate evidence was for another backend.
    #[error("candidate evidence does not describe APFS file clone")]
    WrongCandidate,
    /// Preflight proved that APFS clone is unavailable.
    #[error("APFS file clone is unsupported for this path combination")]
    CandidateUnsupported,
    /// A required APFS Volume UUID was unknown.
    #[error("materialization path volume identity is unknown")]
    UnknownVolume,
    /// At least one role is not on APFS.
    #[error("materialization path is not on APFS")]
    NotApfs,
    /// The four path roles do not belong to one APFS Volume.
    #[error("materialization paths are on different volumes")]
    DifferentVolume,
}

/// Immutable execution choice derived from one combined path report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializationPlan {
    requested_mode: MaterializationMode,
    effective_mode: MaterializationMode,
    selected_adapter: MaterializerKind,
    fallback_policy: FallbackPolicy,
    probe_evidence_digest: ProbeEvidenceDigest,
    source_path: AbsolutePath,
    target_path: AbsolutePath,
    staging_path: AbsolutePath,
    trash_path: AbsolutePath,
    source_volume_id: VolumeId,
    target_volume_id: VolumeId,
}

impl MaterializationPlan {
    /// Applies the Phase 1 APFS-clone policy to one probe report.
    pub fn for_apfs_clone(
        report: &MaterializationPathReport,
        fallback_policy: FallbackPolicy,
    ) -> Result<Self, MaterializationPlanError> {
        if report.apfs_clone.kind() != MaterializerKind::ApfsFileClone {
            return Err(MaterializationPlanError::WrongCandidate);
        }
        if report.apfs_clone.state() == SupportState::Unsupported {
            return Err(MaterializationPlanError::CandidateUnsupported);
        }
        let source_volume_id = volume_id(report.source())?;
        let target_volume_id = volume_id(report.target_root())?;
        let staging_volume_id = volume_id(report.staging())?;
        let trash_volume_id = volume_id(report.trash())?;
        if [target_volume_id, staging_volume_id, trash_volume_id]
            .into_iter()
            .any(|volume_id| volume_id != source_volume_id)
        {
            return Err(MaterializationPlanError::DifferentVolume);
        }
        Ok(Self {
            requested_mode: MaterializationMode::CowClone,
            effective_mode: MaterializationMode::CowClone,
            selected_adapter: MaterializerKind::ApfsFileClone,
            fallback_policy,
            probe_evidence_digest: report.evidence_digest(),
            source_path: report.source().requested_path().clone(),
            target_path: report.target_root().requested_path().clone(),
            staging_path: report.staging().requested_path().clone(),
            trash_path: report.trash().requested_path().clone(),
            source_volume_id,
            target_volume_id,
        })
    }

    /// Returns the original requested mode.
    #[must_use]
    pub const fn requested_mode(&self) -> MaterializationMode {
        self.requested_mode
    }

    /// Returns the selected effective mode.
    #[must_use]
    pub const fn effective_mode(&self) -> MaterializationMode {
        self.effective_mode
    }

    /// Returns the selected Adapter.
    #[must_use]
    pub const fn selected_adapter(&self) -> MaterializerKind {
        self.selected_adapter
    }

    /// Returns the fallback policy frozen into the plan.
    #[must_use]
    pub const fn fallback_policy(&self) -> FallbackPolicy {
        self.fallback_policy
    }

    /// Returns the probe evidence digest.
    #[must_use]
    pub const fn probe_evidence_digest(&self) -> ProbeEvidenceDigest {
        self.probe_evidence_digest
    }

    /// Returns the source path frozen by the plan.
    #[must_use]
    pub const fn source_path(&self) -> &AbsolutePath {
        &self.source_path
    }

    /// Returns the target path frozen by the plan.
    #[must_use]
    pub const fn target_path(&self) -> &AbsolutePath {
        &self.target_path
    }

    /// Returns the staging path frozen by the plan.
    #[must_use]
    pub const fn staging_path(&self) -> &AbsolutePath {
        &self.staging_path
    }

    /// Returns the trash path frozen by the plan.
    #[must_use]
    pub const fn trash_path(&self) -> &AbsolutePath {
        &self.trash_path
    }

    /// Returns the source APFS Volume UUID.
    #[must_use]
    pub const fn source_volume_id(&self) -> VolumeId {
        self.source_volume_id
    }

    /// Returns the target APFS Volume UUID.
    #[must_use]
    pub const fn target_volume_id(&self) -> VolumeId {
        self.target_volume_id
    }
}

fn volume_id(report: &PathCapabilityReport) -> Result<VolumeId, MaterializationPlanError> {
    if report.filesystem().type_name() != "apfs" {
        return Err(MaterializationPlanError::NotApfs);
    }
    report
        .filesystem()
        .volume_id()
        .known()
        .copied()
        .ok_or(MaterializationPlanError::UnknownVolume)
}

/// Lossless relative pathname used by manifests and receipts.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RelativePath(Vec<u8>);

impl RelativePath {
    /// Parses a non-empty slash-separated relative path without normalization.
    pub fn try_from_bytes(bytes: impl Into<Vec<u8>>) -> Result<Self, RelativePathError> {
        let bytes = bytes.into();
        if bytes.is_empty() || bytes[0] == b'/' || bytes.contains(&0) {
            return Err(RelativePathError);
        }
        if bytes
            .split(|byte| *byte == b'/')
            .any(|component| component.is_empty() || component == b"." || component == b"..")
        {
            return Err(RelativePathError);
        }
        Ok(Self(bytes))
    }

    /// Returns the validated relative-path bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Bytes did not name a safe relative path.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("relative path must contain only non-empty normal components")]
pub struct RelativePathError;

/// One object created by a materialization attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreatedObjectEvidence {
    path: RelativePath,
    kind: MaterializedEntryKind,
    identity: Option<FileIdentity>,
}

impl CreatedObjectEvidence {
    /// Records a created entry and whether its identity was confirmed.
    #[must_use]
    pub const fn new(
        path: RelativePath,
        kind: MaterializedEntryKind,
        identity: Option<FileIdentity>,
    ) -> Self {
        Self {
            path,
            kind,
            identity,
        }
    }

    /// Returns the lossless relative path.
    #[must_use]
    pub const fn path(&self) -> &RelativePath {
        &self.path
    }

    /// Returns the created entry type.
    #[must_use]
    pub const fn kind(&self) -> MaterializedEntryKind {
        self.kind
    }

    /// Returns the identity captured immediately after creation.
    #[must_use]
    pub const fn identity(&self) -> Option<FileIdentity> {
        self.identity
    }
}

/// Structured rollback result included in every failed receipt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RollbackEvidence {
    status: RollbackStatus,
    removed: Vec<RelativePath>,
    quarantined: Vec<RelativePath>,
    unconfirmed_quarantined: Vec<RelativePath>,
    remaining: Vec<CreatedObjectEvidence>,
}

impl RollbackEvidence {
    /// Creates rollback evidence from identity-checked results.
    #[must_use]
    pub const fn new(
        status: RollbackStatus,
        removed: Vec<RelativePath>,
        remaining: Vec<CreatedObjectEvidence>,
    ) -> Self {
        Self {
            status,
            removed,
            quarantined: Vec::new(),
            unconfirmed_quarantined: Vec::new(),
            remaining,
        }
    }

    /// Adds identities safely detached under the plan-bound trash root.
    #[must_use]
    pub fn with_quarantined(mut self, quarantined: Vec<RelativePath>) -> Self {
        self.quarantined = quarantined;
        self
    }

    /// Adds trash locations whose moved identity or restoration was not confirmed.
    #[must_use]
    pub fn with_unconfirmed_quarantined(
        mut self,
        unconfirmed_quarantined: Vec<RelativePath>,
    ) -> Self {
        self.unconfirmed_quarantined = unconfirmed_quarantined;
        self
    }

    /// Returns the overall rollback status.
    #[must_use]
    pub const fn status(&self) -> RollbackStatus {
        self.status
    }

    /// Returns target-relative paths confirmed removed in rollback order.
    #[must_use]
    pub fn removed(&self) -> &[RelativePath] {
        &self.removed
    }

    /// Returns trash-relative quarantine paths retained for later explicit cleanup.
    #[must_use]
    pub fn quarantined(&self) -> &[RelativePath] {
        &self.quarantined
    }

    /// Returns trash-relative locations retained after an unconfirmed restoration.
    #[must_use]
    pub fn unconfirmed_quarantined(&self) -> &[RelativePath] {
        &self.unconfirmed_quarantined
    }

    /// Returns objects that could not be confirmed removed.
    #[must_use]
    pub fn remaining(&self) -> &[CreatedObjectEvidence] {
        &self.remaining
    }
}

/// A validated request containing all four materialization path roles.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializeRequest {
    source: AbsolutePath,
    target: AbsolutePath,
    staging: AbsolutePath,
    trash: AbsolutePath,
}

impl MaterializeRequest {
    /// Creates one request from canonical absolute paths.
    #[must_use]
    pub const fn new(
        source: AbsolutePath,
        target: AbsolutePath,
        staging: AbsolutePath,
        trash: AbsolutePath,
    ) -> Self {
        Self {
            source,
            target,
            staging,
            trash,
        }
    }

    /// Returns the source root.
    #[must_use]
    pub const fn source(&self) -> &AbsolutePath {
        &self.source
    }

    /// Returns the controlled target root.
    #[must_use]
    pub const fn target(&self) -> &AbsolutePath {
        &self.target
    }

    /// Returns the staging root.
    #[must_use]
    pub const fn staging(&self) -> &AbsolutePath {
        &self.staging
    }

    /// Returns the trash root.
    #[must_use]
    pub const fn trash(&self) -> &AbsolutePath {
        &self.trash
    }
}

/// Receipt invariant was violated by an Adapter.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum MaterializationReceiptError {
    /// Successful clone counts do not cover every ordinary file.
    #[error("successful APFS receipt requires one successful clone per ordinary file")]
    CloneCountMismatch,
}

/// Final or partial evidence emitted by one materialization attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializationReceipt {
    requested_mode: MaterializationMode,
    effective_mode: MaterializationMode,
    actual_adapter: MaterializerKind,
    outcome: MaterializationOutcome,
    cow_evidence: CowEvidence,
    source_volume_id: Option<VolumeId>,
    target_volume_id: Option<VolumeId>,
    created: Vec<CreatedObjectEvidence>,
    rollback: RollbackEvidence,
    elapsed_millis: u64,
    logical_bytes: Option<u64>,
    physical_bytes: Option<u64>,
    regular_file_count: Option<u64>,
    clone_calls_succeeded: u64,
    source_manifest_digest: Option<TreeDigest>,
    target_manifest_digest: Option<TreeDigest>,
    failure_kind: Option<MaterializationFailureKind>,
}

/// Facts observed during an APFS attempt, including partial or failed attempts.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MaterializationAttemptEvidence {
    logical_bytes: Option<u64>,
    physical_bytes: Option<u64>,
    regular_file_count: Option<u64>,
    clone_calls_succeeded: u64,
    source_manifest_digest: Option<TreeDigest>,
    target_manifest_digest: Option<TreeDigest>,
}

impl MaterializationAttemptEvidence {
    /// Creates attempt evidence without turning unavailable observations into zeroes.
    #[must_use]
    pub const fn new(
        logical_bytes: Option<u64>,
        physical_bytes: Option<u64>,
        regular_file_count: Option<u64>,
        clone_calls_succeeded: u64,
        source_manifest_digest: Option<TreeDigest>,
        target_manifest_digest: Option<TreeDigest>,
    ) -> Self {
        Self {
            logical_bytes,
            physical_bytes,
            regular_file_count,
            clone_calls_succeeded,
            source_manifest_digest,
            target_manifest_digest,
        }
    }
}

impl MaterializationReceipt {
    /// Builds a successful APFS clone receipt and enforces its CoW claim.
    #[allow(clippy::too_many_arguments)]
    pub fn successful_apfs_clone(
        plan: &MaterializationPlan,
        regular_file_count: u64,
        clone_calls_succeeded: u64,
        created: Vec<CreatedObjectEvidence>,
        source_manifest_digest: TreeDigest,
        target_manifest_digest: TreeDigest,
        elapsed_millis: u64,
        logical_bytes: u64,
        physical_bytes: Option<u64>,
    ) -> Result<Self, MaterializationReceiptError> {
        if regular_file_count != clone_calls_succeeded {
            return Err(MaterializationReceiptError::CloneCountMismatch);
        }
        let cow_evidence = if clone_calls_succeeded == 0 {
            CowEvidence::NotUsed
        } else {
            CowEvidence::Confirmed
        };
        Ok(Self {
            requested_mode: plan.requested_mode(),
            effective_mode: plan.effective_mode(),
            actual_adapter: MaterializerKind::ApfsFileClone,
            outcome: MaterializationOutcome::Succeeded,
            cow_evidence,
            source_volume_id: Some(plan.source_volume_id()),
            target_volume_id: Some(plan.target_volume_id()),
            created,
            rollback: RollbackEvidence::new(RollbackStatus::NotNeeded, Vec::new(), Vec::new()),
            elapsed_millis,
            logical_bytes: Some(logical_bytes),
            physical_bytes,
            regular_file_count: Some(regular_file_count),
            clone_calls_succeeded,
            source_manifest_digest: Some(source_manifest_digest),
            target_manifest_digest: Some(target_manifest_digest),
            failure_kind: None,
        })
    }

    /// Builds a failed or partial APFS receipt without claiming CoW.
    #[must_use]
    pub fn failed_apfs_clone(
        plan: &MaterializationPlan,
        failure_kind: MaterializationFailureKind,
        created: Vec<CreatedObjectEvidence>,
        target_modified: bool,
        rollback: RollbackEvidence,
        evidence: MaterializationAttemptEvidence,
        elapsed_millis: u64,
    ) -> Self {
        let outcome = if target_modified || !created.is_empty() {
            MaterializationOutcome::Partial
        } else {
            MaterializationOutcome::Failed
        };
        Self {
            requested_mode: plan.requested_mode(),
            effective_mode: plan.effective_mode(),
            actual_adapter: MaterializerKind::ApfsFileClone,
            outcome,
            cow_evidence: CowEvidence::Unknown,
            source_volume_id: Some(plan.source_volume_id()),
            target_volume_id: Some(plan.target_volume_id()),
            created,
            rollback,
            elapsed_millis,
            logical_bytes: evidence.logical_bytes,
            physical_bytes: evidence.physical_bytes,
            regular_file_count: evidence.regular_file_count,
            clone_calls_succeeded: evidence.clone_calls_succeeded,
            source_manifest_digest: evidence.source_manifest_digest,
            target_manifest_digest: evidence.target_manifest_digest,
            failure_kind: Some(failure_kind),
        }
    }

    /// Returns the original requested mode.
    #[must_use]
    pub const fn requested_mode(&self) -> MaterializationMode {
        self.requested_mode
    }

    /// Returns the effective planned mode.
    #[must_use]
    pub const fn effective_mode(&self) -> MaterializationMode {
        self.effective_mode
    }

    /// Returns the backend that actually ran.
    #[must_use]
    pub const fn actual_adapter(&self) -> MaterializerKind {
        self.actual_adapter
    }

    /// Returns the attempt outcome.
    #[must_use]
    pub const fn outcome(&self) -> MaterializationOutcome {
        self.outcome
    }

    /// Returns CoW execution evidence.
    #[must_use]
    pub const fn cow_evidence(&self) -> CowEvidence {
        self.cow_evidence
    }

    /// Returns the source Volume UUID when a plan existed.
    #[must_use]
    pub const fn source_volume_id(&self) -> Option<VolumeId> {
        self.source_volume_id
    }

    /// Returns the target Volume UUID when a plan existed.
    #[must_use]
    pub const fn target_volume_id(&self) -> Option<VolumeId> {
        self.target_volume_id
    }

    /// Returns every object created by the attempt in creation order.
    #[must_use]
    pub fn created(&self) -> &[CreatedObjectEvidence] {
        &self.created
    }

    /// Returns rollback evidence.
    #[must_use]
    pub const fn rollback(&self) -> &RollbackEvidence {
        &self.rollback
    }

    /// Returns elapsed wall-clock milliseconds.
    #[must_use]
    pub const fn elapsed_millis(&self) -> u64 {
        self.elapsed_millis
    }

    /// Returns logical ordinary-file bytes.
    #[must_use]
    pub const fn logical_bytes(&self) -> Option<u64> {
        self.logical_bytes
    }

    /// Returns a platform physical-size estimate when available.
    #[must_use]
    pub const fn physical_bytes(&self) -> Option<u64> {
        self.physical_bytes
    }

    /// Returns the number of ordinary files in the promised tree.
    #[must_use]
    pub const fn regular_file_count(&self) -> Option<u64> {
        self.regular_file_count
    }

    /// Returns successful real clone calls.
    #[must_use]
    pub const fn clone_calls_succeeded(&self) -> u64 {
        self.clone_calls_succeeded
    }

    /// Returns the initial source manifest digest when available.
    #[must_use]
    pub const fn source_manifest_digest(&self) -> Option<TreeDigest> {
        self.source_manifest_digest
    }

    /// Returns the final target manifest digest when available.
    #[must_use]
    pub const fn target_manifest_digest(&self) -> Option<TreeDigest> {
        self.target_manifest_digest
    }

    /// Returns the stable failure category for a non-success receipt.
    #[must_use]
    pub const fn failure_kind(&self) -> Option<MaterializationFailureKind> {
        self.failure_kind
    }
}
