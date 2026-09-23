use thiserror::Error;

/// A non-negative UTC Unix timestamp in milliseconds.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct UnixMillis(i64);

impl UnixMillis {
    /// Validates a persisted signed SQLite integer as a timestamp.
    pub fn new(value: i64) -> Result<Self, UnixMillisError> {
        if value < 0 {
            return Err(UnixMillisError { value });
        }
        Ok(Self(value))
    }

    /// Returns the value suitable for SQLite integer persistence.
    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }
}

/// A timestamp was earlier than the Unix epoch.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("Unix millisecond timestamp must be non-negative, got {value}")]
pub struct UnixMillisError {
    value: i64,
}
