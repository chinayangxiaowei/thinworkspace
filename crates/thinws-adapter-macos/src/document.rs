use std::error::Error;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thinws_core::{
    AbsolutePath, InstallationIdentity, InstanceId, RootMarker, RootMarkerState, VolumeId,
};

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
