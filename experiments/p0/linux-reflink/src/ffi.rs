use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;

/// Clones the source file's data into an already-open destination via Linux FICLONE.
///
/// This is only a syscall experiment. It does not validate path ownership, open
/// files with no-follow semantics, preserve metadata, or publish a workspace.
pub fn clone_file_data(source: &File, destination: &File) -> io::Result<()> {
    // SAFETY: Both descriptors are borrowed from live File values for this call.
    // FICLONE takes the destination descriptor and a source descriptor passed
    // by value; it does not dereference a userspace pointer. The kernel checks
    // descriptor modes, file types, and same-filesystem reflink support.
    let status = unsafe { libc::ioctl(destination.as_raw_fd(), libc::FICLONE, source.as_raw_fd()) };
    if status == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
