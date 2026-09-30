use std::io;
use std::os::fd::{AsRawFd, OwnedFd};

use linux_raw_sys::btrfs::btrfs_ioctl_fs_info_args;

/// Reads the filesystem UUID through the held Btrfs directory descriptor.
pub(crate) fn btrfs_fsid(directory: &OwnedFd) -> io::Result<[u8; 16]> {
    let mut info = btrfs_ioctl_fs_info_args {
        max_id: 0,
        num_devices: 0,
        fsid: [0; 16],
        nodesize: 0,
        sectorsize: 0,
        clone_alignment: 0,
        csum_type: 0,
        csum_size: 0,
        flags: 0,
        generation: 0,
        metadata_uuid: [0; 16],
        reserved: [0; 944],
    };
    // SAFETY: BTRFS_IOC_FS_INFO writes into this fully initialized kernel
    // UAPI struct; the held descriptor remains owned by the caller.
    let result = unsafe {
        libc::ioctl(
            directory.as_raw_fd(),
            linux_raw_sys::ioctl::BTRFS_IOC_FS_INFO as libc::c_ulong,
            &raw mut info,
        )
    };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    if info.fsid == [0; 16] {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "zero Btrfs FSID",
        ));
    }
    Ok(info.fsid)
}

/// Clones ordinary-file data between two held descriptors on Btrfs.
pub(crate) fn reflink_clone(source: &OwnedFd, destination: &OwnedFd) -> io::Result<()> {
    // SAFETY: FICLONE consumes two live file descriptors by value. The kernel
    // does not retain either descriptor or an application pointer.
    let result = unsafe {
        libc::ioctl(
            destination.as_raw_fd(),
            libc::FICLONE as libc::c_ulong,
            source.as_raw_fd(),
        )
    };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs::{self, File};

    use super::*;

    #[test]
    fn btrfs_fsid_matches_the_kernel_mount_identity() {
        let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to the dedicated Btrfs mount");
        let directory: OwnedFd = File::open(root).unwrap().into();
        let fsid = uuid::Uuid::from_bytes(btrfs_fsid(&directory).unwrap()).to_string();
        let found = fs::read_dir("/sys/fs/btrfs")
            .unwrap()
            .filter_map(Result::ok)
            .any(|entry| entry.file_name() == fsid.as_str());
        assert!(
            found,
            "Btrfs ioctl FSID must name a mounted kernel filesystem"
        );
    }

    #[test]
    fn btrfs_fsid_rejects_a_non_btrfs_descriptor_with_the_ioctl_error() {
        let root = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT")
            .expect("set THINWS_LINUX_EXT4_TEST_ROOT to a real ext4 directory");
        let directory: OwnedFd = File::open(root).unwrap().into();
        assert_eq!(
            btrfs_fsid(&directory).unwrap_err().raw_os_error(),
            Some(libc::ENOTTY)
        );
    }
}
