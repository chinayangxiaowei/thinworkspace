#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Phase 1 boundaries between use-case orchestration and external systems.

mod bootstrap;
mod error;
mod git;
mod materialization;
mod metadata;
mod process;

pub use bootstrap::{
    BootstrapStore, DataRootLayoutEvidence, LifecycleLock, LifecycleLockGuard, LifecycleScope,
    PreparedDataRootEvidence, PreparedWorkspaceEvidence, PublishResult,
};
pub use error::{PortConflict, PortError, PortErrorKind};
pub use git::{
    CleanupIoFailure, CleanupOperation, DirectChildExit, GitExit, GitInspection,
    GitInspectionIssue, GitInspector, GitQueryFailureKind, InputField, IoOperation, OutputStream,
    RepositoryInspection,
};
pub use materialization::{
    MaterializationFailure, MaterializationPathProbeRequest, MaterializationPathRole,
    PlatformProbe, WorkspaceMaterializer,
};
pub use metadata::{
    FinalMaterializationSummary, MetadataSnapshot, MetadataStore, MetadataStoreFactory,
};
pub use process::{ProcessObservation, ProcessProbe};
