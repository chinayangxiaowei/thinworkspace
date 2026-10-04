use std::fs::File;
use std::io::{self, Read};

const MAX_MOUNTINFO_BYTES: u64 = 4 * 1024 * 1024;

pub(crate) fn filesystem_type(mount_id: u64) -> io::Result<Option<String>> {
    let file = File::open("/proc/self/mountinfo")?;
    parse_bounded_mountinfo(file, mount_id)
}

fn parse_bounded_mountinfo(reader: impl Read, mount_id: u64) -> io::Result<Option<String>> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_MOUNTINFO_BYTES + 1)
        .read_to_end(&mut bytes)?;
    // The capped reader can return one sentinel byte beyond the accepted size.
    if bytes.len() as u64 == MAX_MOUNTINFO_BYTES + 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "mountinfo exceeds limit",
        ));
    }
    let content = std::str::from_utf8(&bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "mountinfo is not UTF-8"))?;
    parse_filesystem_type(content, mount_id)
}

fn parse_filesystem_type(content: &str, mount_id: u64) -> io::Result<Option<String>> {
    let mut found = None;
    for line in content.lines() {
        let Some(id) = line.split(' ').next().and_then(|id| id.parse::<u64>().ok()) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid mount ID",
            ));
        };
        if id != mount_id {
            continue;
        }
        let kind = line
            .split_once(" - ")
            .and_then(|(_, right)| right.split(' ').next())
            .filter(|kind| !kind.is_empty() && kind.bytes().all(|byte| byte.is_ascii_graphic()))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid mount type"))?;
        if found.replace(kind.to_owned()).is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "duplicate mount ID",
            ));
        }
    }
    Ok(found)
}

/// Exercises bounded Linux mountinfo parsing without reading the host's `/proc`.
#[cfg(fuzzing)]
pub fn fuzz_mountinfo(bytes: &[u8]) {
    use std::io::Cursor;

    let _ = parse_bounded_mountinfo(Cursor::new(bytes), 41);

    let generated = bytes
        .iter()
        .take(24)
        .map(|byte| char::from(b'a' + *byte % 26))
        .collect::<String>();
    let kind = if generated.is_empty() {
        "btrfs"
    } else {
        &generated
    };
    let valid = format!("41 1 0:43 / / rw - {kind} /dev/sdd rw\n");
    assert_eq!(
        parse_bounded_mountinfo(Cursor::new(valid.as_bytes()), 41)
            .expect("generated mountinfo is valid")
            .as_deref(),
        Some(kind)
    );
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::parse_filesystem_type;
    use super::{MAX_MOUNTINFO_BYTES, parse_bounded_mountinfo};

    fn padded_mountinfo(total: usize) -> Vec<u8> {
        let matched = b"41 1 0:43 / / rw - ext4";
        let padding = total - matched.len();
        let mut bytes = b"2 ".to_vec();
        bytes.resize(padding - 1, b'x');
        bytes.push(b'\n');
        bytes.extend_from_slice(matched);
        assert_eq!(bytes.len(), total);
        bytes
    }

    #[test]
    fn matches_exact_mount_id_and_rejects_ambiguous_entries() {
        let input =
            "41 1 0:43 / / rw - ext4 /dev/sda rw\n414 1 0:50 / /mnt rw - btrfs /dev/sdb rw\n";
        assert_eq!(
            parse_filesystem_type(input, 41).unwrap().as_deref(),
            Some("ext4")
        );
        assert_eq!(
            parse_filesystem_type(input, 414).unwrap().as_deref(),
            Some("btrfs")
        );
        assert_eq!(parse_filesystem_type(input, 4).unwrap(), None);
        assert!(
            parse_filesystem_type(
                &format!("{input}41 1 0:44 / /other rw - xfs /dev/sdc rw\n"),
                41
            )
            .is_err()
        );
    }

    #[test]
    fn fuzz_corpus_contains_a_valid_btrfs_mount() {
        let seed = include_bytes!("../../../fuzz/corpus/thinws_linux_mountinfo/valid-btrfs");
        assert_eq!(
            parse_bounded_mountinfo(Cursor::new(seed), 41)
                .unwrap()
                .as_deref(),
            Some("btrfs")
        );
    }

    #[test]
    fn rejects_empty_and_control_character_filesystem_types() {
        assert!(parse_filesystem_type("41 1 0:43 / / rw -  /dev/sda rw\n", 41).is_err());
        assert!(parse_filesystem_type("41 1 0:43 / / rw - ext4\t /dev/sda rw\n", 41).is_err());
    }

    #[test]
    fn mountinfo_reader_accepts_valid_input_below_and_at_the_limit() {
        for size in [2 * 1024 * 1024, MAX_MOUNTINFO_BYTES as usize] {
            assert_eq!(
                parse_bounded_mountinfo(Cursor::new(padded_mountinfo(size)), 41)
                    .unwrap()
                    .as_deref(),
                Some("ext4")
            );
        }
    }

    #[test]
    fn mountinfo_reader_rejects_one_byte_beyond_the_limit() {
        assert!(
            parse_bounded_mountinfo(
                Cursor::new(padded_mountinfo(MAX_MOUNTINFO_BYTES as usize + 1)),
                41
            )
            .is_err()
        );
    }
}
