use std::fs::File;
use std::io;

use rustix::fs::{AtFlags, StatxFlags, fstatfs, statx};

/// Path-level Btrfs reflink eligibility, not evidence that FICLONE succeeded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BtrfsPairPreflight {
    /// Both held files are on Btrfs and the same observed mount.
    EligibleSameMount,
    /// At least one held file is not on Btrfs.
    UnsupportedFilesystem,
    /// At least one held descriptor does not refer to a regular file.
    UnsupportedFileType,
    /// Both files are on Btrfs but the observed mount IDs differ.
    DifferentMount,
    /// The kernel did not provide mount IDs for both held files.
    UnknownMountIdentity,
}

/// Inspects an already-open source and destination without changing either file.
///
/// A positive result is only a conservative path-level preflight. Per-file
/// attributes, permissions and the real FICLONE result remain to be checked.
pub fn inspect_btrfs_pair(source: &File, destination: &File) -> io::Result<BtrfsPairPreflight> {
    if !source.metadata()?.file_type().is_file() || !destination.metadata()?.file_type().is_file() {
        return Ok(BtrfsPairPreflight::UnsupportedFileType);
    }
    let source_fs = fstatfs(source)?;
    let destination_fs = fstatfs(destination)?;
    if source_fs.f_type != libc::BTRFS_SUPER_MAGIC
        || destination_fs.f_type != libc::BTRFS_SUPER_MAGIC
    {
        return Ok(BtrfsPairPreflight::UnsupportedFilesystem);
    }

    let source_mount = mount_id(source)?;
    let destination_mount = mount_id(destination)?;
    Ok(classify_mounts(source_mount, destination_mount))
}

fn mount_id(file: &File) -> io::Result<Option<u64>> {
    let observed = statx(file, "", AtFlags::EMPTY_PATH, StatxFlags::MNT_ID)?;
    Ok((observed.stx_mask & StatxFlags::MNT_ID.bits() != 0).then_some(observed.stx_mnt_id))
}

fn classify_mounts(source: Option<u64>, destination: Option<u64>) -> BtrfsPairPreflight {
    match (source, destination) {
        (Some(source), Some(destination)) if source == destination => {
            BtrfsPairPreflight::EligibleSameMount
        }
        (Some(_), Some(_)) => BtrfsPairPreflight::DifferentMount,
        _ => BtrfsPairPreflight::UnknownMountIdentity,
    }
}

#[cfg(test)]
mod tests {
    use super::{BtrfsPairPreflight, classify_mounts};

    #[test]
    fn requires_both_mount_ids_to_match() {
        assert_eq!(
            classify_mounts(Some(11), Some(11)),
            BtrfsPairPreflight::EligibleSameMount
        );
        assert_eq!(
            classify_mounts(Some(11), Some(12)),
            BtrfsPairPreflight::DifferentMount
        );
        assert_eq!(
            classify_mounts(Some(11), None),
            BtrfsPairPreflight::UnknownMountIdentity
        );
    }
}
