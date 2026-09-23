use std::error::Error;
use std::fmt;

use thinws_ports::{PortConflict, PortError, PortErrorKind};

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
    assert!(!error.to_string().contains("/Users"));
    assert!(!format!("{error:?}").contains("/Users"));
}
