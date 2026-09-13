#![allow(dead_code)]

use std::collections::BTreeMap;
use std::ffi::{CStr, CString, OsStr};
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rustix::fs::{
    AtFlags, Dir, FileType, Mode, OFlags, RenameFlags, Timespec, Timestamps, UTIME_OMIT,
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TrackedKind {
    Directory,
    RegularFile,
    SymbolicLink,
    Special,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TrackedIdentity {
    pub(crate) device: u64,
    pub(crate) inode: u64,
    pub(crate) kind: TrackedKind,
}

/// Test-only owned tree. It has no `Drop` cleanup: every removal requires an
/// explicit, no-follow identity and complete-set check against this ledger.
pub(crate) struct ControlledTree {
    existing_parent_path: PathBuf,
    existing_parent: OwnedFd,
    existing_parent_identity: TrackedIdentity,
    fixture_parent_path: PathBuf,
    fixture_parent_name: CString,
    fixture_parent: OwnedFd,
    fixture_parent_identity: TrackedIdentity,
    root_name: CString,
    root: OwnedFd,
    root_identity: TrackedIdentity,
    root_path: PathBuf,
    tracked: BTreeMap<PathBuf, TrackedIdentity>,
}

impl ControlledTree {
    pub(crate) fn create_in(existing_parent: &Path, label: &str) -> io::Result<Self> {
        let existing_parent_path = existing_parent.canonicalize()?;
        if !existing_parent_path.is_absolute() {
            return Err(io::Error::from_raw_os_error(libc::EINVAL));
        }
        let existing_parent = open_absolute_directory(&existing_parent_path)?;
        let existing_parent_identity = tracked_identity(rustix::fs::fstat(&existing_parent)?)?;

        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let fixture_parent_name = component_name(OsStr::new(&format!(
            "fixture-parent-{label}-{}-{sequence}",
            std::process::id()
        )))?;
        rustix::fs::mkdirat(
            &existing_parent,
            &fixture_parent_name,
            Mode::from_raw_mode(0o700),
        )
        .map_err(rustix_error)?;
        let fixture_parent = open_directory_at(&existing_parent, &fixture_parent_name)?;
        rustix::fs::fchmod(&fixture_parent, Mode::from_raw_mode(0o700)).map_err(rustix_error)?;
        let fixture_parent_identity = tracked_identity(rustix::fs::fstat(&fixture_parent)?)?;
        let fixture_parent_path =
            existing_parent_path.join(OsStr::from_bytes(fixture_parent_name.to_bytes()));

        let root_name = CString::new("case").expect("static component contains no NUL");
        rustix::fs::mkdirat(&fixture_parent, &root_name, Mode::from_raw_mode(0o700))
            .map_err(rustix_error)?;
        let root = open_directory_at(&fixture_parent, &root_name)?;
        rustix::fs::fchmod(&root, Mode::from_raw_mode(0o700)).map_err(rustix_error)?;
        let root_identity = tracked_identity(rustix::fs::fstat(&root)?)?;
        let root_path = fixture_parent_path.join("case");
        let mut tracked = BTreeMap::new();
        tracked.insert(PathBuf::new(), root_identity);

        Ok(Self {
            existing_parent_path,
            existing_parent,
            existing_parent_identity,
            fixture_parent_path,
            fixture_parent_name,
            fixture_parent,
            fixture_parent_identity,
            root_name,
            root,
            root_identity,
            root_path,
            tracked,
        })
    }

    pub(crate) fn root_path(&self) -> &Path {
        &self.root_path
    }

    pub(crate) fn root_fd(&self) -> &OwnedFd {
        &self.root
    }

    pub(crate) fn root_identity(&self) -> TrackedIdentity {
        self.root_identity
    }

    pub(crate) fn tracked(&self) -> &BTreeMap<PathBuf, TrackedIdentity> {
        &self.tracked
    }

    pub(crate) fn create_directory(
        &mut self,
        relative: impl AsRef<Path>,
        mode: u32,
    ) -> io::Result<()> {
        let relative = relative.as_ref();
        let (parent, name) = self.open_parent(relative)?;
        let mode = fixture_mode(mode)?;
        rustix::fs::mkdirat(&parent, &name, mode).map_err(rustix_error)?;
        let child = open_directory_at(&parent, &name)?;
        rustix::fs::fchmod(&child, mode).map_err(rustix_error)?;
        self.track_existing(relative)?;
        Ok(())
    }

    pub(crate) fn create_file(
        &mut self,
        relative: impl AsRef<Path>,
        contents: &[u8],
        mode: u32,
    ) -> io::Result<()> {
        let relative = relative.as_ref();
        let (parent, name) = self.open_parent(relative)?;
        let mode = fixture_mode(mode)?;
        let file = rustix::fs::openat(
            &parent,
            &name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            mode,
        )
        .map_err(rustix_error)?;
        write_all(&file, contents)?;
        rustix::fs::fchmod(&file, mode).map_err(rustix_error)?;
        self.track_existing(relative)?;
        Ok(())
    }

    pub(crate) fn create_symlink(
        &mut self,
        relative: impl AsRef<Path>,
        link_text: &[u8],
    ) -> io::Result<()> {
        let relative = relative.as_ref();
        let (parent, name) = self.open_parent(relative)?;
        let link_text =
            CString::new(link_text).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))?;
        rustix::fs::symlinkat(&link_text, &parent, &name).map_err(rustix_error)?;
        self.track_existing(relative)?;
        Ok(())
    }

    /// Register an object just created by this fixture, after independently
    /// observing its no-follow identity. This must not be used to adopt an
    /// unconfirmed object left by the experiment under test.
    pub(crate) fn track_existing(&mut self, relative: &Path) -> io::Result<TrackedIdentity> {
        validate_relative(relative)?;
        let observed = self.identity_at(relative)?;
        self.tracked.insert(relative.to_path_buf(), observed);
        Ok(observed)
    }

    pub(crate) fn adopt_confirmed(
        &mut self,
        relative: &Path,
        expected_device: u64,
        expected_inode: u64,
        expected_kind: TrackedKind,
    ) -> io::Result<()> {
        let observed = self.identity_at(relative)?;
        if observed.device != expected_device
            || observed.inode != expected_inode
            || observed.kind != expected_kind
        {
            return Err(io::Error::other(
                "independent fixture observation does not match confirmed receipt identity",
            ));
        }
        self.tracked.insert(relative.to_path_buf(), observed);
        Ok(())
    }

    pub(crate) fn confirm_removed(&mut self, relative: &Path) -> io::Result<()> {
        let expected = self
            .tracked
            .get(relative)
            .copied()
            .ok_or_else(|| io::Error::other("removed fixture entry was not registered"))?;
        let (parent, name) = self.open_parent(relative)?;
        match rustix::fs::statat(&parent, &name, AtFlags::SYMLINK_NOFOLLOW) {
            Err(rustix::io::Errno::NOENT) => {
                self.tracked.remove(relative);
                Ok(())
            }
            Ok(current) => Err(io::Error::other(format!(
                "expected registered entry to be absent, but still observed {:?}",
                tracked_identity(current).unwrap_or(expected)
            ))),
            Err(error) => Err(rustix_error(error)),
        }
    }

    pub(crate) fn remove_tracked_exact(&mut self, relative: &Path) -> io::Result<()> {
        self.verify_exact_tree()?;
        let expected = self
            .tracked
            .get(relative)
            .copied()
            .ok_or_else(|| io::Error::other("remove target is not fixture-owned"))?;
        let (parent, name) = self.open_parent(relative)?;
        let observed = tracked_identity(
            rustix::fs::statat(&parent, &name, AtFlags::SYMLINK_NOFOLLOW).map_err(rustix_error)?,
        )?;
        if observed != expected {
            return Err(io::Error::other(
                "refusing to remove a fixture entry with a different identity",
            ));
        }
        remove_at(&parent, &name, expected.kind)?;
        self.tracked.remove(relative);
        self.verify_exact_tree()
    }

    pub(crate) fn rename_tracked_exclusive(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        self.verify_exact_tree()?;
        validate_relative(from)?;
        validate_relative(to)?;
        let expected = self
            .tracked
            .get(from)
            .copied()
            .ok_or_else(|| io::Error::other("rename source is not fixture-owned"))?;
        let affected: Vec<_> = self
            .tracked
            .iter()
            .filter_map(|(path, identity)| {
                path.strip_prefix(from)
                    .ok()
                    .map(|suffix| (path.clone(), to.join(suffix), *identity))
            })
            .collect();
        if affected.is_empty()
            || affected.iter().any(|(_, destination, _)| {
                self.tracked.contains_key(destination)
                    && !affected.iter().any(|(source, _, _)| source == destination)
            })
        {
            return Err(io::Error::other(
                "rename destination conflicts with a fixture-owned entry",
            ));
        }
        let (from_parent, from_name) = self.open_parent(from)?;
        let (to_parent, to_name) = self.open_parent(to)?;
        rustix::fs::renameat_with(
            &from_parent,
            &from_name,
            &to_parent,
            &to_name,
            RenameFlags::NOREPLACE,
        )
        .map_err(rustix_error)?;
        let observed = tracked_identity(
            rustix::fs::statat(&to_parent, &to_name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(rustix_error)?,
        )?;
        if observed != expected {
            return Err(io::Error::other(
                "renamed fixture identity did not match the owned source",
            ));
        }
        for (source, _, _) in &affected {
            self.tracked.remove(source);
        }
        for (_, destination, identity) in affected {
            self.tracked.insert(destination, identity);
        }
        self.verify_exact_tree()
    }

    pub(crate) fn identity_at(&self, relative: &Path) -> io::Result<TrackedIdentity> {
        validate_relative(relative)?;
        if relative.as_os_str().is_empty() {
            return tracked_identity(rustix::fs::fstat(&self.root)?);
        }
        let (parent, name) = self.open_parent(relative)?;
        tracked_identity(
            rustix::fs::statat(&parent, &name, AtFlags::SYMLINK_NOFOLLOW).map_err(rustix_error)?,
        )
    }

    pub(crate) fn observed_tree(&self) -> io::Result<BTreeMap<PathBuf, TrackedIdentity>> {
        observe_tree(&self.root)
    }

    pub(crate) fn set_mode(&self, relative: &Path, mode: u32) -> io::Result<()> {
        let object = self.open_tracked(relative)?;
        rustix::fs::fchmod(&object, fixture_mode(mode)?).map_err(rustix_error)
    }

    pub(crate) fn set_modified_time(
        &self,
        relative: &Path,
        seconds: i64,
        nanoseconds: i64,
    ) -> io::Result<()> {
        let object = self.open_tracked(relative)?;
        rustix::fs::futimens(
            &object,
            &Timestamps {
                last_access: Timespec {
                    tv_sec: 0,
                    tv_nsec: UTIME_OMIT,
                },
                last_modification: Timespec {
                    tv_sec: seconds,
                    tv_nsec: nanoseconds,
                },
            },
        )
        .map_err(rustix_error)
    }

    pub(crate) fn mode_and_modified_time_at(&self, relative: &Path) -> io::Result<(u32, i64, i64)> {
        let object = self.open_tracked(relative)?;
        let metadata = rustix::fs::fstat(&object).map_err(rustix_error)?;
        Ok((
            u32::from(metadata.st_mode & 0o7777),
            metadata.st_mtime,
            metadata.st_mtime_nsec,
        ))
    }

    pub(crate) fn verify_roots(&self) -> io::Result<()> {
        self.verify_parent()?;
        if tracked_identity(rustix::fs::fstat(&self.root)?)? != self.root_identity
            || tracked_identity(
                rustix::fs::statat(
                    &self.fixture_parent,
                    &self.root_name,
                    AtFlags::SYMLINK_NOFOLLOW,
                )
                .map_err(rustix_error)?,
            )? != self.root_identity
        {
            return Err(io::Error::other(format!(
                "fixture root identity changed; preserving {}",
                self.root_path.display()
            )));
        }
        let reopened_root = open_directory_at(&self.fixture_parent, &self.root_name)?;
        if tracked_identity(rustix::fs::fstat(&reopened_root)?)? != self.root_identity {
            return Err(io::Error::other(format!(
                "fixture root reopen did not match; preserving {}",
                self.root_path.display()
            )));
        }
        Ok(())
    }

    pub(crate) fn verify_exact_tree(&self) -> io::Result<()> {
        self.verify_roots()?;
        if self.observed_tree()? != self.tracked {
            return Err(io::Error::other(format!(
                "controlled fixture set or identity changed; preserving {}",
                self.root_path.display()
            )));
        }
        Ok(())
    }

    pub(crate) fn move_fixture_parent_exclusive(&self) -> io::Result<CString> {
        self.verify_exact_tree()?;
        let moved_name =
            CString::new([b"moved-".as_slice(), self.fixture_parent_name.to_bytes()].concat())
                .expect("generated moved fixture name contains no NUL");
        rustix::fs::renameat_with(
            &self.existing_parent,
            &self.fixture_parent_name,
            &self.existing_parent,
            &moved_name,
            RenameFlags::NOREPLACE,
        )
        .map_err(rustix_error)?;
        let moved = tracked_identity(
            rustix::fs::statat(
                &self.existing_parent,
                &moved_name,
                AtFlags::SYMLINK_NOFOLLOW,
            )
            .map_err(rustix_error)?,
        )?;
        if moved != self.fixture_parent_identity {
            return Err(io::Error::other(
                "moved fixture parent identity did not match the owned object",
            ));
        }
        Ok(moved_name)
    }

    pub(crate) fn restore_fixture_parent_exclusive(&self, moved_name: &CStr) -> io::Result<()> {
        let reopened = open_absolute_directory(&self.existing_parent_path)?;
        if tracked_identity(rustix::fs::fstat(&reopened)?)? != self.existing_parent_identity {
            return Err(io::Error::other("existing fixture parent path changed"));
        }
        let moved = tracked_identity(
            rustix::fs::statat(&reopened, moved_name, AtFlags::SYMLINK_NOFOLLOW)
                .map_err(rustix_error)?,
        )?;
        if moved != self.fixture_parent_identity {
            return Err(io::Error::other(
                "refusing to restore a moved fixture parent with a different identity",
            ));
        }
        match rustix::fs::statat(
            &reopened,
            &self.fixture_parent_name,
            AtFlags::SYMLINK_NOFOLLOW,
        ) {
            Err(rustix::io::Errno::NOENT) => {}
            Ok(_) => {
                return Err(io::Error::other(
                    "original fixture parent name is occupied; preserving both entries",
                ));
            }
            Err(error) => return Err(rustix_error(error)),
        }
        rustix::fs::renameat_with(
            &reopened,
            moved_name,
            &reopened,
            &self.fixture_parent_name,
            RenameFlags::NOREPLACE,
        )
        .map_err(rustix_error)?;
        self.verify_exact_tree()
    }

    pub(crate) fn cleanup(mut self) -> io::Result<()> {
        self.verify_exact_tree()?;
        let mut entries: Vec<_> = self
            .tracked
            .iter()
            .filter(|(path, _)| !path.as_os_str().is_empty())
            .map(|(path, identity)| (path.clone(), *identity))
            .collect();
        entries.sort_by(|(left, _), (right, _)| {
            right
                .components()
                .count()
                .cmp(&left.components().count())
                .then_with(|| right.cmp(left))
        });
        for (relative, expected) in entries {
            self.verify_exact_tree()?;
            let (parent, name) = self.open_parent(&relative)?;
            let current = tracked_identity(
                rustix::fs::statat(&parent, &name, AtFlags::SYMLINK_NOFOLLOW)
                    .map_err(rustix_error)?,
            )?;
            if current != expected {
                return Err(io::Error::other(format!(
                    "fixture identity changed before cleanup; preserving {}",
                    self.root_path.join(&relative).display()
                )));
            }
            remove_at(&parent, &name, expected.kind)?;
            self.tracked.remove(&relative);
        }
        self.verify_exact_tree()?;
        remove_at(
            &self.fixture_parent,
            &self.root_name,
            TrackedKind::Directory,
        )?;
        self.verify_parent()?;
        match rustix::fs::statat(
            &self.fixture_parent,
            &self.root_name,
            AtFlags::SYMLINK_NOFOLLOW,
        ) {
            Err(rustix::io::Errno::NOENT) => {}
            Ok(_) => return Err(io::Error::other("removed fixture root name reappeared")),
            Err(error) => return Err(rustix_error(error)),
        }
        remove_at(
            &self.existing_parent,
            &self.fixture_parent_name,
            TrackedKind::Directory,
        )
    }

    fn verify_parent(&self) -> io::Result<()> {
        let reopened_existing_parent = open_absolute_directory(&self.existing_parent_path)?;
        if tracked_identity(rustix::fs::fstat(&reopened_existing_parent)?)?
            != self.existing_parent_identity
            || tracked_identity(rustix::fs::fstat(&self.existing_parent)?)?
                != self.existing_parent_identity
            || tracked_identity(
                rustix::fs::statat(
                    &self.existing_parent,
                    &self.fixture_parent_name,
                    AtFlags::SYMLINK_NOFOLLOW,
                )
                .map_err(rustix_error)?,
            )? != self.fixture_parent_identity
            || tracked_identity(rustix::fs::fstat(&self.fixture_parent)?)?
                != self.fixture_parent_identity
        {
            return Err(io::Error::other(format!(
                "fixture parent path or identity changed; preserving {}",
                self.fixture_parent_path.display()
            )));
        }
        let reopened_fixture_parent =
            open_directory_at(&reopened_existing_parent, &self.fixture_parent_name)?;
        if tracked_identity(rustix::fs::fstat(&reopened_fixture_parent)?)?
            != self.fixture_parent_identity
        {
            return Err(io::Error::other(format!(
                "fixture parent reopen did not match; preserving {}",
                self.fixture_parent_path.display()
            )));
        }
        Ok(())
    }

    fn open_parent(&self, relative: &Path) -> io::Result<(OwnedFd, CString)> {
        validate_relative(relative)?;
        let mut components = relative.components().peekable();
        let mut current = self.root.try_clone()?;
        let mut prefix = PathBuf::new();
        while let Some(component) = components.next() {
            let Component::Normal(name) = component else {
                return Err(io::Error::from_raw_os_error(libc::EINVAL));
            };
            let bytes = name.as_bytes().to_vec();
            let name = component_name(name)?;
            if components.peek().is_none() {
                return Ok((current, name));
            }
            prefix.push(OsStr::from_bytes(&bytes));
            let expected = self
                .tracked
                .get(&prefix)
                .ok_or_else(|| io::Error::other("untracked fixture ancestor"))?;
            let next = open_directory_at(&current, &name)?;
            if tracked_identity(rustix::fs::fstat(&next)?)? != *expected
                || expected.kind != TrackedKind::Directory
            {
                return Err(io::Error::other("fixture ancestor identity changed"));
            }
            current = next;
        }
        Err(io::Error::from_raw_os_error(libc::EINVAL))
    }

    fn open_tracked(&self, relative: &Path) -> io::Result<OwnedFd> {
        validate_relative(relative)?;
        let expected = self
            .tracked
            .get(relative)
            .copied()
            .ok_or_else(|| io::Error::other("fixture metadata target is not tracked"))?;
        let object = if relative.as_os_str().is_empty() {
            self.root.try_clone()?
        } else {
            let (parent, name) = self.open_parent(relative)?;
            let flags = match expected.kind {
                TrackedKind::Directory => directory_flags(),
                TrackedKind::RegularFile => {
                    OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC
                }
                TrackedKind::SymbolicLink | TrackedKind::Special => {
                    return Err(io::Error::from_raw_os_error(libc::EINVAL));
                }
            };
            rustix::fs::openat(&parent, &name, flags, Mode::empty()).map_err(rustix_error)?
        };
        if tracked_identity(rustix::fs::fstat(&object).map_err(rustix_error)?)? != expected {
            return Err(io::Error::other("fixture metadata target identity changed"));
        }
        Ok(object)
    }
}

pub(crate) fn tracked_identity(metadata: rustix::fs::Stat) -> io::Result<TrackedIdentity> {
    let kind = match FileType::from_raw_mode(metadata.st_mode) {
        FileType::Directory => TrackedKind::Directory,
        FileType::RegularFile => TrackedKind::RegularFile,
        FileType::Symlink => TrackedKind::SymbolicLink,
        _ => TrackedKind::Special,
    };
    Ok(TrackedIdentity {
        device: metadata.st_dev as u64,
        inode: metadata.st_ino,
        kind,
    })
}

fn validate_relative(relative: &Path) -> io::Result<()> {
    if relative.as_os_str().is_empty() {
        return Ok(());
    }
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(io::Error::from_raw_os_error(libc::EINVAL));
    }
    Ok(())
}

fn component_name(name: &OsStr) -> io::Result<CString> {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes == b"." || bytes == b".." || bytes.contains(&b'/') {
        return Err(io::Error::from_raw_os_error(libc::EINVAL));
    }
    CString::new(bytes).map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))
}

fn directory_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}

fn fixture_mode(mode: u32) -> io::Result<Mode> {
    u16::try_from(mode)
        .map(Mode::from_raw_mode)
        .map_err(|_| io::Error::from_raw_os_error(libc::EINVAL))
}

fn open_directory_at(parent: &OwnedFd, name: &CStr) -> io::Result<OwnedFd> {
    rustix::fs::openat(parent, name, directory_flags(), Mode::empty()).map_err(rustix_error)
}

fn open_absolute_directory(path: &Path) -> io::Result<OwnedFd> {
    if !path.is_absolute() {
        return Err(io::Error::from_raw_os_error(libc::EINVAL));
    }
    let mut current =
        rustix::fs::open(Path::new("/"), directory_flags(), Mode::empty()).map_err(rustix_error)?;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                current = open_directory_at(&current, &component_name(name)?)?;
            }
            _ => return Err(io::Error::from_raw_os_error(libc::EINVAL)),
        }
    }
    Ok(current)
}

fn remove_at(parent: &OwnedFd, name: &CStr, kind: TrackedKind) -> io::Result<()> {
    let flags = if kind == TrackedKind::Directory {
        AtFlags::REMOVEDIR
    } else {
        AtFlags::empty()
    };
    rustix::fs::unlinkat(parent, name, flags).map_err(rustix_error)
}

fn write_all(file: &OwnedFd, mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        let written = rustix::io::write(file, bytes).map_err(rustix_error)?;
        if written == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "fixture write returned zero",
            ));
        }
        bytes = &bytes[written..];
    }
    Ok(())
}

fn observe_tree(root: &OwnedFd) -> io::Result<BTreeMap<PathBuf, TrackedIdentity>> {
    let root_identity = tracked_identity(rustix::fs::fstat(root)?)?;
    if root_identity.kind != TrackedKind::Directory {
        return Err(io::Error::other("fixture root is not a directory"));
    }
    let mut observed = BTreeMap::new();
    observed.insert(PathBuf::new(), root_identity);
    observe_directory(root, Path::new(""), &mut observed)?;
    Ok(observed)
}

fn observe_directory(
    directory: &OwnedFd,
    relative: &Path,
    observed: &mut BTreeMap<PathBuf, TrackedIdentity>,
) -> io::Result<()> {
    let readable = rustix::fs::openat(directory, c".", directory_flags(), Mode::empty())
        .map_err(rustix_error)?;
    let mut names = Vec::new();
    for entry in Dir::new(readable).map_err(rustix_error)? {
        let entry = entry.map_err(rustix_error)?;
        let bytes = entry.file_name().to_bytes();
        if bytes != b"." && bytes != b".." {
            names.push(bytes.to_vec());
        }
    }
    names.sort();
    for name in names {
        let child = relative.join(OsStr::from_bytes(&name));
        validate_relative(&child)?;
        let name = component_name(OsStr::from_bytes(&name))?;
        let metadata = rustix::fs::statat(directory, &name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(rustix_error)?;
        let entry_identity = tracked_identity(metadata)?;
        observed.insert(child.clone(), entry_identity);
        if entry_identity.kind == TrackedKind::Directory {
            let child_fd = open_directory_at(directory, &name)?;
            if tracked_identity(rustix::fs::fstat(&child_fd)?)? != entry_identity {
                return Err(io::Error::other("fixture directory changed during scan"));
            }
            observe_directory(&child_fd, &child, observed)?;
        }
    }
    Ok(())
}

fn rustix_error(error: rustix::io::Errno) -> io::Error {
    io::Error::from_raw_os_error(error.raw_os_error())
}
