#![no_main]

use libfuzzer_sys::fuzz_target;
use thinws_core::{AbsolutePath, RelativePath};

fuzz_target!(|input: &[u8]| {
    let absolute = AbsolutePath::try_from_bytes(input.to_vec());
    assert_eq!(absolute.is_ok(), valid_absolute(input));
    if let Ok(path) = absolute {
        assert_eq!(path.as_bytes(), input);
    }

    let relative = RelativePath::try_from_bytes(input.to_vec());
    assert_eq!(relative.is_ok(), valid_relative(input));
    if let Ok(path) = relative {
        assert_eq!(path.as_bytes(), input);
    }
});

fn valid_absolute(bytes: &[u8]) -> bool {
    if bytes.first() != Some(&b'/') || bytes.contains(&0) {
        return false;
    }
    bytes == b"/" || valid_components(&bytes[1..])
}

fn valid_relative(bytes: &[u8]) -> bool {
    !bytes.is_empty()
        && bytes.first() != Some(&b'/')
        && !bytes.contains(&0)
        && valid_components(bytes)
}

fn valid_components(bytes: &[u8]) -> bool {
    !bytes.is_empty()
        && bytes.last() != Some(&b'/')
        && bytes
            .split(|byte| *byte == b'/')
            .all(|component| !component.is_empty() && component != b"." && component != b"..")
}
