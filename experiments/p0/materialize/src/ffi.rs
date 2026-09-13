use std::ffi::{CStr, OsString};
use std::io;
use std::mem::MaybeUninit;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStringExt;

const CLONE_NOFOLLOW_ANY: u32 = 0x0008;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NodeKind {
    Directory,
    RegularFile,
    SymbolicLink,
    Special,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NodeMetadata {
    pub(crate) device: u64,
    pub(crate) inode: u64,
    pub(crate) kind: NodeKind,
    pub(crate) mode: u32,
    pub(crate) size: u64,
    pub(crate) modified_seconds: i64,
    pub(crate) modified_nanoseconds: i64,
}

pub(crate) fn open_root_directory() -> io::Result<OwnedFd> {
    let flags = libc::O_SEARCH | libc::O_NOFOLLOW | libc::O_CLOEXEC;

    // SAFETY: the static C string is valid for the call and the returned
    // descriptor is checked before ownership is constructed.
    let raw_fd = unsafe { libc::open(c"/".as_ptr(), flags) };
    owned_fd(raw_fd)
}

pub(crate) fn open_directory_at(parent: &OwnedFd, name: &CStr) -> io::Result<OwnedFd> {
    validate_component(name)?;
    // Darwin defines O_SEARCH as O_EXEC | O_DIRECTORY, so spelling the
    // directory bit again would be redundant for this macOS-only adapter.
    let flags = libc::O_SEARCH | libc::O_NOFOLLOW | libc::O_CLOEXEC;

    // SAFETY: `parent` remains open, `name` is a validated NUL-terminated
    // component, and openat does not retain either argument.
    let raw_fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    owned_fd(raw_fd)
}

pub(crate) fn open_file_read_at(parent: &OwnedFd, name: &CStr) -> io::Result<OwnedFd> {
    validate_component(name)?;
    let flags = libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC;

    // SAFETY: the live directory descriptor and validated component remain
    // valid for the duration of the call.
    let raw_fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    owned_fd(raw_fd)
}

pub(crate) fn open_file_write_at(parent: &OwnedFd, name: &CStr) -> io::Result<OwnedFd> {
    validate_component(name)?;
    let flags = libc::O_WRONLY | libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC;

    // SAFETY: the live directory descriptor and validated component remain
    // valid for the duration of the call. No entry is created or truncated.
    let raw_fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
    owned_fd(raw_fd)
}

pub(crate) fn create_file_at(parent: &OwnedFd, name: &CStr, mode: u32) -> io::Result<OwnedFd> {
    validate_component(name)?;
    let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;

    // SAFETY: the live directory descriptor and validated component remain
    // valid for the call. O_EXCL gives this invocation exclusive creation.
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

pub(crate) fn metadata(fd: &impl AsRawFd) -> io::Result<NodeMetadata> {
    let mut stat = MaybeUninit::<libc::stat>::uninit();

    // SAFETY: `fd` is live and the output points to writable storage of the
    // exact type expected by fstat.
    let result = unsafe { libc::fstat(fd.as_raw_fd(), stat.as_mut_ptr()) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: successful fstat initialized the structure.
    Ok(metadata_from_stat(unsafe { stat.assume_init() }))
}

pub(crate) fn metadata_at(parent: &OwnedFd, name: &CStr) -> io::Result<NodeMetadata> {
    validate_component(name)?;
    let mut stat = MaybeUninit::<libc::stat>::uninit();

    // SAFETY: arguments remain live for the call. AT_SYMLINK_NOFOLLOW makes
    // this describe the entry itself instead of a link target.
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

    // SAFETY: successful fstatat initialized the structure.
    Ok(metadata_from_stat(unsafe { stat.assume_init() }))
}

pub(crate) fn read_directory(directory: &OwnedFd) -> io::Result<Vec<OsString>> {
    // On Darwin O_RDONLY is zero, and the fixed `.` names the held directory
    // itself rather than a replaceable symlink entry. O_CLOEXEC remains
    // explicit while this fresh read-only open enforces readability. A
    // parameterized name or cross-platform port must re-evaluate these flags.
    let flags = libc::O_CLOEXEC;
    // SAFETY: `directory` remains live and the static `.` component is valid.
    // A fresh open file description is required because dup would share its
    // directory offset and make later enumerations observe EOF.
    let readable_raw = unsafe { libc::openat(directory.as_raw_fd(), c".".as_ptr(), flags) };
    let readable = owned_fd(readable_raw)?;
    let readable_raw = readable.as_raw_fd();

    // SAFETY: fdopendir takes ownership of a valid directory descriptor on
    // success. `forget` below transfers that ownership exactly once.
    let stream = unsafe { libc::fdopendir(readable_raw) };
    if stream.is_null() {
        return Err(io::Error::last_os_error());
    }
    std::mem::forget(readable);
    let stream = DirectoryStream(stream);

    let mut names = Vec::new();
    loop {
        // SAFETY: Darwin exposes the calling thread's errno slot through
        // __error. It is cleared immediately before readdir so EOF and error
        // can be distinguished without another errno-mutating call.
        unsafe { *libc::__error() = 0 };
        // SAFETY: `stream` owns a live DIR pointer and readdir borrows it only
        // until the next call.
        let entry = unsafe { libc::readdir(stream.0) };
        if entry.is_null() {
            // SAFETY: errno is read immediately after the failed/EOF readdir.
            let errno = unsafe { *libc::__error() };
            if errno == 0 {
                break;
            }
            return Err(io::Error::from_raw_os_error(errno));
        }

        // SAFETY: a non-null dirent returned by readdir remains valid until
        // the next readdir call. Darwin guarantees d_namlen name bytes in the
        // variable-length record. `raw const` avoids creating a reference to
        // the nominal full d_name array beyond that record allocation.
        let bytes = unsafe {
            let length = usize::from((*entry).d_namlen);
            let name = (&raw const (*entry).d_name).cast::<u8>();
            std::slice::from_raw_parts(name, length)
        };
        if bytes != b"." && bytes != b".." {
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

    // SAFETY: both descriptors are live, target_name is one validated
    // NUL-terminated component, and fclonefileat does not retain arguments.
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

    // SAFETY: parent and component stay valid, and mkdirat does not retain
    // them. Existing entries are never overwritten.
    let result = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), mode as libc::mode_t) };
    syscall_unit(result)
}

pub(crate) fn set_mode(fd: &impl AsRawFd, mode: u32) -> io::Result<()> {
    // SAFETY: fd is live and fchmod only changes that referenced object.
    let result = unsafe { libc::fchmod(fd.as_raw_fd(), mode as libc::mode_t) };
    syscall_unit(result)
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

    // SAFETY: fd is live, `times` is a two-element array with a valid mtime
    // nanosecond value, and futimens borrows both only for the duration of the
    // call. UTIME_OMIT tells the kernel not to alter atime.
    let result = unsafe { libc::futimens(fd.as_raw_fd(), times.as_ptr()) };
    syscall_unit(result)
}

pub(crate) fn read_link_at(parent: &OwnedFd, name: &CStr) -> io::Result<Vec<u8>> {
    validate_component(name)?;
    let mut capacity = 256usize;
    loop {
        let mut bytes = vec![0u8; capacity];
        // SAFETY: parent and name remain valid, and bytes exposes capacity
        // writable bytes for readlinkat. The call does not NUL-terminate.
        let result = unsafe {
            libc::readlinkat(
                parent.as_raw_fd(),
                name.as_ptr(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
            )
        };
        // This integer conversion is pure and cannot alter errno, so a
        // negative result retains readlinkat's original error. Zero remains a
        // successful empty link target.
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

    // SAFETY: both C strings are valid for the call, the parent descriptor is
    // live, and symlinkat does not retain any argument.
    let result = unsafe {
        libc::symlinkat(
            link_text.as_ptr(),
            target_parent.as_raw_fd(),
            target_name.as_ptr(),
        )
    };
    syscall_unit(result)
}

pub(crate) fn remove_at(parent: &OwnedFd, name: &CStr, kind: NodeKind) -> io::Result<()> {
    validate_component(name)?;
    let flags = if kind == NodeKind::Directory {
        libc::AT_REMOVEDIR
    } else {
        0
    };

    // SAFETY: parent and name remain valid, and flags are restricted to the
    // entry kind already revalidated by the safe caller.
    let result = unsafe { libc::unlinkat(parent.as_raw_fd(), name.as_ptr(), flags) };
    syscall_unit(result)
}

fn validate_component(name: &CStr) -> io::Result<()> {
    let bytes = name.to_bytes();
    if bytes.is_empty() || bytes == b"." || bytes == b".." || bytes.contains(&b'/') {
        return Err(io::Error::from_raw_os_error(libc::EINVAL));
    }
    Ok(())
}

fn owned_fd(raw_fd: RawFd) -> io::Result<OwnedFd> {
    if raw_fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        // SAFETY: a nonnegative descriptor returned by an ownership-producing
        // syscall is transferred exactly once into OwnedFd.
        Ok(unsafe { OwnedFd::from_raw_fd(raw_fd) })
    }
}

fn syscall_unit(result: libc::c_int) -> io::Result<()> {
    if result == 0 {
        Ok(())
    } else {
        // errno is captured immediately after the failed syscall.
        Err(io::Error::last_os_error())
    }
}

fn metadata_from_stat(stat: libc::stat) -> NodeMetadata {
    let kind = match stat.st_mode & libc::S_IFMT {
        libc::S_IFDIR => NodeKind::Directory,
        libc::S_IFREG => NodeKind::RegularFile,
        libc::S_IFLNK => NodeKind::SymbolicLink,
        _ => NodeKind::Special,
    };
    NodeMetadata {
        device: stat.st_dev as u64,
        inode: stat.st_ino,
        kind,
        mode: u32::from(stat.st_mode & 0o7777),
        size: stat.st_size.max(0) as u64,
        modified_seconds: stat.st_mtime,
        modified_nanoseconds: stat.st_mtime_nsec,
    }
}

struct DirectoryStream(*mut libc::DIR);

impl Drop for DirectoryStream {
    fn drop(&mut self) {
        // SAFETY: this wrapper owns the non-null DIR pointer exactly once.
        let _ = unsafe { libc::closedir(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::{CString, OsString};
    use std::fs;
    use std::os::fd::{AsRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};

    use rustix::fs::{Mode, OFlags, Timespec, Timestamps};
    use rustix::io::FdFlags;

    use crate::test_support::ControlledTree;

    use super::{
        DirectoryStream, NodeKind, clone_file_at, create_file_at, metadata, open_directory_at,
        open_file_read_at, open_file_write_at, open_root_directory, owned_fd, read_directory,
        read_link_at, remove_at, set_modified_time, validate_component,
    };

    fn fixture() -> ControlledTree {
        ControlledTree::create_in(Path::new("/private/tmp"), "ffi")
            .expect("create controlled FFI fixture")
    }

    #[test]
    fn production_open_helpers_set_required_status_and_descriptor_flags() {
        let mut fixture = fixture();
        fixture
            .create_directory(Path::new("nested"), 0o700)
            .expect("create nested fixture directory");
        fixture
            .create_file(Path::new("existing"), b"existing bytes", 0o600)
            .expect("create existing fixture file");

        let root = open_root_directory().expect("open root with production flags");
        let production_root = open_with_production_search_flags(&fixture);
        let directory =
            open_directory_at(&production_root, c"nested").expect("open nested directory");
        let readable =
            open_file_read_at(&production_root, c"existing").expect("open readable file");
        let writable =
            open_file_write_at(&production_root, c"existing").expect("open writable file");
        let created =
            create_file_at(&production_root, c"created", 0o600).expect("create exclusive file");
        fixture
            .track_existing(Path::new("created"))
            .expect("register production-created file from independent identity");

        let root_status = rustix::fs::fcntl_getfl(&root).expect("F_GETFL root");
        let directory_status = rustix::fs::fcntl_getfl(&directory).expect("F_GETFL directory");
        let readable_status = rustix::fs::fcntl_getfl(&readable).expect("F_GETFL readable file");
        let writable_status = rustix::fs::fcntl_getfl(&writable).expect("F_GETFL writable file");
        let created_status = rustix::fs::fcntl_getfl(&created).expect("F_GETFL created file");
        let descriptor_flags = [
            rustix::io::fcntl_getfd(&root).expect("F_GETFD root"),
            rustix::io::fcntl_getfd(&directory).expect("F_GETFD directory"),
            rustix::io::fcntl_getfd(&readable).expect("F_GETFD readable file"),
            rustix::io::fcntl_getfd(&writable).expect("F_GETFD writable file"),
            rustix::io::fcntl_getfd(&created).expect("F_GETFD created file"),
        ];

        drop((
            created,
            writable,
            readable,
            directory,
            production_root,
            root,
        ));
        fixture.cleanup().expect("remove verified FFI fixture");

        let search_access_bit = libc::O_EXEC as u32;
        assert_eq!(root_status.bits() & search_access_bit, search_access_bit);
        assert_eq!(
            directory_status.bits() & search_access_bit,
            search_access_bit
        );
        assert_eq!(readable_status & OFlags::ACCMODE, OFlags::RDONLY);
        assert_eq!(writable_status & OFlags::ACCMODE, OFlags::WRONLY);
        assert_eq!(created_status & OFlags::ACCMODE, OFlags::WRONLY);
        assert!(readable_status.contains(OFlags::NONBLOCK));
        assert!(writable_status.contains(OFlags::NONBLOCK));
        assert!(
            descriptor_flags
                .iter()
                .all(|flags| flags.contains(FdFlags::CLOEXEC))
        );
    }

    #[test]
    fn production_open_helpers_reject_symlink_entries() {
        let mut fixture = fixture();
        fixture
            .create_directory(Path::new("real-directory"), 0o700)
            .expect("create real directory");
        fixture
            .create_file(Path::new("real-file"), b"original", 0o600)
            .expect("create real file");
        fixture
            .create_symlink(Path::new("directory-link"), b"real-directory")
            .expect("create directory symlink");
        fixture
            .create_symlink(Path::new("file-link"), b"real-file")
            .expect("create file symlink");
        let production_root = open_with_production_search_flags(&fixture);

        let directory_rejected = open_directory_at(&production_root, c"directory-link").is_err();
        let read_rejected = open_file_read_at(&production_root, c"file-link").is_err();
        let write_rejected = open_file_write_at(&production_root, c"file-link").is_err();
        let create_rejected = create_file_at(&production_root, c"file-link", 0o600).is_err();
        let original = fs::read(fixture.root_path().join("real-file"))
            .expect("read original file after rejected opens");

        drop(production_root);
        fixture.cleanup().expect("remove verified FFI fixture");
        assert!(directory_rejected);
        assert!(read_rejected);
        assert!(write_rejected);
        assert!(create_rejected);
        assert_eq!(original, b"original");
    }

    #[test]
    fn exclusive_file_creation_does_not_overwrite_existing_content() {
        let mut fixture = fixture();
        fixture
            .create_file(Path::new("existing"), b"preserve me", 0o600)
            .expect("create existing fixture file");
        let production_root = open_with_production_search_flags(&fixture);

        let error = create_file_at(&production_root, c"existing", 0o600)
            .expect_err("exclusive creation must reject an existing entry");
        let contents =
            fs::read(fixture.root_path().join("existing")).expect("read preserved contents");

        drop(production_root);
        fixture.cleanup().expect("remove verified FFI fixture");
        assert_eq!(error.raw_os_error(), Some(libc::EEXIST));
        assert_eq!(contents, b"preserve me");
    }

    #[test]
    fn futimens_wrapper_sets_exact_mtime_omits_atime_and_preserves_einval() {
        let mut fixture = fixture();
        fixture
            .create_file(Path::new("timestamps"), b"payload", 0o600)
            .expect("create timestamp fixture");
        let production_root = open_with_production_search_flags(&fixture);
        let file = open_file_read_at(&production_root, c"timestamps")
            .expect("open timestamp fixture with production flags");
        rustix::fs::futimens(
            &file,
            &Timestamps {
                last_access: Timespec {
                    tv_sec: 1_640_000_001,
                    tv_nsec: 123_456_789,
                },
                last_modification: Timespec {
                    tv_sec: 1_640_000_002,
                    tv_nsec: 234_567_890,
                },
            },
        )
        .expect("set independent initial timestamps");
        let before = rustix::fs::fstat(&file).expect("observe timestamps before production call");

        set_modified_time(&file, 1_650_000_003, 345_678_901)
            .expect("production futimens wrapper succeeds");
        let after = rustix::fs::fstat(&file).expect("observe timestamps after production call");
        set_modified_time(&file, 1_660_000_004, 0).expect("zero nanoseconds is a valid endpoint");
        let after_zero = rustix::fs::fstat(&file).expect("observe zero-nanosecond endpoint");
        set_modified_time(&file, 1_670_000_005, 999_999_999)
            .expect("maximum nanoseconds is a valid endpoint");
        let after_maximum = rustix::fs::fstat(&file).expect("observe maximum endpoint");
        let invalid_negative = set_modified_time(&file, 1_680_000_006, -1)
            .expect_err("negative nanoseconds must not invoke Darwin sentinel semantics");
        let after_negative =
            rustix::fs::fstat(&file).expect("observe timestamps after negative rejection");
        let invalid_upper = set_modified_time(&file, 1_690_000_007, 1_000_000_000)
            .expect_err("upper-exclusive nanoseconds must preserve EINVAL");
        let after_upper =
            rustix::fs::fstat(&file).expect("observe timestamps after upper rejection");

        drop((file, production_root));
        fixture.cleanup().expect("remove verified FFI fixture");
        assert_eq!(after.st_atime, before.st_atime);
        assert_eq!(after.st_atime_nsec, before.st_atime_nsec);
        assert_eq!(after.st_mtime, 1_650_000_003);
        assert_eq!(after.st_mtime_nsec, 345_678_901);
        assert_eq!(after_zero.st_atime, before.st_atime);
        assert_eq!(after_zero.st_atime_nsec, before.st_atime_nsec);
        assert_eq!(after_zero.st_mtime, 1_660_000_004);
        assert_eq!(after_zero.st_mtime_nsec, 0);
        assert_eq!(after_maximum.st_atime, before.st_atime);
        assert_eq!(after_maximum.st_atime_nsec, before.st_atime_nsec);
        assert_eq!(after_maximum.st_mtime, 1_670_000_005);
        assert_eq!(after_maximum.st_mtime_nsec, 999_999_999);
        assert_eq!(invalid_negative.raw_os_error(), Some(libc::EINVAL));
        assert_eq!(invalid_upper.raw_os_error(), Some(libc::EINVAL));
        assert_eq!(after_negative.st_mtime, after_maximum.st_mtime);
        assert_eq!(after_negative.st_mtime_nsec, after_maximum.st_mtime_nsec);
        assert_eq!(after_upper.st_mtime, after_maximum.st_mtime);
        assert_eq!(after_upper.st_mtime_nsec, after_maximum.st_mtime_nsec);
    }

    #[test]
    fn component_validation_rejects_unsafe_names_and_accepts_normal_names() {
        for invalid in [c"", c".", c"..", c"nested/name"] {
            assert_eq!(
                validate_component(invalid)
                    .expect_err("unsafe component must be rejected")
                    .raw_os_error(),
                Some(libc::EINVAL)
            );
        }
        validate_component(c"valid").expect("normal component must be accepted");
        validate_component(c".git").expect("ordinary structurally valid name is accepted");
    }

    #[test]
    fn missing_symlink_preserves_enoent() {
        let mut fixture = fixture();
        fixture
            .create_file(Path::new("regular-parent"), b"not a directory", 0o600)
            .expect("create regular-file parent fixture");
        let production_root = open_with_production_search_flags(&fixture);
        let missing_error = read_link_at(&production_root, c"absent")
            .expect_err("missing symlink must report an error");
        let regular_parent = open_file_read_at(&production_root, c"regular-parent")
            .expect("open regular-file parent fixture");
        let wrong_parent_error = read_link_at(&regular_parent, c"child")
            .expect_err("regular-file parent must report an error");

        drop((regular_parent, production_root));
        fixture.cleanup().expect("remove verified FFI fixture");
        assert_eq!(missing_error.raw_os_error(), Some(libc::ENOENT));
        assert_eq!(wrong_parent_error.raw_os_error(), Some(libc::ENOTDIR));
    }

    #[test]
    fn owned_fd_accepts_valid_fd_zero_only_in_isolated_child() {
        let executable = std::env::current_exe().expect("locate current test executable");
        let status = Command::new(executable)
            .arg("--exact")
            .arg("ffi::tests::owned_fd_zero_child")
            .arg("--nocapture")
            .env("THINWS_P0_OWNED_FD_ZERO_CHILD", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("spawn isolated fd-zero child test");
        assert!(status.success(), "fd-zero child test must succeed");
    }

    #[test]
    fn owned_fd_zero_child() {
        if std::env::var_os("THINWS_P0_OWNED_FD_ZERO_CHILD").is_none() {
            return;
        }
        let mut fixture = fixture();
        fixture
            .create_file(Path::new("fd-zero"), b"input", 0o600)
            .expect("create fd-zero fixture file");
        let replacement = rustix::fs::openat(
            fixture.root_fd(),
            c"fd-zero",
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .expect("open fd-zero replacement before transferring descriptor zero");

        // SAFETY: this is an isolated child process. `replacement` stays live,
        // dup2 atomically replaces only the child's descriptor zero, and no
        // Rust owner exists for the new descriptor until `owned_fd` below.
        let duplicated = unsafe { libc::dup2(replacement.as_raw_fd(), 0) };
        assert_eq!(duplicated, 0, "dup2 must install the child-only fd zero");
        let wrapped = owned_fd(duplicated);
        let observed = match wrapped {
            Ok(fd) => {
                let raw = fd.as_raw_fd();
                drop(fd);
                Ok(raw)
            }
            Err(error) => Err(error),
        };

        drop(replacement);
        fixture.cleanup().expect("remove verified FFI fixture");
        assert_eq!(observed.expect("valid fd zero must be accepted"), 0);
    }

    #[test]
    fn directory_stream_drop_closes_owned_fd_in_isolated_child() {
        let executable = std::env::current_exe().expect("locate current test executable");
        let status = Command::new(executable)
            .arg("--exact")
            .arg("ffi::tests::directory_stream_drop_child")
            .arg("--nocapture")
            .env("THINWS_P0_DIRECTORY_STREAM_DROP_CHILD", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("spawn isolated directory-stream child test");
        assert!(status.success(), "directory-stream child test must succeed");
    }

    #[test]
    fn directory_stream_drop_child() {
        if std::env::var_os("THINWS_P0_DIRECTORY_STREAM_DROP_CHILD").is_none() {
            return;
        }
        let fixture = fixture();
        let readable =
            rustix::fs::openat(fixture.root_fd(), c".", directory_flags(), Mode::empty())
                .expect("open independently owned directory descriptor");
        rustix::io::fcntl_getfd(&readable).expect("F_GETFD live directory descriptor");
        let raw_fd = readable.as_raw_fd();

        // SAFETY: `readable` is a live directory descriptor. On success,
        // fdopendir takes ownership and the OwnedFd is forgotten exactly once.
        let stream = unsafe { libc::fdopendir(raw_fd) };
        assert!(
            !stream.is_null(),
            "fdopendir must accept readable directory"
        );
        std::mem::forget(readable);
        drop(DirectoryStream(stream));

        // SAFETY: fcntl accepts a raw descriptor number and reports EBADF for
        // the just-closed descriptor. This isolated child performs no FD-
        // allocating operation between DirectoryStream::drop and this check.
        let getfd_result = unsafe { libc::fcntl(raw_fd, libc::F_GETFD) };
        let getfd_error = std::io::Error::last_os_error().raw_os_error();

        fixture.cleanup().expect("remove verified FFI fixture");
        assert_eq!(getfd_result, -1);
        assert_eq!(getfd_error, Some(libc::EBADF));
    }

    #[test]
    fn held_search_fd_repeated_scans_are_complete_and_see_new_entries() {
        let mut fixture = fixture();
        fixture
            .create_file(Path::new("short"), b"one", 0o600)
            .expect("create short fixture file");
        for index in 0..48 {
            let name = format!("long-{index:02}-{}", "x".repeat(180));
            fixture
                .create_file(Path::new(&name), b"payload", 0o600)
                .expect("create long fixture file");
        }
        let held = open_with_production_search_flags(&fixture);
        let mut expected: Vec<_> = fixture
            .tracked()
            .keys()
            .filter_map(|path| path.file_name().map(OsString::from))
            .collect();
        expected.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));

        for _ in 0..2 {
            let mut observed = read_directory(&held).expect("scan held O_SEARCH descriptor");
            observed.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
            assert_eq!(observed, expected);
        }

        fixture
            .create_file(Path::new("visible-after-two-scans"), b"new", 0o600)
            .expect("create file visible to later scan");
        let mut observed = read_directory(&held).expect("scan fresh open-file-description");
        observed.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        assert!(
            observed
                .iter()
                .any(|name| name.as_bytes() == b"visible-after-two-scans")
        );
        assert_eq!(observed.len(), expected.len() + 1);
        fixture.cleanup().expect("remove verified FFI fixture");
    }

    #[test]
    fn read_directory_preserves_enotdir_for_a_regular_fd() {
        let mut fixture = fixture();
        fixture
            .create_file(Path::new("regular"), b"not a directory", 0o600)
            .expect("create regular fixture file");
        let production_root = open_with_production_search_flags(&fixture);
        let regular =
            open_file_read_at(&production_root, c"regular").expect("open regular fixture file");
        let error = read_directory(&regular).expect_err("regular fd cannot be enumerated");

        drop((regular, production_root));
        fixture.cleanup().expect("remove verified FFI fixture");
        assert_eq!(error.raw_os_error(), Some(libc::ENOTDIR));
    }

    #[test]
    fn unreadable_directory_is_an_error_not_an_empty_scan() {
        let mut fixture = fixture();
        fixture
            .create_directory(Path::new("locked"), 0o700)
            .expect("create locked fixture directory");
        fixture
            .create_file(Path::new("locked/present"), b"data", 0o600)
            .expect("create locked fixture child");
        let locked = rustix::fs::openat(
            fixture.root_fd(),
            c"locked",
            directory_flags(),
            Mode::empty(),
        )
        .expect("open locked fixture directory");
        let production_root = open_with_production_search_flags(&fixture);
        let held = open_directory_at(&production_root, c"locked")
            .expect("hold searchable directory before permissions change");
        rustix::fs::fchmod(&locked, Mode::empty()).expect("remove read/search permissions");
        let result = read_directory(&held);
        rustix::fs::fchmod(&locked, Mode::from_raw_mode(0o700))
            .expect("restore permissions before assertions and cleanup");

        assert_eq!(
            result
                .expect_err("unreadable directory cannot be reported as empty")
                .raw_os_error(),
            Some(libc::EACCES)
        );
        fixture.cleanup().expect("remove verified FFI fixture");
    }

    #[test]
    fn fifo_without_writer_returns_promptly_and_is_not_a_regular_file() {
        let mut fixture = fixture();
        create_fifo(&mut fixture, b"no-writer");
        let executable = std::env::current_exe().expect("locate current test executable");
        let mut child = Command::new(executable)
            .arg("--exact")
            .arg("ffi::tests::fifo_open_child")
            .arg("--nocapture")
            .env("THINWS_P0_FIFO_FIXTURE", fixture.root_path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn bounded FIFO child test");
        let deadline = Instant::now() + Duration::from_secs(2);
        let status = loop {
            if let Some(status) = child.try_wait().expect("poll FIFO child") {
                break Some(status);
            }
            if Instant::now() >= deadline {
                child
                    .kill()
                    .expect("terminate only the owned hung test child");
                child.wait().expect("reap owned test child");
                break None;
            }
            thread::sleep(Duration::from_millis(10));
        };
        fixture.cleanup().expect("remove verified FFI fixture");
        assert!(
            status.is_some_and(|status| status.success()),
            "FIFO open must return without waiting for a writer"
        );
    }

    #[test]
    fn fifo_open_child() {
        let Some(root) = std::env::var_os("THINWS_P0_FIFO_FIXTURE") else {
            return;
        };
        let root = PathBuf::from(root);
        let mut current = open_root_directory().expect("open root in FIFO child");
        for path_component in root.components() {
            let std::path::Component::Normal(name) = path_component else {
                continue;
            };
            current = open_directory_at(&current, &component(name.as_bytes()))
                .expect("walk FIFO fixture in child");
        }
        let fifo = open_file_read_at(&current, c"no-writer")
            .expect("nonblocking no-follow FIFO open must return");
        assert_eq!(metadata(&fifo).expect("fstat FIFO").kind, NodeKind::Special);
    }

    #[test]
    fn readlink_preserves_boundary_and_long_targets() {
        let mut fixture = fixture();
        let production_root = open_with_production_search_flags(&fixture);
        for length in [255usize, 256, 257, 513, 1023] {
            let name = format!("link-{length}");
            let link_text = vec![b'a'; length];
            fixture
                .create_symlink(Path::new(&name), &link_text)
                .expect("create fixture symlink");
            assert_eq!(
                read_link_at(&production_root, &component(name.as_bytes()))
                    .expect("read exact symlink target bytes"),
                link_text
            );
        }
        fixture.cleanup().expect("remove verified FFI fixture");
    }

    #[test]
    fn clone_conflict_and_unlinkat_type_boundaries_preserve_unrelated_entries() {
        let mut fixture = fixture();
        fixture
            .create_file(Path::new("source"), b"source bytes", 0o600)
            .expect("create clone source");
        fixture
            .create_file(Path::new("existing"), b"existing bytes", 0o600)
            .expect("create clone conflict");
        fixture
            .create_file(Path::new("sentinel"), b"outside link target", 0o600)
            .expect("create sentinel");
        fixture
            .create_symlink(Path::new("link"), b"sentinel")
            .expect("create link");
        fixture
            .create_directory(Path::new("nonempty"), 0o700)
            .expect("create nonempty directory");
        fixture
            .create_file(Path::new("nonempty/child"), b"child", 0o600)
            .expect("create nested child");
        let production_root = open_with_production_search_flags(&fixture);
        let source = open_file_read_at(&production_root, c"source").expect("open clone source");

        assert_eq!(
            clone_file_at(&source, &production_root, c"existing")
                .expect_err("exclusive clone target must reject a conflict")
                .raw_os_error(),
            Some(libc::EEXIST)
        );
        assert_eq!(
            fs::read(fixture.root_path().join("existing")).unwrap(),
            b"existing bytes"
        );

        remove_at(&production_root, c"link", NodeKind::SymbolicLink)
            .expect("unlinkat must remove the link itself");
        fixture
            .confirm_removed(Path::new("link"))
            .expect("confirm removed registered link");
        assert_eq!(
            fs::read(fixture.root_path().join("sentinel")).unwrap(),
            b"outside link target"
        );
        assert_eq!(
            remove_at(&production_root, c"nonempty", NodeKind::Directory)
                .expect_err("nonempty directory removal must fail")
                .raw_os_error(),
            Some(libc::ENOTEMPTY)
        );
        assert_eq!(
            fs::read(fixture.root_path().join("nonempty/child")).unwrap(),
            b"child"
        );
        fixture.cleanup().expect("remove verified FFI fixture");
    }

    fn component(bytes: &[u8]) -> CString {
        assert!(!bytes.is_empty() && bytes != b"." && bytes != b".." && !bytes.contains(&b'/'));
        CString::new(bytes).expect("fixture component contains no NUL")
    }

    fn directory_flags() -> OFlags {
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
    }

    fn open_with_production_search_flags(fixture: &ControlledTree) -> OwnedFd {
        let mut current = open_root_directory().expect("open root with production FFI");
        for path_component in fixture.root_path().components() {
            let std::path::Component::Normal(name) = path_component else {
                continue;
            };
            current = open_directory_at(&current, &component(name.as_bytes()))
                .expect("walk fixture with production no-follow search descriptors");
        }
        current
    }

    fn create_fifo(fixture: &mut ControlledTree, name: &[u8]) {
        let relative = Path::new(std::ffi::OsStr::from_bytes(name));
        let path = CString::new(fixture.root_path().join(relative).as_os_str().as_bytes())
            .expect("fixture FIFO path contains no NUL");
        // SAFETY: the absolute path is wholly inside this test's unique,
        // identity-held fixture root, is a valid C string, and is not retained.
        let result = unsafe { libc::mkfifo(path.as_ptr(), 0o600) };
        assert_eq!(
            result,
            0,
            "create fixture FIFO: {}",
            std::io::Error::last_os_error()
        );
        fixture
            .track_existing(relative)
            .expect("register fixture-created FIFO");
    }
}
