#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Pure Phase 1 domain types for ThinWorkspace.

mod diagnostic;
mod id;
mod installation;
mod materialization;
mod path;
mod time;
mod volume;
mod workspace;
mod workspace_name;

pub use diagnostic::{ContextValue, CoreError, ErrorCode, ErrorCodeParseError};
pub use id::{IdParseError, InstanceId, OperationId, WorkspaceId};
pub use installation::{InstallationIdentity, InstallationRecord, RootMarker, RootMarkerState};
pub use materialization::{
    CandidateEvidence, CowEvidence, CreatedObjectEvidence, DirectoryIdentityEvidence, Evidence,
    FallbackPolicy, FileIdentity, FileSystemIdentity, HostCapabilityReport,
    MaterializationFailureKind, MaterializationMode, MaterializationOutcome,
    MaterializationPathReport, MaterializationPlan, MaterializationPlanError,
    MaterializationReceipt, MaterializationReceiptError, MaterializeRequest, MaterializedEntryKind,
    MaterializerKind, MountEvidence, PathCapabilityReport, PathCapabilityReportError,
    PathResolution, ProbeEvidenceDigest, RelativePath, RelativePathError, RollbackEvidence,
    RollbackStatus, SupportState, TreeDigest,
};
pub use path::{AbsolutePath, AbsolutePathError};
pub use time::{UnixMillis, UnixMillisError};
pub use volume::{VolumeId, VolumeIdParseError};
pub use workspace::{
    DeletionTombstone, RemovalMode, WorkspaceEvent, WorkspaceRecord, WorkspaceRecordError,
    WorkspaceReservation, WorkspaceState, WorkspaceStateParseError, WorkspaceTransitionError,
};
pub use workspace_name::{WorkspaceName, WorkspaceNameError};
