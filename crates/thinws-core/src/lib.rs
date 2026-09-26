#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Pure Phase 1 domain types for ThinWorkspace.

mod diagnostic;
mod git;
mod id;
mod installation;
mod materialization;
mod path;
mod removal;
mod time;
mod volume;
mod workspace;
mod workspace_name;

pub use diagnostic::{ContextValue, CoreError, ErrorCode, ErrorCodeParseError};
pub use git::{
    DiscoveryCompleteness, GitState, RepositoryState, StatusField, StatusParseError,
    aggregate_git_state, parse_tracked_change_count,
};
pub use id::{IdParseError, InstanceId, OperationId, WorkspaceId};
pub use installation::{InstallationIdentity, InstallationRecord, RootMarker, RootMarkerState};
pub use materialization::{
    CandidateEvidence, CowEvidence, CreatedObjectEvidence, DirectoryIdentityEvidence, Evidence,
    FailedMaterializationAttempt, FallbackPolicy, FallbackReason, FileIdentity, FileSystemIdentity,
    HostCapabilityReport, MaterializationAttemptEvidence, MaterializationFailureKind,
    MaterializationMode, MaterializationOutcome, MaterializationPathReport, MaterializationPlan,
    MaterializationPlanError, MaterializationReceipt, MaterializationReceiptError,
    MaterializeRequest, MaterializedEntryKind, MaterializerKind, MountEvidence,
    PathCapabilityReport, PathCapabilityReportError, PathResolution, ProbeEvidenceDigest,
    RelativePath, RelativePathError, RollbackEvidence, RollbackStatus, SupportState, TreeDigest,
};
pub use path::{AbsolutePath, AbsolutePathError};
pub use removal::{ProcessUse, RemovalDecision, RemovalRefusal, RemovalWarning, decide_removal};
pub use time::{UnixMillis, UnixMillisError};
pub use volume::{VolumeId, VolumeIdParseError};
pub use workspace::{
    DeletionTombstone, RemovalMode, WorkspaceEvent, WorkspaceRecord, WorkspaceRecordError,
    WorkspaceReservation, WorkspaceState, WorkspaceStateParseError, WorkspaceTransitionError,
};
pub use workspace_name::{WorkspaceName, WorkspaceNameError};
