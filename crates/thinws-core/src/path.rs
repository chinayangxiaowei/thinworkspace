use thiserror::Error;

/// A canonical absolute filesystem path represented by lossless platform bytes.
///
/// Phase 1 persists macOS path bytes rather than a display string. Conversion
/// to or from an operating-system path belongs to the platform Adapter.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AbsolutePath(Vec<u8>);

impl AbsolutePath {
    /// Parses a canonical absolute path without normalizing the caller's bytes.
    pub fn try_from_bytes(bytes: impl Into<Vec<u8>>) -> Result<Self, AbsolutePathError> {
        let bytes = bytes.into();
        validate(&bytes)?;
        Ok(Self(bytes))
    }

    /// Returns the exact validated path bytes used for identity comparisons.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// A raw path did not satisfy the persisted absolute-path contract.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum AbsolutePathError {
    /// The path was empty or did not start at the filesystem root.
    #[error("path must be absolute")]
    NotAbsolute,
    /// The path contained a NUL byte, which cannot name a filesystem entry.
    #[error("path must not contain NUL")]
    ContainsNul,
    /// The path contained a repeated separator, trailing separator, `.` or `..`.
    #[error("path must contain only canonical normal components")]
    NonCanonicalComponent,
}

fn validate(bytes: &[u8]) -> Result<(), AbsolutePathError> {
    if bytes.first() != Some(&b'/') {
        return Err(AbsolutePathError::NotAbsolute);
    }
    if bytes.contains(&0) {
        return Err(AbsolutePathError::ContainsNul);
    }
    if bytes.len() == 1 {
        return Ok(());
    }
    if bytes.last() == Some(&b'/') {
        return Err(AbsolutePathError::NonCanonicalComponent);
    }
    for component in bytes[1..].split(|byte| *byte == b'/') {
        if component.is_empty() || component == b"." || component == b".." {
            return Err(AbsolutePathError::NonCanonicalComponent);
        }
    }
    Ok(())
}
