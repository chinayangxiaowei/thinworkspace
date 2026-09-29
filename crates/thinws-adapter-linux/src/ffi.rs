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
