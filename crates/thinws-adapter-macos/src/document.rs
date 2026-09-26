use std::error::Error;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thinws_core::{
    AbsolutePath, InstallationIdentity, InstanceId, RootMarker, RootMarkerState, VolumeId,
    WorkspaceId,
};

use crate::filesystem::HistoricalDirectoryIdentity;

/// Maximum encoded size of either bootstrap TOML document.
pub const MAX_DOCUMENT_BYTES: usize = 64 * 1024;
const DOCUMENT_SCHEMA_VERSION: u32 = 1;

/// Pure bootstrap TOML decoding failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentError {
    /// The input exceeds the 64 KiB contract.
    TooLarge,
    /// The byte stream is not UTF-8.
    InvalidEncoding,
    /// TOML syntax, fields, or enum values are invalid.
    InvalidToml,
    /// The schema version is not supported by this binary.
    UnsupportedVersion,
    /// The data-root hex is odd, uppercase, or contains a non-hex byte.
    InvalidPathHex,
    /// One or more identity fields violate their Core value-object contract.
    InvalidIdentity,
}

impl fmt::Display for DocumentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid bootstrap document ({self:?})")
    }
}

impl Error for DocumentError {}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ConfigDocument {
    schema_version: u32,
    instance_id: String,
    data_root_hex: String,
    volume_id: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct MarkerDocument {
    schema_version: u32,
    instance_id: String,
    data_root_hex: String,
    volume_id: String,
    state: MarkerState,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OwnershipDocument {
    schema_version: u32,
    instance_id: String,
    workspace_id: String,
    volume_id: String,
    container_inode: u64,
    container_birth_seconds: i64,
    container_birth_nanoseconds: u32,
    root_inode: u64,
    root_birth_seconds: i64,
    root_birth_nanoseconds: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WorkspaceOwnership {
    pub(crate) instance_id: InstanceId,
    pub(crate) workspace_id: WorkspaceId,
    pub(crate) volume_id: VolumeId,
    pub(crate) container: HistoricalDirectoryIdentity,
    pub(crate) root: HistoricalDirectoryIdentity,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum MarkerState {
    Initializing,
    Ready,
}

/// Decodes a versioned bootstrap config without touching the filesystem.
pub fn decode_bootstrap_config(bytes: &[u8]) -> Result<InstallationIdentity, DocumentError> {
    let document: ConfigDocument = decode_toml(bytes)?;
    if document.schema_version != DOCUMENT_SCHEMA_VERSION {
        return Err(DocumentError::UnsupportedVersion);
    }
    decode_identity(
        &document.instance_id,
        &document.data_root_hex,
        &document.volume_id,
    )
}

/// Decodes a versioned data-root marker without touching the filesystem.
pub fn decode_root_marker(bytes: &[u8]) -> Result<RootMarker, DocumentError> {
    let document: MarkerDocument = decode_toml(bytes)?;
    if document.schema_version != DOCUMENT_SCHEMA_VERSION {
        return Err(DocumentError::UnsupportedVersion);
    }
    let identity = decode_identity(
        &document.instance_id,
        &document.data_root_hex,
        &document.volume_id,
    )?;
    let state = match document.state {
        MarkerState::Initializing => RootMarkerState::Initializing,
        MarkerState::Ready => RootMarkerState::Ready,
    };
    Ok(RootMarker::new(identity, state))
}

pub(crate) fn encode_config(identity: &InstallationIdentity) -> Result<Vec<u8>, DocumentError> {
    encode_toml(&ConfigDocument {
        schema_version: DOCUMENT_SCHEMA_VERSION,
        instance_id: identity.instance_id().to_string(),
        data_root_hex: encode_hex(identity.data_root().as_bytes()),
        volume_id: identity.volume_id().to_string(),
    })
}

pub(crate) fn encode_marker(marker: &RootMarker) -> Result<Vec<u8>, DocumentError> {
    let state = match marker.state() {
        RootMarkerState::Initializing => MarkerState::Initializing,
        RootMarkerState::Ready => MarkerState::Ready,
    };
    encode_toml(&MarkerDocument {
        schema_version: DOCUMENT_SCHEMA_VERSION,
        instance_id: marker.identity().instance_id().to_string(),
        data_root_hex: encode_hex(marker.identity().data_root().as_bytes()),
        volume_id: marker.identity().volume_id().to_string(),
        state,
    })
}

pub(crate) fn encode_workspace_ownership(
    ownership: WorkspaceOwnership,
) -> Result<Vec<u8>, DocumentError> {
    encode_toml(&OwnershipDocument {
        schema_version: DOCUMENT_SCHEMA_VERSION,
        instance_id: ownership.instance_id.to_string(),
        workspace_id: ownership.workspace_id.to_string(),
        volume_id: ownership.volume_id.to_string(),
        container_inode: ownership.container.inode,
        container_birth_seconds: ownership.container.birth_seconds,
        container_birth_nanoseconds: ownership.container.birth_nanoseconds,
        root_inode: ownership.root.inode,
        root_birth_seconds: ownership.root.birth_seconds,
        root_birth_nanoseconds: ownership.root.birth_nanoseconds,
    })
}

pub(crate) fn decode_workspace_ownership(
    bytes: &[u8],
) -> Result<WorkspaceOwnership, DocumentError> {
    let document: OwnershipDocument = decode_toml(bytes)?;
    if document.schema_version != DOCUMENT_SCHEMA_VERSION {
        return Err(DocumentError::UnsupportedVersion);
    }
    if document.container_birth_nanoseconds >= 1_000_000_000
        || document.root_birth_nanoseconds >= 1_000_000_000
        || document.container_inode == 0
        || document.root_inode == 0
        || document.container_birth_seconds < 0
        || document.root_birth_seconds < 0
    {
        return Err(DocumentError::InvalidIdentity);
    }
    Ok(WorkspaceOwnership {
        instance_id: InstanceId::from_str(&document.instance_id)
            .map_err(|_| DocumentError::InvalidIdentity)?,
        workspace_id: WorkspaceId::from_str(&document.workspace_id)
            .map_err(|_| DocumentError::InvalidIdentity)?,
        volume_id: VolumeId::from_str(&document.volume_id)
            .map_err(|_| DocumentError::InvalidIdentity)?,
        container: HistoricalDirectoryIdentity {
            inode: document.container_inode,
            birth_seconds: document.container_birth_seconds,
            birth_nanoseconds: document.container_birth_nanoseconds,
        },
        root: HistoricalDirectoryIdentity {
            inode: document.root_inode,
            birth_seconds: document.root_birth_seconds,
            birth_nanoseconds: document.root_birth_nanoseconds,
        },
    })
}

fn decode_toml<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, DocumentError> {
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(DocumentError::TooLarge);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| DocumentError::InvalidEncoding)?;
    toml::from_str(text).map_err(|_| DocumentError::InvalidToml)
}

fn encode_toml<T: Serialize>(document: &T) -> Result<Vec<u8>, DocumentError> {
    let bytes = toml::to_string(document)
        .map_err(|_| DocumentError::InvalidToml)?
        .into_bytes();
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(DocumentError::TooLarge);
    }
    Ok(bytes)
}

fn decode_identity(
    instance_id: &str,
    data_root_hex: &str,
    volume_id: &str,
) -> Result<InstallationIdentity, DocumentError> {
    Ok(InstallationIdentity::new(
        InstanceId::from_str(instance_id).map_err(|_| DocumentError::InvalidIdentity)?,
        AbsolutePath::try_from_bytes(decode_hex(data_root_hex)?)
            .map_err(|_| DocumentError::InvalidIdentity)?,
        VolumeId::from_str(volume_id).map_err(|_| DocumentError::InvalidIdentity)?,
    ))
}

fn decode_hex(value: &str) -> Result<Vec<u8>, DocumentError> {
    let bytes = value.as_bytes();
    if !bytes.len().is_multiple_of(2)
        || bytes
            .iter()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(byte))
    {
        return Err(DocumentError::InvalidPathHex);
    }
    bytes
        .chunks_exact(2)
        .map(|pair| Ok((hex_nibble(pair[0]) << 4) | hex_nibble(pair[1])))
        .collect()
}

fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => unreachable!("decode_hex validates every nibble"),
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    const INSTANCE_ID: &str = "01890a5d-ac96-774b-bd5b-55c7b8d09f33";
    const VOLUME_ID: &str = "550e8400-e29b-41d4-a716-446655440000";

    fn identity_with_path_length(length: usize) -> InstallationIdentity {
        assert!(length >= 1);
        let mut path = vec![b'a'; length];
        path[0] = b'/';
        InstallationIdentity::new(
            InstanceId::from_str(INSTANCE_ID).unwrap(),
            AbsolutePath::try_from_bytes(path).unwrap(),
            VolumeId::from_str(VOLUME_ID).unwrap(),
        )
    }

    #[test]
    fn config_encoder_accepts_the_exact_limit_and_rejects_the_next_path_size() {
        let base = encode_config(&identity_with_path_length(1)).unwrap().len() - 2;
        assert_eq!((MAX_DOCUMENT_BYTES - base) % 2, 0);
        let exact_path_length = (MAX_DOCUMENT_BYTES - base) / 2;

        assert_eq!(
            encode_config(&identity_with_path_length(exact_path_length))
                .unwrap()
                .len(),
            MAX_DOCUMENT_BYTES
        );
        assert_eq!(
            encode_config(&identity_with_path_length(exact_path_length + 1)).unwrap_err(),
            DocumentError::TooLarge
        );
    }
}
