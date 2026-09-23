use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rustix::fs::{AtFlags, FileType, Mode, OFlags, RenameFlags};
use thinws_core::{AbsolutePath, AbsolutePathError};
use thinws_ports::{PortError, PortErrorKind};

use crate::document::MAX_DOCUMENT_BYTES;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
                OFlags::CREATE
                    | OFlags::EXCL
                    | OFlags::RDWR
                    | OFlags::CLOEXEC
                    | OFlags::NOFOLLOW
                    | OFlags::NONBLOCK,
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
        if self.active {
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
    let mut current = rustix::fs::open(
        Path::new("/"),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
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
        current = match rustix::fs::openat(
            &current,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        ) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(error) => return Err(io_error("open private directory component", error)),
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
    let fd = match rustix::fs::openat(
        directory,
        name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
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
