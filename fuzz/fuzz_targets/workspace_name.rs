#![no_main]

use std::str::FromStr;

use libfuzzer_sys::fuzz_target;
use thinws_core::WorkspaceName;

fuzz_target!(|input: &[u8]| {
    let Ok(text) = std::str::from_utf8(input) else {
        return;
    };
    let expected = is_valid_workspace_name(input);
    let parsed = WorkspaceName::from_str(text);
    assert_eq!(parsed.is_ok(), expected);

    if let Ok(name) = parsed {
        assert_eq!(name.as_str().as_bytes(), input);
        assert_eq!(name.to_string(), text);
        assert_eq!(WorkspaceName::from_str(name.as_str()), Ok(name));
    }
});

fn is_valid_workspace_name(input: &[u8]) -> bool {
    (1..=63).contains(&input.len())
        && input.first().is_some_and(|byte| is_boundary(*byte))
        && input.last().is_some_and(|byte| is_boundary(*byte))
        && input.iter().all(|byte| is_allowed(*byte))
        && !input.windows(2).any(|pair| pair == b"..")
}

const fn is_boundary(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit()
}

const fn is_allowed(byte: u8) -> bool {
    is_boundary(byte) || matches!(byte, b'.' | b'_' | b'-')
}
