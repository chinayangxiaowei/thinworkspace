use std::error::Error;
use std::fmt;

use thinws_core::{
    AbsolutePath, HostCapabilityReport, MaterializationPathReport, MaterializationPlan,
    MaterializationReceipt, MaterializeRequest, MaterializerKind, PathCapabilityReport,
};

use crate::PortError;

/// Four actual path roles inspected together for one materialization decision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializationPathProbeRequest {
    source: AbsolutePath,
    target_root: AbsolutePath,
    staging: AbsolutePath,
    trash: AbsolutePath,
}

impl MaterializationPathProbeRequest {
    /// Creates one combined read-only probe request.
    #[must_use]
    pub const fn new(
        source: AbsolutePath,
        target_root: AbsolutePath,
        staging: AbsolutePath,
        trash: AbsolutePath,
    ) -> Self {
        Self {
            source,
            target_root,
            staging,
            trash,
        }
    }

    /// Returns the source root.
    #[must_use]
    pub const fn source(&self) -> &AbsolutePath {
        &self.source
    }

    /// Returns the controlled target root or its future location.
    #[must_use]
    pub const fn target_root(&self) -> &AbsolutePath {
        &self.target_root
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

impl From<&MaterializeRequest> for MaterializationPathProbeRequest {
    fn from(request: &MaterializeRequest) -> Self {
        Self::new(
            request.source().clone(),
            request.target().clone(),
            request.staging().clone(),
            request.trash().clone(),
        )
    }
}

/// Read-only host and path capability boundary.
pub trait PlatformProbe {
    /// Reports measured host facts without executing a clone.
    fn inspect_host(&self) -> Result<HostCapabilityReport, PortError>;

    /// Reports one path or missing suffix anchored to held descriptors.
    fn inspect_path(&self, path: &AbsolutePath) -> Result<PathCapabilityReport, PortError>;

    /// Combines all four actual path roles into candidate evidence.
    fn inspect_materialization_paths(
        &self,
        request: &MaterializationPathProbeRequest,
    ) -> Result<MaterializationPathReport, PortError>;
}

/// A materialization failure together with all observable attempt evidence.
pub struct MaterializationFailure {
    error: PortError,
    receipt: Box<MaterializationReceipt>,
}

impl MaterializationFailure {
    /// Combines the typed external error with its partial receipt.
    #[must_use]
    pub fn new(error: PortError, receipt: MaterializationReceipt) -> Self {
        Self {
            error,
            receipt: Box::new(receipt),
        }
    }

    /// Returns the classified external-boundary error.
    #[must_use]
    pub const fn error(&self) -> &PortError {
        &self.error
    }

    /// Returns the final partial receipt for this failed attempt.
    #[must_use]
    pub fn receipt(&self) -> &MaterializationReceipt {
        self.receipt.as_ref()
    }
}

impl fmt::Debug for MaterializationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MaterializationFailure")
            .field("error", &self.error)
            .field("receipt", &self.receipt)
            .finish()
    }
}

impl fmt::Display for MaterializationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(formatter)
    }
}

impl Error for MaterializationFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.error)
    }
}

/// Phase 1 directory materialization boundary.
pub trait WorkspaceMaterializer {
    /// Returns the concrete backend kind.
    fn kind(&self) -> MaterializerKind;

    /// Executes one frozen plan after revalidating all path evidence.
    fn materialize(
        &self,
        request: &MaterializeRequest,
        plan: &MaterializationPlan,
    ) -> Result<MaterializationReceipt, MaterializationFailure>;
}
