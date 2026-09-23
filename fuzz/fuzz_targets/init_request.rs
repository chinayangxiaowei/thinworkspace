#![no_main]

use libfuzzer_sys::fuzz_target;
use thinws_application::InitRequest;
use thinws_core::{AbsolutePath, ErrorCode};

fuzz_target!(|data: &[u8]| {
    let expected = AbsolutePath::try_from_bytes(data.to_vec());
    let actual = InitRequest::try_from_raw(data.to_vec(), 0);
    assert_eq!(actual.is_ok(), expected.is_ok());

    if let (Ok(request), Ok(path)) = (actual, expected) {
        assert_eq!(request.data_root(), &path);
        assert_eq!(request.now().get(), 0);
    }

    let error = InitRequest::try_from_raw(data.to_vec(), -1)
        .expect_err("negative Unix milliseconds must always be rejected");
    assert!(matches!(error.diagnostic().code(), ErrorCode::Usage));
});
