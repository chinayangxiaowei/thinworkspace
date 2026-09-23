use std::fmt;
use std::str::FromStr;

use thiserror::Error;
use uuid::{Uuid, Variant};

/// A failure to parse one of ThinWorkspace's typed UUIDv7 identifiers.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum IdParseError {
    /// The required type prefix was absent or belonged to another ID type.
    #[error("identifier must start with `{expected}`")]
    InvalidPrefix {
        /// The exact prefix required by the target ID type.
        expected: &'static str,
    },
    /// The payload was not a canonical lowercase, hyphenated UUID.
    #[error("identifier must contain a canonical lowercase, hyphenated UUID")]
    InvalidFormat,
    /// The UUID was syntactically valid but was not version 7.
    #[error("identifier UUID must be version 7")]
    WrongVersion,
    /// The UUID did not use the RFC 4122 variant required by UUIDv7.
    #[error("identifier UUID must use the RFC 4122 variant")]
    WrongVariant,
}

fn parse_uuid_v7(value: &str) -> Result<Uuid, IdParseError> {
    let parsed = Uuid::parse_str(value).map_err(|_| IdParseError::InvalidFormat)?;
    if parsed.hyphenated().to_string() != value {
        return Err(IdParseError::InvalidFormat);
    }
    validate_uuid_v7(parsed)?;
    Ok(parsed)
}

fn validate_uuid_v7(value: Uuid) -> Result<(), IdParseError> {
    if value.get_version_num() != 7 {
        return Err(IdParseError::WrongVersion);
    }
    if value.get_variant() != Variant::RFC4122 {
        return Err(IdParseError::WrongVariant);
    }
    Ok(())
}

macro_rules! typed_uuid {
    ($name:ident, $prefix:literal, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(Uuid);

        // Identity allocation is a meaningful action; `Default` would hide it at call sites.
        #[allow(clippy::new_without_default)]
        impl $name {
            /// Generates a new UUIDv7 identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            /// Returns the underlying UUID without erasing the outer Rust type.
            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl TryFrom<Uuid> for $name {
            type Error = IdParseError;

            fn try_from(value: Uuid) -> Result<Self, Self::Error> {
                validate_uuid_v7(value)?;
                Ok(Self(value))
            }
        }

        impl FromStr for $name {
            type Err = IdParseError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                let payload = value
                    .strip_prefix($prefix)
                    .ok_or(IdParseError::InvalidPrefix { expected: $prefix })?;
                parse_uuid_v7(payload).map(Self)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}{}", $prefix, self.0.hyphenated())
            }
        }
    };
}

/// Identity of one initialized ThinWorkspace installation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct InstanceId(Uuid);

// Identity allocation is a meaningful action; `Default` would hide it at call sites.
#[allow(clippy::new_without_default)]
impl InstanceId {
    /// Generates a new UUIDv7 instance identity.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// Returns the underlying UUID without erasing the outer Rust type.
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl TryFrom<Uuid> for InstanceId {
    type Error = IdParseError;

    fn try_from(value: Uuid) -> Result<Self, Self::Error> {
        validate_uuid_v7(value)?;
        Ok(Self(value))
    }
}

impl FromStr for InstanceId {
    type Err = IdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        parse_uuid_v7(value).map(Self)
    }
}

impl fmt::Display for InstanceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.hyphenated().fmt(formatter)
    }
}

typed_uuid!(
    WorkspaceId,
    "ws_",
    "Stable identity of one managed Workspace. Its persisted form is `ws_` followed by a canonical UUIDv7."
);
typed_uuid!(
    OperationId,
    "op_",
    "Identifier used to correlate one logging attempt. It does not imply that an operation can be replayed."
);
