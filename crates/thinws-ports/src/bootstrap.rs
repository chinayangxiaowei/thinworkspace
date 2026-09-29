use std::time::Duration;

use thinws_core::{
    AbsolutePath, ErrorCode, FileIdentity, GitState, InstallationIdentity, OperationId, ProcessUse,
    RemovalMode, RemovalRefusal, RootMarker, UnixMillis, VolumeId, WorkspaceId,
    WorkspaceReservation,
};

use crate::{PortError, RepositoryInspection};

/// Lifecycle-lock namespace; both variants use the fixed control-root lock file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleScope {
    /// Serializes first-time installation publication.
    Bootstrap,
    /// Serializes lifecycle mutations under the verified control root.
    DataRoot,
}

/// Evidence that an Adapter still owns one advisory lock file descriptor.
pub trait LifecycleLockGuard {
    /// Returns the namespace protected by this guard.
    fn scope(&self) -> LifecycleScope;

    /// Revalidates that the current lock path still names the held descriptor.
    fn revalidate(&self) -> Result<(), PortError>;
}

/// Bounded advisory locking for bootstrap and control-root lifecycle mutations.
pub trait LifecycleLock {
    /// Concrete guard whose drop closes the locked descriptor.
    type Guard: LifecycleLockGuard;

    /// Acquires the fixed bootstrap lock without modifying target state while waiting.
    fn acquire_bootstrap(&self, timeout: Duration) -> Result<Self::Guard, PortError>;

    /// Acquires the lifecycle lock in the already verified control root.
    fn acquire_data_root(
        &self,
        data_root: &AbsolutePath,
        timeout: Duration,
    ) -> Result<Self::Guard, PortError>;
}

/// Result of publishing a durable document with idempotent conflict handling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublishResult {
    /// This call created or advanced the document.
    Published,
    /// The exact already-published document was verified without rewriting it.
    AlreadyCurrent,
}

/// Result of an explicitly authorized, ownership-checked Workspace cleanup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceRemoval {
    /// A previously tombstoned Workspace was selected again; no path was deleted.
    AlreadyAbsent,
    /// The controlled container was removed after deleting this many entries inside root.
    Removed {
        /// Number of entries removed from the former root tree.
        root_entries: usize,
    },
}

/// Current, best-effort byte measurement of one verified Ready copy.
/// Allocated bytes can include APFS blocks shared with other clones.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceSpace {
    /// The bounded scan completed without a detected identity change.
    Complete {
        /// Sum of regular-file and symbolic-link sizes at scan time.
        logical_bytes: u64,
        /// Sum of filesystem-reported allocated bytes, deduplicated by inode.
        allocated_bytes_estimate: u64,
    },
    /// The scan cannot safely supply complete current numbers.
    Unknown,
}

/// One synchronous, ordinary persistent cleanup-log event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemovalLogEvent {
    /// A policy protection stopped cleanup before destructive work.
    Refused,
    /// A cleanup attempt was durably announced before entering Deleting.
    Started,
    /// The controlled container and active metadata were removed.
    Completed,
    /// An attempt began but did not reach complete cleanup.
    Failed,
}

/// Typed fields for one durable JSONL event outside the Workspace copy.
pub struct RemovalLogRecord<'a> {
    /// UTC event time.
    pub occurred_at: UnixMillis,
    /// Correlation identifier for this one explicit attempt.
    pub operation_id: OperationId,
    /// The exact registered Workspace identifier.
    pub workspace_id: WorkspaceId,
    /// Lifecycle result of this event.
    pub event: RemovalLogEvent,
    /// Whether the user explicitly requested force.
    pub mode: RemovalMode,
    /// Conservative Git aggregate; Unknown also covers skipped inspection.
    pub git_state: GitState,
    /// Whether all relevant Git discovery and queries completed.
    pub git_check_complete: bool,
    /// Repository-relative positions and tracked-change counts only.
    pub repositories: &'a [RepositoryInspection],
    /// Best-effort external process scan result, absent if no container was scanned.
    pub process_use: Option<ProcessUse>,
    /// Specific protection that refused the attempt, when applicable.
    pub protection: Option<RemovalRefusal>,
    /// Stable public error code when the attempt failed.
    pub error_code: Option<ErrorCode>,
    /// Actual confirmed cleanup result on completion.
    pub outcome: Option<WorkspaceRemoval>,
}

/// Opaque evidence for the prepared control root held by a platform Adapter.
pub trait PreparedDataRootEvidence {
    /// Returns the canonical path bound to the held directory descriptor.
    fn data_root(&self) -> &AbsolutePath;

    /// Returns the platform's stable filesystem identity observed through that descriptor.
    fn volume_id(&self) -> VolumeId;
}

/// Opaque, revalidatable capability for one controlled metadata database path.
pub trait DataRootLayoutEvidence {
    /// Returns the canonical database path derived by the platform Adapter.
    fn database_path(&self) -> &AbsolutePath;

    /// Revalidates every held directory, database entry and volume identity.
    fn revalidate(&self) -> Result<(), PortError>;
}

/// Descriptor-backed proof of a newly created, incomplete Workspace container.
pub trait PreparedWorkspaceEvidence {
    /// Returns the exact ordinary empty target directory requested by the caller.
    fn target_root(&self) -> &AbsolutePath;

    /// Returns this operation's private staging directory beside the target.
    fn staging_root(&self) -> &AbsolutePath;

    /// Returns this operation's private rollback directory beside the target.
    fn trash_root(&self) -> &AbsolutePath;

    /// Returns the same-run identity of the root held since its creation.
    fn target_identity(&self) -> FileIdentity;

    /// Rechecks directory and incomplete-marker identities without modifying them.
    fn revalidate(&self) -> Result<(), PortError>;
}

/// Versioned bootstrap config and control-root marker persistence.
pub trait BootstrapStore {
    /// Lock guard type accepted by mutating bootstrap operations.
    type LockGuard: LifecycleLockGuard;
    /// Opaque descriptor-backed evidence returned while preparing the control root.
    type PreparedDataRoot: PreparedDataRootEvidence;
    /// Non-copyable same-process evidence returned by initial marker creation.
    type InitializingProof;
    /// Opaque descriptor-backed evidence for the controlled on-disk layout.
    type DataRootLayout: DataRootLayoutEvidence;
    /// Opaque proof of one newly created Workspace container and incomplete marker.
    type PreparedWorkspace: PreparedWorkspaceEvidence;

    /// Creates or validates the fixed private bootstrap directory.
    fn prepare_bootstrap(&self) -> Result<(), PortError>;

    /// Creates or validates the empty private control root and probes its volume.
    fn prepare_data_root(
        &self,
        data_root: &AbsolutePath,
    ) -> Result<Self::PreparedDataRoot, PortError>;

    /// Reads and validates the fixed bootstrap config without following its leaf.
    fn read_config(&self) -> Result<Option<InstallationIdentity>, PortError>;

    /// Reads and validates the marker under the supplied canonical control root.
    fn read_root_marker(&self, data_root: &AbsolutePath) -> Result<Option<RootMarker>, PortError>;

    /// Publishes an initializing marker with no-replace semantics.
    fn create_initializing(
        &self,
        lock: &Self::LockGuard,
        prepared: Self::PreparedDataRoot,
        identity: &InstallationIdentity,
    ) -> Result<Self::InitializingProof, PortError>;

    /// Creates the controlled private layout using the held initializing proof.
    fn initialize_layout(
        &self,
        lock: &Self::LockGuard,
        proof: &Self::InitializingProof,
    ) -> Result<Self::DataRootLayout, PortError>;

    /// Validates an existing Ready layout without creating or repairing entries.
    fn validate_layout(
        &self,
        identity: &InstallationIdentity,
    ) -> Result<Self::DataRootLayout, PortError>;

    /// Creates the exact absent target and same-parent private operation directories.
    /// Requires the matching held lifecycle lock and never adopts an existing entry.
    fn prepare_workspace(
        &self,
        lock: &Self::LockGuard,
        layout: &Self::DataRootLayout,
        workspace_id: WorkspaceId,
        target: &AbsolutePath,
    ) -> Result<Self::PreparedWorkspace, PortError>;

    /// Removes only the exact incomplete marker represented by the held proof.
    fn clear_workspace_incomplete(
        &self,
        lock: &Self::LockGuard,
        layout: &Self::DataRootLayout,
        prepared: Self::PreparedWorkspace,
    ) -> Result<(), PortError>;

    /// Read-only check that a previously Ready Workspace still has its exact
    /// registered target and creation-time ownership proof.
    /// This does not lock out another process or promise future path validity.
    fn validate_ready_workspace(
        &self,
        layout: &Self::DataRootLayout,
        reservation: &WorkspaceReservation,
    ) -> Result<AbsolutePath, PortError>;

    /// Measures the current contents of an ownership-verified Ready root
    /// without following symlinks. Scan incompleteness is Unknown, not a
    /// partial byte count; invalid layout remains an error.
    fn measure_ready_workspace_space(
        &self,
        layout: &Self::DataRootLayout,
        reservation: &WorkspaceReservation,
    ) -> Result<WorkspaceSpace, PortError>;

    /// Finds the single proven target before a process-use scan. An absent
    /// target returns None; active/isolated conflicts and unproven entries fail.
    fn inspect_removal_container(
        &self,
        lock: &Self::LockGuard,
        layout: &Self::DataRootLayout,
        reservation: &WorkspaceReservation,
    ) -> Result<Option<AbsolutePath>, PortError>;

    /// Appends and synchronizes one structured cleanup event in the control root.
    /// Returns the verified log path for user-facing results. Failure before
    /// Started prevents destructive work; this does not create an audit service.
    fn append_removal_log(
        &self,
        lock: &Self::LockGuard,
        layout: &Self::DataRootLayout,
        record: &RemovalLogRecord<'_>,
    ) -> Result<AbsolutePath, PortError>;

    /// Removes one proven-owned Workspace target after Application has persisted
    /// its cleanup intent and authorized the destructive operation. Missing
    /// containers fail without deleting any path or authorizing a tombstone;
    /// existing containers require creation-time ownership proof even for force.
    fn remove_workspace(
        &self,
        lock: &Self::LockGuard,
        layout: &Self::DataRootLayout,
        reservation: &WorkspaceReservation,
    ) -> Result<WorkspaceRemoval, PortError>;

    /// Consumes same-run evidence and atomically advances that exact marker to Ready.
    fn publish_ready(
        &self,
        lock: &Self::LockGuard,
        proof: Self::InitializingProof,
    ) -> Result<RootMarker, PortError>;

    /// Publishes bootstrap config last, accepting only exact idempotence.
    fn publish_config(
        &self,
        lock: &Self::LockGuard,
        identity: &InstallationIdentity,
    ) -> Result<PublishResult, PortError>;
}
