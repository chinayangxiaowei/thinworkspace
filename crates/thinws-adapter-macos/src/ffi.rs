use std::ffi::{CStr, CString, OsString};
use std::io;
use std::mem::MaybeUninit;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStringExt;

const CLONE_NOFOLLOW_ANY: u32 = 0x0008;

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
    pub(crate) mode: u32,
    pub(crate) size: u64,
    pub(crate) allocated_bytes: u64,
    pub(crate) modified_seconds: i64,
    pub(crate) modified_nanoseconds: i64,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RawProcessIdentity {
    pub(crate) pid: i32,
    pub(crate) uid: u32,
    pub(crate) start_seconds: u64,
    pub(crate) start_microseconds: u64,
    pub(crate) open_file_count: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RawProcessVnode {
    pub(crate) path: OsString,
    pub(crate) device: u64,
    pub(crate) inode: u64,
}

#[repr(C)]
struct ProcFileInfo {
    open_flags: u32,
    status: u32,
    offset: i64,
    kind: i32,
    guard_flags: u32,
}

#[repr(C)]
struct VnodeFdInfoWithPath {
    file: ProcFileInfo,
    vnode: libc::vnode_info_path,
}

const PROC_UID_ONLY: u32 = 4;
const PROC_PIDFDVNODEPATHINFO: i32 = 2;
const MAX_VISIBLE_PIDS: usize = 16_384;
const MAX_PROCESS_FDS: usize = 32_768;

pub(crate) fn visible_user_pids() -> io::Result<(Vec<i32>, bool)> {
    // SAFETY: a null buffer asks libproc for a byte capacity estimate.
    let estimated_bytes =
        unsafe { libc::proc_listpids(PROC_UID_ONLY, libc::geteuid(), std::ptr::null_mut(), 0) };
    if estimated_bytes <= 0 {
        return Err(libproc_error("list current-user processes"));
    }
    let item_size = std::mem::size_of::<i32>();
    let capacity = list_capacity(estimated_bytes, item_size, 64, MAX_VISIBLE_PIDS);
    let mut pids = vec![0_i32; capacity];
    let buffer_size = c_int_buffer_bytes(capacity, item_size)?;
    // SAFETY: the vector owns writable storage for exactly buffer_size bytes.
    let bytes = unsafe {
        libc::proc_listpids(
            PROC_UID_ONLY,
            libc::geteuid(),
            pids.as_mut_ptr().cast(),
            buffer_size,
        )
    };
    let count = checked_list_count(bytes, item_size, capacity, "read current-user process list")?;
    Ok(complete_pid_list(pids, count, capacity))
}

pub(crate) fn fd_kernel_path(fd: &impl AsRawFd) -> io::Result<OsString> {
    let mut bytes = [0_u8; libc::MAXPATHLEN as usize];
    // SAFETY: F_GETPATH writes at most MAXPATHLEN bytes to the live buffer and
    // does not retain the descriptor or pointer after this call.
    let result = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETPATH, bytes.as_mut_ptr()) };
    if result == -1 {
        return Err(io::Error::last_os_error());
    }
    decode_kernel_path(&bytes)
}

fn decode_kernel_path(bytes: &[u8]) -> io::Result<OsString> {
    let length = bytes
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| io::Error::other("unterminated kernel directory path"))?;
    if length == 0 || bytes[0] != b'/' {
        return Err(io::Error::other("kernel directory path is not absolute"));
    }
    Ok(OsString::from_vec(bytes[..length].to_vec()))
}

pub(crate) fn process_identity(pid: i32) -> io::Result<RawProcessIdentity> {
    let mut info = MaybeUninit::<libc::proc_bsdinfo>::uninit();
    let size = c_int_buffer_bytes(1, std::mem::size_of::<libc::proc_bsdinfo>())?;
    // SAFETY: libproc writes at most size bytes to the correctly typed buffer.
    let bytes = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if bytes != size {
        return Err(libproc_error("inspect process identity"));
    }
    // SAFETY: an exact-size successful return initializes the full ABI struct.
    let info = unsafe { info.assume_init() };
    Ok(RawProcessIdentity {
        pid: i32::try_from(info.pbi_pid).map_err(|_| io::Error::other("invalid process PID"))?,
        uid: info.pbi_uid,
        start_seconds: info.pbi_start_tvsec,
        start_microseconds: info.pbi_start_tvusec,
        open_file_count: info.pbi_nfiles,
    })
}

pub(crate) fn process_cwd(pid: i32) -> io::Result<Option<RawProcessVnode>> {
    let mut info = MaybeUninit::<libc::proc_vnodepathinfo>::uninit();
    let size = c_int_buffer_bytes(1, std::mem::size_of::<libc::proc_vnodepathinfo>())?;
    // SAFETY: libproc writes at most size bytes to the correctly typed buffer.
    let bytes = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if bytes != size {
        return Err(libproc_error("inspect process cwd"));
    }
    // SAFETY: an exact-size successful return initializes the full ABI struct.
    let info = unsafe { info.assume_init() };
    vnode_path(&info.pvi_cdir)
}

pub(crate) fn process_vnode_fds(pid: i32, expected: u32) -> io::Result<(Vec<i32>, bool)> {
    if expected == 0 {
        return Ok((Vec::new(), false));
    }
    // SAFETY: a null buffer asks libproc for a byte capacity estimate.
    let estimated_bytes =
        unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0) };
    if estimated_bytes <= 0 {
        return Err(libproc_error("size process fd list"));
    }
    let item_size = std::mem::size_of::<libc::proc_fdinfo>();
    let capacity = list_capacity(estimated_bytes, item_size, 32, MAX_PROCESS_FDS);
    let mut fds = vec![
        libc::proc_fdinfo {
            proc_fd: 0,
            proc_fdtype: 0
        };
        capacity
    ];
    let buffer_size = c_int_buffer_bytes(capacity, item_size)?;
    // SAFETY: the vector owns writable storage for exactly buffer_size bytes.
    let bytes = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDLISTFDS,
            0,
            fds.as_mut_ptr().cast(),
            buffer_size,
        )
    };
    let count = checked_list_count(bytes, item_size, capacity, "read process fd list")?;
    Ok(complete_fd_list(fds, count, capacity))
}

fn list_capacity(estimated_bytes: i32, item_size: usize, spare: usize, limit: usize) -> usize {
    (estimated_bytes as usize / item_size)
        .saturating_add(spare)
        .min(limit)
}

fn checked_list_count(
    bytes: i32,
    item_size: usize,
    capacity: usize,
    operation: &'static str,
) -> io::Result<usize> {
    if bytes <= 0
        || !(bytes as usize).is_multiple_of(item_size)
        || bytes as usize > capacity * item_size
    {
        Err(libproc_error(operation))
    } else {
        Ok(bytes as usize / item_size)
    }
}

fn complete_pid_list(mut pids: Vec<i32>, count: usize, capacity: usize) -> (Vec<i32>, bool) {
    pids.truncate(count);
    pids.retain(|pid| *pid > 0);
    (pids, count == capacity)
}

fn complete_fd_list(
    mut fds: Vec<libc::proc_fdinfo>,
    count: usize,
    capacity: usize,
) -> (Vec<i32>, bool) {
    fds.truncate(count);
    (
        fds.into_iter()
            .filter(|fd| fd.proc_fdtype == libc::PROX_FDTYPE_VNODE as u32)
            .map(|fd| fd.proc_fd)
            .collect(),
        count == capacity,
    )
}

pub(crate) fn process_vnode_fd(pid: i32, fd: i32) -> io::Result<Option<RawProcessVnode>> {
    let mut info = MaybeUninit::<VnodeFdInfoWithPath>::uninit();
    let size = c_int_buffer_bytes(1, std::mem::size_of::<VnodeFdInfoWithPath>())?;
    // SAFETY: the C-repr buffer matches the SDK's vnode_fdinfowithpath layout.
    let bytes = unsafe {
        libc::proc_pidfdinfo(
            pid,
            fd,
            PROC_PIDFDVNODEPATHINFO,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if bytes != size {
        return Err(libproc_error("inspect process vnode fd"));
    }
    // SAFETY: an exact-size successful return initializes the full ABI struct.
    let info = unsafe { info.assume_init() };
    vnode_path(&info.vnode)
}

fn vnode_path(info: &libc::vnode_info_path) -> io::Result<Option<RawProcessVnode>> {
    // SAFETY: vip_path is a live, fixed-size C-char array in the ABI struct.
    let bytes = unsafe {
        std::slice::from_raw_parts(
            (&raw const info.vip_path).cast::<u8>(),
            std::mem::size_of_val(&info.vip_path),
        )
    };
    let Some(length) = bytes.iter().position(|byte| *byte == 0) else {
        return Err(io::Error::other("unterminated process vnode path"));
    };
    if length == 0 {
        return Ok(None);
    }
    Ok(Some(RawProcessVnode {
        path: OsString::from_vec(bytes[..length].to_vec()),
        device: u64::from(info.vip_vi.vi_stat.vst_dev),
        inode: info.vip_vi.vi_stat.vst_ino,
    }))
}

fn c_int_buffer_bytes(count: usize, item_size: usize) -> io::Result<i32> {
    count
        .checked_mul(item_size)
        .and_then(|bytes| i32::try_from(bytes).ok())
        .ok_or_else(|| io::Error::other("libproc buffer exceeds int range"))
}

fn libproc_error(operation: &'static str) -> io::Error {
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(0) {
        io::Error::other(operation)
    } else {
        error
    }
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
    let flags = libc::O_SEARCH | libc::O_NOFOLLOW | libc::O_CLOEXEC;

    // SAFETY: the static root path is NUL-terminated and the returned
    // descriptor is checked before unique ownership is constructed.
    let raw_fd = unsafe { libc::open(c"/".as_ptr(), flags) };
    owned_fd(raw_fd)
}

pub(crate) fn open_directory_at(parent: &OwnedFd, name: &CStr) -> io::Result<OwnedFd> {
    validate_component(name)?;
    let flags = libc::O_SEARCH | libc::O_NOFOLLOW | libc::O_CLOEXEC;

    // SAFETY: `parent` and the validated NUL-terminated component remain live
    // for the non-retaining call; a successful descriptor is newly owned.
    let raw_fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    owned_fd(raw_fd)
}

pub(crate) fn open_file_read_at(parent: &OwnedFd, name: &CStr) -> io::Result<OwnedFd> {
    validate_component(name)?;
    let flags = libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC;

    // SAFETY: the live directory descriptor and validated component remain
    // valid for the duration of the non-retaining call.
    let raw_fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    owned_fd(raw_fd)
}

pub(crate) fn create_file_at(parent: &OwnedFd, name: &CStr, mode: u32) -> io::Result<OwnedFd> {
    validate_component(name)?;
    let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;

    // SAFETY: the live directory descriptor and validated component remain
    // valid for the call. O_EXCL prevents overwriting any existing entry and
    // a successful descriptor is newly owned by this process.
    let raw_fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags,
            mode as libc::c_int,
        )
    };
    owned_fd(raw_fd)
}

pub(crate) fn node_metadata(fd: &impl AsRawFd) -> io::Result<RawNodeMetadata> {
    let mut stat = MaybeUninit::<libc::stat>::uninit();

    // SAFETY: `fd` is live and `stat` is writable storage of the exact ABI type.
    let result = unsafe { libc::fstat(fd.as_raw_fd(), stat.as_mut_ptr()) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: successful `fstat` initialized the whole structure.
    Ok(raw_node_metadata(unsafe { stat.assume_init() }))
}

pub(crate) fn node_metadata_at(parent: &OwnedFd, name: &CStr) -> io::Result<RawNodeMetadata> {
    validate_component(name)?;
    let mut stat = MaybeUninit::<libc::stat>::uninit();

    // SAFETY: arguments stay live for the call and AT_SYMLINK_NOFOLLOW makes
    // the result describe the directory entry rather than a link target.
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

    // SAFETY: successful `fstatat` initialized the whole structure.
    Ok(raw_node_metadata(unsafe { stat.assume_init() }))
}

pub(crate) fn read_directory(directory: &OwnedFd) -> io::Result<Vec<OsString>> {
    read_directory_with_limit(directory, None)
}

pub(crate) fn read_directory_bounded(
    directory: &OwnedFd,
    max_entries: usize,
) -> io::Result<Vec<OsString>> {
    read_directory_with_limit(directory, Some(max_entries))
}

fn read_directory_with_limit(
    directory: &OwnedFd,
    max_entries: Option<usize>,
) -> io::Result<Vec<OsString>> {
    // A fresh open file description is required because dup would share the
    // directory offset and make later enumerations observe EOF.
    // SAFETY: `directory` remains live and the static `.` is NUL-terminated.
    let readable_raw =
        unsafe { libc::openat(directory.as_raw_fd(), c".".as_ptr(), libc::O_CLOEXEC) };
    let readable = owned_fd(readable_raw)?;

    // SAFETY: fdopendir takes ownership of the descriptor on success.
    let stream = unsafe { libc::fdopendir(readable.as_raw_fd()) };
    if stream.is_null() {
        return Err(io::Error::last_os_error());
    }
    std::mem::forget(readable);
    let stream = DirectoryStream(stream);

    let mut names = Vec::new();
    loop {
        // SAFETY: Darwin exposes the current thread's errno through __error.
        unsafe { *libc::__error() = 0 };
        // SAFETY: `stream` owns a live DIR pointer for the duration of the call.
        let entry = unsafe { libc::readdir(stream.0) };
        if entry.is_null() {
            // SAFETY: errno is read immediately after readdir.
            let errno = unsafe { *libc::__error() };
            if errno == 0 {
                break;
            }
            return Err(io::Error::from_raw_os_error(errno));
        }

        // SAFETY: Darwin guarantees d_namlen bytes in the returned record,
        // which remains valid until the next readdir call.
        let bytes = unsafe {
            let length = usize::from((*entry).d_namlen);
            let name = (&raw const (*entry).d_name).cast::<u8>();
            std::slice::from_raw_parts(name, length)
        };
        if bytes != b"." && bytes != b".." {
            if max_entries.is_some_and(|limit| names.len() >= limit) {
                return Err(io::Error::from_raw_os_error(libc::EOVERFLOW));
            }
            names.push(OsString::from_vec(bytes.to_vec()));
        }
    }
    Ok(names)
}

pub(crate) fn clone_file_at(
    source: &OwnedFd,
    target_parent: &OwnedFd,
    target_name: &CStr,
) -> io::Result<()> {
    validate_component(target_name)?;

    // SAFETY: descriptors are live, the component is validated, and the
    // syscall retains none of its arguments.
    let result = unsafe {
        libc::fclonefileat(
            source.as_raw_fd(),
            target_parent.as_raw_fd(),
            target_name.as_ptr(),
            CLONE_NOFOLLOW_ANY,
        )
    };
    syscall_unit(result)
}

pub(crate) fn create_directory_at(parent: &OwnedFd, name: &CStr, mode: u32) -> io::Result<()> {
    validate_component(name)?;

    // SAFETY: parent and component remain valid for the non-retaining call.
    let result = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), mode as libc::mode_t) };
    syscall_unit(result)
}

pub(crate) fn set_mode(fd: &impl AsRawFd, mode: u32) -> io::Result<()> {
    // SAFETY: the live descriptor identifies the exact object to update.
    syscall_unit(unsafe { libc::fchmod(fd.as_raw_fd(), mode as libc::mode_t) })
}

pub(crate) fn set_modified_time(
    fd: &impl AsRawFd,
    seconds: i64,
    nanoseconds: i64,
) -> io::Result<()> {
    if !(0..1_000_000_000).contains(&nanoseconds) {
        return Err(io::Error::from_raw_os_error(libc::EINVAL));
    }
    let times = [
        libc::timespec {
            tv_sec: 0,
            tv_nsec: libc::UTIME_OMIT,
        },
        libc::timespec {
            tv_sec: seconds,
            tv_nsec: nanoseconds,
        },
    ];

    // SAFETY: the descriptor is live and `times` is a two-element array with
    // a valid mtime nanosecond value. ATIME is explicitly left unchanged.
    syscall_unit(unsafe { libc::futimens(fd.as_raw_fd(), times.as_ptr()) })
}

pub(crate) fn read_link_at(parent: &OwnedFd, name: &CStr) -> io::Result<Vec<u8>> {
    validate_component(name)?;
    let mut capacity = 256_usize;
    loop {
        let mut bytes = vec![0_u8; capacity];
        // SAFETY: the buffer exposes its full writable length and readlinkat
        // retains no argument.
        let result = unsafe {
            libc::readlinkat(
                parent.as_raw_fd(),
                name.as_ptr(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
            )
        };
        let length = usize::try_from(result).map_err(|_| io::Error::last_os_error())?;
        if length < bytes.len() {
            bytes.truncate(length);
            return Ok(bytes);
        }
        if capacity >= 1 << 20 {
            return Err(io::Error::from_raw_os_error(libc::ENAMETOOLONG));
        }
        capacity = capacity
            .checked_mul(2)
            .ok_or_else(|| io::Error::from_raw_os_error(libc::ENAMETOOLONG))?;
    }
}

pub(crate) fn create_symlink_at(
    link_text: &CStr,
    target_parent: &OwnedFd,
    target_name: &CStr,
) -> io::Result<()> {
    validate_component(target_name)?;

    // SAFETY: all arguments remain valid for this non-retaining call.
    let result = unsafe {
        libc::symlinkat(
            link_text.as_ptr(),
            target_parent.as_raw_fd(),
            target_name.as_ptr(),
        )
    };
    syscall_unit(result)
}

pub(crate) fn rename_exclusive_at(
    source_parent: &OwnedFd,
    source_name: &CStr,
    target_parent: &OwnedFd,
    target_name: &CStr,
) -> io::Result<()> {
    validate_component(source_name)?;
    validate_component(target_name)?;

    // SAFETY: both directory descriptors and validated component names remain
    // live for this non-retaining call. RENAME_EXCL prevents replacement of an
    // object already present at the destination name.
    let result = unsafe {
        libc::renameatx_np(
            source_parent.as_raw_fd(),
            source_name.as_ptr(),
            target_parent.as_raw_fd(),
            target_name.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    syscall_unit(result)
}

pub(crate) fn remove_staged_at(parent: &OwnedFd, name: &CStr, kind: RawFileKind) -> io::Result<()> {
    validate_component(name)?;
    let flags = if kind == RawFileKind::Directory {
        libc::AT_REMOVEDIR
    } else {
        0
    };

    // SAFETY: the live parent descriptor and validated component remain valid
    // for this non-retaining call. The caller verifies the staging identity.
    syscall_unit(unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), flags) })
}

pub(crate) fn file_system_metadata(fd: &OwnedFd) -> io::Result<RawFileSystemMetadata> {
    let mut stat = MaybeUninit::<libc::statfs>::uninit();

    // SAFETY: `fd` is live and the output points to correctly sized writable storage.
    let result = unsafe { libc::fstatfs(fd.as_raw_fd(), stat.as_mut_ptr()) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: successful `fstatfs` initialized the full fixed-size structure.
    let stat = unsafe { stat.assume_init() };
    Ok(RawFileSystemMetadata {
        type_name: decode_file_system_type(&stat.f_fstypename),
        fsid: fsid_components(&stat.f_fsid),
        mount_flags: stat.f_flags,
    })
}

pub(crate) fn file_system_type(fd: &OwnedFd) -> io::Result<String> {
    let mut stat = MaybeUninit::<libc::statfs>::uninit();

    // SAFETY: `fd` remains live and `stat` points to writable storage of the
    // exact type required by `fstatfs`; the output is read only after success.
    let result = unsafe { libc::fstatfs(fd.as_raw_fd(), stat.as_mut_ptr()) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: successful `fstatfs` initialized the fixed-size structure.
    let stat = unsafe { stat.assume_init() };
    Ok(decode_file_system_type(&stat.f_fstypename))
}

fn decode_file_system_type(value: &[libc::c_char]) -> String {
    let end = value
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(value.len());
    let bytes = value[..end]
        .iter()
        .map(|byte| *byte as u8)
        .collect::<Vec<_>>();
    String::from_utf8_lossy(&bytes).into_owned()
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

    // SAFETY: `fd` is live; both pointers address writable C representations
    // of the declared sizes for the duration of this non-retaining syscall.
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

    // SAFETY: the buffer was zeroed and the successful syscall initialized all
    // returned fields. Length and returned-attribute bits gate UUID use below.
    let buffer = unsafe { buffer.assume_init() };
    Ok(decode_volume_uuid_buffer(&buffer))
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

    // SAFETY: both pointers address writable values with their exact declared
    // sizes for this non-retaining syscall while `fd` remains live.
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

    // SAFETY: the zeroed buffer is read only after syscall success; length and
    // returned bits below gate use of the capability payload.
    let buffer = unsafe { buffer.assume_init() };
    Ok(decode_clone_capability_buffer(&buffer))
}

fn decode_clone_capability_buffer(buffer: &VolumeCapabilitiesBuffer) -> Option<RawCloneCapability> {
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
    // SAFETY: `fd` is a live directory and the fixed `.` component cannot be
    // redirected; faccessat borrows both only for this call.
    let result = unsafe { libc::faccessat(fd.as_raw_fd(), c".".as_ptr(), mode, libc::AT_EACCESS) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

pub(crate) fn host_identity() -> io::Result<RawHostIdentity> {
    let mut name = MaybeUninit::<libc::utsname>::uninit();

    // SAFETY: the pointer names writable storage of the exact ABI type.
    let result = unsafe { libc::uname(name.as_mut_ptr()) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: successful `uname` initialized each fixed-size string field.
    let name = unsafe { name.assume_init() };
    Ok(RawHostIdentity {
        system_name: decode_c_char_array(&name.sysname),
        kernel_release: decode_c_char_array(&name.release),
        architecture: decode_c_char_array(&name.machine),
    })
}

pub(crate) fn product_version() -> io::Result<String> {
    let mut length = 0_usize;

    // SAFETY: the fixed name is NUL-terminated; null output performs the
    // documented size query and no pointer is retained.
    let size_result = unsafe {
        libc::sysctlbyname(
            c"kern.osproductversion".as_ptr(),
            std::ptr::null_mut(),
            &raw mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    if size_result != 0 {
        return Err(io::Error::last_os_error());
    }
    if !valid_product_version_length(length) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "kern.osproductversion returned an invalid size",
        ));
    }

    let mut value = vec![0_u8; length];
    // SAFETY: `value` owns `length` writable bytes and sysctlbyname updates the
    // length without retaining either pointer.
    let value_result = unsafe {
        libc::sysctlbyname(
            c"kern.osproductversion".as_ptr(),
            value.as_mut_ptr().cast(),
            &raw mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    if value_result != 0 {
        return Err(io::Error::last_os_error());
    }
    decode_product_version_bytes(value, length)
}

fn valid_product_version_length(length: usize) -> bool {
    (2..=4096).contains(&length)
}

fn decode_product_version_bytes(mut value: Vec<u8>, length: usize) -> io::Result<String> {
    value.truncate(length);
    if value.last() == Some(&0) {
        value.pop();
    }
    String::from_utf8(value).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

pub(crate) fn c_string(bytes: &[u8]) -> io::Result<CString> {
    CString::new(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))
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

fn owned_fd(raw_fd: libc::c_int) -> io::Result<OwnedFd> {
    if raw_fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: a successful open/openat returned a new descriptor whose
        // unique ownership is transferred exactly once.
        Ok(unsafe { OwnedFd::from_raw_fd(raw_fd) })
    }
}

fn syscall_unit(result: libc::c_int) -> io::Result<()> {
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn validate_component(name: &CStr) -> io::Result<()> {
    let bytes = name.to_bytes();
    if bytes.is_empty() || bytes == b"." || bytes == b".." || bytes.contains(&b'/') {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "dirfd operation requires one normal path component",
        ))
    } else {
        Ok(())
    }
}

fn raw_node_metadata(stat: libc::stat) -> RawNodeMetadata {
    RawNodeMetadata {
        device: u64::from(stat.st_dev.cast_unsigned()),
        inode: stat.st_ino,
        mode: u32::from(stat.st_mode & 0o7777),
        size: u64::try_from(stat.st_size.max(0)).unwrap_or(u64::MAX),
        allocated_bytes: u64::try_from(stat.st_blocks.max(0))
            .unwrap_or(u64::MAX)
            .saturating_mul(512),
        modified_seconds: stat.st_mtime,
        modified_nanoseconds: stat.st_mtime_nsec,
        kind: match stat.st_mode & libc::S_IFMT {
            libc::S_IFDIR => RawFileKind::Directory,
            libc::S_IFREG => RawFileKind::RegularFile,
            libc::S_IFLNK => RawFileKind::SymbolicLink,
            libc::S_IFIFO => RawFileKind::Fifo,
            libc::S_IFSOCK => RawFileKind::Socket,
            libc::S_IFCHR => RawFileKind::CharacterDevice,
            libc::S_IFBLK => RawFileKind::BlockDevice,
            _ => RawFileKind::Unknown,
        },
    }
}

fn fsid_components(fsid: &libc::fsid_t) -> [i32; 2] {
    const _: () = assert!(size_of::<libc::fsid_t>() == size_of::<[i32; 2]>());
    const _: () = assert!(align_of::<libc::fsid_t>() == align_of::<[i32; 2]>());

    // SAFETY: Darwin declares fsid_t as two adjacent int32_t values; the
    // compile-time size and alignment assertions bind this representation.
    unsafe { std::ptr::read((fsid as *const libc::fsid_t).cast::<[i32; 2]>()) }
}

fn decode_c_char_array<const N: usize>(value: &[libc::c_char; N]) -> String {
    // SAFETY: uname returns NUL-terminated fixed-size fields.
    unsafe { CStr::from_ptr(value.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

struct DirectoryStream(*mut libc::DIR);

impl Drop for DirectoryStream {
    fn drop(&mut self) {
        // SAFETY: this wrapper uniquely owns the non-null DIR pointer.
        let _ = unsafe { libc::closedir(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::process::{Command, Stdio};

    use super::*;

    #[test]
    fn bounded_directory_scan_stops_before_collecting_more_than_its_limit() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("one"), b"1").unwrap();
        std::fs::write(temp.path().join("two"), b"2").unwrap();
        let directory: OwnedFd = std::fs::File::open(temp.path()).unwrap().into();

        assert_eq!(
            read_directory_bounded(&directory, 1)
                .unwrap_err()
                .raw_os_error(),
            Some(libc::EOVERFLOW)
        );
        assert_eq!(read_directory_bounded(&directory, 2).unwrap().len(), 2);
        assert_eq!(read_directory(&directory).unwrap().len(), 2);
    }

    #[test]
    fn libproc_list_capacity_and_byte_count_enforce_each_boundary() {
        assert_eq!(list_capacity(8, 4, 64, MAX_VISIBLE_PIDS), 66);
        assert_eq!(list_capacity(64, 16, 32, MAX_PROCESS_FDS), 36);
        assert_eq!(list_capacity(i32::MAX, 4, 64, 100), 100);

        assert_eq!(checked_list_count(4, 4, 3, "test").unwrap(), 1);
        assert_eq!(checked_list_count(12, 4, 3, "test").unwrap(), 3);
        for invalid_bytes in [0, -1, 3, 16] {
            assert!(checked_list_count(invalid_bytes, 4, 3, "test").is_err());
        }
    }

    #[test]
    fn libproc_error_preserves_real_errno_and_explains_a_zero_errno_failure() {
        // SAFETY: Darwin's __error points to this test thread's errno slot.
        let saved_errno = unsafe { *libc::__error() };
        // SAFETY: only this thread's errno slot is changed and it is restored
        // before assertions can panic.
        unsafe { *libc::__error() = libc::EBADF };
        let real_error = libproc_error("read process fd list");
        // SAFETY: this is the same thread-local errno slot.
        unsafe { *libc::__error() = 0 };
        let zero_errno_error = libproc_error("read process fd list");
        // SAFETY: restore the caller-visible errno before checking results.
        unsafe { *libc::__error() = saved_errno };

        assert_eq!(real_error.raw_os_error(), Some(libc::EBADF));
        assert_eq!(zero_errno_error.raw_os_error(), None);
        assert_eq!(zero_errno_error.to_string(), "read process fd list");
    }

    #[test]
    fn libproc_lists_filter_invalid_entries_and_keep_truncation_evidence() {
        assert_eq!(
            complete_pid_list(vec![0, 42, -1, 23], 4, 4),
            (vec![42, 23], true)
        );
        assert_eq!(
            complete_pid_list(vec![0, 42, -1, 23], 2, 4),
            (vec![42], false)
        );

        let vnode = libc::proc_fdinfo {
            proc_fd: 7,
            proc_fdtype: libc::PROX_FDTYPE_VNODE as u32,
        };
        let other = libc::proc_fdinfo {
            proc_fd: 8,
            proc_fdtype: 0,
        };
        assert_eq!(complete_fd_list(vec![vnode, other], 2, 2), (vec![7], true));
        assert_eq!(complete_fd_list(vec![vnode, other], 1, 2), (vec![7], false));
    }

    #[test]
    fn staged_publication_never_replaces_an_existing_target() {
        use std::fs;

        let temp = tempfile::tempdir().unwrap();
        let staging_path = temp.path().join("staging");
        let target_path = temp.path().join("target");
        fs::create_dir(&staging_path).unwrap();
        fs::create_dir(&target_path).unwrap();
        fs::write(staging_path.join("staged"), b"created by this operation").unwrap();
        fs::write(target_path.join("entry"), b"foreign target").unwrap();
        let staging: OwnedFd = fs::File::open(&staging_path).unwrap().into();
        let target: OwnedFd = fs::File::open(&target_path).unwrap().into();

        let error = rename_exclusive_at(&staging, c"staged", &target, c"entry").unwrap_err();

        assert_eq!(error.raw_os_error(), Some(libc::EEXIST));
        assert_eq!(
            fs::read(target_path.join("entry")).unwrap(),
            b"foreign target"
        );
        assert_eq!(
            fs::read(staging_path.join("staged")).unwrap(),
            b"created by this operation"
        );
    }

    #[test]
    fn exclusive_full_copy_file_creation_preserves_existing_entries() {
        use std::fs;
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("existing"), b"caller bytes").unwrap();
        symlink("existing", temp.path().join("link")).unwrap();
        let parent: OwnedFd = fs::File::open(temp.path()).unwrap().into();

        let existing = create_file_at(&parent, c"existing", 0o600).unwrap_err();
        let link = create_file_at(&parent, c"link", 0o600).unwrap_err();
        assert_eq!(existing.raw_os_error(), Some(libc::EEXIST));
        assert_eq!(link.raw_os_error(), Some(libc::EEXIST));
        assert_eq!(
            fs::read(temp.path().join("existing")).unwrap(),
            b"caller bytes"
        );
        assert_eq!(
            fs::read_link(temp.path().join("link")).unwrap(),
            std::path::Path::new("existing")
        );

        let created = create_file_at(&parent, c"created", 0o600).unwrap();
        // SAFETY: `created` is a live descriptor and F_GETFL takes no variadic argument.
        let flags = unsafe { libc::fcntl(created.as_raw_fd(), libc::F_GETFL) };
        // SAFETY: `created` is a live descriptor and F_GETFD takes no variadic argument.
        let descriptor_flags = unsafe { libc::fcntl(created.as_raw_fd(), libc::F_GETFD) };
        assert_eq!(flags & libc::O_ACCMODE, libc::O_WRONLY);
        assert_ne!(descriptor_flags & libc::FD_CLOEXEC, 0);
        assert_eq!(
            node_metadata(&created).unwrap().kind,
            RawFileKind::RegularFile
        );
    }

    #[test]
    fn volume_uuid_requires_both_returned_bit_and_complete_buffer() {
        let complete = u32::try_from(size_of::<VolumeUuidBuffer>()).unwrap();
        let value = [1_u8; 16];
        let mut buffer = VolumeUuidBuffer {
            length: complete,
            returned: libc::attribute_set_t {
                commonattr: 0,
                volattr: libc::ATTR_VOL_UUID,
                dirattr: 0,
                fileattr: 0,
                forkattr: 0,
            },
            uuid: value,
        };
        assert_eq!(decode_volume_uuid_buffer(&buffer), Some(value));

        buffer.returned.volattr = 0;
        assert_eq!(decode_volume_uuid_buffer(&buffer), None);
        buffer.returned.volattr = libc::ATTR_VOL_UUID;
        buffer.length = complete - 1;
        assert_eq!(decode_volume_uuid_buffer(&buffer), None);
    }

    #[test]
    fn clone_capability_requires_complete_payload_and_returned_bit() {
        let complete = u32::try_from(size_of::<VolumeCapabilitiesBuffer>()).unwrap();
        let mut buffer = VolumeCapabilitiesBuffer {
            length: complete,
            returned: libc::attribute_set_t {
                commonattr: 0,
                volattr: libc::ATTR_VOL_CAPABILITIES,
                dirattr: 0,
                fileattr: 0,
                forkattr: 0,
            },
            capabilities: libc::vol_capabilities_attr_t {
                capabilities: [0; 4],
                valid: [0; 4],
            },
        };
        buffer.capabilities.capabilities[libc::VOL_CAPABILITIES_INTERFACES] =
            libc::VOL_CAP_INT_CLONE;
        buffer.capabilities.valid[libc::VOL_CAPABILITIES_INTERFACES] = libc::VOL_CAP_INT_CLONE;
        assert_eq!(
            decode_clone_capability_buffer(&buffer),
            Some(RawCloneCapability {
                interface_capabilities: libc::VOL_CAP_INT_CLONE,
                interface_valid: libc::VOL_CAP_INT_CLONE,
            })
        );
        buffer.length = complete - 1;
        assert_eq!(decode_clone_capability_buffer(&buffer), None);
        buffer.length = complete;
        buffer.returned.volattr = 0;
        assert_eq!(decode_clone_capability_buffer(&buffer), None);
    }

    #[test]
    fn product_version_length_and_terminator_are_validated_independently() {
        assert!(!valid_product_version_length(1));
        assert!(valid_product_version_length(2));
        assert!(valid_product_version_length(4096));
        assert!(!valid_product_version_length(4097));
        assert_eq!(
            decode_product_version_bytes(b"15.7.2\0".to_vec(), 7).unwrap(),
            "15.7.2"
        );
        assert_eq!(
            decode_product_version_bytes(b"15.7.2".to_vec(), 6).unwrap(),
            "15.7.2"
        );
        assert!(decode_product_version_bytes(vec![0xff, 0], 2).is_err());
    }

    #[test]
    fn filesystem_type_decode_is_bounded_when_the_fixed_buffer_has_no_nul() {
        let full = [b'a' as libc::c_char; 16];
        assert_eq!(decode_file_system_type(&full), "aaaaaaaaaaaaaaaa");

        let terminated = [
            b'a' as libc::c_char,
            b'p' as libc::c_char,
            b'f' as libc::c_char,
            b's' as libc::c_char,
            0,
            b'x' as libc::c_char,
        ];
        assert_eq!(decode_file_system_type(&terminated), "apfs");
    }

    #[test]
    fn dirfd_components_reject_every_non_normal_name() {
        for invalid in [c"", c".", c"..", c"a/b"] {
            assert_eq!(
                validate_component(invalid).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
        validate_component(c"ordinary").unwrap();
    }

    #[test]
    fn kernel_path_decode_requires_a_terminated_absolute_path() {
        assert_eq!(
            decode_kernel_path(b"/private/tmp\0tail")
                .unwrap()
                .as_bytes(),
            b"/private/tmp"
        );
        for invalid in [b"\0".as_slice(), b"relative\0", b"/unterminated"] {
            assert!(decode_kernel_path(invalid).is_err());
        }

        struct InvalidFd;
        impl AsRawFd for InvalidFd {
            fn as_raw_fd(&self) -> i32 {
                -1
            }
        }
        assert_eq!(
            fd_kernel_path(&InvalidFd).unwrap_err().raw_os_error(),
            Some(libc::EBADF)
        );
    }

    #[test]
    fn owned_fd_accepts_descriptor_zero() {
        const CHILD: &str = "THINWS_TEST_OWNED_FD_ZERO_CHILD";
        if std::env::var_os(CHILD).is_some() {
            let owned = owned_fd(0).expect("stdin is explicitly installed in this child");
            assert_eq!(owned.as_raw_fd(), 0);
            return;
        }
        let output = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("ffi::tests::owned_fd_accepts_descriptor_zero")
            .env(CHILD, "1")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    }

    #[test]
    fn kernel_open_helpers_do_not_follow_links_and_keep_required_flags() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("directory")).unwrap();
        fs::write(temp.path().join("file"), b"content").unwrap();
        symlink("directory", temp.path().join("directory-link")).unwrap();
        symlink("file", temp.path().join("file-link")).unwrap();
        let parent: OwnedFd = fs::File::open(temp.path()).unwrap().into();

        let root = open_root_directory().unwrap();
        let directory = open_directory_at(&parent, c"directory").unwrap();
        let file = open_file_read_at(&parent, c"file").unwrap();
        assert!(open_directory_at(&parent, c"directory-link").is_err());
        assert!(open_file_read_at(&parent, c"file-link").is_err());

        for fd in [&root, &directory, &file] {
            // SAFETY: both fcntl commands only query a live descriptor.
            let descriptor_flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) };
            assert_ne!(descriptor_flags & libc::FD_CLOEXEC, 0);
        }
        // SAFETY: F_GETFL only queries live descriptors.
        let root_flags = unsafe { libc::fcntl(root.as_raw_fd(), libc::F_GETFL) };
        // SAFETY: F_GETFL only queries live descriptors.
        let directory_flags = unsafe { libc::fcntl(directory.as_raw_fd(), libc::F_GETFL) };
        // SAFETY: F_GETFL only queries live descriptors.
        let file_flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
        assert_ne!(root_flags & libc::O_SEARCH, 0);
        assert_ne!(directory_flags & libc::O_SEARCH, 0);
        assert_ne!(file_flags & libc::O_NONBLOCK, 0);
        assert_eq!(file_flags & libc::O_ACCMODE, libc::O_RDONLY);
    }

    #[test]
    fn readlink_grows_beyond_the_initial_buffer_without_truncation() {
        let temp = tempfile::tempdir().unwrap();
        let text = "x".repeat(300);
        symlink(&text, temp.path().join("long-link")).unwrap();
        let parent: OwnedFd = fs::File::open(temp.path()).unwrap().into();
        assert_eq!(
            read_link_at(&parent, c"long-link").unwrap(),
            text.as_bytes()
        );
    }

    #[test]
    fn live_host_and_apfs_volume_facts_are_not_placeholder_values() {
        let temp = tempfile::tempdir().unwrap();
        let directory: OwnedFd = fs::File::open(temp.path()).unwrap().into();
        let uuid = volume_uuid(&directory)
            .unwrap()
            .expect("APFS must return a UUID");
        assert_ne!(uuid, [1; 16]);
        assert_ne!(uuid, [0; 16]);
        let host = host_identity().unwrap();
        assert_eq!(host.system_name, "Darwin");
        assert!(!host.kernel_release.is_empty());
        assert!(!host.architecture.is_empty());
        let version = product_version().unwrap();
        assert!(version.starts_with(|byte: char| byte.is_ascii_digit()));
        assert!(version.contains('.'));
    }

    #[test]
    fn effective_access_requires_both_write_and_search() {
        let temp = tempfile::tempdir().unwrap();
        let directory_path = temp.path().join("read-only-directory");
        fs::create_dir(&directory_path).unwrap();
        fs::set_permissions(&directory_path, fs::Permissions::from_mode(0o500)).unwrap();
        let directory: OwnedFd = fs::File::open(&directory_path).unwrap().into();
        effective_read_search_access(&directory).unwrap();
        assert_eq!(
            effective_write_search_access(&directory)
                .unwrap_err()
                .raw_os_error(),
            Some(libc::EACCES)
        );
    }

    #[test]
    fn metadata_decoders_keep_nondefault_fsid_and_special_file_kinds() {
        let mut fsid = MaybeUninit::<libc::fsid_t>::zeroed();
        // SAFETY: fsid_t has the asserted representation of two adjacent i32 words.
        let fsid = unsafe {
            fsid.as_mut_ptr().cast::<[i32; 2]>().write([123, -456]);
            fsid.assume_init()
        };
        assert_eq!(fsid_components(&fsid), [123, -456]);
        assert_eq!(decode_c_char_array(&[b'A' as libc::c_char, 0, 0]), "A");

        let temp = tempfile::tempdir().unwrap();
        let file = fs::File::create(temp.path().join("metadata-file")).unwrap();
        let mut stat = MaybeUninit::<libc::stat>::uninit();
        // SAFETY: fstat fully initializes the output on success and file owns a live FD.
        let mut stat = unsafe {
            assert_eq!(libc::fstat(file.as_raw_fd(), stat.as_mut_ptr()), 0);
            stat.assume_init()
        };
        for (mode, expected) in [
            (libc::S_IFIFO, RawFileKind::Fifo),
            (libc::S_IFSOCK, RawFileKind::Socket),
            (libc::S_IFCHR, RawFileKind::CharacterDevice),
            (libc::S_IFBLK, RawFileKind::BlockDevice),
        ] {
            stat.st_mode = (stat.st_mode & !libc::S_IFMT) | mode;
            assert_eq!(raw_node_metadata(stat).kind, expected);
        }
    }

    #[test]
    fn directory_stream_drop_closes_its_owned_descriptor() {
        let temp = tempfile::tempdir().unwrap();
        let directory = fs::File::open(temp.path()).unwrap();
        // SAFETY: dup returns a separate owned descriptor or -1, checked below.
        let raw = unsafe { libc::dup(directory.as_raw_fd()) };
        assert!(raw >= 0);
        // SAFETY: fdopendir takes ownership of raw on success; the wrapper closes it on drop.
        let stream = unsafe { libc::fdopendir(raw) };
        assert!(!stream.is_null());
        drop(DirectoryStream(stream));
        // SAFETY: F_GETFD reports EBADF for the descriptor closed by DirectoryStream.
        assert_eq!(unsafe { libc::fcntl(raw, libc::F_GETFD) }, -1);
        assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::EBADF));
    }
}
