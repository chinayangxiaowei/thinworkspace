use std::collections::BTreeMap;
use std::fmt;

/// Stable machine-readable error codes and their process exit statuses.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[repr(u8)]
pub enum ErrorCode {
    /// Command or parameter usage error.
    Usage = 2,
    /// ThinWorkspace has not been initialized.
    NotInitialized = 10,
    /// A required platform capability is unavailable.
    CapabilityUnavailable = 11,
    /// CoW is unavailable and explicit full copy was not allowed.
    CowUnavailable = 12,
    /// An active Workspace already uses the requested name with other inputs.
    NameConflict = 15,
    /// Changing an initialized data root is unsupported.
    DataRootChangeUnsupported = 16,
    /// The requested Workspace does not exist.
    WorkspaceNotFound = 20,
    /// The requested operation requires a Ready Workspace.
    WorkspaceNotReady = 21,
    /// Tracked changes prevent normal cleanup.
    WorkspaceDirty = 22,
    /// Confirmed external process use prevents cleanup.
    WorkspaceBusy = 23,
    /// The read-only Git check could not establish a complete result.
    GitCheckIncomplete = 25,
    /// A Git failure could not be mapped more specifically.
    Git = 30,
    /// A filesystem or materialization operation failed.
    Filesystem = 31,
    /// The registered data root or volume is unavailable.
    DataRootUnavailable = 32,
    /// Source, target, volume, or controlled path layout is invalid.
    DataRootLayout = 33,
    /// SQLite, schema, or metadata persistence failed.
    Metadata = 35,
    /// A non-empty data root has no valid ThinWorkspace ownership marker.
    DataRootNotEmpty = 36,
    /// Workspace creation or cleanup is incomplete.
    WorkspaceIncomplete = 40,
    /// A lifecycle lock was not acquired within the public timeout.
    LockTimeout = 41,
}

impl ErrorCode {
    /// Every currently assigned public error code in stable numeric order.
    pub const ALL: [Self; 19] = [
        Self::Usage,
        Self::NotInitialized,
        Self::CapabilityUnavailable,
        Self::CowUnavailable,
        Self::NameConflict,
        Self::DataRootChangeUnsupported,
        Self::WorkspaceNotFound,
        Self::WorkspaceNotReady,
        Self::WorkspaceDirty,
        Self::WorkspaceBusy,
        Self::GitCheckIncomplete,
        Self::Git,
        Self::Filesystem,
        Self::DataRootUnavailable,
        Self::DataRootLayout,
        Self::Metadata,
        Self::DataRootNotEmpty,
        Self::WorkspaceIncomplete,
        Self::LockTimeout,
    ];

    /// Returns the stable symbolic code used by machine-readable output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Usage => "E_USAGE",
            Self::NotInitialized => "E_NOT_INITIALIZED",
            Self::CapabilityUnavailable => "E_CAPABILITY_UNAVAILABLE",
            Self::CowUnavailable => "E_COW_UNAVAILABLE",
            Self::NameConflict => "E_NAME_CONFLICT",
            Self::DataRootChangeUnsupported => "E_DATA_ROOT_CHANGE_UNSUPPORTED",
            Self::WorkspaceNotFound => "E_WORKSPACE_NOT_FOUND",
            Self::WorkspaceNotReady => "E_WORKSPACE_NOT_READY",
            Self::WorkspaceDirty => "E_WORKSPACE_DIRTY",
            Self::WorkspaceBusy => "E_WORKSPACE_BUSY",
            Self::GitCheckIncomplete => "E_GIT_CHECK_INCOMPLETE",
            Self::Git => "E_GIT",
            Self::Filesystem => "E_FILESYSTEM",
            Self::DataRootUnavailable => "E_DATA_ROOT_UNAVAILABLE",
            Self::DataRootLayout => "E_DATA_ROOT_LAYOUT",
            Self::Metadata => "E_METADATA",
            Self::DataRootNotEmpty => "E_DATA_ROOT_NOT_EMPTY",
            Self::WorkspaceIncomplete => "E_WORKSPACE_INCOMPLETE",
            Self::LockTimeout => "E_LOCK_TIMEOUT",
        }
    }

    /// Returns the frozen process exit status for this error code.
    #[must_use]
    pub const fn exit_status(self) -> u8 {
        self as u8
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One diagnostic context value with an explicit logging sensitivity.
#[derive(Clone, Eq, PartialEq)]
pub struct ContextValue {
    value: String,
    sensitive: bool,
}

impl ContextValue {
    /// Marks a value as safe for ordinary diagnostic logging.
    pub fn public(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            sensitive: false,
        }
    }

    /// Marks a value as user-visible but redacted from ordinary diagnostic logging.
    pub fn sensitive(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            sensitive: true,
        }
    }

    /// Returns the original value for an explicit user-facing renderer.
    #[must_use]
    pub fn user_value(&self) -> &str {
        &self.value
    }

    /// Returns a value safe for ordinary diagnostic logging.
    #[must_use]
    pub fn log_value(&self) -> &str {
        if self.sensitive {
            "[redacted]"
        } else {
            &self.value
        }
    }
}

impl fmt::Debug for ContextValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContextValue")
            .field("value", &self.log_value())
            .field("sensitive", &self.sensitive)
            .finish()
    }
}

/// A structured Core diagnostic independent of CLI rendering and serialization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoreError {
    code: ErrorCode,
    message: &'static str,
    context: BTreeMap<&'static str, ContextValue>,
    remediation: Option<&'static str>,
}

impl CoreError {
    /// Creates an error with a stable code and implementation-owned safe message.
    #[must_use]
    pub fn new(code: ErrorCode, message: &'static str) -> Self {
        Self {
            code,
            message,
            context: BTreeMap::new(),
            remediation: None,
        }
    }

    /// Adds or replaces a typed context field.
    #[must_use]
    pub fn with_context(mut self, key: &'static str, value: ContextValue) -> Self {
        self.context.insert(key, value);
        self
    }

    /// Adds implementation-owned remediation text.
    #[must_use]
    pub fn with_remediation(mut self, remediation: &'static str) -> Self {
        self.remediation = Some(remediation);
        self
    }

    /// Returns the stable error code.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        self.code
    }

    /// Returns the safe human-readable message.
    #[must_use]
    pub const fn message(&self) -> &'static str {
        self.message
    }

    /// Returns structured context for an explicit renderer or logger.
    #[must_use]
    pub const fn context(&self) -> &BTreeMap<&'static str, ContextValue> {
        &self.context
    }

    /// Returns optional implementation-owned remediation text.
    #[must_use]
    pub const fn remediation(&self) -> Option<&'static str> {
        self.remediation
    }
}

impl fmt::Display for CoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CoreError {}
