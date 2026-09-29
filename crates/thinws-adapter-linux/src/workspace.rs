use std::ffi::OsStr;
use std::fs::File;
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use rustix::fs::{AtFlags, Mode, OFlags, RenameFlags, StatxFlags};
use thinws_core::{
    AbsolutePath, FileIdentity, PathResolution, SupportState, VolumeId, WorkspaceId,
    WorkspaceReservation,
};
use thinws_ports::{
    DataRootLayoutEvidence, LifecycleLockGuard, LifecycleScope, PlatformProbe, PortConflict,
    PortError, PortErrorKind, PreparedWorkspaceEvidence, WorkspaceSpace,
};

use crate::control::{document_error, read_private_document};
use crate::document::{
    HistoricalDirectoryIdentity, OperationDirectoryOwnership, WorkspaceOwnership,
    decode_workspace_ownership, encode_workspace_ownership,
};
use crate::lock::{LinuxLockGuard, PrivateDirectory, revalidate_private_directory};
use crate::publication::{PrivateTemp, sync_directory, validate_private_file};
use crate::{LinuxDataRootLayout, LinuxHostAdapter, LinuxPlatformProbe};

const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);

#[derive(Debug)]
struct TargetDirectory {
    fd: OwnedFd,
    path: PathBuf,
    identity: HistoricalDirectoryIdentity,
    device: u64,
}

impl TargetDirectory {
    fn revalidate(&self) -> Result<(), PortError> {
        let reopened = open_target_directory(&self.path)?;
        if self.device != reopened.device || self.identity != reopened.identity {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "Workspace directory identity changed",
            ));
        }
        let held = rustix::fs::fstat(&self.fd)
            .map_err(|error| io_error("inspect held Workspace directory", error))?;
        if (held.st_dev, held.st_ino) != (self.device, self.identity.inode) {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "held Workspace directory identity changed",
            ));
        }
        Ok(())
    }

    fn file_identity(&self) -> FileIdentity {
        FileIdentity::new(self.device, self.identity.inode)
    }
}

/// Held evidence for a newly created ordinary Btrfs Workspace container.
#[derive(Debug)]
pub struct LinuxPreparedWorkspace {
    parent: TargetDirectory,
    root: TargetDirectory,
    staging: TargetDirectory,
    trash: TargetDirectory,
    ownership_metadata: PrivateDirectory,
    ownership_file: File,
    ownership_file_identity: (u64, u64),
    ownership: WorkspaceOwnership,
    target_root: AbsolutePath,
    staging_root: AbsolutePath,
    trash_root: AbsolutePath,
}

impl PreparedWorkspaceEvidence for LinuxPreparedWorkspace {
    fn target_root(&self) -> &AbsolutePath {
        &self.target_root
    }

    fn staging_root(&self) -> &AbsolutePath {
        &self.staging_root
    }

    fn trash_root(&self) -> &AbsolutePath {
        &self.trash_root
    }

    fn target_identity(&self) -> FileIdentity {
        self.root.file_identity()
    }

    fn revalidate(&self) -> Result<(), PortError> {
        self.parent.revalidate()?;
        for child in [&self.root, &self.staging, &self.trash] {
            child.revalidate()?;
            if child.path.parent() != Some(self.parent.path.as_path()) {
                return Err(PortError::new(
                    PortErrorKind::InvalidLayout,
                    "Workspace directory moved outside its registered parent",
                ));
            }
            require_btrfs_mount(child, self.ownership.volume_id, self.ownership.mount_id)?;
        }
        for private in [&self.staging, &self.trash] {
            require_private_operation_directory(private)?;
        }
        require_btrfs_mount(
            &self.parent,
            self.ownership.volume_id,
            self.ownership.mount_id,
        )?;
        if self.ownership.parent != self.parent.identity
            || self.ownership.target != self.root.identity
            || self.ownership.staging.identity != self.staging.identity
            || self.ownership.trash.identity != self.trash.identity
            || self.ownership.target_path != self.target_root
            || self.ownership.staging.path != self.staging_root
            || self.ownership.trash.path != self.trash_root
        {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "Workspace ownership no longer matches the held directories",
            ));
        }
        revalidate_private_directory(&self.ownership_metadata)?;
        let name = ownership_name(self.ownership.workspace_id);
        validate_private_file(
            &self.ownership_metadata,
            &self.ownership_file,
            &name,
            self.ownership_file_identity,
        )?;
        let current =
            read_workspace_ownership(&self.ownership_metadata, self.ownership.workspace_id)?;
        if current != self.ownership {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "Workspace ownership proof changed",
            ));
        }
        Ok(())
    }
}

impl LinuxHostAdapter {
    /// Creates the exact absent Btrfs target and sibling operation directories,
    /// then persists their creation-time ownership under the control root.
    pub fn prepare_workspace(
        &self,
        lock: &LinuxLockGuard,
        layout: &LinuxDataRootLayout,
        workspace_id: WorkspaceId,
        target: &AbsolutePath,
    ) -> Result<LinuxPreparedWorkspace, PortError> {
        validate_data_root_lock(self, lock, layout)?;
        layout.revalidate()?;
        let target_path = PathBuf::from(OsStr::from_bytes(target.as_bytes()));
        let parent_path = target_path
            .parent()
            .ok_or_else(|| PortError::new(PortErrorKind::InvalidData, "target has no parent"))?;
        let target_name = target_path.file_name().ok_or_else(|| {
            PortError::new(PortErrorKind::InvalidData, "target has no final component")
        })?;
        let parent = open_target_directory(parent_path)?;
        let parent_report = LinuxPlatformProbe.inspect_path(&absolute(parent_path)?)?;
        if parent_report.resolution() != PathResolution::ExistingDirectory
            || parent_report
                .ancestry()
                .last()
                .map(|entry| entry.identity())
                != Some(parent.file_identity())
        {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "Workspace target parent changed before creation",
            ));
        }
        if parent_report
            .ancestry()
            .iter()
            .any(|entry| entry.identity() == layout.data_root.file_identity())
            || contains_path(
                target.as_bytes(),
                layout.data_root.path().as_os_str().as_bytes(),
            )
            || contains_path(
                layout.data_root.path().as_os_str().as_bytes(),
                target.as_bytes(),
            )
        {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "Workspace target overlaps the control root",
            ));
        }
        let (volume_id, mount_id) = btrfs_identity(&parent_report)?;
        if parent_report.writability() != SupportState::Supported {
            return Err(PortError::new(
                PortErrorKind::CapabilityUnavailable,
                "Workspace target parent is not writable",
            ));
        }
        let staging_name = format!(".thinws-staging-{workspace_id}");
        let trash_name = format!(".thinws-trash-{workspace_id}");
        let isolated_name = format!(".thinws-remove-{workspace_id}");
        if [
            staging_name.as_str(),
            trash_name.as_str(),
            isolated_name.as_str(),
        ]
        .iter()
        .any(|reserved| target_name == OsStr::new(*reserved))
        {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "Workspace target collides with an operation directory name",
            ));
        }
        let metadata = crate::lock::open_private_directory(layout.metadata.path())?;
        if !metadata.same_identity(&layout.metadata) {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "Workspace ownership metadata directory changed",
            ));
        }
        if read_private_document(&metadata, &ownership_name(workspace_id))?.is_some() {
            return Err(PortError::conflict(
                "Workspace ownership proof already exists",
                PortConflict::WorkspaceId,
            ));
        }
        for name in [
            target_name,
            OsStr::new(&staging_name),
            OsStr::new(&trash_name),
        ] {
            require_absent_child(&parent, name)?;
        }
        let root = create_child(&parent, target_name)?;
        let staging = create_child(&parent, OsStr::new(&staging_name))?;
        let trash = create_child(&parent, OsStr::new(&trash_name))?;
        for child in [&root, &staging, &trash] {
            require_btrfs_mount(child, volume_id, mount_id)?;
        }
        let staging_root = absolute(&staging.path)?;
        let trash_root = absolute(&trash.path)?;
        let ownership = WorkspaceOwnership {
            instance_id: layout.instance_id,
            workspace_id,
            volume_id,
            mount_id,
            target_path: target.clone(),
            parent: parent.identity,
            target: root.identity,
            staging: OperationDirectoryOwnership {
                path: staging_root.clone(),
                identity: staging.identity,
            },
            trash: OperationDirectoryOwnership {
                path: trash_root.clone(),
                identity: trash.identity,
            },
            isolated_path: None,
        };
        let bytes = encode_workspace_ownership(&ownership).map_err(document_error)?;
        let (ownership_file, ownership_file_identity) =
            PrivateTemp::create(&metadata, "workspace-ownership", &bytes)?
                .publish_noreplace(&ownership_name(workspace_id))
                .map_err(|error| {
                    if error.kind() == PortErrorKind::Conflict {
                        PortError::conflict(
                            "Workspace ownership proof already exists",
                            PortConflict::WorkspaceId,
                        )
                    } else {
                        error
                    }
                })?;
        let prepared = LinuxPreparedWorkspace {
            parent,
            root,
            staging,
            trash,
            ownership_metadata: metadata,
            ownership_file,
            ownership_file_identity,
            ownership,
            target_root: target.clone(),
            staging_root,
            trash_root,
        };
        prepared.revalidate()?;
        layout.revalidate()?;
        Ok(prepared)
    }

    /// Removes only the empty, ownership-verified operation directories after
    /// materialization; the ordinary target and historical proof remain.
    pub fn clear_workspace_incomplete(
        &self,
        lock: &LinuxLockGuard,
        layout: &LinuxDataRootLayout,
        prepared: LinuxPreparedWorkspace,
    ) -> Result<(), PortError> {
        validate_data_root_lock(self, lock, layout)?;
        layout.revalidate()?;
        prepared.revalidate()?;
        // Preflight both before removing either, so a routine NotEmpty refusal
        // leaves the original incomplete layout intact.
        for directory in [&prepared.staging, &prepared.trash] {
            require_empty_directory(directory)?;
        }
        for directory in [&prepared.staging, &prepared.trash] {
            validate_data_root_lock(self, lock, layout)?;
            prepared.parent.revalidate()?;
            directory.revalidate()?;
            require_btrfs_mount(
                directory,
                prepared.ownership.volume_id,
                prepared.ownership.mount_id,
            )?;
            require_private_operation_directory(directory)?;
            require_empty_directory(directory)?;
            let name = directory.path.file_name().ok_or_else(|| {
                PortError::new(
                    PortErrorKind::InvalidLayout,
                    "Workspace operation directory has no name",
                )
            })?;
            rustix::fs::unlinkat(&prepared.parent.fd, name, AtFlags::REMOVEDIR).map_err(
                |error| io_error("remove completed Workspace operation directory", error),
            )?;
            sync_directory(&prepared.parent.fd)?;
        }
        prepared.root.revalidate()?;
        require_btrfs_mount(
            &prepared.root,
            prepared.ownership.volume_id,
            prepared.ownership.mount_id,
        )?;
        layout.revalidate()
    }

    /// Checks one registered ordinary target against its persistent creation
    /// identity without taking a lifecycle lock or changing product state.
    pub fn validate_ready_workspace(
        &self,
        layout: &LinuxDataRootLayout,
        reservation: &WorkspaceReservation,
    ) -> Result<AbsolutePath, PortError> {
        layout.revalidate()?;
        let ownership = read_workspace_ownership(&layout.metadata, reservation.workspace_id())?;
        if reservation.instance_id() != layout.instance_id
            || ownership.instance_id != reservation.instance_id()
            || ownership.target_path != *reservation.target_path()
            || ownership.volume_id != reservation.target_volume_id()
            || reservation.source_volume_id() != reservation.target_volume_id()
            || ownership.isolated_path.is_some()
        {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "Workspace registration and ownership proof differ",
            ));
        }
        let target_path = PathBuf::from(OsStr::from_bytes(reservation.target_path().as_bytes()));
        let parent_path = target_path.parent().ok_or_else(|| {
            PortError::new(
                PortErrorKind::InvalidLayout,
                "registered target has no parent",
            )
        })?;
        let parent = open_target_directory(parent_path)?;
        if parent.identity != ownership.parent {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "registered Workspace parent identity changed",
            ));
        }
        require_btrfs_mount(&parent, ownership.volume_id, ownership.mount_id)?;
        let target_name = target_path.file_name().ok_or_else(|| {
            PortError::new(
                PortErrorKind::InvalidLayout,
                "registered target has no name",
            )
        })?;
        match rustix::fs::statat(&parent.fd, target_name, AtFlags::SYMLINK_NOFOLLOW) {
            Err(rustix::io::Errno::NOENT) => {
                return Err(PortError::new(
                    PortErrorKind::NotFound,
                    "registered Ready target is missing",
                ));
            }
            Err(error) => return Err(io_error("inspect registered Ready target", error)),
            Ok(_) => {}
        }
        let root = open_target_directory(&target_path)?;
        if root.identity != ownership.target {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "registered Workspace target identity changed",
            ));
        }
        require_btrfs_mount(&root, ownership.volume_id, ownership.mount_id)?;
        for operation in [&ownership.staging, &ownership.trash] {
            let name = Path::new(OsStr::from_bytes(operation.path.as_bytes()))
                .file_name()
                .ok_or_else(|| {
                    PortError::new(PortErrorKind::InvalidLayout, "operation path has no name")
                })?;
            require_missing_child(&parent, name)?;
        }
        if read_workspace_ownership(&layout.metadata, reservation.workspace_id())? != ownership {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "Ready ownership proof changed during validation",
            ));
        }
        root.revalidate()?;
        layout.revalidate()?;
        Ok(reservation.target_path().clone())
    }

    /// Measures only an ownership-verified Ready target; an incomplete inner
    /// scan is Unknown while invalid target ownership remains an error.
    pub fn measure_ready_workspace_space(
        &self,
        layout: &LinuxDataRootLayout,
        reservation: &WorkspaceReservation,
    ) -> Result<WorkspaceSpace, PortError> {
        let expected_path = self.validate_ready_workspace(layout, reservation)?;
        let ownership = read_workspace_ownership(&layout.metadata, reservation.workspace_id())?;
        let root =
            open_target_directory(&PathBuf::from(OsStr::from_bytes(expected_path.as_bytes())))?;
        if root.identity != ownership.target {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "Workspace target identity changed before space scan",
            ));
        }
        require_btrfs_mount(&root, ownership.volume_id, ownership.mount_id)?;
        let measurement = crate::space::measure_root(&root.fd, ownership.mount_id);
        root.revalidate()?;
        if self.validate_ready_workspace(layout, reservation)? != expected_path {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "Ready Workspace changed during space scan",
            ));
        }
        Ok(measurement)
    }
}

fn validate_data_root_lock(
    adapter: &LinuxHostAdapter,
    lock: &LinuxLockGuard,
    layout: &LinuxDataRootLayout,
) -> Result<(), PortError> {
    if lock.scope() != LifecycleScope::DataRoot
        || layout.data_root.path() != adapter.control_root()
        || !lock.protects_directory(&layout.data_root)
    {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "require matching Linux Workspace lifecycle lock",
        ));
    }
    lock.revalidate()
}

fn read_workspace_ownership(
    metadata: &PrivateDirectory,
    workspace_id: WorkspaceId,
) -> Result<WorkspaceOwnership, PortError> {
    let bytes =
        read_private_document(metadata, &ownership_name(workspace_id))?.ok_or_else(|| {
            PortError::new(
                PortErrorKind::InvalidLayout,
                "Workspace ownership proof is missing",
            )
        })?;
    let ownership = decode_workspace_ownership(&bytes).map_err(|error| {
        PortError::new(
            PortErrorKind::InvalidLayout,
            "decode Workspace ownership proof",
        )
        .with_source(error)
    })?;
    if ownership.workspace_id != workspace_id {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "Workspace ownership ID does not match",
        ));
    }
    Ok(ownership)
}

fn require_empty_directory(directory: &TargetDirectory) -> Result<(), PortError> {
    directory.revalidate()?;
    let mut entries = rustix::fs::Dir::read_from(&directory.fd)
        .map_err(|error| io_error("read Workspace operation directory", error))?;
    for entry in &mut entries {
        let entry = entry.map_err(|error| io_error("read Workspace operation entry", error))?;
        if !matches!(entry.file_name().to_bytes(), b"." | b"..") {
            return Err(PortError::new(
                PortErrorKind::NotEmpty,
                "Workspace operation directory is not empty",
            ));
        }
    }
    directory.revalidate()
}

fn require_missing_child(parent: &TargetDirectory, name: &OsStr) -> Result<(), PortError> {
    parent.revalidate()?;
    match rustix::fs::statat(&parent.fd, name, AtFlags::SYMLINK_NOFOLLOW) {
        Err(rustix::io::Errno::NOENT) => parent.revalidate(),
        Err(error) => Err(io_error("inspect registered operation directory", error)),
        Ok(_) => Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "operation directory remains beside Ready target",
        )),
    }
}

fn ownership_name(workspace_id: WorkspaceId) -> String {
    format!("ownership-{workspace_id}.toml")
}

fn open_target_directory(path: &Path) -> Result<TargetDirectory, PortError> {
    let absolute = absolute(path)?;
    let mut fd = rustix::fs::open("/", DIRECTORY_FLAGS, Mode::empty())
        .map_err(|error| io_error("open Workspace path root", error))?;
    for component in absolute.as_bytes()[1..].split(|byte| *byte == b'/') {
        if component.is_empty() {
            continue;
        }
        fd = rustix::fs::openat(
            &fd,
            OsStr::from_bytes(component),
            DIRECTORY_FLAGS,
            Mode::empty(),
        )
        .map_err(|error| {
            PortError::new(
                PortErrorKind::InvalidLayout,
                "open Workspace path without links",
            )
            .with_source(error)
        })?;
    }
    let stat =
        rustix::fs::fstat(&fd).map_err(|error| io_error("inspect Workspace directory", error))?;
    let birth = rustix::fs::statx(&fd, "", AtFlags::EMPTY_PATH, StatxFlags::BTIME)
        .map_err(|error| io_error("inspect Workspace directory birthtime", error))?;
    if birth.stx_mask & StatxFlags::BTIME.bits() == 0
        || birth.stx_btime.tv_sec <= 0
        || birth.stx_btime.tv_nsec >= 1_000_000_000
    {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "Workspace directory birthtime is unavailable",
        ));
    }
    Ok(TargetDirectory {
        fd,
        path: path.to_path_buf(),
        identity: HistoricalDirectoryIdentity {
            inode: stat.st_ino,
            birth_seconds: birth.stx_btime.tv_sec,
            birth_nanoseconds: birth.stx_btime.tv_nsec,
        },
        device: stat.st_dev,
    })
}

fn create_child(parent: &TargetDirectory, name: &OsStr) -> Result<TargetDirectory, PortError> {
    parent.revalidate()?;
    require_absent_child(parent, name)?;
    for _ in 0..32 {
        let temporary_name = format!(".thinws-dir.tmp-{}", uuid::Uuid::now_v7());
        match rustix::fs::mkdirat(
            &parent.fd,
            temporary_name.as_str(),
            Mode::from_bits_retain(0o700),
        ) {
            Err(rustix::io::Errno::EXIST) => continue,
            Err(error) => return Err(io_error("stage Workspace directory", error)),
            Ok(()) => {}
        }
        sync_directory(&parent.fd)?;
        let mut staged = open_target_directory(&parent.path.join(&temporary_name))?;
        require_private_operation_directory(&staged)?;
        parent.revalidate()?;
        match rustix::fs::renameat_with(
            &parent.fd,
            temporary_name.as_str(),
            &parent.fd,
            name,
            RenameFlags::NOREPLACE,
        ) {
            Ok(()) => {}
            Err(error) => {
                cleanup_staged_child(parent, &temporary_name, &staged);
                return Err(if error == rustix::io::Errno::EXIST {
                    PortError::new(PortErrorKind::NotEmpty, "Workspace target became occupied")
                } else {
                    io_error("publish Workspace directory", error)
                });
            }
        }
        staged.path = parent.path.join(name);
        sync_directory(&parent.fd)?;
        staged.revalidate()?;
        parent.revalidate()?;
        return Ok(staged);
    }
    Err(PortError::new(
        PortErrorKind::Io,
        "allocate Workspace staging name",
    ))
}

fn require_absent_child(parent: &TargetDirectory, name: &OsStr) -> Result<(), PortError> {
    match rustix::fs::statat(&parent.fd, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => {
            return Err(PortError::new(
                PortErrorKind::NotEmpty,
                "Workspace target is occupied",
            ));
        }
        Err(rustix::io::Errno::NOENT) => {}
        Err(error) => return Err(io_error("inspect Workspace target leaf", error)),
    }
    Ok(())
}

fn cleanup_staged_child(parent: &TargetDirectory, name: &str, staged: &TargetDirectory) {
    if let Ok(named) = rustix::fs::statat(&parent.fd, name, AtFlags::SYMLINK_NOFOLLOW)
        && (named.st_dev, named.st_ino) == (staged.device, staged.identity.inode)
        && named.st_mode & libc::S_IFMT == libc::S_IFDIR
    {
        let _ = rustix::fs::unlinkat(&parent.fd, name, AtFlags::REMOVEDIR);
        let _ = sync_directory(&parent.fd);
    }
}

fn require_private_operation_directory(directory: &TargetDirectory) -> Result<(), PortError> {
    let stat = rustix::fs::fstat(&directory.fd)
        .map_err(|error| io_error("inspect Workspace operation directory", error))?;
    if stat.st_mode & libc::S_IFMT != libc::S_IFDIR
        || stat.st_mode & 0o7777 != 0o700
        || stat.st_uid != rustix::process::geteuid().as_raw()
    {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "Workspace operation directory owner or mode changed",
        ));
    }
    Ok(())
}

fn contains_path(parent: &[u8], child: &[u8]) -> bool {
    parent == child
        || (child.starts_with(parent) && (parent == b"/" || child.get(parent.len()) == Some(&b'/')))
}

fn require_btrfs_mount(
    directory: &TargetDirectory,
    expected_volume: VolumeId,
    expected_mount: u64,
) -> Result<(), PortError> {
    directory.revalidate()?;
    let report = LinuxPlatformProbe.inspect_path(&absolute(&directory.path)?)?;
    if report.resolution() != PathResolution::ExistingDirectory
        || report.ancestry().last().map(|entry| entry.identity()) != Some(directory.file_identity())
        || btrfs_identity(&report)? != (expected_volume, expected_mount)
    {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "Workspace Btrfs mount or directory identity changed",
        ));
    }
    Ok(())
}

fn btrfs_identity(
    report: &thinws_core::PathCapabilityReport,
) -> Result<(VolumeId, u64), PortError> {
    if report.filesystem().type_name() != "btrfs" {
        return Err(PortError::new(
            PortErrorKind::CapabilityUnavailable,
            "Workspace target parent must be on Btrfs",
        ));
    }
    let volume = report
        .filesystem()
        .volume_id()
        .known()
        .copied()
        .ok_or_else(|| {
            PortError::new(
                PortErrorKind::CapabilityUnavailable,
                "Btrfs FSID is unavailable",
            )
        })?;
    let mount = report.mount().mount_id().ok_or_else(|| {
        PortError::new(
            PortErrorKind::CapabilityUnavailable,
            "Btrfs mount ID is unavailable",
        )
    })?;
    Ok((volume, mount))
}

fn absolute(path: &Path) -> Result<AbsolutePath, PortError> {
    AbsolutePath::try_from_bytes(path.as_os_str().as_bytes().to_vec()).map_err(|error| {
        PortError::new(PortErrorKind::InvalidData, "derive Workspace path").with_source(error)
    })
}

fn io_error(
    operation: &'static str,
    error: impl std::error::Error + Send + Sync + 'static,
) -> PortError {
    PortError::new(PortErrorKind::Io, operation).with_source(error)
}
