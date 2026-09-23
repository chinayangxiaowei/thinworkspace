use thinws_adapter_macos::{
    DocumentError, MAX_DOCUMENT_BYTES, decode_bootstrap_config, decode_root_marker,
};
use thinws_core::RootMarkerState;

const PREFIX: &str = "schema_version = 1\ninstance_id = \"01890a5d-ac96-774b-bd5b-55c7b8d09f33\"\n";
const SUFFIX: &str = "volume_id = \"550e8400-e29b-41d4-a716-446655440000\"\n";

#[test]
fn valid_config_decodes_lossless_path_identity() {
    let document = b"schema_version = 1\ninstance_id = \"01890a5d-ac96-774b-bd5b-55c7b8d09f33\"\ndata_root_hex = \"2f566f6c756d65732f646174612fff\"\nvolume_id = \"550e8400-e29b-41d4-a716-446655440000\"\n";

    let identity = decode_bootstrap_config(document).expect("valid document must decode");
    assert_eq!(identity.data_root().as_bytes(), b"/Volumes/data/\xff");
}

#[test]
fn marker_parser_rejects_unknown_versions_fields_states_and_ambiguous_path_hex() {
    let ready = format!(
        "{PREFIX}data_root_hex = \"2f566f6c756d65732f64617461\"\n{SUFFIX}state = \"ready\"\n"
    );
    assert_eq!(
        decode_root_marker(ready.as_bytes()).unwrap().state(),
        RootMarkerState::Ready
    );

    for (document, expected) in [
        (
            ready.replace("schema_version = 1", "schema_version = 2"),
            DocumentError::UnsupportedVersion,
        ),
        (format!("{ready}extra = true\n"), DocumentError::InvalidToml),
        (
            ready.replace("state = \"ready\"", "state = \"unknown\""),
            DocumentError::InvalidToml,
        ),
        (
            ready.replace("2f566f6c756d65732f64617461", "2F"),
            DocumentError::InvalidPathHex,
        ),
        (
            ready.replace("2f566f6c756d65732f64617461", "2"),
            DocumentError::InvalidPathHex,
        ),
        (
            ready.replace("2f566f6c756d65732f64617461", "2f746d702f2e2e2f78"),
            DocumentError::InvalidIdentity,
        ),
        (
            ready.replace("2f566f6c756d65732f64617461", "2f746d702f610062"),
            DocumentError::InvalidIdentity,
        ),
    ] {
        assert_eq!(
            decode_root_marker(document.as_bytes()).unwrap_err(),
            expected
        );
    }
}

#[test]
fn parser_enforces_size_utf8_and_typed_ids_before_toml_values_escape() {
    assert_eq!(MAX_DOCUMENT_BYTES, 65_536);
    let mut exact = format!("{PREFIX}data_root_hex = \"2f746d70\"\n{SUFFIX}").into_bytes();
    exact.resize(MAX_DOCUMENT_BYTES, b' ');
    assert!(decode_bootstrap_config(&exact).is_ok());
    assert_eq!(
        decode_bootstrap_config(&vec![b'a'; MAX_DOCUMENT_BYTES + 1]).unwrap_err(),
        DocumentError::TooLarge
    );
    assert_eq!(
        decode_bootstrap_config(&[0xff]).unwrap_err(),
        DocumentError::InvalidEncoding
    );
    let wrong_instance = format!(
        "schema_version = 1\ninstance_id = \"550e8400-e29b-41d4-a716-446655440000\"\ndata_root_hex = \"2f746d70\"\n{SUFFIX}"
    );
    assert_eq!(
        decode_bootstrap_config(wrong_instance.as_bytes()).unwrap_err(),
        DocumentError::InvalidIdentity
    );
    let trailing_volume = format!(
        "{PREFIX}data_root_hex = \"2f746d70\"\nvolume_id = \"550e8400-e29b-41d4-a716-446655440000tail\"\n"
    );
    assert_eq!(
        decode_bootstrap_config(trailing_volume.as_bytes()).unwrap_err(),
        DocumentError::InvalidIdentity
    );
    assert_eq!(
        DocumentError::InvalidToml.to_string(),
        "invalid bootstrap document (InvalidToml)"
    );
}
