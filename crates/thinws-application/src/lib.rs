#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Phase 1 use-case orchestration without platform or persistence details.

mod create;

pub use create::{CreateOutcome, CreatePreview, CreateRequest};
pub use thinws_core::{CowEvidence, FallbackReason, MaterializationMode, MaterializerKind};

use std::error::Error;
use std::fmt;
use std::time::Duration;

use thinws_core::{
    AbsolutePath, CoreError, ErrorCode, InstallationIdentity, InstallationRecord, InstanceId,
    RootMarkerState, UnixMillis, WorkspaceState,
};
use thinws_ports::{
    BootstrapStore, DataRootLayoutEvidence, LifecycleLock, LifecycleLockGuard,
    MetadataStoreFactory, PortError, PortErrorKind, PreparedDataRootEvidence,
};

/// Public initialization request after CLI path validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitRequest {
    data_root: AbsolutePath,
    now: UnixMillis,
}

impl InitRequest {
    /// Validates lossless path bytes and a Unix-millisecond clock value at the CLI boundary.
    pub fn try_from_raw(data_root: Vec<u8>, now_ms: i64) -> Result<Self, UseCaseError> {
        let data_root = AbsolutePath::try_from_bytes(data_root).map_err(|_| {
            semantic_error(
                ErrorCode::Usage,
                "data root must be a canonical absolute path",
            )
        })?;
        let now = UnixMillis::new(now_ms).map_err(|_| {
            semantic_error(ErrorCode::Usage, "system clock predates the Unix epoch")
        })?;
        Ok(Self::new(data_root, now))
    }

    /// Creates an initialization request with an explicit clock value.
    #[must_use]
    pub const fn new(data_root: AbsolutePath, now: UnixMillis) -> Self {
        Self { data_root, now }
    }

    /// Returns the requested canonical data-root path.
    #[must_use]
    pub const fn data_root(&self) -> &AbsolutePath {
        &self.data_root
    }

    /// Returns the caller-supplied current time.
    #[must_use]
    pub const fn now(&self) -> UnixMillis {
        self.now
    }
}

/// Whether init created the installation or verified exact idempotence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InitResult {
    /// A new installation reached Ready and published its config.
    Initialized,
    /// The existing installation was validated without rewriting product state.
    AlreadyInitialized,
}

impl InitResult {
    /// Returns the stable CLI/JSON representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Initialized => "initialized",
            Self::AlreadyInitialized => "already-initialized",
        }
    }
}

/// Successful init result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InitOutcome {
    result: InitResult,
    installation: InstallationRecord,
}

impl InitOutcome {
    /// Creates a successful result from an already durable installation.
    #[must_use]
    pub const fn new(result: InitResult, installation: InstallationRecord) -> Self {
        Self {
            result,
            installation,
        }
    }

    /// Returns whether the installation was created or already current.
    #[must_use]
    pub const fn result(&self) -> InitResult {
        self.result
    }

    /// Returns the durable installation identity and creation time.
    #[must_use]
    pub const fn installation(&self) -> &InstallationRecord {
        &self.installation
    }
}

/// Successful product-state diagnostic result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DoctorOutcome {
    installation: InstallationRecord,
    incomplete_workspaces: usize,
}

impl DoctorOutcome {
    /// Creates a successful diagnostic result.
    #[must_use]
    pub const fn new(installation: InstallationRecord, incomplete_workspaces: usize) -> Self {
        Self {
            installation,
            incomplete_workspaces,
        }
    }

    /// Returns the verified installation.
    #[must_use]
    pub const fn installation(&self) -> &InstallationRecord {
        &self.installation
    }

    /// Returns active Workspace records whose state is not Ready.
    #[must_use]
    pub const fn incomplete_workspaces(&self) -> usize {
        self.incomplete_workspaces
    }
}

/// Application failure preserving a public diagnostic and typed Port source.
pub struct UseCaseError {
    diagnostic: CoreError,
    source: Option<PortError>,
}

impl UseCaseError {
    /// Returns the stable public diagnostic.
    #[must_use]
    pub const fn diagnostic(&self) -> &CoreError {
        &self.diagnostic
    }
}

impl fmt::Debug for UseCaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UseCaseError")
            .field("diagnostic", &self.diagnostic)
            .field("has_source", &self.source.is_some())
            .finish()
    }
}

impl fmt::Display for UseCaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.diagnostic.fmt(formatter)
    }
}

impl Error for UseCaseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source
            .as_ref()
            .map(|source| source as &(dyn Error + 'static))
    }
}

/// Single-process Phase 1 init and doctor service.
pub struct ThinWorkspaceService<B, M> {
    bootstrap: B,
    metadata: M,
    lock_timeout: Duration,
    sqlite_timeout: Duration,
}

/// Public init/doctor use cases consumed by the CLI renderer and composition root.
pub trait UseCases {
    /// Initializes a new installation or verifies exact idempotence.
    fn init(&self, request: InitRequest) -> Result<InitOutcome, UseCaseError>;

    /// Inspects the existing installation without mutating product state.
    fn doctor(&self) -> Result<DoctorOutcome, UseCaseError>;
}

impl<B, M> ThinWorkspaceService<B, M> {
    /// Creates a service with explicit timeouts for production or tests.
    #[must_use]
    pub const fn new(
        bootstrap: B,
        metadata: M,
        lock_timeout: Duration,
        sqlite_timeout: Duration,
    ) -> Self {
        Self {
            bootstrap,
            metadata,
            lock_timeout,
            sqlite_timeout,
        }
    }
}

impl<B, M> ThinWorkspaceService<B, M>
where
    B: BootstrapStore + LifecycleLock<Guard = <B as BootstrapStore>::LockGuard>,
    M: MetadataStoreFactory<<B as BootstrapStore>::DataRootLayout>,
{
    /// Initializes a new installation or verifies exact idempotence.
    pub fn init(&self, request: InitRequest) -> Result<InitOutcome, UseCaseError> {
        self.bootstrap
            .prepare_bootstrap()
            .map_err(|error| map_port(Stage::Bootstrap, error))?;
        let lock = self
            .bootstrap
            .acquire_bootstrap(self.lock_timeout)
            .map_err(|error| map_port(Stage::Lock, error))?;
        let current = self
            .bootstrap
            .read_config()
            .map_err(|error| map_port(Stage::Bootstrap, error))?;
        lock.revalidate()
            .map_err(|error| map_port(Stage::Layout, error))?;

        if let Some(identity) = current {
            if identity.data_root() != request.data_root() {
                return Err(semantic_error(
                    ErrorCode::DataRootChangeUnsupported,
                    "changing the initialized data root is unsupported",
                ));
            }
            let (installation, _) = self.inspect_existing(&identity, request.now())?;
            return Ok(InitOutcome {
                result: InitResult::AlreadyInitialized,
                installation,
            });
        }

        let prepared = self
            .bootstrap
            .prepare_data_root(request.data_root())
            .map_err(|error| map_port(Stage::PrepareDataRoot, error))?;
        let identity = InstallationIdentity::new(
            InstanceId::new(),
            prepared.data_root().clone(),
            prepared.volume_id(),
        );
        let proof = self
            .bootstrap
            .create_initializing(&lock, prepared, &identity)
            .map_err(|error| map_port(Stage::Layout, error))?;
        let layout = self
            .bootstrap
            .initialize_layout(&lock, &proof)
            .map_err(|error| map_port(Stage::Layout, error))?;
        let expected = InstallationRecord::new(identity.clone(), request.now());
        let installation = self
            .metadata
            .initialize(&layout, &expected, self.sqlite_timeout)
            .map_err(|error| map_port(Stage::Metadata, error))?;
        layout
            .revalidate()
            .map_err(|error| map_port(Stage::Layout, error))?;
        let ready = self
            .bootstrap
            .publish_ready(&lock, proof)
            .map_err(|error| map_port(Stage::Publish, error))?;
        if ready.state() != RootMarkerState::Ready || ready.identity() != installation.identity() {
            return Err(semantic_error(
                ErrorCode::DataRootLayout,
                "published root marker does not match metadata",
            ));
        }
        self.bootstrap
            .publish_config(&lock, installation.identity())
            .map_err(|error| map_port(Stage::Publish, error))?;
        Ok(InitOutcome {
            result: InitResult::Initialized,
            installation,
        })
    }

    /// Inspects the existing installation without mutating product state.
    pub fn doctor(&self) -> Result<DoctorOutcome, UseCaseError> {
        let identity = self
            .bootstrap
            .read_config()
            .map_err(|error| map_port(Stage::Bootstrap, error))?
            .ok_or_else(|| {
                semantic_error(
                    ErrorCode::NotInitialized,
                    "ThinWorkspace is not initialized",
                )
            })?;
        let epoch = UnixMillis::new(0).expect("zero is a valid Unix millisecond timestamp");
        let (installation, incomplete_workspaces) = self.inspect_existing(&identity, epoch)?;
        Ok(DoctorOutcome {
            installation,
            incomplete_workspaces,
        })
    }

    fn inspect_existing(
        &self,
        identity: &InstallationIdentity,
        placeholder_time: UnixMillis,
    ) -> Result<(InstallationRecord, usize), UseCaseError> {
        let layout = self
            .bootstrap
            .validate_layout(identity)
            .map_err(|error| map_port(Stage::Layout, error))?;
        self.require_ready_marker(identity)?;
        let expected = InstallationRecord::new(identity.clone(), placeholder_time);
        let snapshot = self
            .metadata
            .inspect(&layout, &expected, self.sqlite_timeout)
            .map_err(|error| map_port(Stage::Metadata, error))?;
        layout
            .revalidate()
            .map_err(|error| map_port(Stage::Layout, error))?;
        self.require_ready_marker(identity)?;
        let incomplete = snapshot
            .workspaces()
            .iter()
            .filter(|workspace| _workspace_state_is_incomplete(workspace.state()))
            .count();
        Ok((snapshot.installation().clone(), incomplete))
    }

    fn require_ready_marker(&self, identity: &InstallationIdentity) -> Result<(), UseCaseError> {
        let marker = self
            .bootstrap
            .read_root_marker(identity.data_root())
            .map_err(|error| map_port(Stage::Layout, error))?;
        match marker {
            Some(marker)
                if marker.state() == RootMarkerState::Ready && marker.identity() == identity =>
            {
                Ok(())
            }
            _ => Err(semantic_error(
                ErrorCode::DataRootLayout,
                "data-root marker is missing, incomplete, or inconsistent",
            )),
        }
    }
}

impl<B, M> UseCases for ThinWorkspaceService<B, M>
where
    B: BootstrapStore + LifecycleLock<Guard = <B as BootstrapStore>::LockGuard>,
    M: MetadataStoreFactory<<B as BootstrapStore>::DataRootLayout>,
{
    fn init(&self, request: InitRequest) -> Result<InitOutcome, UseCaseError> {
        ThinWorkspaceService::init(self, request)
    }

    fn doctor(&self) -> Result<DoctorOutcome, UseCaseError> {
        ThinWorkspaceService::doctor(self)
    }
}

fn _workspace_state_is_incomplete(state: WorkspaceState) -> bool {
    state != WorkspaceState::Ready
}

#[derive(Clone, Copy)]
enum Stage {
    Bootstrap,
    Lock,
    PrepareDataRoot,
    Layout,
    Metadata,
    Publish,
}

fn map_port(stage: Stage, error: PortError) -> UseCaseError {
    let code = match error.kind() {
        PortErrorKind::Timeout => ErrorCode::LockTimeout,
        PortErrorKind::CapabilityUnavailable => ErrorCode::CapabilityUnavailable,
        PortErrorKind::NotEmpty => ErrorCode::DataRootNotEmpty,
        PortErrorKind::Unavailable => ErrorCode::DataRootUnavailable,
        PortErrorKind::InvalidLayout => ErrorCode::DataRootLayout,
        PortErrorKind::Io => ErrorCode::Filesystem,
        _ if matches!(stage, Stage::Metadata) => ErrorCode::Metadata,
        PortErrorKind::Conflict
        | PortErrorKind::InvalidData
        | PortErrorKind::UnsupportedVersion
        | PortErrorKind::NotFound
            if matches!(stage, Stage::Layout) =>
        {
            ErrorCode::DataRootLayout
        }
        PortErrorKind::Conflict | PortErrorKind::InvalidData if matches!(stage, Stage::Publish) => {
            ErrorCode::DataRootLayout
        }
        PortErrorKind::InvalidData | PortErrorKind::UnsupportedVersion
            if matches!(stage, Stage::Bootstrap) =>
        {
            ErrorCode::DataRootLayout
        }
        PortErrorKind::InvalidData | PortErrorKind::Conflict
            if matches!(stage, Stage::PrepareDataRoot) =>
        {
            ErrorCode::DataRootLayout
        }
        _ => ErrorCode::Filesystem,
    };
    let message = match code {
        ErrorCode::LockTimeout => "lifecycle lock wait timed out",
        ErrorCode::CapabilityUnavailable => "required platform capability is unavailable",
        ErrorCode::DataRootNotEmpty => "data root is not empty",
        ErrorCode::DataRootUnavailable => "registered data root is unavailable",
        ErrorCode::DataRootLayout => "data-root identity or layout is invalid",
        ErrorCode::Metadata => "metadata database validation failed",
        _ => "filesystem operation failed",
    };
    UseCaseError {
        diagnostic: CoreError::new(code, message),
        source: Some(error),
    }
}

fn semantic_error(code: ErrorCode, message: &'static str) -> UseCaseError {
    UseCaseError {
        diagnostic: CoreError::new(code, message),
        source: None,
    }
}
