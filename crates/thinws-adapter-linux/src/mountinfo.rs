use std::fs::File;
use std::io::{self, Read};

const MAX_MOUNTINFO_BYTES: u64 = 4 * 1024 * 1024;

pub(crate) fn filesystem_type(mount_id: u64) -> io::Result<Option<String>> {
    let file = File::open("/proc/self/mountinfo")?;
    let mut bytes = Vec::new();
    file.take(MAX_MOUNTINFO_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_MOUNTINFO_BYTES {
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

#[cfg(test)]
mod tests {
    use super::parse_filesystem_type;

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
}
