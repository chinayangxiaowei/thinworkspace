use std::ffi::{CStr, CString};
use std::io;
use std::mem::MaybeUninit;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RawFileKind {
    Directory,
    RegularFile,
    SymbolicLink,
    Fifo,
    Socket,
    CharacterDevice,
    BlockDevice,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RawNodeMetadata {
    pub(crate) device: u64,
    pub(crate) inode: u64,
    pub(crate) kind: RawFileKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RawFileSystemMetadata {
    pub(crate) type_name: String,
    pub(crate) fsid: [i32; 2],
    pub(crate) mount_flags: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RawHostIdentity {
    pub(crate) system_name: String,
    pub(crate) kernel_release: String,
    pub(crate) architecture: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RawCloneCapability {
    pub(crate) interface_capabilities: u32,
    pub(crate) interface_valid: u32,
}

#[repr(C)]
struct VolumeUuidBuffer {
    length: u32,
    returned: libc::attribute_set_t,
    uuid: libc::uuid_t,
}

#[repr(C)]
struct VolumeCapabilitiesBuffer {
    length: u32,
    returned: libc::attribute_set_t,
    capabilities: libc::vol_capabilities_attr_t,
}

pub(crate) fn open_root_directory() -> io::Result<OwnedFd> {
    let root = c"/";
    let flags = libc::O_SEARCH | (libc::O_NOFOLLOW | libc::O_CLOEXEC);

    // SAFETY: `root` is a static NUL-terminated path and `open` does not retain
    // the pointer. The returned descriptor is checked before ownership is taken.
    let raw_fd = unsafe { libc::open(root.as_ptr(), flags) };
    owned_fd_from_syscall(raw_fd)
}

pub(crate) fn open_directory_at(parent: &OwnedFd, name: &CStr) -> io::Result<OwnedFd> {
    validate_component(name)?;
    // Darwin defines O_SEARCH as O_EXEC | O_DIRECTORY.
    let flags = libc::O_SEARCH | (libc::O_NOFOLLOW | libc::O_CLOEXEC);

    // SAFETY: `parent` stays alive for the call, `name` is NUL-terminated and
    // borrowed for the call only, and a successful descriptor is uniquely owned.
    let raw_fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    owned_fd_from_syscall(raw_fd)
}

pub(crate) fn metadata(fd: &OwnedFd) -> io::Result<RawNodeMetadata> {
    let mut stat = MaybeUninit::<libc::stat>::uninit();

    // SAFETY: `fd` is live and `stat` points to writable storage of the exact
    // type required by `fstat`. The storage is read only after a zero return.
    let result = unsafe { libc::fstat(fd.as_raw_fd(), stat.as_mut_ptr()) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: a successful `fstat` initialized the full structure.
    let stat = unsafe { stat.assume_init() };
    Ok(raw_node_metadata(&stat))
}

pub(crate) fn metadata_at(parent: &OwnedFd, name: &CStr) -> io::Result<RawNodeMetadata> {
    validate_component(name)?;
    let mut stat = MaybeUninit::<libc::stat>::uninit();

    // SAFETY: `parent` and `name` remain valid for the call. The no-follow flag
    // makes this observation describe the directory entry, including a symlink.
    let result = unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: a successful `fstatat` initialized the full structure.
    let stat = unsafe { stat.assume_init() };
    Ok(raw_node_metadata(&stat))
}

pub(crate) fn file_system_metadata(fd: &OwnedFd) -> io::Result<RawFileSystemMetadata> {
    let mut stat = MaybeUninit::<libc::statfs>::uninit();

    // SAFETY: `fd` is live and the output points to writable storage of the
    // exact structure expected by `fstatfs`.
    let result = unsafe { libc::fstatfs(fd.as_raw_fd(), stat.as_mut_ptr()) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: a successful `fstatfs` initialized the full structure, including
    // a fixed-size NUL-terminated filesystem-name array supplied by the kernel.
    let stat = unsafe { stat.assume_init() };
    // SAFETY: Darwin guarantees `f_fstypename` is a NUL-terminated fixed buffer.
    let type_name = unsafe { CStr::from_ptr(stat.f_fstypename.as_ptr()) }
        .to_string_lossy()
        .into_owned();

    Ok(RawFileSystemMetadata {
        type_name,
        fsid: fsid_components(&stat.f_fsid),
        mount_flags: stat.f_flags,
    })
}

pub(crate) fn volume_uuid(fd: &OwnedFd) -> io::Result<Option<[u8; 16]>> {
    let mut attributes = libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: libc::ATTR_CMN_RETURNED_ATTRS,
        volattr: libc::ATTR_VOL_INFO | libc::ATTR_VOL_UUID,
        dirattr: 0,
        fileattr: 0,
        forkattr: 0,
    };
    let mut buffer = MaybeUninit::<VolumeUuidBuffer>::zeroed();

    // SAFETY: `fd` is live; both pointers refer to writable, correctly sized C
    // representations for the duration of the call. No pointer is retained.
    let result = unsafe {
        libc::fgetattrlist(
            fd.as_raw_fd(),
            (&raw mut attributes).cast(),
            buffer.as_mut_ptr().cast(),
            size_of::<VolumeUuidBuffer>(),
            0,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: the buffer was zero-initialized, and `fgetattrlist` returned
    // success. The explicit length/returned-bitmap checks below determine
    // whether the UUID field itself was supplied by the filesystem.
    let buffer = unsafe { buffer.assume_init() };
    Ok(decode_volume_uuid_buffer(&buffer))
}

fn decode_volume_uuid_buffer(buffer: &VolumeUuidBuffer) -> Option<[u8; 16]> {
    let uuid_end =
        size_of::<u32>() + size_of::<libc::attribute_set_t>() + size_of::<libc::uuid_t>();
    if usize::try_from(buffer.length).unwrap_or(0) < uuid_end
        || buffer.returned.volattr & libc::ATTR_VOL_UUID == 0
    {
        return None;
    }

    Some(buffer.uuid)
}

pub(crate) fn clone_capability(fd: &OwnedFd) -> io::Result<Option<RawCloneCapability>> {
    let mut attributes = libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: libc::ATTR_CMN_RETURNED_ATTRS,
        volattr: libc::ATTR_VOL_INFO | libc::ATTR_VOL_CAPABILITIES,
        dirattr: 0,
        fileattr: 0,
        forkattr: 0,
    };
    let mut buffer = MaybeUninit::<VolumeCapabilitiesBuffer>::zeroed();

    // SAFETY: `fd` is live; both pointers refer to writable, correctly sized C
    // representations for the duration of the call. No pointer is retained.
    let result = unsafe {
        libc::fgetattrlist(
            fd.as_raw_fd(),
            (&raw mut attributes).cast(),
            buffer.as_mut_ptr().cast(),
            size_of::<VolumeCapabilitiesBuffer>(),
            0,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: the buffer was zero-initialized and the syscall succeeded. The
    // length and returned-attribute bitmap are checked before using payload.
    let buffer = unsafe { buffer.assume_init() };
    Ok(decode_volume_capabilities_buffer(&buffer))
}

fn decode_volume_capabilities_buffer(
    buffer: &VolumeCapabilitiesBuffer,
) -> Option<RawCloneCapability> {
    if usize::try_from(buffer.length).unwrap_or(0) < size_of::<VolumeCapabilitiesBuffer>()
        || buffer.returned.volattr & libc::ATTR_VOL_CAPABILITIES == 0
    {
        return None;
    }

    Some(RawCloneCapability {
        interface_capabilities: buffer.capabilities.capabilities[libc::VOL_CAPABILITIES_INTERFACES],
        interface_valid: buffer.capabilities.valid[libc::VOL_CAPABILITIES_INTERFACES],
    })
}

pub(crate) fn effective_write_search_access(fd: &OwnedFd) -> io::Result<()> {
    effective_access(fd, libc::W_OK | libc::X_OK)
}

pub(crate) fn effective_read_search_access(fd: &OwnedFd) -> io::Result<()> {
    effective_access(fd, libc::R_OK | libc::X_OK)
}

fn effective_access(fd: &OwnedFd, mode: libc::c_int) -> io::Result<()> {
    let current_directory = c".";

    // SAFETY: `fd` is a held directory descriptor and `current_directory` is a
    // static NUL-terminated relative path. The literal `.` cannot name a
    // symlink, so AT_SYMLINK_NOFOLLOW is redundant here; parameterizing this
    // path would require a new no-follow review. This is preflight evidence only.
    let result = unsafe {
        libc::faccessat(
            fd.as_raw_fd(),
            current_directory.as_ptr(),
            mode,
            libc::AT_EACCESS,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub(crate) fn host_identity() -> io::Result<RawHostIdentity> {
    let mut name = MaybeUninit::<libc::utsname>::uninit();

    // SAFETY: `name` points to writable storage of the exact type required by
    // `uname`; it is read only after the call reports success.
    let result = unsafe { libc::uname(name.as_mut_ptr()) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: successful `uname` initialized every fixed-size C string field.
    let name = unsafe { name.assume_init() };
    Ok(RawHostIdentity {
        system_name: c_char_array_to_string(&name.sysname),
        kernel_release: c_char_array_to_string(&name.release),
        architecture: c_char_array_to_string(&name.machine),
    })
}

pub(crate) fn product_version() -> io::Result<String> {
    let name = c"kern.osproductversion";
    let mut length = 0_usize;

    // SAFETY: `name` is NUL-terminated; a null output with a valid length
    // pointer is the documented size query and no pointer is retained.
    let size_result = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            std::ptr::null_mut(),
            &raw mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    if size_result != 0 {
        return Err(io::Error::last_os_error());
    }
    if !(2..=4096).contains(&length) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "kern.osproductversion returned an invalid size",
        ));
    }

    let mut value = vec![0_u8; length];
    // SAFETY: `value` owns at least `length` writable bytes, `name` remains
    // valid, and sysctlbyname updates `length` without retaining either pointer.
    let value_result = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            value.as_mut_ptr().cast(),
            &raw mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    if value_result != 0 {
        return Err(io::Error::last_os_error());
    }
    value.truncate(length);
    if value.last() == Some(&0) {
        value.pop();
    }
    String::from_utf8(value).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub(crate) fn c_string(bytes: &[u8]) -> io::Result<CString> {
    CString::new(bytes).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "path component contains a NUL byte",
        )
    })
}

fn owned_fd_from_syscall(raw_fd: libc::c_int) -> io::Result<OwnedFd> {
    if raw_fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: a non-negative descriptor returned by `open`/`openat` is new
        // and uniquely transferred into this `OwnedFd` exactly once.
        Ok(unsafe { OwnedFd::from_raw_fd(raw_fd) })
    }
}

fn validate_component(name: &CStr) -> io::Result<()> {
    let bytes = name.to_bytes();
    if bytes.is_empty() || bytes == b"." || bytes == b".." || bytes.contains(&b'/') {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "dirfd operation requires one non-special path component",
        ))
    } else {
        Ok(())
    }
}

fn raw_node_metadata(stat: &libc::stat) -> RawNodeMetadata {
    RawNodeMetadata {
        device: u64::from(stat.st_dev.cast_unsigned()),
        inode: stat.st_ino,
        kind: raw_file_kind(stat.st_mode),
    }
}

fn fsid_components(fsid: &libc::fsid_t) -> [i32; 2] {
    const _: () = assert!(size_of::<libc::fsid_t>() == size_of::<[i32; 2]>());
    const _: () = assert!(align_of::<libc::fsid_t>() == align_of::<[i32; 2]>());

    // SAFETY: Darwin's SDK declares `fsid_t` as exactly two `int32_t` values.
    // The compile-time size/alignment assertions guard the matching libc ABI.
    unsafe { std::ptr::read((fsid as *const libc::fsid_t).cast::<[i32; 2]>()) }
}

fn raw_file_kind(mode: libc::mode_t) -> RawFileKind {
    match mode & libc::S_IFMT {
        libc::S_IFDIR => RawFileKind::Directory,
        libc::S_IFREG => RawFileKind::RegularFile,
        libc::S_IFLNK => RawFileKind::SymbolicLink,
        libc::S_IFIFO => RawFileKind::Fifo,
        libc::S_IFSOCK => RawFileKind::Socket,
        libc::S_IFCHR => RawFileKind::CharacterDevice,
        libc::S_IFBLK => RawFileKind::BlockDevice,
        _ => RawFileKind::Unknown,
    }
}

fn c_char_array_to_string<const N: usize>(value: &[libc::c_char; N]) -> String {
    // SAFETY: `uname` guarantees each field is a NUL-terminated fixed buffer.
    unsafe { CStr::from_ptr(value.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::os::fd::{AsRawFd, OwnedFd};
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    use tempfile::tempdir_in;

    use super::{
        RawFileKind, VolumeCapabilitiesBuffer, VolumeUuidBuffer, decode_volume_capabilities_buffer,
        decode_volume_uuid_buffer, effective_read_search_access, effective_write_search_access,
        fsid_components, open_directory_at, open_root_directory, owned_fd_from_syscall,
        raw_file_kind, validate_component,
    };

    fn returned_attributes(volume: u32) -> libc::attribute_set_t {
        libc::attribute_set_t {
            commonattr: 0,
            volattr: volume,
            dirattr: 0,
            fileattr: 0,
            forkattr: 0,
        }
    }

    #[test]
    fn volume_uuid_buffer_requires_complete_length_and_returned_bit() {
        let uuid = [7; 16];
        let complete_length = u32::try_from(
            size_of::<u32>() + size_of::<libc::attribute_set_t>() + size_of::<libc::uuid_t>(),
        )
        .expect("attribute buffer length fits u32");
        let complete = VolumeUuidBuffer {
            length: complete_length,
            returned: returned_attributes(libc::ATTR_VOL_UUID),
            uuid,
        };
        assert_eq!(decode_volume_uuid_buffer(&complete), Some(uuid));

        let short = VolumeUuidBuffer {
            length: complete_length - 1,
            ..complete
        };
        assert_eq!(decode_volume_uuid_buffer(&short), None);

        let missing_bit = VolumeUuidBuffer {
            returned: returned_attributes(0),
            ..complete
        };
        assert_eq!(decode_volume_uuid_buffer(&missing_bit), None);
    }

    #[test]
    fn clone_capability_buffer_requires_complete_length_and_returned_bit() {
        let mut capabilities = libc::vol_capabilities_attr_t {
            capabilities: [0; 4],
            valid: [0; 4],
        };
        capabilities.capabilities[libc::VOL_CAPABILITIES_INTERFACES] = libc::VOL_CAP_INT_CLONE;
        capabilities.valid[libc::VOL_CAPABILITIES_INTERFACES] = libc::VOL_CAP_INT_CLONE;
        let complete_length =
            u32::try_from(size_of::<VolumeCapabilitiesBuffer>()).expect("buffer length fits u32");
        let complete = VolumeCapabilitiesBuffer {
            length: complete_length,
            returned: returned_attributes(libc::ATTR_VOL_CAPABILITIES),
            capabilities,
        };
        let decoded = decode_volume_capabilities_buffer(&complete).expect("complete payload");
        assert_eq!(decoded.interface_capabilities, libc::VOL_CAP_INT_CLONE);
        assert_eq!(decoded.interface_valid, libc::VOL_CAP_INT_CLONE);

        let short = VolumeCapabilitiesBuffer {
            length: complete_length - 1,
            ..complete
        };
        assert_eq!(decode_volume_capabilities_buffer(&short), None);

        let missing_bit = VolumeCapabilitiesBuffer {
            returned: returned_attributes(0),
            ..complete
        };
        assert_eq!(decode_volume_capabilities_buffer(&missing_bit), None);
    }

    #[test]
    fn opened_directory_descriptors_are_close_on_exec() {
        let root = open_root_directory().expect("open root directory");
        let private = open_directory_at(&root, c"private").expect("open one directory component");

        for fd in [&root, &private] {
            // SAFETY: `fd` is live for this non-mutating descriptor flag query.
            let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) };
            assert!(
                flags >= 0,
                "F_GETFD failed: {}",
                std::io::Error::last_os_error()
            );
            assert_ne!(flags & libc::FD_CLOEXEC, 0);
        }
    }

    #[test]
    fn effective_access_distinguishes_source_read_from_destination_write() {
        let temp = tempdir_in("/private/tmp").expect("create controlled directory");
        let descriptor: OwnedFd = File::open(temp.path())
            .expect("open controlled directory")
            .into();
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o500))
            .expect("make controlled directory read-only");

        let read = effective_read_search_access(&descriptor);
        let write = effective_write_search_access(&descriptor);
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700))
            .expect("restore permissions before assertions and cleanup");

        assert!(read.is_ok());
        assert_eq!(
            write
                .expect_err("write/search must be denied")
                .raw_os_error(),
            Some(libc::EACCES)
        );
    }

    #[test]
    fn component_validation_rejects_every_non_normal_shape() {
        for invalid in [c"", c".", c"..", c"a/b"] {
            assert!(validate_component(invalid).is_err());
        }
        assert!(validate_component(c"normal").is_ok());
    }

    #[test]
    fn raw_file_kind_maps_every_supported_mode() {
        let cases = [
            (libc::S_IFDIR, RawFileKind::Directory),
            (libc::S_IFREG, RawFileKind::RegularFile),
            (libc::S_IFLNK, RawFileKind::SymbolicLink),
            (libc::S_IFIFO, RawFileKind::Fifo),
            (libc::S_IFSOCK, RawFileKind::Socket),
            (libc::S_IFCHR, RawFileKind::CharacterDevice),
            (libc::S_IFBLK, RawFileKind::BlockDevice),
            (0, RawFileKind::Unknown),
        ];
        for (mode, expected) in cases {
            assert_eq!(raw_file_kind(mode), expected);
        }
    }

    #[test]
    fn fsid_components_preserve_both_abi_words() {
        let expected = [23_i32, -17_i32];
        let mut storage = std::mem::MaybeUninit::<libc::fsid_t>::uninit();
        // SAFETY: the production compile-time assertions establish that Darwin
        // `fsid_t` has the size and alignment of two i32 words.
        unsafe { storage.as_mut_ptr().cast::<[i32; 2]>().write(expected) };
        // SAFETY: the preceding write initialized every byte of `fsid_t`.
        let fsid = unsafe { storage.assume_init() };
        assert_eq!(fsid_components(&fsid), expected);
    }

    #[test]
    fn owned_fd_from_syscall_accepts_descriptor_zero_in_an_isolated_process() {
        const CHILD: &str = "THINWS_P0_FD_ZERO_CHILD";
        if std::env::var_os(CHILD).is_some() {
            // SAFETY: this branch runs in a dedicated child process. Closing
            // stdin cannot affect the parent test runner.
            assert_eq!(unsafe { libc::close(libc::STDIN_FILENO) }, 0);
            // SAFETY: `/dev/null` is a static C string and the returned FD is
            // checked before it is transferred to `OwnedFd`.
            let raw_fd = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY) };
            assert_eq!(raw_fd, libc::STDIN_FILENO);
            let fd = owned_fd_from_syscall(raw_fd).expect("descriptor zero is valid");
            assert_eq!(fd.as_raw_fd(), libc::STDIN_FILENO);
            return;
        }

        let status = Command::new(std::env::current_exe().expect("current unit-test binary"))
            .args([
                "--exact",
                "ffi::tests::owned_fd_from_syscall_accepts_descriptor_zero_in_an_isolated_process",
            ])
            .env(CHILD, "1")
            .status()
            .expect("run isolated fd-zero test");
        assert!(status.success());
    }
}
