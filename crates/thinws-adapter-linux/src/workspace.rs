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
    PortError, PortErrorKind, PreparedWorkspaceEvidence, RemovalLogRecord, WorkspaceRemoval,
    WorkspaceSpace,
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

struct RegisteredTarget {
    parent: TargetDirectory,
    target_path: PathBuf,
    isolated_path: PathBuf,
    ownership: WorkspaceOwnership,
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

    /// Locates only a creation-owned target (or its durably registered
    /// isolation path) before Application asks the process-use probe.
    pub fn inspect_removal_container(
        &self,
        lock: &LinuxLockGuard,
        layout: &LinuxDataRootLayout,
        reservation: &WorkspaceReservation,
    ) -> Result<Option<AbsolutePath>, PortError> {
        validate_data_root_lock(self, lock, layout)?;
        layout.revalidate()?;
        let registered = registered_target(layout, reservation)?;
        let selected = locate_target(&registered)?.map(|(directory, _)| directory);
        registered.parent.revalidate()?;
        if read_workspace_ownership(&layout.metadata, reservation.workspace_id())?
            != registered.ownership
        {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "Workspace ownership proof changed during removal lookup",
            ));
        }
        if let Some(directory) = &selected {
            directory.revalidate()?;
            require_btrfs_mount(
                directory,
                registered.ownership.volume_id,
                registered.ownership.mount_id,
            )?;
        }
        layout.revalidate()?;
        validate_data_root_lock(self, lock, layout)?;
        selected
            .as_ref()
            .map(|directory| absolute(&directory.path))
            .transpose()
    }

    /// Durably appends one ordinary cleanup event in the verified control root.
    pub fn append_removal_log(
        &self,
        lock: &LinuxLockGuard,
        layout: &LinuxDataRootLayout,
        record: &RemovalLogRecord<'_>,
    ) -> Result<AbsolutePath, PortError> {
        validate_data_root_lock(self, lock, layout)?;
        layout.revalidate()?;
        let path = crate::operation_log::append_removal_record(&layout.logs, record)?;
        layout.revalidate()?;
        validate_data_root_lock(self, lock, layout)?;
        Ok(path)
    }

    /// Removes only a Workspace whose registered target still exists at its
    /// creation identity. Application must authorize this after its checks.
    pub fn remove_workspace(
        &self,
        lock: &LinuxLockGuard,
        layout: &LinuxDataRootLayout,
        reservation: &WorkspaceReservation,
    ) -> Result<WorkspaceRemoval, PortError> {
        validate_data_root_lock(self, lock, layout)?;
        layout.revalidate()?;
        let mut registered = registered_target(layout, reservation)?;
        let (target, at_isolated_path) = locate_target(&registered)?.ok_or_else(|| {
            PortError::new(
                PortErrorKind::NotFound,
                "registered Workspace target is missing",
            )
        })?;
        // Keep an addressable, historically proven target until its independently
        // registered staging and rollback siblings have been cleared.
        for kind in ["staging", "trash"] {
            remove_operation_directory(self, lock, layout, &registered, &target, kind)?;
        }
        let isolated = if at_isolated_path {
            target
        } else {
            if registered.ownership.isolated_path.is_none() {
                let mut desired = registered.ownership.clone();
                desired.isolated_path = Some(absolute(&registered.isolated_path)?);
                persist_workspace_ownership(layout, &desired)?;
                registered.ownership = desired;
            }
            verify_selected_target(self, lock, layout, &registered, &target)?;
            let target_name = registered.target_path.file_name().ok_or_else(|| {
                PortError::new(
                    PortErrorKind::InvalidLayout,
                    "registered target has no name",
                )
            })?;
            let isolated_name = registered.isolated_path.file_name().ok_or_else(|| {
                PortError::new(PortErrorKind::InvalidLayout, "isolation path has no name")
            })?;
            rustix::fs::renameat_with(
                &registered.parent.fd,
                target_name,
                &registered.parent.fd,
                isolated_name,
                RenameFlags::NOREPLACE,
            )
            .map_err(|error| io_error("isolate Workspace target without replacement", error))?;
            sync_target_parent(&registered.parent)?;
            let moved = open_target_directory(&registered.isolated_path)?;
            if moved.identity != target.identity || moved.device != target.device {
                return Err(PortError::new(
                    PortErrorKind::InvalidLayout,
                    "isolated Workspace target identity changed",
                ));
            }
            moved
        };
        verify_selected_target(self, lock, layout, &registered, &isolated)?;
        let verify_scope = || verify_selected_target(self, lock, layout, &registered, &isolated);
        let root_entries = crate::destroy::remove_root_contents(
            &isolated.fd,
            isolated.file_identity(),
            registered.ownership.mount_id,
            &verify_scope,
        )?;
        verify_scope()?;
        let isolated_name = registered.isolated_path.file_name().ok_or_else(|| {
            PortError::new(PortErrorKind::InvalidLayout, "isolation path has no name")
        })?;
        rustix::fs::unlinkat(&registered.parent.fd, isolated_name, AtFlags::REMOVEDIR)
            .map_err(|error| io_error("remove isolated Workspace root", error))?;
        sync_target_parent(&registered.parent)?;
        validate_data_root_lock(self, lock, layout)?;
        registered.parent.revalidate()?;
        require_btrfs_mount(
            &registered.parent,
            registered.ownership.volume_id,
            registered.ownership.mount_id,
        )?;
        require_missing_child(&registered.parent, isolated_name)?;
        let target_name = registered.target_path.file_name().ok_or_else(|| {
            PortError::new(
                PortErrorKind::InvalidLayout,
                "registered target has no name",
            )
        })?;
        require_missing_child(&registered.parent, target_name)?;
        if read_workspace_ownership(&layout.metadata, reservation.workspace_id())?
            != registered.ownership
        {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "Workspace ownership proof changed after deletion",
            ));
        }
        layout.revalidate()?;
        Ok(WorkspaceRemoval::Removed { root_entries })
    }
}

fn registered_target(
    layout: &LinuxDataRootLayout,
    reservation: &WorkspaceReservation,
) -> Result<RegisteredTarget, PortError> {
    let ownership = read_workspace_ownership(&layout.metadata, reservation.workspace_id())?;
    if reservation.instance_id() != layout.instance_id
        || ownership.instance_id != reservation.instance_id()
        || ownership.target_path != *reservation.target_path()
        || ownership.volume_id != reservation.target_volume_id()
        || reservation.source_volume_id() != reservation.target_volume_id()
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
    let isolated_path = parent_path.join(format!(".thinws-remove-{}", reservation.workspace_id()));
    Ok(RegisteredTarget {
        parent,
        target_path,
        isolated_path,
        ownership,
    })
}

fn locate_target(
    registered: &RegisteredTarget,
) -> Result<Option<(TargetDirectory, bool)>, PortError> {
    let active = open_optional_target(&registered.parent, &registered.target_path)?;
    let isolated = open_optional_target(&registered.parent, &registered.isolated_path)?;
    if isolated.is_some() && registered.ownership.isolated_path.is_none() {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "unregistered isolated Workspace target exists",
        ));
    }
    let (directory, at_isolated_path) = match (active, isolated) {
        (Some(_), Some(_)) => {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "Workspace target exists at active and isolated paths",
            ));
        }
        (Some(directory), None) => (directory, false),
        (None, Some(directory)) => (directory, true),
        (None, None) => return Ok(None),
    };
    if directory.identity != registered.ownership.target {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "registered Workspace target identity changed",
        ));
    }
    require_btrfs_mount(
        &directory,
        registered.ownership.volume_id,
        registered.ownership.mount_id,
    )?;
    registered.parent.revalidate()?;
    Ok(Some((directory, at_isolated_path)))
}

fn verify_selected_target(
    adapter: &LinuxHostAdapter,
    lock: &LinuxLockGuard,
    layout: &LinuxDataRootLayout,
    registered: &RegisteredTarget,
    selected: &TargetDirectory,
) -> Result<(), PortError> {
    validate_data_root_lock(adapter, lock, layout)?;
    layout.revalidate()?;
    if read_workspace_ownership(&layout.metadata, registered.ownership.workspace_id)?
        != registered.ownership
    {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "Workspace ownership proof changed during deletion",
        ));
    }
    let (current, _) = locate_target(registered)?.ok_or_else(|| {
        PortError::new(
            PortErrorKind::NotFound,
            "registered Workspace target is missing",
        )
    })?;
    selected.revalidate()?;
    if !selected_target_matches(selected, &current) {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "Workspace deletion target changed",
        ));
    }
    Ok(())
}

fn selected_target_matches(selected: &TargetDirectory, current: &TargetDirectory) -> bool {
    selected.path == current.path
        && selected.identity == current.identity
        && selected.device == current.device
}

fn remove_operation_directory(
    adapter: &LinuxHostAdapter,
    lock: &LinuxLockGuard,
    layout: &LinuxDataRootLayout,
    registered: &RegisteredTarget,
    target: &TargetDirectory,
    kind: &str,
) -> Result<(), PortError> {
    let evidence = match kind {
        "staging" => &registered.ownership.staging,
        "trash" => &registered.ownership.trash,
        _ => {
            return Err(PortError::new(
                PortErrorKind::InvalidData,
                "unknown operation directory",
            ));
        }
    };
    let path = PathBuf::from(OsStr::from_bytes(evidence.path.as_bytes()));
    let Some(directory) = open_optional_target(&registered.parent, &path)? else {
        return Ok(());
    };
    if directory.identity != evidence.identity {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "registered Workspace operation directory identity changed",
        ));
    }
    require_btrfs_mount(
        &directory,
        registered.ownership.volume_id,
        registered.ownership.mount_id,
    )?;
    require_private_operation_directory(&directory)?;
    let verify_scope = || {
        verify_selected_target(adapter, lock, layout, registered, target)?;
        directory.revalidate()?;
        require_btrfs_mount(
            &directory,
            registered.ownership.volume_id,
            registered.ownership.mount_id,
        )?;
        require_private_operation_directory(&directory)
    };
    crate::destroy::remove_root_contents(
        &directory.fd,
        directory.file_identity(),
        registered.ownership.mount_id,
        &verify_scope,
    )?;
    verify_scope()?;
    let name = path.file_name().ok_or_else(|| {
        PortError::new(
            PortErrorKind::InvalidLayout,
            "operation directory has no name",
        )
    })?;
    rustix::fs::unlinkat(&registered.parent.fd, name, AtFlags::REMOVEDIR)
        .map_err(|error| io_error("remove Workspace operation directory", error))?;
    sync_target_parent(&registered.parent)
}

fn persist_workspace_ownership(
    layout: &LinuxDataRootLayout,
    desired: &WorkspaceOwnership,
) -> Result<(), PortError> {
    let current = read_workspace_ownership(&layout.metadata, desired.workspace_id)?;
    if current == *desired {
        return Ok(());
    }
    let mut previous = desired.clone();
    previous.isolated_path = None;
    if current != previous {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "Workspace ownership changed before isolation",
        ));
    }
    let bytes = encode_workspace_ownership(desired).map_err(document_error)?;
    let mut temporary =
        PrivateTemp::create(&layout.metadata, "workspace-ownership-update", &bytes)?;
    let name = ownership_name(desired.workspace_id);
    if read_workspace_ownership(&layout.metadata, desired.workspace_id)? != current {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "Workspace ownership changed before publication",
        ));
    }
    temporary.exchange_with(&name)?;
    if read_workspace_ownership(&layout.metadata, desired.workspace_id)? != *desired {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "isolated ownership publication changed",
        ));
    }
    let old = read_private_document(&layout.metadata, temporary.name())?.ok_or_else(|| {
        PortError::new(
            PortErrorKind::InvalidLayout,
            "old ownership proof is missing",
        )
    })?;
    if decode_workspace_ownership(&old).map_err(document_error)? != current {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "displaced ownership proof changed",
        ));
    }
    rustix::fs::unlinkat(&layout.metadata.fd, temporary.name(), AtFlags::empty())
        .map_err(|error| io_error("remove displaced ownership proof", error))?;
    sync_directory(&layout.metadata.fd)
}

fn sync_target_parent(parent: &TargetDirectory) -> Result<(), PortError> {
    rustix::fs::fsync(&parent.fd).map_err(|error| io_error("sync Workspace target parent", error))
}

fn open_optional_target(
    parent: &TargetDirectory,
    path: &Path,
) -> Result<Option<TargetDirectory>, PortError> {
    let name = path.file_name().ok_or_else(|| {
        PortError::new(PortErrorKind::InvalidLayout, "Workspace target has no name")
    })?;
    match rustix::fs::statat(&parent.fd, name, AtFlags::SYMLINK_NOFOLLOW) {
        Err(rustix::io::Errno::NOENT) => Ok(None),
        Err(error) => Err(io_error("inspect registered Workspace target", error)),
        Ok(_) => open_target_directory(path).map(Some),
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
    if !birthtime_is_usable(
        birth.stx_mask,
        birth.stx_btime.tv_sec,
        birth.stx_btime.tv_nsec,
    ) {
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

fn birthtime_is_usable(mask: u32, seconds: i64, nanoseconds: u32) -> bool {
    // A value without its statx evidence bit cannot establish directory ownership.
    mask & StatxFlags::BTIME.bits() != 0 && seconds > 0 && nanoseconds < 1_000_000_000
}

fn create_child(parent: &TargetDirectory, name: &OsStr) -> Result<TargetDirectory, PortError> {
    create_child_with_hook(parent, name, || {})
}

fn create_child_with_hook(
    parent: &TargetDirectory,
    name: &OsStr,
    before_publish: impl FnOnce(),
) -> Result<TargetDirectory, PortError> {
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
        before_publish();
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
    require_btrfs_mount_with_hook(directory, expected_volume, expected_mount, || {})
}

fn require_btrfs_mount_with_hook(
    directory: &TargetDirectory,
    expected_volume: VolumeId,
    expected_mount: u64,
    before_probe: impl FnOnce(),
) -> Result<(), PortError> {
    directory.revalidate()?;
    before_probe();
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

#[cfg(test)]
mod tests {
    use std::env;
    use std::ffi::OsString;
    use std::os::unix::fs::PermissionsExt;
    use std::time::Duration;

    use thinws_core::{InstallationIdentity, InstanceId, UnixMillis, WorkspaceName};
    use thinws_ports::LifecycleLock;

    use super::*;

    #[test]
    fn directory_birthtime_requires_each_independent_evidence_field() {
        let birthtime = StatxFlags::BTIME.bits();
        let unrelated = StatxFlags::SIZE.bits();
        assert_eq!(birthtime & unrelated, 0);
        assert!(birthtime_is_usable(birthtime, 1, 0));
        assert!(birthtime_is_usable(birthtime | unrelated, 1, 999_999_999));
        assert!(!birthtime_is_usable(unrelated, 1, 0));
        assert!(!birthtime_is_usable(0, 1, 0));
        assert!(!birthtime_is_usable(birthtime, 0, 0));
        assert!(!birthtime_is_usable(birthtime, -1, 0));
        assert!(!birthtime_is_usable(birthtime, 1, 1_000_000_000));
    }

    #[test]
    fn deletion_scope_rejects_changed_ownership_proof_after_target_selection() {
        let control_root = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT")
            .expect("set THINWS_LINUX_EXT4_TEST_ROOT to a writable ext4 test directory");
        let target_root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs test directory");
        let control_fixture = tempfile::Builder::new()
            .prefix("thinws-scope-control-")
            .tempdir_in(control_root)
            .unwrap();
        let target_fixture = tempfile::Builder::new()
            .prefix("thinws-scope-target-")
            .tempdir_in(target_root)
            .unwrap();
        let control = control_fixture.path().join("control");
        for path in [&control, &control.join("metadata"), &control.join("logs")] {
            std::fs::create_dir(path).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let database = control.join("metadata/state.db");
        std::fs::write(&database, b"").unwrap();
        std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o600)).unwrap();

        let adapter = LinuxHostAdapter::new(&control).unwrap();
        let control_volume = LinuxPlatformProbe
            .inspect_path(&absolute(&control).unwrap())
            .unwrap()
            .filesystem()
            .volume_id()
            .known()
            .copied()
            .unwrap();
        let identity = InstallationIdentity::new(
            "01890a5d-ac96-774b-bd5b-55c7b8d09f33"
                .parse::<InstanceId>()
                .unwrap(),
            absolute(&control).unwrap(),
            control_volume,
        );
        let layout = adapter.validate_layout(&identity).unwrap();
        let lock = adapter
            .acquire_data_root(identity.data_root(), Duration::from_millis(200))
            .unwrap();
        let workspace_id = "ws_01890a5d-ac96-774b-bd5b-55c7b8d09f34"
            .parse::<WorkspaceId>()
            .unwrap();
        let source = target_fixture.path().join("source");
        std::fs::create_dir(&source).unwrap();
        let target = target_fixture.path().join("copy");
        adapter
            .prepare_workspace(&lock, &layout, workspace_id, &absolute(&target).unwrap())
            .unwrap();
        let target_volume = LinuxPlatformProbe
            .inspect_path(&absolute(&source).unwrap())
            .unwrap()
            .filesystem()
            .volume_id()
            .known()
            .copied()
            .unwrap();
        let reservation = WorkspaceReservation::new(
            workspace_id,
            identity.instance_id(),
            "scope-check".parse::<WorkspaceName>().unwrap(),
            absolute(&source).unwrap(),
            absolute(&target).unwrap(),
            target_volume,
            target_volume,
            false,
            UnixMillis::new(1_700_000_000_000).unwrap(),
        );
        let registered = registered_target(&layout, &reservation).unwrap();
        let (selected, _) = locate_target(&registered).unwrap().unwrap();
        verify_selected_target(&adapter, &lock, &layout, &registered, &selected).unwrap();

        let mut forged = registered.ownership.clone();
        forged.staging.identity.inode += 1;
        let ownership_path = control.join(format!("metadata/ownership-{workspace_id}.toml"));
        std::fs::write(
            &ownership_path,
            encode_workspace_ownership(&forged).unwrap(),
        )
        .unwrap();
        assert_eq!(
            verify_selected_target(&adapter, &lock, &layout, &registered, &selected)
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );
        assert!(target.is_dir());
    }

    #[test]
    fn ready_workspace_rejects_individual_proof_mismatches() {
        let control_root = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT")
            .expect("set THINWS_LINUX_EXT4_TEST_ROOT to a writable ext4 test directory");
        let target_root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs test directory");
        let control_fixture = tempfile::Builder::new()
            .prefix("thinws-prepared-proof-control-")
            .tempdir_in(control_root)
            .unwrap();
        let target_fixture = tempfile::Builder::new()
            .prefix("thinws-prepared-proof-target-")
            .tempdir_in(target_root)
            .unwrap();
        let control = control_fixture.path().join("control");
        for path in [&control, &control.join("metadata"), &control.join("logs")] {
            std::fs::create_dir(path).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let database = control.join("metadata/state.db");
        std::fs::write(&database, b"").unwrap();
        std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o600)).unwrap();

        let adapter = LinuxHostAdapter::new(&control).unwrap();
        let control_volume = LinuxPlatformProbe
            .inspect_path(&absolute(&control).unwrap())
            .unwrap()
            .filesystem()
            .volume_id()
            .known()
            .copied()
            .unwrap();
        let identity = InstallationIdentity::new(
            "01890a5d-ac96-774b-bd5b-55c7b8d09f36"
                .parse::<InstanceId>()
                .unwrap(),
            absolute(&control).unwrap(),
            control_volume,
        );
        let layout = adapter.validate_layout(&identity).unwrap();
        let lock = adapter
            .acquire_data_root(identity.data_root(), Duration::from_millis(200))
            .unwrap();
        let workspace_id = "ws_01890a5d-ac96-774b-bd5b-55c7b8d09f37"
            .parse::<WorkspaceId>()
            .unwrap();
        let target = target_fixture.path().join("working-copy");
        let prepared = adapter
            .prepare_workspace(&lock, &layout, workspace_id, &absolute(&target).unwrap())
            .unwrap();
        let original = prepared.ownership.clone();
        let proof = control.join(format!("metadata/ownership-{workspace_id}.toml"));
        prepared.revalidate().unwrap();

        adapter
            .clear_workspace_incomplete(&lock, &layout, prepared)
            .unwrap();
        let registered = WorkspaceReservation::new(
            workspace_id,
            identity.instance_id(),
            "ready-check".parse::<WorkspaceName>().unwrap(),
            absolute(target_fixture.path()).unwrap(),
            absolute(&target).unwrap(),
            original.volume_id,
            original.volume_id,
            false,
            UnixMillis::new(1_700_000_000_000).unwrap(),
        );
        assert_eq!(
            adapter
                .validate_ready_workspace(&layout, &registered)
                .unwrap(),
            absolute(&target).unwrap()
        );
        for case in 0..3 {
            let mut altered = original.clone();
            match case {
                0 => {
                    altered.instance_id = "01890a5d-ac96-774b-bd5b-55c7b8d09f38"
                        .parse::<InstanceId>()
                        .unwrap()
                }
                1 => {
                    altered.target_path =
                        absolute(&target_fixture.path().join("other-copy")).unwrap()
                }
                2 => {
                    altered.isolated_path = Some(
                        absolute(
                            &target_fixture
                                .path()
                                .join(format!(".thinws-remove-{workspace_id}")),
                        )
                        .unwrap(),
                    )
                }
                _ => unreachable!(),
            }
            std::fs::write(&proof, encode_workspace_ownership(&altered).unwrap()).unwrap();
            assert_eq!(
                adapter
                    .validate_ready_workspace(&layout, &registered)
                    .unwrap_err()
                    .kind(),
                PortErrorKind::InvalidLayout,
                "mismatched Ready proof case {case} must be refused"
            );
            std::fs::write(&proof, encode_workspace_ownership(&original).unwrap()).unwrap();
            assert_eq!(
                adapter
                    .validate_ready_workspace(&layout, &registered)
                    .unwrap(),
                absolute(&target).unwrap()
            );
        }
    }

    #[test]
    fn selected_target_evidence_requires_each_field_to_match() {
        let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs test directory");
        let fixture = tempfile::Builder::new()
            .prefix("thinws-selected-evidence-")
            .tempdir_in(root)
            .unwrap();
        let expected = open_target_directory(fixture.path()).unwrap();
        let mut selected = open_target_directory(fixture.path()).unwrap();
        assert!(selected_target_matches(&selected, &expected));

        selected.path.push("another-name");
        assert!(!selected_target_matches(&selected, &expected));
        selected.path.pop();

        selected.identity.inode += 1;
        assert!(!selected_target_matches(&selected, &expected));
        selected.identity.inode -= 1;

        selected.device += 1;
        assert!(!selected_target_matches(&selected, &expected));
    }

    #[test]
    fn target_directory_revalidation_requires_device_and_historical_identity_independently() {
        let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs test directory");
        let fixture = tempfile::Builder::new()
            .prefix("thinws-target-revalidate-")
            .tempdir_in(root)
            .unwrap();
        let mut directory = open_target_directory(fixture.path()).unwrap();
        directory.revalidate().unwrap();

        directory.device += 1;
        assert_eq!(
            directory.revalidate().unwrap_err().kind(),
            PortErrorKind::InvalidLayout
        );
        directory.device -= 1;

        directory.identity.inode += 1;
        assert_eq!(
            directory.revalidate().unwrap_err().kind(),
            PortErrorKind::InvalidLayout
        );
        directory.identity.inode -= 1;
        directory.revalidate().unwrap();
    }

    #[test]
    fn target_directory_revalidation_rejects_a_same_device_path_replacement() {
        let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs test directory");
        let fixture = tempfile::Builder::new()
            .prefix("thinws-target-replacement-")
            .tempdir_in(root)
            .unwrap();
        let target = fixture.path().join("target");
        std::fs::create_dir(&target).unwrap();
        let held = open_target_directory(&target).unwrap();
        held.revalidate().unwrap();

        std::fs::rename(&target, fixture.path().join("displaced")).unwrap();
        std::fs::create_dir(&target).unwrap();
        let replacement = open_target_directory(&target).unwrap();
        assert_eq!(held.device, replacement.device);
        assert_ne!(held.identity, replacement.identity);
        assert_eq!(
            held.revalidate().unwrap_err().kind(),
            PortErrorKind::InvalidLayout
        );
    }

    #[test]
    fn btrfs_mount_check_rejects_wrong_volume_and_mount_independently() {
        let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs test directory");
        let fixture = tempfile::Builder::new()
            .prefix("thinws-btrfs-mount-identity-")
            .tempdir_in(root)
            .unwrap();
        let directory = open_target_directory(fixture.path()).unwrap();
        let report = LinuxPlatformProbe
            .inspect_path(&absolute(fixture.path()).unwrap())
            .unwrap();
        let (volume, mount) = btrfs_identity(&report).unwrap();
        require_btrfs_mount(&directory, volume, mount).unwrap();

        let first: VolumeId = "550e8400-e29b-41d4-a716-446655440000".parse().unwrap();
        let second: VolumeId = "550e8400-e29b-41d4-a716-446655440001".parse().unwrap();
        let other_volume = if volume == first { second } else { first };
        assert_eq!(
            require_btrfs_mount(&directory, other_volume, mount)
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );
        assert_eq!(
            require_btrfs_mount(&directory, volume, mount.wrapping_add(1))
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );
    }

    #[test]
    fn btrfs_mount_check_rejects_a_path_replaced_before_probe() {
        let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs test directory");
        let fixture = tempfile::Builder::new()
            .prefix("thinws-btrfs-mount-race-")
            .tempdir_in(root)
            .unwrap();
        let target = fixture.path().join("target");
        let displaced = fixture.path().join("displaced");
        std::fs::create_dir(&target).unwrap();
        let held = open_target_directory(&target).unwrap();
        let initial_report = LinuxPlatformProbe
            .inspect_path(&absolute(&target).unwrap())
            .unwrap();
        let (volume, mount) = btrfs_identity(&initial_report).unwrap();

        let error = require_btrfs_mount_with_hook(&held, volume, mount, || {
            std::fs::rename(&target, &displaced).unwrap();
            std::fs::create_dir(&target).unwrap();
        })
        .unwrap_err();
        assert_eq!(error.kind(), PortErrorKind::InvalidLayout);
        let replacement = open_target_directory(&target).unwrap();
        assert_eq!(replacement.device, held.device);
        assert_ne!(replacement.file_identity(), held.file_identity());
        assert!(displaced.is_dir());
    }

    #[test]
    fn staged_child_cleanup_removes_only_the_original_directory() {
        let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs test directory");
        let fixture = tempfile::Builder::new()
            .prefix("thinws-staged-child-cleanup-")
            .tempdir_in(root)
            .unwrap();
        let parent = open_target_directory(fixture.path()).unwrap();
        let name = ".thinws-dir.tmp-owned";
        let staged_path = fixture.path().join(name);

        std::fs::create_dir(&staged_path).unwrap();
        let staged = open_target_directory(&staged_path).unwrap();
        cleanup_staged_child(&parent, name, &staged);
        assert!(!staged_path.exists());

        std::fs::create_dir(&staged_path).unwrap();
        let held = open_target_directory(&staged_path).unwrap();
        let displaced = fixture.path().join("displaced-staged-child");
        std::fs::rename(&staged_path, &displaced).unwrap();
        std::fs::create_dir(&staged_path).unwrap();
        std::fs::write(staged_path.join("foreign"), b"leave in place").unwrap();
        cleanup_staged_child(&parent, name, &held);
        assert_eq!(
            std::fs::read(staged_path.join("foreign")).unwrap(),
            b"leave in place"
        );
        assert!(displaced.is_dir());
    }

    #[test]
    fn child_publish_collision_preserves_the_other_directory_and_cleans_our_stage() {
        let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs test directory");
        let fixture = tempfile::Builder::new()
            .prefix("thinws-child-publish-collision-")
            .tempdir_in(root)
            .unwrap();
        let parent = open_target_directory(fixture.path()).unwrap();
        let target = fixture.path().join("copy");

        let error = create_child_with_hook(&parent, OsStr::new("copy"), || {
            std::fs::create_dir(&target).unwrap();
            std::fs::write(target.join("keep"), b"other process").unwrap();
        })
        .unwrap_err();

        assert_eq!(error.kind(), PortErrorKind::NotEmpty);
        assert_eq!(
            std::fs::read(target.join("keep")).unwrap(),
            b"other process"
        );
        let names: Vec<_> = std::fs::read_dir(fixture.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, [OsString::from("copy")]);
    }

    #[test]
    fn path_containment_requires_a_component_boundary() {
        for (parent, child, expected) in [
            (b"/".as_slice(), b"/work".as_slice(), true),
            (b"/work".as_slice(), b"/work".as_slice(), true),
            (b"/work".as_slice(), b"/work/project".as_slice(), true),
            (b"/work".as_slice(), b"/worker".as_slice(), false),
            (b"/work/project".as_slice(), b"/work".as_slice(), false),
            (b"/work".as_slice(), b"/else/project".as_slice(), false),
        ] {
            assert_eq!(
                contains_path(parent, child),
                expected,
                "parent={parent:?}, child={child:?}"
            );
        }
    }

    #[test]
    fn operation_directory_rejects_a_mode_change_without_other_identity_changes() {
        let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs test directory");
        let fixture = tempfile::Builder::new()
            .prefix("thinws-operation-mode-")
            .tempdir_in(root)
            .unwrap();
        std::fs::set_permissions(fixture.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let directory = open_target_directory(fixture.path()).unwrap();
        require_private_operation_directory(&directory).unwrap();

        std::fs::set_permissions(fixture.path(), std::fs::Permissions::from_mode(0o750)).unwrap();
        assert_eq!(
            require_private_operation_directory(&directory)
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );

        std::fs::set_permissions(fixture.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        require_private_operation_directory(&directory).unwrap();
    }

    #[test]
    fn workspace_lifecycle_lock_requires_scope_adapter_and_directory_match() {
        let root = env::var_os("THINWS_LINUX_EXT4_TEST_ROOT")
            .expect("set THINWS_LINUX_EXT4_TEST_ROOT to a writable ext4 test directory");
        let fixture = tempfile::Builder::new()
            .prefix("thinws-workspace-lock-")
            .tempdir_in(root)
            .unwrap();
        let control = fixture.path().join("control");
        let other_control = fixture.path().join("other-control");
        for path in [
            &control,
            &control.join("metadata"),
            &control.join("logs"),
            &other_control,
        ] {
            std::fs::create_dir(path).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let database = control.join("metadata/state.db");
        std::fs::write(&database, b"").unwrap();
        std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o600)).unwrap();

        let adapter = LinuxHostAdapter::new(&control).unwrap();
        let control_volume = LinuxPlatformProbe
            .inspect_path(&absolute(&control).unwrap())
            .unwrap()
            .filesystem()
            .volume_id()
            .known()
            .copied()
            .unwrap();
        let identity = InstallationIdentity::new(
            "01890a5d-ac96-774b-bd5b-55c7b8d09f35"
                .parse::<InstanceId>()
                .unwrap(),
            absolute(&control).unwrap(),
            control_volume,
        );
        let layout = adapter.validate_layout(&identity).unwrap();
        let bootstrap = adapter
            .acquire_bootstrap(Duration::from_millis(200))
            .unwrap();
        assert_eq!(
            validate_data_root_lock(&adapter, &bootstrap, &layout)
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );
        drop(bootstrap);

        let lock = adapter
            .acquire_data_root(identity.data_root(), Duration::from_millis(200))
            .unwrap();
        validate_data_root_lock(&adapter, &lock, &layout).unwrap();

        let other_adapter = LinuxHostAdapter::new(&other_control).unwrap();
        assert_eq!(
            validate_data_root_lock(&other_adapter, &lock, &layout)
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );
        let other_lock = other_adapter
            .acquire_data_root(
                &absolute(&other_control).unwrap(),
                Duration::from_millis(200),
            )
            .unwrap();
        assert_eq!(
            validate_data_root_lock(&adapter, &other_lock, &layout)
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );
    }
}
