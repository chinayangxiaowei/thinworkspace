#![no_main]

use std::str::FromStr;

use libfuzzer_sys::fuzz_target;
use thinws_application::RemoveRequest;
use thinws_core::{ErrorCode, WorkspaceId, WorkspaceName};

fuzz_target!(|data: &[u8]| {
    let Ok(target) = std::str::from_utf8(data) else {
        return;
    };
    // Text corpus files include a final newline; exercise both those literal
    // bytes and the corresponding exact public argument.
    for target in std::iter::once(target).chain(target.strip_suffix('\n')) {
        let expected = if let Some(name) = target.strip_prefix("name:") {
            WorkspaceName::from_str(name).is_ok()
        } else if let Some(id) = target.strip_prefix("id:") {
            WorkspaceId::from_str(id).is_ok()
        } else {
            WorkspaceName::from_str(target).is_ok() || WorkspaceId::from_str(target).is_ok()
        };

        for force in [false, true] {
            let parsed = RemoveRequest::try_from_raw(target, force, 0);
            assert_eq!(parsed.is_ok(), expected);
            if let Err(error) = parsed {
                assert_eq!(error.diagnostic().code(), ErrorCode::Usage);
            }
            let negative_clock = RemoveRequest::try_from_raw(target, force, -1);
            assert!(negative_clock.is_err());
            assert_eq!(
                negative_clock.unwrap_err().diagnostic().code(),
                ErrorCode::Usage
            );
        }
    }
});
