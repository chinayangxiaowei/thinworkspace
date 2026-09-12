#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../../experiments/p0/git-base/src/path_output.rs"]
mod path_output;

const MAX_INPUT_LEN: usize = 4096;

fuzz_target!(|input: &[u8]| {
    if input.len() > MAX_INPUT_LEN {
        return;
    }

    let actual = path_output::parse_path_list(input.to_vec());
    let expected = reference_parse(input);
    assert_eq!(actual.is_ok(), expected.is_some());

    let (Ok(actual), Some(expected)) = (actual, expected) else {
        return;
    };
    assert_eq!(actual, expected.sorted);
    assert_eq!(actual.len(), expected.original.len());

    for original in expected.original {
        let expected_count = input_field_count(input, &original);
        let actual_count = actual
            .iter()
            .filter(|path| path.as_slice() == original.as_slice())
            .count();
        assert_eq!(actual_count, expected_count);
    }
});

struct ReferencePaths {
    original: Vec<Vec<u8>>,
    sorted: Vec<Vec<u8>>,
}

fn reference_parse(input: &[u8]) -> Option<ReferencePaths> {
    if input.is_empty() {
        return Some(ReferencePaths {
            original: Vec::new(),
            sorted: Vec::new(),
        });
    }
    if input.last() != Some(&0) {
        return None;
    }

    let mut original = Vec::new();
    let mut field = Vec::new();
    for byte in input {
        if *byte == 0 {
            if field.is_empty() {
                return None;
            }
            original.push(std::mem::take(&mut field));
        } else {
            field.push(*byte);
        }
    }

    let mut sorted = Vec::<Vec<u8>>::with_capacity(original.len());
    for field in &original {
        let position = sorted
            .iter()
            .position(|existing| bytewise_less(field, existing))
            .unwrap_or(sorted.len());
        sorted.insert(position, field.clone());
    }

    Some(ReferencePaths { original, sorted })
}

fn bytewise_less(left: &[u8], right: &[u8]) -> bool {
    for (left_byte, right_byte) in left.iter().zip(right) {
        if left_byte != right_byte {
            return left_byte < right_byte;
        }
    }
    left.len() < right.len()
}

fn input_field_count(input: &[u8], needle: &[u8]) -> usize {
    let mut count = 0;
    let mut start = 0;
    for (index, byte) in input.iter().enumerate() {
        if *byte == 0 {
            if &input[start..index] == needle {
                count += 1;
            }
            start = index + 1;
        }
    }
    count
}
