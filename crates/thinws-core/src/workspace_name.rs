use std::fmt;
use std::str::FromStr;

use thiserror::Error;

/// Maximum byte length of a Workspace name.
pub const MAX_WORKSPACE_NAME_LEN: usize = 63;

/// A validated, non-normalized user-visible Workspace name.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WorkspaceName(String);

impl WorkspaceName {
    /// Returns the exact validated input without case or separator normalization.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for WorkspaceName {
    type Err = WorkspaceNameError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value.to_owned())
    }
}

impl TryFrom<String> for WorkspaceName {
    type Error = WorkspaceNameError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        validate_workspace_name(&value)?;
        Ok(Self(value))
    }
}

impl TryFrom<&str> for WorkspaceName {
    type Error = WorkspaceNameError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl fmt::Display for WorkspaceName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A precise reason why a Workspace name violates the public grammar.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum WorkspaceNameError {
    /// No name was provided.
    #[error("workspace name must not be empty")]
    Empty,
    /// The name exceeded the public 63-byte limit.
    #[error("workspace name is {length} bytes; the maximum is 63")]
    TooLong {
        /// Actual byte length of the rejected input.
        length: usize,
    },
    /// The first or final byte was not an ASCII lowercase letter or digit.
    #[error("workspace name must start and end with a lowercase ASCII letter or digit")]
    InvalidBoundary,
    /// A byte outside the frozen ASCII grammar was present.
    #[error("workspace name contains an invalid byte at index {index}")]
    InvalidCharacter {
        /// Zero-based byte offset of the first invalid byte.
        index: usize,
    },
    /// Two consecutive dots would make the name ambiguous in surrounding tools.
    #[error("workspace name must not contain consecutive dots")]
    ConsecutiveDots,
}

fn validate_workspace_name(value: &str) -> Result<(), WorkspaceNameError> {
    let bytes = value.as_bytes();
    if bytes.is_empty() {
        return Err(WorkspaceNameError::Empty);
    }
    if bytes.len() > MAX_WORKSPACE_NAME_LEN {
        return Err(WorkspaceNameError::TooLong {
            length: bytes.len(),
        });
    }
    if !is_boundary(bytes[0]) || !is_boundary(bytes[bytes.len() - 1]) {
        return Err(WorkspaceNameError::InvalidBoundary);
    }
    if let Some(index) = bytes.iter().position(|byte| !is_allowed(*byte)) {
        return Err(WorkspaceNameError::InvalidCharacter { index });
    }
    if bytes.windows(2).any(|pair| pair == b"..") {
        return Err(WorkspaceNameError::ConsecutiveDots);
    }
    Ok(())
}

const fn is_boundary(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit()
}

const fn is_allowed(byte: u8) -> bool {
    is_boundary(byte) || matches!(byte, b'.' | b'_' | b'-')
}
