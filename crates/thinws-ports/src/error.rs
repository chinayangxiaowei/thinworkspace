use std::error::Error;
use std::fmt;

use crate::materialization::MaterializationPathRole;

/// Stable high-level classification shared by Phase 1 Port failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortErrorKind {
    /// A required document or durable record does not exist.
    NotFound,
    /// Existing durable state conflicts with the requested identity or update.
    Conflict,
    /// External bytes or durable rows violate the frozen schema.
    InvalidData,
    /// A descriptor-backed controlled layout failed identity, type, or volume validation.
    InvalidLayout,
    /// The durable format is newer, foreign, or otherwise unsupported.
    UnsupportedVersion,
    /// The host filesystem or another required platform feature is unavailable.
    CapabilityUnavailable,
    /// A target that must be new or empty contains unowned entries.
    NotEmpty,
    /// A previously registered path or volume cannot currently be reached.
    Unavailable,
    /// An advisory lock was not acquired within its bounded wait.
    Timeout,
    /// A filesystem operation failed.
    Io,
    /// A database operation failed without a more specific classification.
    Storage,
}

/// Resource category for conflicts that a use case may handle differently.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PortConflict {
    /// A bootstrap config or root marker already has different content.
    InstallationIdentity,
    /// A Workspace ID is active or permanently tombstoned.
    WorkspaceId,
    /// Another active Workspace owns the requested name.
    WorkspaceName,
    /// Another active Workspace owns the requested target path.
    TargetPath,
    /// The row no longer has the state expected by the caller.
    ExpectedState,
}

/// Structured external-boundary failure that keeps its concrete source chain.
pub struct PortError {
    kind: PortErrorKind,
    operation: &'static str,
    conflict: Option<PortConflict>,
    materialization_path_role: Option<MaterializationPathRole>,
    source: Option<Box<dyn Error + Send + Sync + 'static>>,
}

impl PortError {
    /// Creates a classified failure without exposing sensitive external text.
    #[must_use]
    pub const fn new(kind: PortErrorKind, operation: &'static str) -> Self {
        Self {
            kind,
            operation,
            conflict: None,
            materialization_path_role: None,
            source: None,
        }
    }

    /// Creates a conflict with the affected durable resource category.
    #[must_use]
    pub const fn conflict(operation: &'static str, conflict: PortConflict) -> Self {
        Self {
            kind: PortErrorKind::Conflict,
            operation,
            conflict: Some(conflict),
            materialization_path_role: None,
            source: None,
        }
    }

    /// Retains a concrete I/O or database failure for typed diagnostics.
    #[must_use]
    pub fn with_source(mut self, source: impl Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    /// Retains an already type-erased source without wrapping away its concrete error.
    #[must_use]
    pub fn with_boxed_source(mut self, source: Box<dyn Error + Send + Sync + 'static>) -> Self {
        self.source = Some(source);
        self
    }

    /// Returns the stable broad failure class.
    #[must_use]
    pub const fn kind(&self) -> PortErrorKind {
        self.kind
    }

    /// Returns the safe operation label.
    #[must_use]
    pub const fn operation(&self) -> &'static str {
        self.operation
    }

    /// Returns a specific conflict category when applicable.
    #[must_use]
    pub const fn conflict_kind(&self) -> Option<PortConflict> {
        self.conflict
    }

    /// Attaches the role that failed during a combined materialization probe.
    #[must_use]
    pub fn with_materialization_path_role(mut self, role: MaterializationPathRole) -> Self {
        self.materialization_path_role = Some(role);
        self
    }

    /// Returns the failed role, without exposing the path bytes.
    #[must_use]
    pub const fn materialization_path_role(&self) -> Option<MaterializationPathRole> {
        self.materialization_path_role
    }
}

impl fmt::Debug for PortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PortError")
            .field("kind", &self.kind)
            .field("operation", &self.operation)
            .field("conflict", &self.conflict)
            .field("materialization_path_role", &self.materialization_path_role)
            .field("has_source", &self.source.is_some())
            .finish()
    }
}

impl fmt::Display for PortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} failed ({:?})", self.operation, self.kind)
    }
}

impl Error for PortError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn Error + 'static))
    }
}
