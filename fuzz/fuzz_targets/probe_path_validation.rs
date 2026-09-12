#![no_main]

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::Path;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let path = Path::new(OsStr::from_bytes(input));
    let result = thinws_p0_probe::validate_probe_path(path);

    let absolute = input.first() == Some(&b'/');
    let contains_nul = input.contains(&0);
    let contains_parent = input.split(|byte| *byte == b'/').any(|part| part == b"..");
    let should_succeed = absolute && !contains_nul && !contains_parent;
    assert_eq!(result.is_ok(), should_succeed);
    let Ok(validated) = result else {
        return;
    };

    let expected_components: Vec<&[u8]> = input
        .split(|byte| *byte == b'/')
        .filter(|part| !part.is_empty() && *part != b".")
        .collect();
    assert_eq!(validated.components.len(), expected_components.len());

    let mut expected_normalized = vec![b'/'];
    for (index, expected) in expected_components.iter().enumerate() {
        if index > 0 {
            expected_normalized.push(b'/');
        }
        expected_normalized.extend_from_slice(expected);
        assert_eq!(
            decode_hex(&validated.components[index].bytes_hex),
            *expected
        );
    }

    let normalized_bytes = decode_hex(&validated.normalized.bytes_hex);
    assert_eq!(normalized_bytes, expected_normalized);
    assert_eq!(normalized_bytes.first(), Some(&b'/'));

    let mut rebuilt = vec![b'/'];
    for (index, component) in validated.components.iter().enumerate() {
        let component_bytes = decode_hex(&component.bytes_hex);
        assert!(!component_bytes.is_empty());
        assert_ne!(component_bytes.as_slice(), b".");
        assert_ne!(component_bytes.as_slice(), b"..");
        assert!(!component_bytes.contains(&b'/'));
        assert!(!component_bytes.contains(&0));
        if index > 0 {
            rebuilt.push(b'/');
        }
        rebuilt.extend(component_bytes);
    }
    assert_eq!(normalized_bytes, rebuilt);

    let normalized = OsString::from_vec(normalized_bytes);
    let second = thinws_p0_probe::validate_probe_path(Path::new(&normalized))
        .expect("normalized validated bytes remain valid");
    assert_eq!(second, validated);
});

fn decode_hex(value: &str) -> Vec<u8> {
    assert!(value.len().is_multiple_of(2));
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| (nibble(pair[0]) << 4) | nibble(pair[1]))
        .collect()
}

fn nibble(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => panic!("probe emitted non-lowercase hexadecimal"),
    }
}
