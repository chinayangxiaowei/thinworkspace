use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rustix::fs::{AtFlags, FileType, Mode, OFlags, RenameFlags};
use thinws_core::{AbsolutePath, AbsolutePathError};
use thinws_ports::{PortError, PortErrorKind};

use crate::document::MAX_DOCUMENT_BYTES;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

const PRIVATE_TEMP_OPEN_FLAGS: OFlags = OFlags::CREATE
    .union(OFlags::EXCL)
    .union(OFlags::RDWR)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::NONBLOCK);
const DIRECTORY_OPEN_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::NONBLOCK);
const PRIVATE_READ_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::CLOEXEC)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::NONBLOCK);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FileIdentity {
    device: u64,
    inode: u64,
}

pub(crate) struct ValidatedDirectory {
    pub(crate) fd: OwnedFd,
    pub(crate) identity: FileIdentity,
    pub(crate) path: PathBuf,
}

pub(crate) struct PrivateTemp {
    parent: OwnedFd,
    name: OsString,
    file: Option<File>,
    identity: FileIdentity,
    active: bool,
}

#[derive(Debug)]
pub(crate) enum NoReplaceError {
    Exists,
    Other(PortError),
}

impl PrivateTemp {
    pub(crate) fn create(parent: &OwnedFd, prefix: &str, bytes: &[u8]) -> Result<Self, PortError> {
        let parent_copy = rustix::io::dup(parent)
            .map_err(|error| io_error("duplicate document directory", error))?;
        for _ in 0..32 {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let name = OsString::from(format!(".{prefix}.tmp-{}-{sequence}", std::process::id()));
            match rustix::fs::openat(
                &parent_copy,
                &name,
                PRIVATE_TEMP_OPEN_FLAGS,
                Mode::from_bits_retain(0o600),
            ) {
                Ok(fd) => {
                    let file = File::from(fd);
                    let identity = validate_private_file(&file, "validate private document")?;
                    let mut temporary = Self {
                        parent: parent_copy,
                        name,
                        file: Some(file),
                        identity,
                        active: true,
                    };
                    let file = temporary
                        .file
                        .as_mut()
                        .expect("private temp always owns its file");
                    file.write_all(bytes)
                        .map_err(|error| io_error("write private document", error))?;
                    file.sync_all()
                        .map_err(|error| io_error("sync private document", error))?;
                    if validate_private_file(file, "revalidate private document")? != identity {
                        return Err(validation_error("private document identity changed"));
                    }
                    return Ok(temporary);
                }
                Err(rustix::io::Errno::EXIST) => continue,
                Err(error) => return Err(io_error("create private document", error)),
            }
        }
        Err(PortError::new(
            PortErrorKind::Io,
            "allocate private document name",
        ))
    }

    pub(crate) fn publish_noreplace(
        mut self,
        target: &OsStr,
    ) -> Result<(File, FileIdentity), NoReplaceError> {
        match rustix::fs::renameat_with(
            &self.parent,
            &self.name,
            &self.parent,
            target,
            RenameFlags::NOREPLACE,
        ) {
            Ok(()) => {
                self.active = false;
                sync_directory(&self.parent).map_err(NoReplaceError::Other)?;
                let file = self.file.take().expect("private temp always owns its file");
                Ok((file, self.identity))
            }
            Err(rustix::io::Errno::EXIST) => Err(NoReplaceError::Exists),
            Err(error) => Err(NoReplaceError::Other(io_error(
                "publish document without replacement",
                error,
            ))),
        }
    }

    pub(crate) fn exchange_with(&mut self, target: &OsStr) -> Result<(), PortError> {
        rustix::fs::renameat_with(
            &self.parent,
            &self.name,
            &self.parent,
            target,
            RenameFlags::EXCHANGE,
        )
        .map_err(|error| io_error("exchange root marker", error))?;
        // The temporary name now refers to the old marker, so Drop must not
        // mistake it for the newly-created file and delete it.
        self.active = false;
        Ok(())
    }

    pub(crate) fn name(&self) -> &OsStr {
        &self.name
    }

    pub(crate) const fn identity(&self) -> FileIdentity {
        self.identity
    }
}

impl Drop for PrivateTemp {
    fn drop(&mut self) {
        if self.active && entry_identity(&self.parent, &self.name).ok() == Some(self.identity) {
            let _ = rustix::fs::unlinkat(&self.parent, &self.name, AtFlags::empty());
        }
    }
}

pub(crate) fn absolute_from_path(path: &Path) -> Result<AbsolutePath, AbsolutePathError> {
    AbsolutePath::try_from_bytes(path.as_os_str().as_bytes().to_vec())
}

pub(crate) fn path_from_absolute(path: &AbsolutePath) -> PathBuf {
    PathBuf::from(OsString::from_vec(path.as_bytes().to_vec()))
}

pub(crate) fn open_private_directory(path: &Path) -> Result<ValidatedDirectory, PortError> {
    open_private_directory_optional(path)?
        .ok_or_else(|| PortError::new(PortErrorKind::NotFound, "open private directory"))
}

pub(crate) fn prepare_private_directory(
    path: &Path,
    require_empty: bool,
) -> Result<ValidatedDirectory, PortError> {
    let mut current = rustix::fs::open(Path::new("/"), DIRECTORY_OPEN_FLAGS, Mode::empty())
        .map_err(|error| io_error("open filesystem root", error))?;
    let mut saw_root = false;
    let mut current_path = PathBuf::from("/");

    for component in path.components() {
        let name = match component {
            Component::RootDir if !saw_root => {
                saw_root = true;
                continue;
            }
            Component::Normal(name) if saw_root => name,
            _ => return Err(validation_error("private directory path is not canonical")),
        };
        let next = match rustix::fs::openat(&current, name, DIRECTORY_OPEN_FLAGS, Mode::empty()) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::NOENT) => {
                rustix::fs::mkdirat(&current, name, Mode::from_bits_retain(0o700))
                    .map_err(|error| io_error("create private directory component", error))?;
                sync_directory(&current)?;
                let fd = rustix::fs::openat(&current, name, DIRECTORY_OPEN_FLAGS, Mode::empty())
                    .map_err(|error| io_error("open created private directory", error))?;
                let stat = rustix::fs::fstat(&fd)
                    .map_err(|error| io_error("inspect created private directory", error))?;
                validate_directory_stat(&stat)?;
                fd
            }
            Err(error) => {
                return Err(secure_open_error(
                    "open private directory component",
                    error,
                    PortErrorKind::InvalidData,
                ));
            }
        };
        current_path.push(name);
        current = next;
    }

    if !saw_root || current_path == Path::new("/") {
        return Err(validation_error("private directory path is not canonical"));
    }
    let stat = rustix::fs::fstat(&current)
        .map_err(|error| io_error("inspect private directory", error))?;
    validate_directory_stat(&stat)?;
    let directory = ValidatedDirectory {
        identity: identity(&stat),
        fd: current,
        path: current_path,
    };
    if require_empty {
        require_empty_directory(&directory)?;
    }
    Ok(directory)
}

pub(crate) fn create_private_child_directory(
    parent: &ValidatedDirectory,
    name: &OsStr,
) -> Result<ValidatedDirectory, PortError> {
    rustix::fs::mkdirat(&parent.fd, name, Mode::from_bits_retain(0o700))
        .map_err(|error| io_error("create controlled directory", error))?;
    sync_directory(&parent.fd)?;
    open_private_child_directory(parent, name)
}

pub(crate) fn open_private_child_directory(
    parent: &ValidatedDirectory,
    name: &OsStr,
) -> Result<ValidatedDirectory, PortError> {
    let fd = rustix::fs::openat(&parent.fd, name, DIRECTORY_OPEN_FLAGS, Mode::empty()).map_err(
        |error| {
            secure_open_error(
                "open controlled directory",
                error,
                PortErrorKind::InvalidData,
            )
        },
    )?;
    let stat =
        rustix::fs::fstat(&fd).map_err(|error| io_error("inspect controlled directory", error))?;
    validate_directory_stat(&stat)?;
    Ok(ValidatedDirectory {
        fd,
        identity: identity(&stat),
        path: parent.path.join(name),
    })
}

pub(crate) fn duplicate_validated_directory(
    directory: &ValidatedDirectory,
) -> Result<ValidatedDirectory, PortError> {
    let fd = rustix::io::dup(&directory.fd)
        .map_err(|error| io_error("duplicate validated directory", error))?;
    Ok(ValidatedDirectory {
        fd,
        identity: directory.identity,
        path: directory.path.clone(),
    })
}

pub(crate) fn create_private_file(
    parent: &ValidatedDirectory,
    name: &OsStr,
) -> Result<(File, FileIdentity), PortError> {
    let fd = rustix::fs::openat(
        &parent.fd,
        name,
        OFlags::CREATE
            .union(OFlags::EXCL)
            .union(OFlags::RDWR)
            .union(OFlags::CLOEXEC)
            .union(OFlags::NOFOLLOW)
            .union(OFlags::NONBLOCK),
        Mode::from_bits_retain(0o600),
    )
    .map_err(|error| io_error("create controlled file", error))?;
    let file = File::from(fd);
    let identity = validate_private_file(&file, "validate controlled file")?;
    file.sync_all()
        .map_err(|error| io_error("sync controlled file", error))?;
    sync_directory(&parent.fd)?;
    Ok((file, identity))
}

pub(crate) fn open_private_file(
    parent: &ValidatedDirectory,
    name: &OsStr,
) -> Result<(File, FileIdentity), PortError> {
    let fd = rustix::fs::openat(parent.fd.as_fd(), name, PRIVATE_READ_FLAGS, Mode::empty())
        .map_err(|error| {
            secure_open_error("open controlled file", error, PortErrorKind::InvalidData)
        })?;
    let file = File::from(fd);
    let identity = validate_private_file(&file, "validate controlled file")?;
    Ok((file, identity))
}

fn directory_is_empty(directory: &ValidatedDirectory) -> Result<bool, PortError> {
    let mut entries = rustix::fs::Dir::read_from(&directory.fd)
        .map_err(|error| io_error("read private directory", error))?;
    for entry in &mut entries {
        let entry = entry.map_err(|error| io_error("read private directory entry", error))?;
        if !matches!(entry.file_name().to_bytes(), b"." | b"..") {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) fn require_empty_directory(directory: &ValidatedDirectory) -> Result<(), PortError> {
    if directory_is_empty(directory)? {
        Ok(())
    } else {
        Err(PortError::new(
            PortErrorKind::NotEmpty,
            "require empty private directory",
        ))
    }
}

pub(crate) fn open_private_directory_optional(
    path: &Path,
) -> Result<Option<ValidatedDirectory>, PortError> {
    let Some(fd) = open_absolute_directory_nofollow(path)? else {
        return Ok(None);
    };
    let stat =
        rustix::fs::fstat(&fd).map_err(|error| io_error("inspect private directory", error))?;
    validate_directory_stat(&stat)?;
    Ok(Some(ValidatedDirectory {
        identity: identity(&stat),
        fd,
        path: path.to_path_buf(),
    }))
}

fn open_absolute_directory_nofollow(path: &Path) -> Result<Option<OwnedFd>, PortError> {
    let mut current = rustix::fs::open(Path::new("/"), DIRECTORY_OPEN_FLAGS, Mode::empty())
        .map_err(|error| io_error("open filesystem root", error))?;
    let mut saw_root = false;
    let mut saw_normal = false;
    for component in path.components() {
        let name = match component {
            Component::RootDir if !saw_root && !saw_normal => {
                saw_root = true;
                continue;
            }
            Component::Normal(name) if saw_root => {
                saw_normal = true;
                name
            }
            _ => return Err(validation_error("private directory path is not canonical")),
        };
        current = match rustix::fs::openat(&current, name, DIRECTORY_OPEN_FLAGS, Mode::empty()) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(error) => {
                return Err(secure_open_error(
                    "open private directory component",
                    error,
                    PortErrorKind::NotFound,
                ));
            }
        };
    }
    if !saw_root || !saw_normal {
        return Err(validation_error("private directory path is not canonical"));
    }
    Ok(Some(current))
}

pub(crate) fn read_private_file(
    directory: &OwnedFd,
    name: &OsStr,
) -> Result<Option<Vec<u8>>, PortError> {
    let fd = match rustix::fs::openat(directory, name, PRIVATE_READ_FLAGS, Mode::empty()) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(error) => return Err(io_error("open private document", error)),
    };
    let mut file = File::from(fd);
    let expected = validate_private_file(&file, "validate private document")?;
    let mut bytes = Vec::new();
    (&mut file)
        .take((MAX_DOCUMENT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| io_error("read private document", error))?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(PortError::new(
            PortErrorKind::InvalidData,
            "reject oversized private document",
        ));
    }
    validate_file_entry(directory, name, &file, expected)?;
    Ok(Some(bytes))
}

pub(crate) fn validate_file_entry(
    directory: &OwnedFd,
    name: &OsStr,
    file: &File,
    expected: FileIdentity,
) -> Result<(), PortError> {
    let held = validate_private_file(file, "revalidate held private file")?;
    let current = rustix::fs::statat(directory, name, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|error| io_error("revalidate private file entry", error))?;
    if held != expected
        || identity(&current) != expected
        || !FileType::from_raw_mode(current.st_mode).is_file()
    {
        return Err(validation_error("private file identity changed"));
    }
    Ok(())
}

pub(crate) fn entry_identity(directory: &OwnedFd, name: &OsStr) -> Result<FileIdentity, PortError> {
    let stat = rustix::fs::statat(directory, name, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|error| io_error("inspect private file entry", error))?;
    if !FileType::from_raw_mode(stat.st_mode).is_file() {
        return Err(validation_error("private file entry is not regular"));
    }
    Ok(identity(&stat))
}

pub(crate) fn unlink_entry(directory: &OwnedFd, name: &OsStr) -> Result<(), PortError> {
    rustix::fs::unlinkat(directory, name, AtFlags::empty())
        .map_err(|error| io_error("remove private temporary document", error))
}

pub(crate) fn sync_directory(directory: &OwnedFd) -> Result<(), PortError> {
    rustix::fs::fsync(directory).map_err(|error| io_error("sync private directory", error))
}

pub(crate) fn revalidate_directory(directory: &ValidatedDirectory) -> Result<(), PortError> {
    let current = open_private_directory(&directory.path)?;
    if current.identity != directory.identity {
        return Err(validation_error("private directory identity changed"));
    }
    Ok(())
}

pub(crate) fn validate_private_file(
    file: &File,
    operation: &'static str,
) -> Result<FileIdentity, PortError> {
    let stat = rustix::fs::fstat(file).map_err(|error| io_error(operation, error))?;
    if !FileType::from_raw_mode(stat.st_mode).is_file()
        || stat.st_uid != rustix::process::geteuid().as_raw()
        || u32::from(stat.st_mode & 0o7777) != 0o600
        || stat.st_nlink != 1
    {
        return Err(validation_error(operation));
    }
    Ok(identity(&stat))
}

fn validate_directory_stat(stat: &rustix::fs::Stat) -> Result<(), PortError> {
    if !FileType::from_raw_mode(stat.st_mode).is_dir()
        || stat.st_uid != rustix::process::geteuid().as_raw()
        || u32::from(stat.st_mode & 0o7777) != 0o700
    {
        return Err(validation_error("private directory metadata is unsafe"));
    }
    Ok(())
}

#[allow(clippy::cast_sign_loss)]
fn identity(stat: &rustix::fs::Stat) -> FileIdentity {
    FileIdentity {
        device: stat.st_dev as u64,
        inode: stat.st_ino,
    }
}

pub(crate) fn io_error(
    operation: &'static str,
    error: impl Error + Send + Sync + 'static,
) -> PortError {
    PortError::new(PortErrorKind::Io, operation).with_source(error)
}

fn secure_open_error(
    operation: &'static str,
    error: rustix::io::Errno,
    missing_kind: PortErrorKind,
) -> PortError {
    let kind = if error == rustix::io::Errno::NOENT {
        missing_kind
    } else if matches!(
        error,
        rustix::io::Errno::NOTDIR
            | rustix::io::Errno::LOOP
            | rustix::io::Errno::ACCESS
            | rustix::io::Errno::PERM
    ) {
        PortErrorKind::InvalidData
    } else {
        PortErrorKind::Io
    };
    PortError::new(kind, operation).with_source(error)
}

pub(crate) fn validation_error(operation: &'static str) -> PortError {
    PortError::new(PortErrorKind::InvalidData, operation)
        .with_source(FilesystemValidationError(operation))
}

#[derive(Debug)]
struct FilesystemValidationError(&'static str);

impl fmt::Display for FilesystemValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl Error for FilesystemValidationError {}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;
    use std::process::Command;

    use tempfile::{Builder, TempDir};

    use super::*;

    fn controlled_directory(prefix: &str) -> (TempDir, PathBuf, ValidatedDirectory) {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-02-macos-tests");
        fs::create_dir_all(&root).unwrap();
        let temp = Builder::new()
            .prefix(prefix)
            .tempdir_in(fs::canonicalize(root).unwrap())
            .unwrap();
        let directory_path = temp.path().join("private");
        fs::create_dir(&directory_path).unwrap();
        fs::set_permissions(&directory_path, fs::Permissions::from_mode(0o700)).unwrap();
        let directory = open_private_directory(&directory_path).unwrap();
        (temp, directory_path, directory)
    }

    #[test]
    fn dropping_a_displaced_private_temp_preserves_its_replacement() {
        let (_temp, directory_path, directory) = controlled_directory("private-temp-");
        let temporary = PrivateTemp::create(&directory.fd, "document", b"original").unwrap();
        let original_name = directory_path.join(temporary.name());
        let displaced = directory_path.join("displaced");

        fs::rename(&original_name, &displaced).unwrap();
        fs::write(&original_name, b"replacement").unwrap();
        fs::set_permissions(&original_name, fs::Permissions::from_mode(0o600)).unwrap();
        drop(temporary);

        assert_eq!(fs::read(&original_name).unwrap(), b"replacement");
        assert_eq!(fs::read(&displaced).unwrap(), b"original");
    }

    #[test]
    fn dropping_an_owned_private_temp_removes_its_entry() {
        let (_temp, directory_path, directory) = controlled_directory("private-temp-cleanup-");
        let temporary = PrivateTemp::create(&directory.fd, "document", b"temporary").unwrap();
        let temporary_path = directory_path.join(temporary.name());

        drop(temporary);

        assert!(!temporary_path.exists());
    }

    #[test]
    fn security_sensitive_open_flag_sets_are_exact() {
        assert_eq!(
            PRIVATE_TEMP_OPEN_FLAGS,
            OFlags::CREATE
                .union(OFlags::EXCL)
                .union(OFlags::RDWR)
                .union(OFlags::CLOEXEC)
                .union(OFlags::NOFOLLOW)
                .union(OFlags::NONBLOCK)
        );
        assert_eq!(
            DIRECTORY_OPEN_FLAGS,
            OFlags::RDONLY
                .union(OFlags::DIRECTORY)
                .union(OFlags::CLOEXEC)
                .union(OFlags::NOFOLLOW)
                .union(OFlags::NONBLOCK)
        );
        assert_eq!(
            PRIVATE_READ_FLAGS,
            OFlags::RDONLY
                .union(OFlags::CLOEXEC)
                .union(OFlags::NOFOLLOW)
                .union(OFlags::NONBLOCK)
        );
    }

    #[test]
    fn path_walker_rejects_relative_and_root_only_inputs() {
        assert!(open_absolute_directory_nofollow(Path::new("relative")).is_err());
        assert!(open_absolute_directory_nofollow(Path::new("/")).is_err());
    }

    #[test]
    fn private_reader_enforces_the_exact_document_size_limit() {
        let (_temp, directory_path, directory) = controlled_directory("private-read-size-");
        let document = directory_path.join("document");
        fs::write(&document, vec![b'a'; MAX_DOCUMENT_BYTES]).unwrap();
        fs::set_permissions(&document, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            read_private_file(&directory.fd, OsStr::new("document"))
                .unwrap()
                .unwrap()
                .len(),
            MAX_DOCUMENT_BYTES
        );

        fs::write(&document, vec![b'a'; MAX_DOCUMENT_BYTES + 1]).unwrap();
        assert_eq!(
            read_private_file(&directory.fd, OsStr::new("document"))
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidData
        );
    }

    #[test]
    fn filesystem_helpers_expose_failure_and_identity_boundaries() {
        let (_temp, directory_path, directory) = controlled_directory("filesystem-helpers-");
        let removable = directory_path.join("removable");
        fs::write(&removable, b"remove").unwrap();
        unlink_entry(&directory.fd, OsStr::new("removable")).unwrap();
        assert!(!removable.exists());

        let (socket, _peer) = UnixStream::pair().unwrap();
        let socket: OwnedFd = socket.into();
        assert!(sync_directory(&socket).is_err());

        let displaced = directory_path.with_extension("displaced");
        fs::rename(&directory_path, &displaced).unwrap();
        fs::create_dir(&directory_path).unwrap();
        fs::set_permissions(&directory_path, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            revalidate_directory(&directory).unwrap_err().kind(),
            PortErrorKind::InvalidData
        );
    }

    #[test]
    fn metadata_validators_reject_each_isolated_unsafe_shape() {
        let (_temp, directory_path, directory) = controlled_directory("metadata-validators-");

        let hardlinked = directory_path.join("hardlinked");
        fs::write(&hardlinked, b"content").unwrap();
        fs::set_permissions(&hardlinked, fs::Permissions::from_mode(0o600)).unwrap();
        fs::hard_link(&hardlinked, directory_path.join("second-link")).unwrap();
        let file = File::open(&hardlinked).unwrap();
        assert!(validate_private_file(&file, "validate hard link").is_err());

        assert!(
            Command::new("/usr/bin/mkfifo")
                .arg(directory_path.join("fifo"))
                .status()
                .unwrap()
                .success()
        );
        fs::set_permissions(
            directory_path.join("fifo"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let fifo = rustix::fs::openat(
            &directory.fd,
            "fifo",
            OFlags::RDONLY
                .union(OFlags::CLOEXEC)
                .union(OFlags::NOFOLLOW)
                .union(OFlags::NONBLOCK),
            Mode::empty(),
        )
        .unwrap();
        let fifo = File::from(fifo);
        assert!(validate_private_file(&fifo, "validate FIFO").is_err());

        let regular = directory_path.join("not-a-directory");
        fs::write(&regular, b"content").unwrap();
        fs::set_permissions(&regular, fs::Permissions::from_mode(0o700)).unwrap();
        let stat = rustix::fs::stat(&regular).unwrap();
        assert!(validate_directory_stat(&stat).is_err());

        fs::set_permissions(&directory_path, fs::Permissions::from_mode(0o755)).unwrap();
        let stat = rustix::fs::fstat(&directory.fd).unwrap();
        assert!(validate_directory_stat(&stat).is_err());

        let diagnostic = validation_error("filesystem diagnostic");
        assert_eq!(
            diagnostic.source().unwrap().to_string(),
            "filesystem diagnostic"
        );
    }
}
