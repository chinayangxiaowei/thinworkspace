use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use thiserror::Error;

/// Stable machine-readable error codes, independent of CLI process exit mapping.
///
/// Exit status selection belongs to the CLI boundary and is deliberately absent
/// from Core:
///
/// ```compile_fail
/// use thinws_core::ErrorCode;
///
/// let _ = ErrorCode::Usage.exit_status();
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ErrorCode {
    /// Command or parameter usage error.
    Usage,
    /// ThinWorkspace has not been initialized.
    NotInitialized,
    /// A required platform capability is unavailable.
    CapabilityUnavailable,
    /// CoW is unavailable and explicit full copy was not allowed.
    CowUnavailable,
    /// An active Workspace already uses the requested name with other inputs.
    NameConflict,
    /// The requested Workspace does not exist.
    WorkspaceNotFound,
    /// The requested operation requires a Ready Workspace.
    WorkspaceNotReady,
    /// Tracked changes prevent normal cleanup.
    WorkspaceDirty,
    /// Confirmed external process use prevents cleanup.
    WorkspaceBusy,
    /// The read-only Git check could not establish a complete result.
    GitCheckIncomplete,
    /// A Git failure could not be mapped more specifically.
    Git,
    /// A filesystem or materialization operation failed.
    Filesystem,
    /// The fixed user control directory cannot currently be accessed.
    ControlUnavailable,
    /// The fixed user control directory or its contents violate the registered layout.
    ControlLayout,
    /// An unowned fixed control directory contains user content.
    ControlNotEmpty,
    /// The registered data root or volume is unavailable.
    DataRootUnavailable,
    /// Source, target, volume, or controlled path layout is invalid.
    DataRootLayout,
    /// SQLite, schema, or metadata persistence failed.
    Metadata,
    /// The registered Workspace target is absent at every proven location.
    TargetMissing,
    /// The registered target exists but does not match durable ownership evidence.
    TargetIdentity,
    /// Workspace creation or cleanup is incomplete.
    WorkspaceIncomplete,
    /// A lifecycle lock was not acquired within the public timeout.
    LockTimeout,
}

impl ErrorCode {
    /// Every currently assigned public error code in documented order.
    pub const ALL: [Self; 22] = [
        Self::Usage,
        Self::NotInitialized,
        Self::CapabilityUnavailable,
        Self::CowUnavailable,
        Self::NameConflict,
        Self::WorkspaceNotFound,
        Self::WorkspaceNotReady,
        Self::WorkspaceDirty,
        Self::WorkspaceBusy,
        Self::GitCheckIncomplete,
        Self::Git,
        Self::Filesystem,
        Self::ControlUnavailable,
        Self::ControlLayout,
        Self::ControlNotEmpty,
        Self::DataRootUnavailable,
        Self::DataRootLayout,
        Self::Metadata,
        Self::TargetMissing,
        Self::TargetIdentity,
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
            Self::WorkspaceNotFound => "E_WORKSPACE_NOT_FOUND",
            Self::WorkspaceNotReady => "E_WORKSPACE_NOT_READY",
            Self::WorkspaceDirty => "E_WORKSPACE_DIRTY",
            Self::WorkspaceBusy => "E_WORKSPACE_BUSY",
            Self::GitCheckIncomplete => "E_GIT_CHECK_INCOMPLETE",
            Self::Git => "E_GIT",
            Self::Filesystem => "E_FILESYSTEM",
            Self::ControlUnavailable => "E_CONTROL_UNAVAILABLE",
            Self::ControlLayout => "E_CONTROL_LAYOUT",
            Self::ControlNotEmpty => "E_CONTROL_NOT_EMPTY",
            Self::DataRootUnavailable => "E_DATA_ROOT_UNAVAILABLE",
            Self::DataRootLayout => "E_DATA_ROOT_LAYOUT",
            Self::Metadata => "E_METADATA",
            Self::TargetMissing => "E_TARGET_MISSING",
            Self::TargetIdentity => "E_TARGET_IDENTITY",
            Self::WorkspaceIncomplete => "E_WORKSPACE_INCOMPLETE",
            Self::LockTimeout => "E_LOCK_TIMEOUT",
        }
    }
}

impl FromStr for ErrorCode {
    type Err = ErrorCodeParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|code| code.as_str() == value)
            .ok_or(ErrorCodeParseError)
    }
}

/// A persisted diagnostic code was not one of the frozen symbolic values.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("unknown ThinWorkspace error code")]
pub struct ErrorCodeParseError;

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
