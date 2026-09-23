use std::str::FromStr;

use thiserror::Error;

use crate::{
    AbsolutePath, ErrorCode, InstanceId, UnixMillis, VolumeId, WorkspaceId, WorkspaceName,
};

/// Durable lifecycle state of an active Workspace record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceState {
    /// Identity and target are reserved; materialization is not complete.
    Creating,
    /// Materialization has a final receipt and the ordinary path is usable.
    Ready,
    /// Explicit cleanup has begun and may have external side effects.
    Deleting,
    /// A failed operation left an explainable non-ready record.
    Error,
}

impl WorkspaceState {
    /// Returns the stable SQLite representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Creating => "creating",
            Self::Ready => "ready",
            Self::Deleting => "deleting",
            Self::Error => "error",
        }
    }

    /// Applies one approved domain event.
    pub fn transition(self, event: WorkspaceEvent) -> Result<Self, WorkspaceTransitionError> {
        let next = match (self, event) {
            (Self::Creating, WorkspaceEvent::Materialized) => Self::Ready,
            (Self::Creating | Self::Ready | Self::Deleting, WorkspaceEvent::Failed) => Self::Error,
            (Self::Ready, WorkspaceEvent::BeginRemoval(_)) => Self::Deleting,
            (
                Self::Creating | Self::Deleting | Self::Error,
                WorkspaceEvent::BeginRemoval(RemovalMode::Force),
            ) => Self::Deleting,
            _ => {
                return Err(WorkspaceTransitionError {
                    current: self,
                    event,
                });
            }
        };
        Ok(next)
    }
}

impl FromStr for WorkspaceState {
    type Err = WorkspaceStateParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "creating" => Ok(Self::Creating),
            "ready" => Ok(Self::Ready),
            "deleting" => Ok(Self::Deleting),
            "error" => Ok(Self::Error),
            _ => Err(WorkspaceStateParseError),
        }
    }
}

/// A persisted Workspace state name was not part of the Phase 1 model.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("unknown Workspace state")]
pub struct WorkspaceStateParseError;

/// User intent for entering the destructive deletion state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemovalMode {
    /// Apply ordinary cleanup protections.
    Normal,
    /// Bypass only protections explicitly assigned to `--force`.
    Force,
}

/// One state-machine event approved by the Phase 1 detailed design.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceEvent {
    /// A final materialization receipt has been persisted.
    Materialized,
    /// The current operation failed and recorded a stable error code.
    Failed,
    /// Cleanup was requested with ordinary or explicit-force intent.
    BeginRemoval(RemovalMode),
}

/// An event is not authorized from the current Workspace state.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("event {event:?} is not allowed from Workspace state {current:?}")]
pub struct WorkspaceTransitionError {
    /// State observed before applying the event.
    pub current: WorkspaceState,
    /// Rejected event.
    pub event: WorkspaceEvent,
}

/// Immutable fields reserved before a Workspace is materialized.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceReservation {
    workspace_id: WorkspaceId,
    instance_id: InstanceId,
    name: WorkspaceName,
    source_path: AbsolutePath,
    target_path: AbsolutePath,
    source_volume_id: VolumeId,
    data_volume_id: VolumeId,
    allow_full_copy: bool,
    created_at: UnixMillis,
}

impl WorkspaceReservation {
    /// Builds a reservation from already validated value objects.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        workspace_id: WorkspaceId,
        instance_id: InstanceId,
        name: WorkspaceName,
        source_path: AbsolutePath,
        target_path: AbsolutePath,
        source_volume_id: VolumeId,
        data_volume_id: VolumeId,
        allow_full_copy: bool,
        created_at: UnixMillis,
    ) -> Self {
        Self {
            workspace_id,
            instance_id,
            name,
            source_path,
            target_path,
            source_volume_id,
            data_volume_id,
            allow_full_copy,
            created_at,
        }
    }

    /// Returns the Workspace identifier.
    #[must_use]
    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }
    /// Returns the owning installation identifier.
    #[must_use]
    pub const fn instance_id(&self) -> InstanceId {
        self.instance_id
    }
    /// Returns the unique Workspace name.
    #[must_use]
    pub const fn name(&self) -> &WorkspaceName {
        &self.name
    }
    /// Returns the canonical source path evidence.
    #[must_use]
    pub const fn source_path(&self) -> &AbsolutePath {
        &self.source_path
    }
    /// Returns the unique controlled target path.
    #[must_use]
    pub const fn target_path(&self) -> &AbsolutePath {
        &self.target_path
    }
    /// Returns the source volume observed at reservation time.
    #[must_use]
    pub const fn source_volume_id(&self) -> VolumeId {
        self.source_volume_id
    }
    /// Returns the registered data-root volume.
    #[must_use]
    pub const fn data_volume_id(&self) -> VolumeId {
        self.data_volume_id
    }
    /// Returns whether explicit full-copy fallback was authorized.
    #[must_use]
    pub const fn allow_full_copy(&self) -> bool {
        self.allow_full_copy
    }
    /// Returns the immutable creation timestamp.
    #[must_use]
    pub const fn created_at(&self) -> UnixMillis {
        self.created_at
    }
}

/// One active Workspace row reconstructed from durable metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceRecord {
    reservation: WorkspaceReservation,
    state: WorkspaceState,
    last_error_code: Option<ErrorCode>,
    updated_at: UnixMillis,
}

impl WorkspaceRecord {
    /// Validates state/error and timestamp relationships from durable metadata.
    pub fn new(
        reservation: WorkspaceReservation,
        state: WorkspaceState,
        last_error_code: Option<ErrorCode>,
        updated_at: UnixMillis,
    ) -> Result<Self, WorkspaceRecordError> {
        if updated_at < reservation.created_at {
            return Err(WorkspaceRecordError::TimestampRegression);
        }
        if (state == WorkspaceState::Error) != last_error_code.is_some() {
            return Err(WorkspaceRecordError::ErrorCodeMismatch);
        }
        Ok(Self {
            reservation,
            state,
            last_error_code,
            updated_at,
        })
    }

    /// Returns the immutable reservation fields.
    #[must_use]
    pub const fn reservation(&self) -> &WorkspaceReservation {
        &self.reservation
    }
    /// Returns the current durable state.
    #[must_use]
    pub const fn state(&self) -> WorkspaceState {
        self.state
    }
    /// Returns the stable failure code only for an Error record.
    #[must_use]
    pub const fn last_error_code(&self) -> Option<ErrorCode> {
        self.last_error_code
    }
    /// Returns the last durable update timestamp.
    #[must_use]
    pub const fn updated_at(&self) -> UnixMillis {
        self.updated_at
    }
}

/// A durable row violates relationships enforced by the v1 schema.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum WorkspaceRecordError {
    /// The update time predates immutable creation time.
    #[error("Workspace updated_at must not precede created_at")]
    TimestampRegression,
    /// Error state and last_error_code were not present together.
    #[error("only Error Workspace records have a last_error_code")]
    ErrorCodeMismatch,
}

/// Minimal immutable evidence retained after deleting an active row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeletionTombstone {
    workspace_id: WorkspaceId,
    instance_id: InstanceId,
    deleted_at: UnixMillis,
}

impl DeletionTombstone {
    /// Creates the minimal deletion record.
    #[must_use]
    pub const fn new(
        workspace_id: WorkspaceId,
        instance_id: InstanceId,
        deleted_at: UnixMillis,
    ) -> Self {
        Self {
            workspace_id,
            instance_id,
            deleted_at,
        }
    }

    /// Returns the deleted Workspace identifier.
    #[must_use]
    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }
    /// Returns the owning installation identifier.
    #[must_use]
    pub const fn instance_id(&self) -> InstanceId {
        self.instance_id
    }
    /// Returns the deletion timestamp.
    #[must_use]
    pub const fn deleted_at(&self) -> UnixMillis {
        self.deleted_at
    }
}
