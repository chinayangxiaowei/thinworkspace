use thinws_core::{
    DeletionTombstone, ErrorCode, InstanceId, RemovalMode, UnixMillis, WorkspaceId,
    WorkspaceRecord, WorkspaceReservation, WorkspaceState,
};

use crate::PortError;

/// Durable Phase 1 Workspace metadata operations exposed to Application.
pub trait MetadataStore {
    /// Returns the installation row validated when this store was opened.
    fn installation(&self) -> &thinws_core::InstallationRecord;

    /// Reserves a new Workspace as Creating with one constrained insert.
    fn reserve_workspace(
        &mut self,
        reservation: &WorkspaceReservation,
    ) -> Result<WorkspaceRecord, PortError>;

    /// Reads one active Workspace without modifying durable state.
    fn workspace(&self, workspace_id: WorkspaceId) -> Result<Option<WorkspaceRecord>, PortError>;

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
