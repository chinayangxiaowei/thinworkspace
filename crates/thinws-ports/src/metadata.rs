use std::time::Duration;

use thinws_core::{
    CowEvidence, DeletionTombstone, ErrorCode, FallbackReason, InstallationRecord, InstanceId,
    MaterializationMode, MaterializationPlan, MaterializationReceipt, MaterializerKind,
    RemovalMode, UnixMillis, WorkspaceId, WorkspaceRecord, WorkspaceReservation, WorkspaceState,
};

use crate::{DataRootLayoutEvidence, PortError};

/// Product-state snapshot returned by a read-only metadata inspection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetadataSnapshot {
    installation: InstallationRecord,
    workspaces: Vec<WorkspaceRecord>,
}

impl MetadataSnapshot {
    /// Creates a snapshot from an already validated installation and stable Workspace list.
    #[must_use]
    pub const fn new(installation: InstallationRecord, workspaces: Vec<WorkspaceRecord>) -> Self {
        Self {
            installation,
            workspaces,
        }
    }

    /// Returns the installation row observed by the read-only connection.
    #[must_use]
    pub const fn installation(&self) -> &InstallationRecord {
        &self.installation
    }

    /// Returns active Workspaces in stable Workspace-ID order.
    #[must_use]
    pub fn workspaces(&self) -> &[WorkspaceRecord] {
        &self.workspaces
    }
}

/// Typed public-facing facts recovered from an immutable final receipt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalMaterializationSummary {
    requested_mode: MaterializationMode,
    effective_mode: MaterializationMode,
    actual_mode: MaterializationMode,
    adapter: MaterializerKind,
    cow: CowEvidence,
    fallback_reason: Option<FallbackReason>,
    failed_attempt_count: usize,
}

impl FinalMaterializationSummary {
    /// Creates a verified summary from one successful final receipt.
    #[must_use]
    pub const fn new(
        requested_mode: MaterializationMode,
        effective_mode: MaterializationMode,
        actual_mode: MaterializationMode,
        adapter: MaterializerKind,
        cow: CowEvidence,
        fallback_reason: Option<FallbackReason>,
        failed_attempt_count: usize,
    ) -> Self {
        Self {
            requested_mode,
            effective_mode,
            actual_mode,
            adapter,
            cow,
            fallback_reason,
            failed_attempt_count,
        }
    }

    /// Returns the caller's requested mode.
    #[must_use]
    pub const fn requested_mode(&self) -> MaterializationMode {
        self.requested_mode
    }

    /// Returns the mode selected after any permitted fallback.
    #[must_use]
    pub const fn effective_mode(&self) -> MaterializationMode {
        self.effective_mode
    }

    /// Returns the mode used by the completed materialization.
    #[must_use]
    pub const fn actual_mode(&self) -> MaterializationMode {
        self.actual_mode
    }

    /// Returns the executed backend.
    #[must_use]
    pub const fn adapter(&self) -> MaterializerKind {
        self.adapter
    }

    /// Returns evidence for whether ordinary files were cloned.
    #[must_use]
    pub const fn cow(&self) -> CowEvidence {
        self.cow
    }

    /// Returns the reason for a permitted Full Copy fallback, if used.
    #[must_use]
    pub const fn fallback_reason(&self) -> Option<FallbackReason> {
        self.fallback_reason
    }

    /// Returns the number of failed attempts retained by a runtime fallback.
    #[must_use]
    pub const fn failed_attempt_count(&self) -> usize {
        self.failed_attempt_count
    }
}

/// Construction and inspection companion for the MetadataStore boundary.
pub trait MetadataStoreFactory<L: DataRootLayoutEvidence> {
    /// Initializes or validates schema v1 using an already prepared database file.
    fn initialize(
        &self,
        layout: &L,
        expected: &InstallationRecord,
        busy_timeout: Duration,
    ) -> Result<InstallationRecord, PortError>;

    /// Opens existing metadata read-only and returns a validated product snapshot.
    fn inspect(
        &self,
        layout: &L,
        expected: &InstallationRecord,
        busy_timeout: Duration,
    ) -> Result<MetadataSnapshot, PortError>;

    /// Opens an existing, identity-validated installation for lifecycle writes.
    /// It must not create or migrate a missing database.
    fn open_existing(
        &self,
        layout: &L,
        expected: &InstallationRecord,
        busy_timeout: Duration,
    ) -> Result<Box<dyn MetadataStore>, PortError>;
}

/// Durable Phase 1 Workspace metadata operations exposed to Application.
pub trait MetadataStore {
    /// Returns the installation row validated when this store was opened.
    fn installation(&self) -> &thinws_core::InstallationRecord;

    /// Reserves a new Workspace as Creating with one constrained insert.
    fn reserve_workspace(
        &mut self,
        reservation: &WorkspaceReservation,
    ) -> Result<WorkspaceRecord, PortError>;

    /// Atomically records a successful final receipt and moves Creating to Ready.
    /// A failed transaction must leave both the receipt and state unchanged.
    fn complete_materialization(
        &mut self,
        workspace_id: WorkspaceId,
        plan: &MaterializationPlan,
        receipt: &MaterializationReceipt,
        recorded_at: UnixMillis,
    ) -> Result<WorkspaceRecord, PortError>;

    /// Reads one active Workspace without modifying durable state.
    fn workspace(&self, workspace_id: WorkspaceId) -> Result<Option<WorkspaceRecord>, PortError>;

    /// Reads the immutable final receipt as typed facts without exposing storage JSON.
    fn final_materialization(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<FinalMaterializationSummary>, PortError>;

    /// Lists every active Workspace in stable name/ID order.
    fn workspaces(&self) -> Result<Vec<WorkspaceRecord>, PortError>;

    /// Records an operation failure only if the expected state is still current.
    fn record_failure(
        &mut self,
        workspace_id: WorkspaceId,
        expected: WorkspaceState,
        error_code: ErrorCode,
        updated_at: UnixMillis,
    ) -> Result<WorkspaceRecord, PortError>;

    /// Enters or reaffirms Deleting according to Core's ordinary/force rules.
    fn begin_removal(
        &mut self,
        workspace_id: WorkspaceId,
        expected: WorkspaceState,
        mode: RemovalMode,
        updated_at: UnixMillis,
    ) -> Result<WorkspaceRecord, PortError>;

    /// Atomically writes a tombstone and removes a Deleting active row.
    fn complete_deletion(
        &mut self,
        workspace_id: WorkspaceId,
        instance_id: InstanceId,
        deleted_at: UnixMillis,
    ) -> Result<DeletionTombstone, PortError>;
}
