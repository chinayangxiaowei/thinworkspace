#![no_main]

use std::str::FromStr;

use libfuzzer_sys::fuzz_target;
use thinws_application::CreateRequest;
use thinws_core::{AbsolutePath, ErrorCode, WorkspaceName};

fuzz_target!(|data: &[u8]| {
    let split = data.first().copied().map_or(0, usize::from) % data.len().max(1);
    let end = data.get(1).copied().map_or(0, usize::from) % data.len().max(1);
    let (split, end) = (split.min(end), split.max(end));
    let source = data.get(2..split).unwrap_or_default().to_vec();
    let target = data.get(split..end).unwrap_or_default().to_vec();
    let name = std::str::from_utf8(data.get(end..).unwrap_or_default()).unwrap_or("");
    let allow_copy = data.first().is_some_and(|byte| byte & 1 != 0);
    let expected_source = AbsolutePath::try_from_bytes(source.clone());
    let expected_target = AbsolutePath::try_from_bytes(target.clone());
    let expected_name = WorkspaceName::from_str(name);
    let actual = CreateRequest::try_from_raw(source.clone(), target.clone(), name, allow_copy, 0);
    assert_eq!(
        actual.is_ok(),
        expected_source.is_ok() && expected_target.is_ok() && expected_name.is_ok()
    );
    if let Ok(request) = actual {
        assert_eq!(request.source(), &expected_source.unwrap());
        assert_eq!(request.target(), &expected_target.unwrap());
        assert_eq!(request.name(), &expected_name.unwrap());
        assert_eq!(request.allow_full_copy(), allow_copy);
        assert_eq!(request.now().get(), 0);
    }
    let negative_clock = CreateRequest::try_from_raw(source, target, name, allow_copy, -1);
    assert!(negative_clock.is_err());
    assert_eq!(
        negative_clock.unwrap_err().diagnostic().code(),
        ErrorCode::Usage
    );
});
