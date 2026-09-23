use std::error::Error;
use std::str::FromStr;

use thinws_core::{
    ContextValue, CoreError, ErrorCode, InstanceId, OperationId, WorkspaceId, WorkspaceName,
    WorkspaceNameError,
};

const UUID_V7: &str = "01890a5d-ac96-774b-bd5b-55c7b8d09f33";
const UUID_V4: &str = "550e8400-e29b-41d4-a716-446655440000";

#[test]
fn typed_ids_require_canonical_uuid_v7_and_their_own_prefix() {
    let instance = InstanceId::from_str(UUID_V7).expect("canonical UUIDv7 is a valid instance ID");
    let workspace = WorkspaceId::from_str(&format!("ws_{UUID_V7}"))
        .expect("canonical prefixed UUIDv7 is a valid workspace ID");
    let operation = OperationId::from_str(&format!("op_{UUID_V7}"))
        .expect("canonical prefixed UUIDv7 is a valid operation ID");

    assert_eq!(instance.to_string(), UUID_V7);
    assert_eq!(workspace.to_string(), format!("ws_{UUID_V7}"));
    assert_eq!(operation.to_string(), format!("op_{UUID_V7}"));
    assert_eq!(instance.as_uuid().get_version_num(), 7);
    assert_eq!(workspace.as_uuid().get_version_num(), 7);
    assert_eq!(operation.as_uuid().get_version_num(), 7);

    assert!(InstanceId::from_str(UUID_V4).is_err());
    assert!(WorkspaceId::from_str(&format!("ws_{UUID_V4}")).is_err());
    assert!(OperationId::from_str(&format!("op_{UUID_V4}")).is_err());
    assert!(WorkspaceId::from_str(&format!("op_{UUID_V7}")).is_err());
    assert!(OperationId::from_str(&format!("ws_{UUID_V7}")).is_err());
    assert!(WorkspaceId::from_str(&format!("ws_{}", UUID_V7.to_uppercase())).is_err());
    assert!(WorkspaceId::from_str(&format!("ws_{}", UUID_V7.replace('-', ""))).is_err());
    assert!(
        WorkspaceId::from_str("ws_01890a5d-ac96-774b-3d5b-55c7b8d09f33").is_err(),
        "UUIDv7 also requires the RFC 4122 variant"
    );
}

#[test]
fn newly_generated_ids_are_distinct_uuid_v7_values() {
    let first_instance = InstanceId::new();
    let second_instance = InstanceId::new();
    let workspace = WorkspaceId::new();
    let operation = OperationId::new();

    assert_ne!(first_instance, second_instance);
    assert_eq!(first_instance.as_uuid().get_version_num(), 7);
    assert_eq!(workspace.as_uuid().get_version_num(), 7);
    assert_eq!(operation.as_uuid().get_version_num(), 7);
    assert!(workspace.to_string().starts_with("ws_"));
    assert!(operation.to_string().starts_with("op_"));
}

#[test]
fn workspace_names_follow_the_frozen_ascii_grammar_without_normalization() {
    for valid in ["a", "0", "auth-refresh", "a.b_c-9", &"a".repeat(63)] {
        let parsed = WorkspaceName::from_str(valid).expect("listed name is valid");
        assert_eq!(parsed.as_str(), valid);
        assert_eq!(parsed.to_string(), valid);
    }

    assert_eq!(
        WorkspaceName::from_str("").unwrap_err(),
        WorkspaceNameError::Empty
    );
    assert_eq!(
        WorkspaceName::from_str(&"a".repeat(64)).unwrap_err(),
        WorkspaceNameError::TooLong { length: 64 }
    );
    assert_eq!(
        WorkspaceName::from_str("-a").unwrap_err(),
        WorkspaceNameError::InvalidBoundary
    );
    assert_eq!(
        WorkspaceName::from_str("a-").unwrap_err(),
        WorkspaceNameError::InvalidBoundary
    );
    assert_eq!(
        WorkspaceName::from_str("aAb").unwrap_err(),
        WorkspaceNameError::InvalidCharacter { index: 1 }
    );
    assert_eq!(
        WorkspaceName::from_str("a..b").unwrap_err(),
        WorkspaceNameError::ConsecutiveDots
    );
    assert!(WorkspaceName::from_str("é").is_err());
}

#[test]
fn error_codes_preserve_the_frozen_names_and_exit_statuses() {
    let expected = [
        (ErrorCode::Usage, "E_USAGE", 2),
        (ErrorCode::NotInitialized, "E_NOT_INITIALIZED", 10),
        (
            ErrorCode::CapabilityUnavailable,
            "E_CAPABILITY_UNAVAILABLE",
            11,
        ),
        (ErrorCode::CowUnavailable, "E_COW_UNAVAILABLE", 12),
        (ErrorCode::NameConflict, "E_NAME_CONFLICT", 15),
        (
            ErrorCode::DataRootChangeUnsupported,
            "E_DATA_ROOT_CHANGE_UNSUPPORTED",
            16,
        ),
        (ErrorCode::WorkspaceNotFound, "E_WORKSPACE_NOT_FOUND", 20),
        (ErrorCode::WorkspaceNotReady, "E_WORKSPACE_NOT_READY", 21),
        (ErrorCode::WorkspaceDirty, "E_WORKSPACE_DIRTY", 22),
        (ErrorCode::WorkspaceBusy, "E_WORKSPACE_BUSY", 23),
        (ErrorCode::GitCheckIncomplete, "E_GIT_CHECK_INCOMPLETE", 25),
        (ErrorCode::Git, "E_GIT", 30),
        (ErrorCode::Filesystem, "E_FILESYSTEM", 31),
        (
            ErrorCode::DataRootUnavailable,
            "E_DATA_ROOT_UNAVAILABLE",
            32,
        ),
        (ErrorCode::DataRootLayout, "E_DATA_ROOT_LAYOUT", 33),
        (ErrorCode::Metadata, "E_METADATA", 35),
        (ErrorCode::DataRootNotEmpty, "E_DATA_ROOT_NOT_EMPTY", 36),
        (ErrorCode::WorkspaceIncomplete, "E_WORKSPACE_INCOMPLETE", 40),
        (ErrorCode::LockTimeout, "E_LOCK_TIMEOUT", 41),
    ];

    assert_eq!(ErrorCode::ALL.len(), expected.len());
    for (actual, expected) in ErrorCode::ALL.iter().copied().zip(expected) {
        assert_eq!(actual, expected.0);
        assert_eq!(actual.as_str(), expected.1);
        assert_eq!(actual.exit_status(), expected.2);
        assert_eq!(actual.to_string(), expected.1);
    }
}

#[test]
fn structured_errors_keep_sensitive_context_out_of_display_and_debug() {
    fn assert_error(value: &dyn Error) {
        assert!(value.source().is_none());
    }

    let error = CoreError::new(ErrorCode::Filesystem, "workspace inspection failed")
        .with_context(
            "workspace_id",
            ContextValue::public(format!("ws_{UUID_V7}")),
        )
        .with_context(
            "source_path",
            ContextValue::sensitive("/Users/example/private/source"),
        )
        .with_remediation("verify that the source is readable and retry");

    assert_error(&error);
    assert_eq!(error.code(), ErrorCode::Filesystem);
    assert_eq!(error.message(), "workspace inspection failed");
    assert_eq!(
        error.remediation(),
        Some("verify that the source is readable and retry")
    );
    assert_eq!(
        error.context()["source_path"].user_value(),
        "/Users/example/private/source"
    );
    assert_eq!(error.context()["source_path"].log_value(), "[redacted]");
    assert_eq!(
        error.context()["workspace_id"].log_value(),
        format!("ws_{UUID_V7}")
    );
    assert_eq!(
        error.to_string(),
        "E_FILESYSTEM: workspace inspection failed"
    );
    let debug = format!("{error:?}");
    assert!(debug.contains("[redacted]"));
    assert!(debug.contains(&format!("ws_{UUID_V7}")));
    assert!(!debug.contains("/Users/example/private/source"));
}
