use std::str::FromStr;

use thinws_core::VolumeId;
use thinws_ports::{PortError, PortErrorKind};

pub(crate) fn decode_volume_id(bytes: Option<[u8; 16]>) -> Result<VolumeId, PortError> {
    let bytes = bytes
        .filter(|bytes| bytes.iter().any(|byte| *byte != 0))
        .ok_or_else(|| {
            PortError::new(
                PortErrorKind::CapabilityUnavailable,
                "require APFS volume UUID",
            )
        })?;
    let encoded = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    );
    VolumeId::from_str(&encoded).map_err(|error| {
        PortError::new(PortErrorKind::InvalidData, "decode APFS volume UUID").with_source(error)
    })
}
