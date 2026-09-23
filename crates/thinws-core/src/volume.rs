use std::fmt;
use std::str::FromStr;

use thiserror::Error;
use uuid::Uuid;

/// Stable APFS volume identity in canonical lowercase UUID form.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct VolumeId(Uuid);

impl VolumeId {
    /// Returns the parsed UUID without erasing the outer domain type.
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl FromStr for VolumeId {
    type Err = VolumeIdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let parsed = Uuid::parse_str(value).map_err(|_| VolumeIdParseError)?;
        if parsed.hyphenated().to_string() != value {
            return Err(VolumeIdParseError);
        }
        Ok(Self(parsed))
    }
}

impl fmt::Display for VolumeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.hyphenated().fmt(formatter)
    }
}

/// A volume UUID was not canonical, lowercase and hyphenated.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("volume ID must be a canonical lowercase, hyphenated UUID")]
pub struct VolumeIdParseError;
