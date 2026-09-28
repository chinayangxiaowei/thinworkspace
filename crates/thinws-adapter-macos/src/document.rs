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
const DOCUMENT_SCHEMA_VERSION: u32 = 2;

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
    /// The control-root hex is odd, uppercase, or contains a non-hex byte.
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkspaceOwnership {
    pub(crate) instance_id: InstanceId,
    pub(crate) workspace_id: WorkspaceId,
    pub(crate) volume_id: VolumeId,
    pub(crate) target_path: AbsolutePath,
    pub(crate) parent: HistoricalDirectoryIdentity,
    pub(crate) target: HistoricalDirectoryIdentity,
    pub(crate) staging: OperationDirectoryOwnership,
    pub(crate) trash: OperationDirectoryOwnership,
    pub(crate) isolated_path: Option<AbsolutePath>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OperationDirectoryOwnership {
    pub(crate) path: AbsolutePath,
    pub(crate) identity: HistoricalDirectoryIdentity,
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
        &document.control_root_hex,
        &document.control_volume_id,
    )
}

/// Decodes a versioned control-root marker without touching the filesystem.
pub fn decode_root_marker(bytes: &[u8]) -> Result<RootMarker, DocumentError> {
    let document: MarkerDocument = decode_toml(bytes)?;
    if document.schema_version != DOCUMENT_SCHEMA_VERSION {
        return Err(DocumentError::UnsupportedVersion);
    }
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
        schema_version: DOCUMENT_SCHEMA_VERSION,
        instance_id: ownership.instance_id.to_string(),
        workspace_id: ownership.workspace_id.to_string(),
        volume_id: ownership.volume_id.to_string(),
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
    if document.schema_version != DOCUMENT_SCHEMA_VERSION {
        return Err(DocumentError::UnsupportedVersion);
    }
    if document.target_parent_birth_nanoseconds >= 1_000_000_000
        || document.target_birth_nanoseconds >= 1_000_000_000
        || document.staging_birth_nanoseconds >= 1_000_000_000
        || document.trash_birth_nanoseconds >= 1_000_000_000
        || document.target_parent_inode == 0
        || document.target_inode == 0
        || document.staging_inode == 0
        || document.trash_inode == 0
        || document.target_parent_birth_seconds < 0
        || document.target_birth_seconds < 0
        || document.staging_birth_seconds < 0
        || document.trash_birth_seconds < 0
    {
        return Err(DocumentError::InvalidIdentity);
    }
    let workspace_id = WorkspaceId::from_str(&document.workspace_id)
        .map_err(|_| DocumentError::InvalidIdentity)?;
    let target_path = AbsolutePath::try_from_bytes(decode_hex(&document.target_path_hex)?)
        .map_err(|_| DocumentError::InvalidIdentity)?;
    let target_parent = parent_path_bytes(&target_path).ok_or(DocumentError::InvalidIdentity)?;
    let decode_operation_directory =
        |hex: &str, inode: u64, birth_seconds: i64, birth_nanoseconds: u32| {
            let path = AbsolutePath::try_from_bytes(decode_hex(hex)?)
                .map_err(|_| DocumentError::InvalidIdentity)?;
            Ok(OperationDirectoryOwnership {
                path,
                identity: HistoricalDirectoryIdentity {
                    inode,
                    birth_seconds,
                    birth_nanoseconds,
                },
            })
        };
    let staging = decode_operation_directory(
        &document.staging_path_hex,
        document.staging_inode,
        document.staging_birth_seconds,
        document.staging_birth_nanoseconds,
    )?;
    let trash = decode_operation_directory(
        &document.trash_path_hex,
        document.trash_inode,
        document.trash_birth_seconds,
        document.trash_birth_nanoseconds,
    )?;
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
        .map(|hex| {
            let path = AbsolutePath::try_from_bytes(decode_hex(hex)?)
                .map_err(|_| DocumentError::InvalidIdentity)?;
            if path.as_bytes()
                != sibling_path_bytes(target_parent, &format!(".thinws-remove-{workspace_id}"))
            {
                return Err(DocumentError::InvalidIdentity);
            }
            Ok(path)
        })
        .transpose()?;
    Ok(WorkspaceOwnership {
        instance_id: InstanceId::from_str(&document.instance_id)
            .map_err(|_| DocumentError::InvalidIdentity)?,
        workspace_id,
        volume_id: VolumeId::from_str(&document.volume_id)
            .map_err(|_| DocumentError::InvalidIdentity)?,
        target_path,
        parent: HistoricalDirectoryIdentity {
            inode: document.target_parent_inode,
            birth_seconds: document.target_parent_birth_seconds,
            birth_nanoseconds: document.target_parent_birth_nanoseconds,
        },
        target: HistoricalDirectoryIdentity {
            inode: document.target_inode,
            birth_seconds: document.target_birth_seconds,
            birth_nanoseconds: document.target_birth_nanoseconds,
        },
        staging,
        trash,
        isolated_path,
    })
}

fn parent_path_bytes(path: &AbsolutePath) -> Option<&[u8]> {
    let bytes = path.as_bytes();
    if bytes == b"/" {
        return None;
    }
    let last_slash = bytes
        .iter()
        .rposition(|byte| *byte == b'/')
        .expect("validated absolute path has a separator");
    Some(if last_slash == 0 {
        b"/"
    } else {
        &bytes[..last_slash]
    })
}

fn sibling_path_bytes(parent: &[u8], leaf: &str) -> Vec<u8> {
    let mut bytes = parent.to_vec();
    if bytes != b"/" {
        bytes.push(b'/');
    }
    bytes.extend_from_slice(leaf.as_bytes());
    bytes
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

#[cfg(test)]
mod ownership_fuzz_seed_tests {
    use super::*;

    #[test]
    fn ownership_fuzz_corpus_reaches_both_valid_and_invalid_identity_paths() {
        let valid =
            include_bytes!("../../../fuzz/corpus/thinws_bootstrap_document/valid-ownership");
        let ownership = decode_workspace_ownership(valid).unwrap();
        assert_eq!(
            decode_workspace_ownership(&encode_workspace_ownership(&ownership).unwrap()),
            Ok(ownership)
        );

        let invalid = include_bytes!(
            "../../../fuzz/corpus/thinws_bootstrap_document/invalid-birth-nanoseconds"
        );
        assert_eq!(
            decode_workspace_ownership(invalid),
            Err(DocumentError::InvalidIdentity)
        );
    }

    #[test]
    fn ownership_rejects_each_negative_birth_seconds_field() {
        let valid =
            include_bytes!("../../../fuzz/corpus/thinws_bootstrap_document/valid-ownership");
        let ownership = decode_workspace_ownership(valid).unwrap();

        let mut invalid_parent = ownership.clone();
        invalid_parent.parent.birth_seconds = -1;
        assert_eq!(
            decode_workspace_ownership(&encode_workspace_ownership(&invalid_parent).unwrap()),
            Err(DocumentError::InvalidIdentity)
        );

        let mut invalid_root = ownership;
        invalid_root.target.birth_seconds = -1;
        assert_eq!(
            decode_workspace_ownership(&encode_workspace_ownership(&invalid_root).unwrap()),
            Err(DocumentError::InvalidIdentity)
        );
    }

    #[test]
    fn ownership_rejects_each_zero_inode_field() {
        let valid =
            include_bytes!("../../../fuzz/corpus/thinws_bootstrap_document/valid-ownership");
        let ownership = decode_workspace_ownership(valid).unwrap();

        let mut invalid_parent = ownership.clone();
        invalid_parent.parent.inode = 0;
        assert_eq!(
            decode_workspace_ownership(&encode_workspace_ownership(&invalid_parent).unwrap()),
            Err(DocumentError::InvalidIdentity)
        );

        let mut invalid_root = ownership;
        invalid_root.target.inode = 0;
        assert_eq!(
            decode_workspace_ownership(&encode_workspace_ownership(&invalid_root).unwrap()),
            Err(DocumentError::InvalidIdentity)
        );
    }

    #[test]
    fn ownership_accepts_each_zero_birth_seconds_field() {
        let valid =
            include_bytes!("../../../fuzz/corpus/thinws_bootstrap_document/valid-ownership");
        let ownership = decode_workspace_ownership(valid).unwrap();

        let mut zero_parent = ownership.clone();
        zero_parent.parent.birth_seconds = 0;
        assert_eq!(
            decode_workspace_ownership(&encode_workspace_ownership(&zero_parent).unwrap()),
            Ok(zero_parent)
        );

        let mut zero_root = ownership;
        zero_root.target.birth_seconds = 0;
        assert_eq!(
            decode_workspace_ownership(&encode_workspace_ownership(&zero_root).unwrap()),
            Ok(zero_root)
        );
    }
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
        let exact_path_length = (MAX_DOCUMENT_BYTES - base) / 2;

        assert_eq!(
            encode_config(&identity_with_path_length(exact_path_length))
                .unwrap()
                .len(),
            base + exact_path_length * 2
        );
        assert!(base + exact_path_length * 2 <= MAX_DOCUMENT_BYTES);
        assert_eq!(
            encode_config(&identity_with_path_length(exact_path_length + 1)).unwrap_err(),
            DocumentError::TooLarge
        );
    }

    #[test]
    fn v2_ownership_records_the_exact_target_and_parent_identity() {
        let valid =
            include_bytes!("../../../fuzz/corpus/thinws_bootstrap_document/valid-ownership");
        let ownership = decode_workspace_ownership(valid).unwrap();
        let encoded = encode_workspace_ownership(&ownership).unwrap();
        let text = std::str::from_utf8(&encoded).unwrap();
        assert!(text.contains("target_path_hex = "), "{text}");
        assert!(text.contains("target_parent_inode = "), "{text}");
        assert!(!text.contains("container_inode = "), "{text}");
        assert_eq!(ownership.target_path.as_bytes(), b"/tmp/clone");
        assert_eq!(ownership.parent.inode, 17);
        assert_eq!(ownership.target.inode, 23);
        assert_eq!(ownership.isolated_path, None);
    }

    #[test]
    fn v2_ownership_rejects_old_fields_and_unrelated_isolation_paths() {
        let valid = std::str::from_utf8(include_bytes!(
            "../../../fuzz/corpus/thinws_bootstrap_document/valid-ownership"
        ))
        .unwrap();
        for invalid in [
            valid.replace("target_path_hex", "data_root_hex"),
            valid.replace("target_parent_inode", "container_inode"),
            valid.replace("target_path_hex = \"2f746d702f636c6f6e65\"\n", ""),
            valid.replace(
                "target_path_hex = \"2f746d702f636c6f6e65\"",
                "target_path_hex = \"2f\"",
            ),
        ] {
            assert!(decode_workspace_ownership(invalid.as_bytes()).is_err());
        }

        let ownership = decode_workspace_ownership(valid.as_bytes()).unwrap();
        let mut isolated = ownership.clone();
        isolated.isolated_path = Some(
            AbsolutePath::try_from_bytes(
                b"/tmp/.thinws-remove-ws_01890a5d-ac96-774b-bd5b-55c7b8d09f42".to_vec(),
            )
            .unwrap(),
        );
        assert_eq!(
            decode_workspace_ownership(&encode_workspace_ownership(&isolated).unwrap()),
            Ok(isolated.clone())
        );
        isolated.isolated_path = Some(
            AbsolutePath::try_from_bytes(
                b"/outside/.thinws-remove-ws_01890a5d-ac96-774b-bd5b-55c7b8d09f42".to_vec(),
            )
            .unwrap(),
        );
        assert_eq!(
            decode_workspace_ownership(&encode_workspace_ownership(&isolated).unwrap()),
            Err(DocumentError::InvalidIdentity)
        );
    }

    #[test]
    fn v2_ownership_validates_each_historical_identity_field_independently() {
        let valid = decode_workspace_ownership(include_bytes!(
            "../../../fuzz/corpus/thinws_bootstrap_document/valid-ownership"
        ))
        .unwrap();
        let invalid_cases = [
            (
                "parent inode",
                (|value: &mut WorkspaceOwnership| value.parent.inode = 0)
                    as fn(&mut WorkspaceOwnership),
            ),
            ("target inode", |value| value.target.inode = 0),
            ("staging inode", |value| value.staging.identity.inode = 0),
            ("trash inode", |value| value.trash.identity.inode = 0),
            ("parent birth seconds", |value| {
                value.parent.birth_seconds = -1
            }),
            ("target birth seconds", |value| {
                value.target.birth_seconds = -1
            }),
            ("staging birth seconds", |value| {
                value.staging.identity.birth_seconds = -1
            }),
            ("trash birth seconds", |value| {
                value.trash.identity.birth_seconds = -1
            }),
            ("parent birth nanos", |value| {
                value.parent.birth_nanoseconds = 1_000_000_000
            }),
            ("target birth nanos", |value| {
                value.target.birth_nanoseconds = 1_000_000_000
            }),
            ("staging birth nanos", |value| {
                value.staging.identity.birth_nanoseconds = 1_000_000_000
            }),
            ("trash birth nanos", |value| {
                value.trash.identity.birth_nanoseconds = 1_000_000_000
            }),
        ];
        for (name, mutate) in invalid_cases {
            let mut invalid = valid.clone();
            mutate(&mut invalid);
            assert_eq!(
                decode_workspace_ownership(&encode_workspace_ownership(&invalid).unwrap()),
                Err(DocumentError::InvalidIdentity),
                "{name}"
            );
        }

        for (name, mutate) in [
            (
                "staging birth seconds",
                (|value: &mut WorkspaceOwnership| {
                    value.staging.identity.birth_seconds = 0;
                }) as fn(&mut WorkspaceOwnership),
            ),
            ("trash birth seconds", |value: &mut WorkspaceOwnership| {
                value.trash.identity.birth_seconds = 0;
            }),
        ] {
            let mut boundary = valid.clone();
            mutate(&mut boundary);
            assert_eq!(
                decode_workspace_ownership(&encode_workspace_ownership(&boundary).unwrap()),
                Ok(boundary),
                "{name}"
            );
        }
    }

    #[test]
    fn v2_ownership_rejects_each_noncanonical_operation_path() {
        let valid = decode_workspace_ownership(include_bytes!(
            "../../../fuzz/corpus/thinws_bootstrap_document/valid-ownership"
        ))
        .unwrap();
        let mut wrong_staging = valid.clone();
        wrong_staging.staging.path =
            AbsolutePath::try_from_bytes(b"/tmp/other-staging".to_vec()).unwrap();
        assert_eq!(
            decode_workspace_ownership(&encode_workspace_ownership(&wrong_staging).unwrap()),
            Err(DocumentError::InvalidIdentity)
        );
        let mut wrong_trash = valid;
        wrong_trash.trash.path =
            AbsolutePath::try_from_bytes(b"/tmp/other-trash".to_vec()).unwrap();
        assert_eq!(
            decode_workspace_ownership(&encode_workspace_ownership(&wrong_trash).unwrap()),
            Err(DocumentError::InvalidIdentity)
        );
    }

    #[test]
    fn canonical_target_parent_handles_root_and_nested_paths() {
        for (path, expected) in [
            ("/", None),
            ("/clone", Some(b"/".as_slice())),
            ("/tmp/clone", Some(b"/tmp".as_slice())),
        ] {
            let path = AbsolutePath::try_from_bytes(path.as_bytes().to_vec()).unwrap();
            assert_eq!(parent_path_bytes(&path), expected);
        }
    }
}
