use std::time::Duration;

use thinws_core::{AbsolutePath, InstallationIdentity, RootMarker, VolumeId};

use crate::PortError;

/// Lifecycle-lock namespace. The data-root path remains in the concrete guard.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleScope {
    /// Serializes first-time installation publication.
    Bootstrap,
    /// Serializes lifecycle mutations in one verified data root.
    DataRoot,
}

/// Evidence that an Adapter still owns one advisory lock file descriptor.
pub trait LifecycleLockGuard {
    /// Returns the namespace protected by this guard.
    fn scope(&self) -> LifecycleScope;

    /// Revalidates that the current lock path still names the held descriptor.
    fn revalidate(&self) -> Result<(), PortError>;
}

/// Bounded advisory locking for bootstrap and data-root lifecycle mutations.
pub trait LifecycleLock {
    /// Concrete guard whose drop closes the locked descriptor.
    type Guard: LifecycleLockGuard;

    /// Acquires the fixed bootstrap lock without modifying target state while waiting.
    fn acquire_bootstrap(&self, timeout: Duration) -> Result<Self::Guard, PortError>;

    /// Acquires the lifecycle lock below one already verified data root.
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

/// Opaque evidence for one prepared data root held by a platform Adapter.
pub trait PreparedDataRootEvidence {
    /// Returns the canonical path bound to the held directory descriptor.
    fn data_root(&self) -> &AbsolutePath;

    /// Returns the actual filesystem volume UUID observed through that descriptor.
    fn volume_id(&self) -> VolumeId;
}

/// Opaque, revalidatable capability for one controlled metadata database path.
pub trait DataRootLayoutEvidence {
    /// Returns the canonical database path derived by the platform Adapter.
    fn database_path(&self) -> &AbsolutePath;

    /// Revalidates every held directory, database entry and volume identity.
    fn revalidate(&self) -> Result<(), PortError>;
}

/// Versioned bootstrap config and data-root marker persistence.
pub trait BootstrapStore {
    /// Lock guard type accepted by mutating bootstrap operations.
    type LockGuard: LifecycleLockGuard;
    /// Opaque descriptor-backed evidence returned while preparing a data root.
    type PreparedDataRoot: PreparedDataRootEvidence;
    /// Non-copyable same-process evidence returned by initial marker creation.
    type InitializingProof;
    /// Opaque descriptor-backed evidence for the controlled on-disk layout.
    type DataRootLayout: DataRootLayoutEvidence;

    /// Creates or validates the fixed private bootstrap directory.
    fn prepare_bootstrap(&self) -> Result<(), PortError>;

    /// Creates or validates an empty private data root and probes its real volume.
    fn prepare_data_root(
        &self,
        data_root: &AbsolutePath,
    ) -> Result<Self::PreparedDataRoot, PortError>;

    /// Reads and validates the fixed bootstrap config without following its leaf.
    fn read_config(&self) -> Result<Option<InstallationIdentity>, PortError>;

    /// Reads and validates the marker under the supplied canonical data root.
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
