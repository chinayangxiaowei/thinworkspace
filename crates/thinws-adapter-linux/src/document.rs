use std::error::Error;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thinws_core::{
    AbsolutePath, InstallationIdentity, InstanceId, RootMarker, RootMarkerState, VolumeId,
    WorkspaceId,
};

pub(crate) const MAX_DOCUMENT_BYTES: usize = 64 * 1024;
const DOCUMENT_SCHEMA_VERSION: u32 = 2;
// Linux ownership adds the creation mount ID to the v2 APFS identity fields.
const LINUX_OWNERSHIP_SCHEMA_VERSION: u32 = 3;

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
        write!(formatter, "invalid control document ({self:?})")
    }
}

impl Error for DocumentError {}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ConfigDocument {
    schema_version: u32,
    instance_id: String,
    control_root_hex: String,
    control_volume_id: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct MarkerDocument {
    schema_version: u32,
    instance_id: String,
    control_root_hex: String,
    control_volume_id: String,
    state: MarkerState,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OwnershipDocument {
    schema_version: u32,
    instance_id: String,
    workspace_id: String,
    volume_id: String,
    mount_id: u64,
    target_path_hex: String,
    target_parent_inode: u64,
    target_parent_birth_seconds: i64,
    target_parent_birth_nanoseconds: u32,
    target_inode: u64,
    target_birth_seconds: i64,
    target_birth_nanoseconds: u32,
    staging_path_hex: String,
    staging_inode: u64,
    staging_birth_seconds: i64,
    staging_birth_nanoseconds: u32,
    trash_path_hex: String,
    trash_inode: u64,
    trash_birth_seconds: i64,
    trash_birth_nanoseconds: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    isolated_path_hex: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HistoricalDirectoryIdentity {
    pub(crate) inode: u64,
    pub(crate) birth_seconds: i64,
    pub(crate) birth_nanoseconds: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OperationDirectoryOwnership {
    pub(crate) path: AbsolutePath,
    pub(crate) identity: HistoricalDirectoryIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkspaceOwnership {
    pub(crate) instance_id: InstanceId,
    pub(crate) workspace_id: WorkspaceId,
    pub(crate) volume_id: VolumeId,
    pub(crate) mount_id: u64,
    pub(crate) target_path: AbsolutePath,
    pub(crate) parent: HistoricalDirectoryIdentity,
    pub(crate) target: HistoricalDirectoryIdentity,
    pub(crate) staging: OperationDirectoryOwnership,
    pub(crate) trash: OperationDirectoryOwnership,
    pub(crate) isolated_path: Option<AbsolutePath>,
}

#[derive(Deserialize, Serialize)]
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

pub(crate) fn encode_config(identity: &InstallationIdentity) -> Result<Vec<u8>, DocumentError> {
    encode_toml(&ConfigDocument {
        schema_version: DOCUMENT_SCHEMA_VERSION,
        instance_id: identity.instance_id().to_string(),
        control_root_hex: encode_hex(identity.data_root().as_bytes()),
        control_volume_id: identity.volume_id().to_string(),
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
        control_root_hex: encode_hex(marker.identity().data_root().as_bytes()),
        control_volume_id: marker.identity().volume_id().to_string(),
        state,
    })
}

pub(crate) fn encode_workspace_ownership(
    ownership: &WorkspaceOwnership,
) -> Result<Vec<u8>, DocumentError> {
    encode_toml(&OwnershipDocument {
        schema_version: LINUX_OWNERSHIP_SCHEMA_VERSION,
        instance_id: ownership.instance_id.to_string(),
        workspace_id: ownership.workspace_id.to_string(),
        volume_id: ownership.volume_id.to_string(),
        mount_id: ownership.mount_id,
        target_path_hex: encode_hex(ownership.target_path.as_bytes()),
        target_parent_inode: ownership.parent.inode,
        target_parent_birth_seconds: ownership.parent.birth_seconds,
        target_parent_birth_nanoseconds: ownership.parent.birth_nanoseconds,
        target_inode: ownership.target.inode,
        target_birth_seconds: ownership.target.birth_seconds,
        target_birth_nanoseconds: ownership.target.birth_nanoseconds,
        staging_path_hex: encode_hex(ownership.staging.path.as_bytes()),
        staging_inode: ownership.staging.identity.inode,
        staging_birth_seconds: ownership.staging.identity.birth_seconds,
        staging_birth_nanoseconds: ownership.staging.identity.birth_nanoseconds,
        trash_path_hex: encode_hex(ownership.trash.path.as_bytes()),
        trash_inode: ownership.trash.identity.inode,
        trash_birth_seconds: ownership.trash.identity.birth_seconds,
        trash_birth_nanoseconds: ownership.trash.identity.birth_nanoseconds,
        isolated_path_hex: ownership
            .isolated_path
            .as_ref()
            .map(|path| encode_hex(path.as_bytes())),
    })
}

pub(crate) fn decode_workspace_ownership(
    bytes: &[u8],
) -> Result<WorkspaceOwnership, DocumentError> {
    let document: OwnershipDocument = decode_toml(bytes)?;
    if document.schema_version != LINUX_OWNERSHIP_SCHEMA_VERSION {
        return Err(DocumentError::UnsupportedVersion);
    }
    let identities = [
        (
            document.target_parent_inode,
            document.target_parent_birth_seconds,
            document.target_parent_birth_nanoseconds,
        ),
        (
            document.target_inode,
            document.target_birth_seconds,
            document.target_birth_nanoseconds,
        ),
        (
            document.staging_inode,
            document.staging_birth_seconds,
            document.staging_birth_nanoseconds,
        ),
        (
            document.trash_inode,
            document.trash_birth_seconds,
            document.trash_birth_nanoseconds,
        ),
    ];
    if document.mount_id == 0
        || identities
            .iter()
            .any(|(inode, seconds, nanos)| *inode == 0 || *seconds < 0 || *nanos >= 1_000_000_000)
    {
        return Err(DocumentError::InvalidIdentity);
    }
    let workspace_id = WorkspaceId::from_str(&document.workspace_id)
        .map_err(|_| DocumentError::InvalidIdentity)?;
    let target_path = decode_path(&document.target_path_hex)?;
    let target_parent = parent_path_bytes(&target_path).ok_or(DocumentError::InvalidIdentity)?;
    let staging = OperationDirectoryOwnership {
        path: decode_path(&document.staging_path_hex)?,
        identity: historical(identities[2]),
    };
    let trash = OperationDirectoryOwnership {
        path: decode_path(&document.trash_path_hex)?,
        identity: historical(identities[3]),
    };
    if staging.path.as_bytes()
        != sibling_path_bytes(target_parent, &format!(".thinws-staging-{workspace_id}"))
        || trash.path.as_bytes()
            != sibling_path_bytes(target_parent, &format!(".thinws-trash-{workspace_id}"))
    {
        return Err(DocumentError::InvalidIdentity);
    }
    let isolated_path = document
        .isolated_path_hex
        .as_deref()
        .map(decode_path)
        .transpose()?;
    if isolated_path.as_ref().is_some_and(|path| {
        path.as_bytes()
            != sibling_path_bytes(target_parent, &format!(".thinws-remove-{workspace_id}"))
    }) {
        return Err(DocumentError::InvalidIdentity);
    }
    Ok(WorkspaceOwnership {
        instance_id: InstanceId::from_str(&document.instance_id)
            .map_err(|_| DocumentError::InvalidIdentity)?,
        workspace_id,
        volume_id: VolumeId::from_str(&document.volume_id)
            .map_err(|_| DocumentError::InvalidIdentity)?,
        mount_id: document.mount_id,
        target_path,
        parent: historical(identities[0]),
        target: historical(identities[1]),
        staging,
        trash,
        isolated_path,
    })
}

fn historical(fields: (u64, i64, u32)) -> HistoricalDirectoryIdentity {
    HistoricalDirectoryIdentity {
        inode: fields.0,
        birth_seconds: fields.1,
        birth_nanoseconds: fields.2,
    }
}

fn decode_path(hex: &str) -> Result<AbsolutePath, DocumentError> {
    AbsolutePath::try_from_bytes(decode_hex(hex)?).map_err(|_| DocumentError::InvalidIdentity)
}

fn parent_path_bytes(path: &AbsolutePath) -> Option<&[u8]> {
    let bytes = path.as_bytes();
    if bytes == b"/" {
        return None;
    }
    let slash = bytes.iter().rposition(|byte| *byte == b'/')?;
    Some(if slash == 0 { b"/" } else { &bytes[..slash] })
}

fn sibling_path_bytes(parent: &[u8], leaf: &str) -> Vec<u8> {
    let mut path = parent.to_vec();
    if path != b"/" {
        path.push(b'/');
    }
    path.extend_from_slice(leaf.as_bytes());
    path
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
        assert_eq!(
            decode_config(&encode_config(&identity).unwrap()),
            Ok(identity.clone())
        );
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
            let marker = RootMarker::new(identity.clone(), state);
            assert_eq!(decode_marker(&encode_marker(&marker).unwrap()), Ok(marker));
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

    #[test]
    fn linux_ownership_v3_roundtrips_and_rejects_foreign_mount_or_paths() {
        let workspace_id =
            WorkspaceId::from_str("ws_01890a5d-ac96-774b-bd5b-55c7b8d09f34").unwrap();
        let operation = |leaf: String, inode| OperationDirectoryOwnership {
            path: AbsolutePath::try_from_bytes(format!("/mnt/btrfs/{leaf}").into_bytes()).unwrap(),
            identity: HistoricalDirectoryIdentity {
                inode,
                birth_seconds: 1_700_000_000,
                birth_nanoseconds: 12,
            },
        };
        let ownership = WorkspaceOwnership {
            instance_id: identity().instance_id(),
            workspace_id,
            volume_id: identity().volume_id(),
            mount_id: 42,
            target_path: AbsolutePath::try_from_bytes(b"/mnt/btrfs/copy".to_vec()).unwrap(),
            parent: HistoricalDirectoryIdentity {
                inode: 100,
                birth_seconds: 1_700_000_000,
                birth_nanoseconds: 10,
            },
            target: HistoricalDirectoryIdentity {
                inode: 101,
                birth_seconds: 1_700_000_000,
                birth_nanoseconds: 11,
            },
            staging: operation(format!(".thinws-staging-{workspace_id}"), 102),
            trash: operation(format!(".thinws-trash-{workspace_id}"), 103),
            isolated_path: None,
        };
        let encoded = String::from_utf8(encode_workspace_ownership(&ownership).unwrap()).unwrap();
        assert_eq!(
            decode_workspace_ownership(encoded.as_bytes()),
            Ok(ownership.clone())
        );
        assert_eq!(
            decode_workspace_ownership(
                encoded
                    .replace("schema_version = 3", "schema_version = 2")
                    .as_bytes()
            ),
            Err(DocumentError::UnsupportedVersion)
        );
        assert_eq!(
            decode_workspace_ownership(encoded.replace("mount_id = 42", "mount_id = 0").as_bytes()),
            Err(DocumentError::InvalidIdentity)
        );
        assert_eq!(
            decode_workspace_ownership(format!("{encoded}unrecognized = 1\n").as_bytes()),
            Err(DocumentError::InvalidToml)
        );
        let mut forged = ownership.clone();
        forged.staging.path = AbsolutePath::try_from_bytes(b"/mnt/btrfs/foreign".to_vec()).unwrap();
        assert_eq!(
            decode_workspace_ownership(&encode_workspace_ownership(&forged).unwrap()),
            Err(DocumentError::InvalidIdentity)
        );
        let mut forged = ownership.clone();
        forged.trash.path = AbsolutePath::try_from_bytes(b"/mnt/btrfs/foreign".to_vec()).unwrap();
        assert_eq!(
            decode_workspace_ownership(&encode_workspace_ownership(&forged).unwrap()),
            Err(DocumentError::InvalidIdentity)
        );
        let mut forged = ownership;
        forged.isolated_path =
            Some(AbsolutePath::try_from_bytes(b"/mnt/btrfs/foreign".to_vec()).unwrap());
        assert_eq!(
            decode_workspace_ownership(&encode_workspace_ownership(&forged).unwrap()),
            Err(DocumentError::InvalidIdentity)
        );
    }

    #[test]
    fn fuzz_corpus_contains_a_valid_linux_ownership_document() {
        let seed = include_bytes!(
            "../../../fuzz/corpus/thinws_platform_ownership_document/valid-linux-ownership"
        );
        assert!(decode_workspace_ownership(seed).is_ok());
    }

    #[test]
    fn ownership_rejects_each_invalid_historical_identity_field() {
        let seed = include_str!(
            "../../../fuzz/corpus/thinws_platform_ownership_document/valid-linux-ownership"
        );
        for (field, original) in [
            ("target_parent_inode", "100"),
            ("target_inode", "101"),
            ("staging_inode", "102"),
            ("trash_inode", "103"),
        ] {
            let invalid = seed.replace(&format!("{field} = {original}"), &format!("{field} = 0"));
            assert_eq!(
                decode_workspace_ownership(invalid.as_bytes()),
                Err(DocumentError::InvalidIdentity),
                "{field}"
            );
        }
        for field in [
            "target_parent_birth_seconds",
            "target_birth_seconds",
            "staging_birth_seconds",
            "trash_birth_seconds",
        ] {
            let invalid = seed.replace(&format!("{field} = 1700000000"), &format!("{field} = -1"));
            assert_eq!(
                decode_workspace_ownership(invalid.as_bytes()),
                Err(DocumentError::InvalidIdentity),
                "{field}"
            );
        }
        for (field, original) in [
            ("target_parent_birth_nanoseconds", "10"),
            ("target_birth_nanoseconds", "11"),
            ("staging_birth_nanoseconds", "12"),
            ("trash_birth_nanoseconds", "12"),
        ] {
            let invalid = seed.replace(
                &format!("{field} = {original}"),
                &format!("{field} = 1000000000"),
            );
            assert_eq!(
                decode_workspace_ownership(invalid.as_bytes()),
                Err(DocumentError::InvalidIdentity),
                "{field}"
            );
        }
        let zero_seconds = seed.replace(
            "target_birth_seconds = 1700000000",
            "target_birth_seconds = 0",
        );
        assert!(decode_workspace_ownership(zero_seconds.as_bytes()).is_ok());
        let max_nanoseconds = seed.replace(
            "target_birth_nanoseconds = 11",
            "target_birth_nanoseconds = 999999999",
        );
        assert!(decode_workspace_ownership(max_nanoseconds.as_bytes()).is_ok());
    }

    #[test]
    fn document_size_limit_accepts_exactly_the_boundary_and_rejects_one_more_byte() {
        #[derive(serde::Serialize)]
        struct Padding {
            value: String,
        }

        let base = encode_toml(&Padding {
            value: String::new(),
        })
        .unwrap();
        let exact_padding = MAX_DOCUMENT_BYTES - base.len();
        let exact = encode_toml(&Padding {
            value: "a".repeat(exact_padding),
        })
        .unwrap();
        assert_eq!(exact.len(), MAX_DOCUMENT_BYTES);
        assert!(decode_toml::<toml::Value>(&exact).is_ok());
        assert_eq!(
            encode_toml(&Padding {
                value: "a".repeat(exact_padding + 1),
            }),
            Err(DocumentError::TooLarge)
        );
        let mut too_large = exact;
        too_large.push(b' ');
        assert_eq!(
            decode_toml::<toml::Value>(&too_large),
            Err(DocumentError::TooLarge)
        );
    }

    #[test]
    fn document_error_display_identifies_the_failed_boundary() {
        assert_eq!(
            DocumentError::InvalidIdentity.to_string(),
            "invalid control document (InvalidIdentity)"
        );
    }
}
