use std::error::Error;
use std::fmt;
use std::str::FromStr;

use serde::Deserialize;
use thinws_core::{
    AbsolutePath, InstallationIdentity, InstanceId, RootMarker, RootMarkerState, VolumeId,
};

pub(crate) const MAX_DOCUMENT_BYTES: usize = 64 * 1024;
const DOCUMENT_SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DocumentError {
    TooLarge,
    InvalidEncoding,
    InvalidToml,
    UnsupportedVersion,
    InvalidPathHex,
    InvalidIdentity,
}

impl fmt::Display for DocumentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid bootstrap document ({self:?})")
    }
}

impl Error for DocumentError {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigDocument {
    schema_version: u32,
    instance_id: String,
    control_root_hex: String,
    control_volume_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MarkerDocument {
    schema_version: u32,
    instance_id: String,
    control_root_hex: String,
    control_volume_id: String,
    state: MarkerState,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum MarkerState {
    Initializing,
    Ready,
}

pub(crate) fn decode_config(bytes: &[u8]) -> Result<InstallationIdentity, DocumentError> {
    let document: ConfigDocument = decode_toml(bytes)?;
    require_schema_version(document.schema_version)?;
    decode_identity(
        &document.instance_id,
        &document.control_root_hex,
        &document.control_volume_id,
    )
}

pub(crate) fn decode_marker(bytes: &[u8]) -> Result<RootMarker, DocumentError> {
    let document: MarkerDocument = decode_toml(bytes)?;
    require_schema_version(document.schema_version)?;
    let identity = decode_identity(
        &document.instance_id,
        &document.control_root_hex,
        &document.control_volume_id,
    )?;
    let state = match document.state {
        MarkerState::Initializing => RootMarkerState::Initializing,
        MarkerState::Ready => RootMarkerState::Ready,
    };
    Ok(RootMarker::new(identity, state))
}

fn require_schema_version(version: u32) -> Result<(), DocumentError> {
    if version == DOCUMENT_SCHEMA_VERSION {
        Ok(())
    } else {
        Err(DocumentError::UnsupportedVersion)
    }
}

fn decode_toml<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, DocumentError> {
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(DocumentError::TooLarge);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| DocumentError::InvalidEncoding)?;
    toml::from_str(text).map_err(|_| DocumentError::InvalidToml)
}

fn decode_identity(
    instance_id: &str,
    control_root_hex: &str,
    control_volume_id: &str,
) -> Result<InstallationIdentity, DocumentError> {
    Ok(InstallationIdentity::new(
        InstanceId::from_str(instance_id).map_err(|_| DocumentError::InvalidIdentity)?,
        AbsolutePath::try_from_bytes(decode_hex(control_root_hex)?)
            .map_err(|_| DocumentError::InvalidIdentity)?,
        VolumeId::from_str(control_volume_id).map_err(|_| DocumentError::InvalidIdentity)?,
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

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use thinws_core::{AbsolutePath, InstanceId, RootMarkerState, VolumeId};

    use super::*;

    fn identity() -> InstallationIdentity {
        InstallationIdentity::new(
            InstanceId::from_str("01890a5d-ac96-774b-bd5b-55c7b8d09f33").unwrap(),
            AbsolutePath::try_from_bytes(b"/home/test/.thinws".to_vec()).unwrap(),
            VolumeId::from_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
        )
    }

    fn config_bytes() -> Vec<u8> {
        b"schema_version = 2\ninstance_id = \"01890a5d-ac96-774b-bd5b-55c7b8d09f33\"\ncontrol_root_hex = \"2f686f6d652f746573742f2e7468696e7773\"\ncontrol_volume_id = \"550e8400-e29b-41d4-a716-446655440000\"\n".to_vec()
    }

    #[test]
    fn config_and_marker_decode_the_frozen_v2_schema() {
        let identity = identity();
        let encoded = config_bytes();
        assert_eq!(decode_config(&encoded), Ok(identity.clone()));
        for (state, word) in [
            (RootMarkerState::Initializing, "initializing"),
            (RootMarkerState::Ready, "ready"),
        ] {
            let mut marker_bytes = encoded.clone();
            marker_bytes.extend_from_slice(format!("state = \"{word}\"\n").as_bytes());
            assert_eq!(
                decode_marker(&marker_bytes),
                Ok(RootMarker::new(identity.clone(), state))
            );
        }
    }

    #[test]
    fn malformed_or_foreign_control_documents_fail_closed() {
        let encoded = String::from_utf8(config_bytes()).unwrap();
        assert_eq!(
            decode_config(
                encoded
                    .replace("schema_version = 2", "schema_version = 1")
                    .as_bytes()
            ),
            Err(DocumentError::UnsupportedVersion)
        );
        assert_eq!(
            decode_config(format!("{encoded}unknown = 1\n").as_bytes()),
            Err(DocumentError::InvalidToml)
        );
        assert_eq!(
            decode_config(encoded.replace("2f", "2F").as_bytes()),
            Err(DocumentError::InvalidPathHex)
        );
        assert_eq!(
            decode_config(&vec![b'x'; MAX_DOCUMENT_BYTES + 1]),
            Err(DocumentError::TooLarge)
        );
    }
}
