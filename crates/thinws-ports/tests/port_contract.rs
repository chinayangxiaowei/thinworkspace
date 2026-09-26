use std::error::Error;
use std::fmt;

use thinws_ports::{MaterializationPathRole, PortConflict, PortError, PortErrorKind};

#[derive(Debug)]
struct SensitiveSource;

impl fmt::Display for SensitiveSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("/Users/example/private/state.db: sqlite extended code 2067")
    }
}

impl Error for SensitiveSource {}

#[test]
fn port_error_keeps_typed_classification_and_source_without_leaking_it() {
    let error = PortError::conflict("reserve workspace", PortConflict::WorkspaceName)
        .with_source(SensitiveSource);

    assert_eq!(error.kind(), PortErrorKind::Conflict);
    assert_eq!(error.operation(), "reserve workspace");
    assert_eq!(error.conflict_kind(), Some(PortConflict::WorkspaceName));
    assert!(error.source().is_some());
    assert_eq!(error.to_string(), "reserve workspace failed (Conflict)");
    let debug = format!("{error:?}");
    assert!(debug.contains("PortError"));
    assert!(debug.contains("reserve workspace"));
    assert!(debug.contains("WorkspaceName"));
    assert!(debug.contains("has_source: true"));
    assert!(!debug.contains("/Users"));
}

#[test]
fn materialization_probe_error_keeps_path_role_without_exposing_a_path() {
    let error = PortError::new(PortErrorKind::Unavailable, "open path component")
        .with_materialization_path_role(MaterializationPathRole::Source)
        .with_source(SensitiveSource);
    assert_eq!(
        error.materialization_path_role(),
        Some(MaterializationPathRole::Source)
    );
    assert!(!error.to_string().contains("/Users"));
}
