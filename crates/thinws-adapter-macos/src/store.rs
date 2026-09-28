use std::ffi::OsStr;
use std::fs::File;
use std::os::unix::ffi::OsStrExt;

use rustix::fs::{AtFlags, RenameFlags};

use thinws_core::{
    AbsolutePath, InstallationIdentity, RootMarker, RootMarkerState, VolumeId, WorkspaceId,
    WorkspaceReservation,
};
use thinws_ports::{
    BootstrapStore, DataRootLayoutEvidence, LifecycleLockGuard, LifecycleScope, PlatformProbe,
    PortConflict, PortError, PortErrorKind, PreparedDataRootEvidence, PreparedWorkspaceEvidence,
    PublishResult, RemovalLogRecord, WorkspaceRemoval, WorkspaceSpace,
};

use crate::destroy::remove_root_contents;
use crate::document::{
    DocumentError, OperationDirectoryOwnership, WorkspaceOwnership, decode_bootstrap_config,
    decode_root_marker, decode_workspace_ownership, encode_config, encode_marker,
    encode_workspace_ownership,
};
use crate::filesystem::{
    FileIdentity, NoReplaceError, PrivateTemp, ValidatedDirectory, create_private_child_directory,
    create_private_file, create_target_child_directory, duplicate_validated_directory,
    entry_identity, historical_directory_identity, historical_target_parent_identity, io_error,
    open_owned_child_directory, open_private_child_directory, open_private_directory,
    open_private_directory_optional, open_private_file, open_target_parent, path_from_absolute,
    prepare_private_directory, read_private_file, require_empty_directory, revalidate_directory,
    revalidate_target_child, revalidate_target_parent, sync_directory, unlink_entry,
    validate_file_entry,
};
use crate::space::measure_root;
use crate::volume::decode_volume_id;
use crate::{MacOsHostAdapter, MacOsLockGuard};

const CONFIG_NAME: &str = "config.toml";
const MARKER_NAME: &str = ".thinws-control.toml";
const STATE_DATABASE_NAME: &str = "state.db";
const CONTROLLED_DIRECTORIES: [&str; 2] = ["metadata", "logs"];

/// Non-copyable proof that this process published one exact initializing marker.
pub struct MacOsInitializingProof {
    data_root: ValidatedDirectory,
    marker_file: File,
    marker_identity: FileIdentity,
    marker: RootMarker,
}

/// Descriptor-backed evidence for the prepared APFS control root.
pub struct MacOsPreparedDataRoot {
    data_root: AbsolutePath,
    volume_id: VolumeId,
    directory: ValidatedDirectory,
}

impl PreparedDataRootEvidence for MacOsPreparedDataRoot {
    fn data_root(&self) -> &AbsolutePath {
        &self.data_root
    }

    fn volume_id(&self) -> VolumeId {
        self.volume_id
    }
}

/// Descriptor-backed evidence for the verified control-root layout.
pub struct MacOsDataRootLayout {
    data_root: ValidatedDirectory,
    controlled_directories: Vec<ValidatedDirectory>,
    database_file: File,
    database_identity: FileIdentity,
    database_path: AbsolutePath,
    volume_id: VolumeId,
    instance_id: thinws_core::InstanceId,
}

/// Proof of one newly created Workspace container.
pub struct MacOsPreparedWorkspace {
    parent: ValidatedDirectory,
    root: ValidatedDirectory,
    staging: ValidatedDirectory,
    trash: ValidatedDirectory,
    ownership_metadata: ValidatedDirectory,
    ownership_file: File,
    ownership_identity: FileIdentity,
    ownership: WorkspaceOwnership,
    target_root: AbsolutePath,
    staging_root: AbsolutePath,
    trash_root: AbsolutePath,
    volume_id: VolumeId,
}

impl PreparedWorkspaceEvidence for MacOsPreparedWorkspace {
    fn target_root(&self) -> &AbsolutePath {
        &self.target_root
    }

    fn staging_root(&self) -> &AbsolutePath {
        &self.staging_root
    }

    fn trash_root(&self) -> &AbsolutePath {
        &self.trash_root
    }

    fn target_identity(&self) -> thinws_core::FileIdentity {
        self.root.identity.as_core()
    }

    fn revalidate(&self) -> Result<(), PortError> {
        self.revalidate_directories()?;
        revalidate_directory(&self.ownership_metadata)?;
        let name = ownership_name(self.ownership.workspace_id);
        validate_file_entry(
            &self.ownership_metadata.fd,
            OsStr::new(&name),
            &self.ownership_file,
            self.ownership_identity,
        )?;
        if read_workspace_ownership(&self.ownership_metadata, self.ownership.workspace_id)?
            != self.ownership
        {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "Workspace ownership proof changed",
            ));
        }
        Ok(())
    }
}

impl MacOsPreparedWorkspace {
    fn revalidate_directories(&self) -> Result<(), PortError> {
        revalidate_target_parent(&self.parent)?;
        for (directory, name) in [
            (&self.root, self.root.path.file_name()),
            (&self.staging, self.staging.path.file_name()),
            (&self.trash, self.trash.path.file_name()),
        ] {
            let name = name.ok_or_else(|| {
                PortError::new(PortErrorKind::InvalidLayout, "Workspace child has no name")
            })?;
            revalidate_target_child(&self.parent, directory, name)?;
            if volume_id_for_directory(directory)? != self.volume_id {
                return Err(PortError::new(
                    PortErrorKind::InvalidLayout,
                    "revalidate Workspace operation directory volume",
                ));
            }
        }
        require_prepared_historical_ownership(&self.ownership, &self.parent, &self.root)?;
        for (evidence, actual) in [
            (&self.ownership.staging, &self.staging),
            (&self.ownership.trash, &self.trash),
        ] {
            if evidence.path.as_bytes() != actual.path.as_os_str().as_bytes()
                || !evidence
                    .identity
                    .permits_current(historical_directory_identity(actual)?)
            {
                return Err(PortError::new(
                    PortErrorKind::InvalidLayout,
                    "Workspace temporary directory ownership changed",
                ));
            }
        }
        Ok(())
    }
}

fn require_prepared_historical_ownership(
    ownership: &WorkspaceOwnership,
    parent: &ValidatedDirectory,
    root: &ValidatedDirectory,
) -> Result<(), PortError> {
    if ownership.target_path.as_bytes() != root.path.as_os_str().as_bytes()
        || ownership.isolated_path.is_some()
        || !ownership
            .parent
            .permits_current(historical_target_parent_identity(parent)?)
        || !ownership
            .target
            .permits_current(historical_directory_identity(root)?)
    {
        Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "Workspace historical directory identity changed",
        ))
    } else {
        Ok(())
    }
}

impl DataRootLayoutEvidence for MacOsDataRootLayout {
    fn database_path(&self) -> &AbsolutePath {
        &self.database_path
    }

    fn revalidate(&self) -> Result<(), PortError> {
        self.revalidate_inner()
            .map_err(classify_layout_revalidation)
    }
}

impl MacOsDataRootLayout {
    fn revalidate_inner(&self) -> Result<(), PortError> {
        revalidate_directory(&self.data_root)?;
        if volume_id_for_directory(&self.data_root)? != self.volume_id {
            return Err(PortError::new(
                PortErrorKind::InvalidData,
                "revalidate control-root volume identity",
            ));
        }
        for directory in &self.controlled_directories {
            revalidate_directory(directory)?;
            if volume_id_for_directory(directory)? != self.volume_id {
                return Err(PortError::new(
                    PortErrorKind::InvalidData,
                    "revalidate controlled-directory volume identity",
                ));
            }
        }
        let metadata = self
            .controlled_directories
            .first()
            .expect("layout always contains metadata first");
        validate_file_entry(
            &metadata.fd,
            OsStr::new(STATE_DATABASE_NAME),
            &self.database_file,
            self.database_identity,
        )
    }
}

impl BootstrapStore for MacOsHostAdapter {
    type LockGuard = MacOsLockGuard;
    type PreparedDataRoot = MacOsPreparedDataRoot;
    type InitializingProof = MacOsInitializingProof;
    type DataRootLayout = MacOsDataRootLayout;
    type PreparedWorkspace = MacOsPreparedWorkspace;

    fn prepare_bootstrap(&self) -> Result<(), PortError> {
        prepare_private_directory(&self.bootstrap_dir, false).map(|_| ())
    }

    fn prepare_data_root(
        &self,
        data_root: &AbsolutePath,
    ) -> Result<Self::PreparedDataRoot, PortError> {
        if path_from_absolute(data_root) != self.bootstrap_dir {
            return Err(PortError::conflict(
                "prepare fixed control root",
                PortConflict::InstallationIdentity,
            ));
        }
        let directory = prepare_private_directory(&path_from_absolute(data_root), false)?;
        require_unclaimed_control_root(&directory)?;
        let volume_id = volume_id_for_directory(&directory)?;
        Ok(MacOsPreparedDataRoot {
            data_root: data_root.clone(),
            volume_id,
            directory,
        })
    }

    fn read_config(&self) -> Result<Option<InstallationIdentity>, PortError> {
        let Some(directory) = open_private_directory_optional(&self.bootstrap_dir)? else {
            return Ok(None);
        };
        read_config_at(&directory)
    }

    fn read_root_marker(
        &self,
        data_root: &thinws_core::AbsolutePath,
    ) -> Result<Option<RootMarker>, PortError> {
        if path_from_absolute(data_root) != self.bootstrap_dir {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "read fixed control marker",
            ));
        }
        let Some(directory) = open_private_directory_optional(&path_from_absolute(data_root))?
        else {
            return Ok(None);
        };
        read_marker_at(&directory, OsStr::new(MARKER_NAME))
    }

    fn create_initializing(
        &self,
        lock: &Self::LockGuard,
        prepared: Self::PreparedDataRoot,
        identity: &InstallationIdentity,
    ) -> Result<Self::InitializingProof, PortError> {
        self.validate_bootstrap_lock(lock)?;
        if prepared.data_root() != identity.data_root()
            || prepared.volume_id() != identity.volume_id()
        {
            return Err(PortError::conflict(
                "validate prepared control root",
                PortConflict::InstallationIdentity,
            ));
        }
        let directory = prepared.directory;
        revalidate_directory(&directory)?;
        if volume_id_for_directory(&directory)? != identity.volume_id() {
            return Err(PortError::conflict(
                "revalidate prepared control-root volume",
                PortConflict::InstallationIdentity,
            ));
        }
        if directory.path != self.bootstrap_dir {
            return Err(PortError::conflict(
                "validate fixed control root",
                PortConflict::InstallationIdentity,
            ));
        }
        require_unclaimed_control_root(&directory)?;
        let marker = RootMarker::new(identity.clone(), RootMarkerState::Initializing);
        let bytes = encode_marker(&marker).map_err(document_error)?;
        let temporary = PrivateTemp::create(&directory.fd, "root-marker", &bytes)?;
        match temporary.publish_noreplace(OsStr::new(MARKER_NAME)) {
            Ok((marker_file, marker_identity)) => {
                let verification = revalidate_directory(&directory).and_then(|()| {
                    validate_file_entry(
                        &directory.fd,
                        OsStr::new(MARKER_NAME),
                        &marker_file,
                        marker_identity,
                    )
                });
                if let Err(error) = verification {
                    remove_exact_entry(&directory, OsStr::new(MARKER_NAME), marker_identity);
                    return Err(error);
                }
                Ok(MacOsInitializingProof {
                    data_root: directory,
                    marker_file,
                    marker_identity,
                    marker,
                })
            }
            Err(NoReplaceError::Exists) => {
                // Existing objects are parsed only for a structured diagnostic;
                // even an identical initializing marker is never auto-adopted.
                let _ = self.read_root_marker(identity.data_root())?;
                Err(PortError::conflict(
                    "publish initializing root marker",
                    PortConflict::InstallationIdentity,
                ))
            }
            Err(NoReplaceError::Other(error)) => Err(error),
        }
    }

    fn initialize_layout(
        &self,
        lock: &Self::LockGuard,
        proof: &Self::InitializingProof,
    ) -> Result<Self::DataRootLayout, PortError> {
        self.validate_bootstrap_lock(lock)?;
        revalidate_directory(&proof.data_root)?;
        if volume_id_for_directory(&proof.data_root)? != proof.marker.identity().volume_id() {
            return Err(PortError::conflict(
                "revalidate initializing control-root volume",
                PortConflict::InstallationIdentity,
            ));
        }

        let mut controlled_directories = Vec::with_capacity(CONTROLLED_DIRECTORIES.len());
        for name in CONTROLLED_DIRECTORIES {
            controlled_directories.push(create_private_child_directory(
                &proof.data_root,
                OsStr::new(name),
            )?);
        }
        let metadata = controlled_directories
            .first()
            .expect("controlled directory list always begins with metadata");
        let (database_file, database_identity) =
            create_private_file(metadata, OsStr::new(STATE_DATABASE_NAME))?;
        let database_path =
            crate::filesystem::absolute_from_path(&metadata.path.join(STATE_DATABASE_NAME))
                .map_err(|error| {
                    PortError::new(PortErrorKind::InvalidData, "derive metadata database path")
                        .with_source(error)
                })?;
        let layout = MacOsDataRootLayout {
            data_root: duplicate_validated_directory(&proof.data_root)?,
            controlled_directories,
            database_file,
            database_identity,
            database_path,
            volume_id: proof.marker.identity().volume_id(),
            instance_id: proof.marker.identity().instance_id(),
        };
        layout.revalidate()?;
        Ok(layout)
    }

    fn validate_layout(
        &self,
        identity: &InstallationIdentity,
    ) -> Result<Self::DataRootLayout, PortError> {
        if path_from_absolute(identity.data_root()) != self.bootstrap_dir {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "validate fixed control root",
            ));
        }
        let Some(data_root) =
            open_private_directory_optional(&path_from_absolute(identity.data_root()))?
        else {
            return Err(PortError::new(
                PortErrorKind::Unavailable,
                "open registered control root",
            ));
        };
        if volume_id_for_directory(&data_root)? != identity.volume_id() {
            return Err(PortError::new(
                PortErrorKind::InvalidData,
                "validate registered control-root volume",
            ));
        }
        let mut controlled_directories = Vec::with_capacity(CONTROLLED_DIRECTORIES.len());
        for name in CONTROLLED_DIRECTORIES {
            controlled_directories
                .push(open_private_child_directory(&data_root, OsStr::new(name))?);
        }
        let metadata = controlled_directories
            .first()
            .expect("controlled directory list always begins with metadata");
        let (database_file, database_identity) =
            open_private_file(metadata, OsStr::new(STATE_DATABASE_NAME))?;
        let database_path =
            crate::filesystem::absolute_from_path(&metadata.path.join(STATE_DATABASE_NAME))
                .map_err(|error| {
                    PortError::new(PortErrorKind::InvalidData, "derive metadata database path")
                        .with_source(error)
                })?;
        let layout = MacOsDataRootLayout {
            data_root,
            controlled_directories,
            database_file,
            database_identity,
            database_path,
            volume_id: identity.volume_id(),
            instance_id: identity.instance_id(),
        };
        layout.revalidate()?;
        Ok(layout)
    }

    fn prepare_workspace(
        &self,
        lock: &Self::LockGuard,
        layout: &Self::DataRootLayout,
        workspace_id: WorkspaceId,
        target: &AbsolutePath,
    ) -> Result<Self::PreparedWorkspace, PortError> {
        self.validate_data_root_lock(lock, layout)?;
        layout.revalidate()?;
        let (parent, target_name) = open_target_parent(target)?;
        let parent_path = crate::filesystem::absolute_from_path(&parent.path).map_err(|error| {
            PortError::new(
                PortErrorKind::InvalidLayout,
                "derive Workspace target parent",
            )
            .with_source(error)
        })?;
        let parent_report = self.inspect_path(&parent_path)?;
        // For a missing path, ancestry ends at a different existing ancestor;
        // one identity comparison also covers that case.
        if parent_report
            .ancestry()
            .last()
            .map(|entry| entry.identity())
            != Some(parent.identity.as_core())
        {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "Workspace target parent changed before creation",
            ));
        }
        if parent_report
            .ancestry()
            .iter()
            .any(|entry| entry.identity() == layout.data_root.identity.as_core())
        {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "Workspace target overlaps the control root",
            ));
        }
        let volume_id = volume_id_for_directory(&parent)?;
        let root = create_target_child_directory(&parent, &target_name)?;
        require_empty_directory(&root)?;
        let staging_name = format!(".thinws-staging-{workspace_id}");
        let trash_name = format!(".thinws-trash-{workspace_id}");
        let staging = create_target_child_directory(&parent, OsStr::new(&staging_name))?;
        let trash = create_target_child_directory(&parent, OsStr::new(&trash_name))?;
        require_empty_directory(&staging)?;
        require_empty_directory(&trash)?;
        let target_root = crate::filesystem::absolute_from_path(&root.path).map_err(|error| {
            PortError::new(PortErrorKind::InvalidData, "derive Workspace target path")
                .with_source(error)
        })?;
        if &target_root != target {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "created target differs from requested target",
            ));
        }
        let staging_root =
            crate::filesystem::absolute_from_path(&staging.path).map_err(|error| {
                PortError::new(PortErrorKind::InvalidData, "derive staging path").with_source(error)
            })?;
        let trash_root = crate::filesystem::absolute_from_path(&trash.path).map_err(|error| {
            PortError::new(PortErrorKind::InvalidData, "derive rollback path").with_source(error)
        })?;
        let ownership = WorkspaceOwnership {
            instance_id: layout.instance_id,
            workspace_id,
            volume_id,
            target_path: target_root.clone(),
            parent: historical_target_parent_identity(&parent)?,
            target: historical_directory_identity(&root)?,
            staging: OperationDirectoryOwnership {
                path: staging_root.clone(),
                identity: historical_directory_identity(&staging)?,
            },
            trash: OperationDirectoryOwnership {
                path: trash_root.clone(),
                identity: historical_directory_identity(&trash)?,
            },
            isolated_path: None,
        };
        let ownership_metadata = duplicate_validated_directory(&layout.controlled_directories[0])?;
        let ownership_bytes = encode_workspace_ownership(&ownership).map_err(document_error)?;
        let name = ownership_name(workspace_id);
        let (ownership_file, ownership_identity) = match PrivateTemp::create(
            &ownership_metadata.fd,
            "workspace-ownership",
            &ownership_bytes,
        )?
        .publish_noreplace(OsStr::new(&name))
        {
            Ok(published) => published,
            Err(NoReplaceError::Exists) => {
                return Err(PortError::new(
                    PortErrorKind::InvalidLayout,
                    "Workspace ownership proof already exists",
                ));
            }
            Err(NoReplaceError::Other(error)) => return Err(error),
        };
        let prepared = MacOsPreparedWorkspace {
            parent,
            root,
            staging,
            trash,
            ownership_metadata,
            ownership_file,
            ownership_identity,
            ownership,
            target_root,
            staging_root,
            trash_root,
            volume_id,
        };
        prepared.revalidate()?;
        layout.revalidate()?;
        Ok(prepared)
    }

    fn clear_workspace_incomplete(
        &self,
        lock: &Self::LockGuard,
        layout: &Self::DataRootLayout,
        prepared: Self::PreparedWorkspace,
    ) -> Result<(), PortError> {
        self.validate_data_root_lock(lock, layout)?;
        layout.revalidate()?;
        prepared.revalidate()?;
        for directory in [&prepared.staging, &prepared.trash] {
            require_empty_directory(directory)?;
            let name = directory.path.file_name().ok_or_else(|| {
                PortError::new(
                    PortErrorKind::InvalidLayout,
                    "Workspace temporary root has no name",
                )
            })?;
            revalidate_target_child(&prepared.parent, directory, name)?;
            rustix::fs::unlinkat(&prepared.parent.fd, name, AtFlags::REMOVEDIR)
                .map_err(|error| io_error("remove completed Workspace temporary root", error))?;
            sync_directory(&prepared.parent.fd)?;
        }
        revalidate_target_child(
            &prepared.parent,
            &prepared.root,
            prepared
                .root
                .path
                .file_name()
                .expect("prepared target has a leaf"),
        )?;
        layout.revalidate()?;
        Ok(())
    }

    fn validate_ready_workspace(
        &self,
        layout: &Self::DataRootLayout,
        reservation: &WorkspaceReservation,
    ) -> Result<AbsolutePath, PortError> {
        layout.revalidate()?;
        let (parent, name, ownership) = registered_target(layout, reservation)?;
        if ownership.isolated_path.is_some() {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "Ready target is isolated",
            ));
        }
        let root = open_optional_owned_child(&parent, &name)?.ok_or_else(|| {
            PortError::new(
                PortErrorKind::NotFound,
                "registered Ready target is missing",
            )
        })?;
        verify_target_directory(&parent, &root, &name, &ownership)?;
        require_operation_directories_absent(&parent, &ownership)?;
        let path = reservation.target_path().clone();
        if read_workspace_ownership(
            &layout.controlled_directories[0],
            reservation.workspace_id(),
        )? != ownership
        {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "Ready ownership proof changed",
            ));
        }
        revalidate_target_child(&parent, &root, &name)?;
        layout.revalidate()?;
        Ok(path)
    }

    fn measure_ready_workspace_space(
        &self,
        layout: &Self::DataRootLayout,
        reservation: &WorkspaceReservation,
    ) -> Result<WorkspaceSpace, PortError> {
        let expected_path = self.validate_ready_workspace(layout, reservation)?;
        let (parent, name, ownership) = registered_target(layout, reservation)?;
        let root = open_optional_owned_child(&parent, &name)?.ok_or_else(|| {
            PortError::new(
                PortErrorKind::NotFound,
                "registered Ready target is missing",
            )
        })?;
        verify_target_directory(&parent, &root, &name, &ownership)?;
        let measurement = measure_root(&root.fd);
        revalidate_target_child(&parent, &root, &name)?;
        if self.validate_ready_workspace(layout, reservation)? != expected_path {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "Ready Workspace path changed during space scan",
            ));
        }
        Ok(measurement)
    }

    fn inspect_removal_container(
        &self,
        lock: &Self::LockGuard,
        layout: &Self::DataRootLayout,
        reservation: &WorkspaceReservation,
    ) -> Result<Option<AbsolutePath>, PortError> {
        self.validate_data_root_lock(lock, layout)?;
        layout.revalidate()?;
        let (parent, target_name, ownership) = registered_target(layout, reservation)?;
        let result = locate_target(&parent, &target_name, &ownership)?
            .map(|(directory, _)| {
                crate::filesystem::absolute_from_path(&directory.path).map_err(|error| {
                    PortError::new(PortErrorKind::InvalidData, "derive removal target path")
                        .with_source(error)
                })
            })
            .transpose()?;
        layout.revalidate()?;
        self.validate_data_root_lock(lock, layout)?;
        Ok(result)
    }

    fn append_removal_log(
        &self,
        lock: &Self::LockGuard,
        layout: &Self::DataRootLayout,
        record: &RemovalLogRecord<'_>,
    ) -> Result<AbsolutePath, PortError> {
        self.validate_data_root_lock(lock, layout)?;
        layout.revalidate()?;
        let path =
            crate::operation_log::append_removal_record(&layout.controlled_directories[1], record)?;
        layout.revalidate()?;
        self.validate_data_root_lock(lock, layout)?;
        Ok(path)
    }

    fn remove_workspace(
        &self,
        lock: &Self::LockGuard,
        layout: &Self::DataRootLayout,
        reservation: &WorkspaceReservation,
    ) -> Result<WorkspaceRemoval, PortError> {
        self.validate_data_root_lock(lock, layout)?;
        layout.revalidate()?;
        let (parent, target_name, mut ownership) = registered_target(layout, reservation)?;
        let (target, at_isolated_path) = locate_target(&parent, &target_name, &ownership)?
            .ok_or_else(|| {
                PortError::new(
                    PortErrorKind::NotFound,
                    "registered Workspace target is missing",
                )
            })?;
        // Operation directories are independently registered. Remove them first,
        // while the exact target still exists, so a failed cleanup leaves an
        // addressable Workspace rather than an orphaned registration.
        for evidence in [&ownership.staging, &ownership.trash] {
            remove_operation_directory(self, lock, layout, &parent, evidence)?;
        }
        let isolated_name = format!(".thinws-remove-{}", reservation.workspace_id());
        let isolated_path = crate::filesystem::absolute_from_path(
            &parent.path.join(&isolated_name),
        )
        .map_err(|error| {
            PortError::new(PortErrorKind::InvalidData, "derive isolated target path")
                .with_source(error)
        })?;
        let isolated = if at_isolated_path {
            if ownership.isolated_path.as_ref() != Some(&isolated_path) {
                return Err(PortError::new(
                    PortErrorKind::InvalidLayout,
                    "isolated target is not registered",
                ));
            }
            target
        } else {
            if ownership.isolated_path.as_ref() != Some(&isolated_path) {
                ownership.isolated_path = Some(isolated_path);
                persist_workspace_ownership(layout, &ownership)?;
            }
            self.validate_data_root_lock(lock, layout)?;
            verify_target_directory(&parent, &target, &target_name, &ownership)?;
            rustix::fs::renameat_with(
                &parent.fd,
                &target_name,
                &parent.fd,
                isolated_name.as_str(),
                RenameFlags::NOREPLACE,
            )
            .map_err(|error| io_error("isolate Workspace target without replacement", error))?;
            sync_directory(&parent.fd)?;
            let moved = open_owned_child_directory(&parent, OsStr::new(&isolated_name))?;
            if moved.identity != target.identity {
                return Err(PortError::new(
                    PortErrorKind::InvalidLayout,
                    "isolated target identity changed",
                ));
            }
            moved
        };
        verify_target_directory(&parent, &isolated, OsStr::new(&isolated_name), &ownership)?;
        let verify_scope = || {
            self.validate_data_root_lock(lock, layout)
                .and_then(|()| {
                    verify_target_directory(
                        &parent,
                        &isolated,
                        OsStr::new(&isolated_name),
                        &ownership,
                    )
                })
                .map_err(|_| std::io::Error::from_raw_os_error(libc::ESTALE))
        };
        let root_entries =
            remove_root_contents(&isolated.fd, isolated.identity.as_core(), &verify_scope)
                .map_err(classify_root_removal_error)?;
        verify_target_directory(&parent, &isolated, OsStr::new(&isolated_name), &ownership)?;
        rustix::fs::unlinkat(&parent.fd, isolated_name.as_str(), AtFlags::REMOVEDIR)
            .map_err(|error| io_error("remove isolated Workspace target", error))?;
        sync_directory(&parent.fd)?;
        self.validate_data_root_lock(lock, layout)?;
        layout.revalidate()?;
        require_missing_entry(&parent, &target_name)?;
        require_missing_entry(&parent, OsStr::new(&isolated_name))?;
        layout.revalidate()?;
        Ok(WorkspaceRemoval::Removed { root_entries })
    }

    fn publish_ready(
        &self,
        lock: &Self::LockGuard,
        proof: Self::InitializingProof,
    ) -> Result<RootMarker, PortError> {
        self.validate_bootstrap_lock(lock)?;
        let directory = &proof.data_root;
        revalidate_directory(directory)?;
        validate_file_entry(
            &directory.fd,
            OsStr::new(MARKER_NAME),
            &proof.marker_file,
            proof.marker_identity,
        )?;
        let current = read_marker_at(directory, OsStr::new(MARKER_NAME))?;
        if (current.as_ref(), proof.marker.state())
            != (Some(&proof.marker), RootMarkerState::Initializing)
        {
            return Err(PortError::conflict(
                "validate initializing root marker",
                PortConflict::InstallationIdentity,
            ));
        }

        let ready = RootMarker::new(proof.marker.identity().clone(), RootMarkerState::Ready);
        let bytes = encode_marker(&ready).map_err(document_error)?;
        let mut temporary = PrivateTemp::create(&directory.fd, "root-marker-ready", &bytes)?;
        temporary.exchange_with(OsStr::new(MARKER_NAME))?;

        let verification = (|| {
            let observed_identities = (
                entry_identity(&directory.fd, OsStr::new(MARKER_NAME))?,
                entry_identity(&directory.fd, temporary.name())?,
            );
            if observed_identities != (temporary.identity(), proof.marker_identity) {
                return Err(PortError::new(
                    PortErrorKind::InvalidData,
                    "verify exchanged root marker identities",
                ));
            }
            validate_file_entry(
                &directory.fd,
                temporary.name(),
                &proof.marker_file,
                proof.marker_identity,
            )?;
            let observed_markers = (
                read_marker_at(directory, OsStr::new(MARKER_NAME))?,
                read_marker_at(directory, temporary.name())?,
            );
            if observed_markers != (Some(ready.clone()), Some(proof.marker.clone())) {
                return Err(PortError::new(
                    PortErrorKind::InvalidData,
                    "verify exchanged root marker content",
                ));
            }
            revalidate_directory(directory)?;
            Ok(())
        })();

        if let Err(error) = verification {
            let can_restore = exchanged_pair_matches(
                (
                    entry_identity(&directory.fd, OsStr::new(MARKER_NAME)).ok(),
                    entry_identity(&directory.fd, temporary.name()).ok(),
                ),
                (temporary.identity(), proof.marker_identity),
            );
            if can_restore {
                // Both names still identify the exact exchanged pair, so a
                // second atomic swap restores the previous marker safely.
                temporary.exchange_with(OsStr::new(MARKER_NAME))?;
                unlink_entry(&directory.fd, temporary.name())?;
                sync_directory(&directory.fd)?;
            }
            return Err(error);
        }

        unlink_entry(&directory.fd, temporary.name())?;
        sync_directory(&directory.fd)?;
        Ok(ready)
    }

    fn publish_config(
        &self,
        lock: &Self::LockGuard,
        identity: &InstallationIdentity,
    ) -> Result<PublishResult, PortError> {
        self.validate_bootstrap_lock(lock)?;
        let marker = self.read_root_marker(identity.data_root())?;
        if marker.as_ref() != Some(&RootMarker::new(identity.clone(), RootMarkerState::Ready)) {
            return Err(PortError::conflict(
                "publish bootstrap config before Ready",
                PortConflict::InstallationIdentity,
            ));
        }

        let directory = open_private_directory(&self.bootstrap_dir)?;
        if !lock.protects_directory(&directory) {
            return Err(PortError::new(
                PortErrorKind::InvalidData,
                "bootstrap lock directory changed",
            ));
        }
        if let Some(current) = read_config_at(&directory)? {
            return if config_is_current(Some(&current), identity) {
                Ok(PublishResult::AlreadyCurrent)
            } else {
                Err(PortError::conflict(
                    "publish bootstrap config",
                    PortConflict::InstallationIdentity,
                ))
            };
        }

        let bytes = encode_config(identity).map_err(document_error)?;
        let temporary = PrivateTemp::create(&directory.fd, "bootstrap-config", &bytes)?;
        match temporary.publish_noreplace(OsStr::new(CONFIG_NAME)) {
            Ok((config_file, config_identity)) => {
                let verification = revalidate_directory(&directory).and_then(|()| {
                    validate_file_entry(
                        &directory.fd,
                        OsStr::new(CONFIG_NAME),
                        &config_file,
                        config_identity,
                    )
                });
                if let Err(error) = verification {
                    remove_exact_entry(&directory, OsStr::new(CONFIG_NAME), config_identity);
                    return Err(error);
                }
                Ok(PublishResult::Published)
            }
            Err(NoReplaceError::Exists) => {
                let current = read_config_at(&directory)?;
                if config_is_current(current.as_ref(), identity) {
                    Ok(PublishResult::AlreadyCurrent)
                } else {
                    Err(PortError::conflict(
                        "publish bootstrap config",
                        PortConflict::InstallationIdentity,
                    ))
                }
            }
            Err(NoReplaceError::Other(error)) => Err(error),
        }
    }
}

fn volume_id_for_directory(directory: &ValidatedDirectory) -> Result<VolumeId, PortError> {
    let filesystem = crate::ffi::file_system_type(&directory.fd)
        .map_err(|error| io_error("inspect control-root filesystem", error))?;
    if filesystem != "apfs" {
        return Err(PortError::new(
            PortErrorKind::CapabilityUnavailable,
            "require APFS control root",
        ));
    }
    let bytes = crate::ffi::volume_uuid(&directory.fd)
        .map_err(|error| io_error("read APFS volume UUID", error))?;
    decode_volume_id(bytes)
}

impl MacOsHostAdapter {
    fn validate_bootstrap_lock(&self, lock: &MacOsLockGuard) -> Result<(), PortError> {
        if lock.scope() != LifecycleScope::Bootstrap || !lock.belongs_to(self) {
            return Err(PortError::new(
                PortErrorKind::InvalidData,
                "require matching bootstrap lock",
            ));
        }
        lock.revalidate()
    }

    fn validate_data_root_lock(
        &self,
        lock: &MacOsLockGuard,
        layout: &MacOsDataRootLayout,
    ) -> Result<(), PortError> {
        if (
            lock.scope(),
            lock.belongs_to(self),
            lock.protects_directory(&layout.data_root),
        ) != (LifecycleScope::DataRoot, true, true)
        {
            return Err(PortError::new(
                PortErrorKind::InvalidData,
                "require matching control-root lifecycle lock",
            ));
        }
        lock.revalidate()
    }
}

fn read_config_at(
    directory: &crate::filesystem::ValidatedDirectory,
) -> Result<Option<InstallationIdentity>, PortError> {
    let result = read_private_file(&directory.fd, OsStr::new(CONFIG_NAME))?
        .map(|bytes| decode_bootstrap_config(&bytes).map_err(document_error))
        .transpose()?;
    revalidate_directory(directory)?;
    Ok(result)
}

fn read_marker_at(
    directory: &crate::filesystem::ValidatedDirectory,
    name: &OsStr,
) -> Result<Option<RootMarker>, PortError> {
    let result = read_private_file(&directory.fd, name)?
        .map(|bytes| decode_root_marker(&bytes).map_err(document_error))
        .transpose()?;
    revalidate_directory(directory)?;
    Ok(result)
}

fn ownership_name(workspace_id: WorkspaceId) -> String {
    format!("ownership-{workspace_id}.toml")
}

fn read_workspace_ownership(
    metadata: &ValidatedDirectory,
    workspace_id: WorkspaceId,
) -> Result<WorkspaceOwnership, PortError> {
    let name = ownership_name(workspace_id);
    let bytes = read_private_file(&metadata.fd, OsStr::new(&name))?.ok_or_else(|| {
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
    revalidate_directory(metadata)?;
    Ok(ownership)
}

fn registered_target(
    layout: &MacOsDataRootLayout,
    reservation: &WorkspaceReservation,
) -> Result<(ValidatedDirectory, std::ffi::OsString, WorkspaceOwnership), PortError> {
    let ownership = read_workspace_ownership(
        &layout.controlled_directories[0],
        reservation.workspace_id(),
    )?;
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
    let (parent, name) = open_target_parent(reservation.target_path())?;
    if volume_id_for_directory(&parent)? != ownership.volume_id
        || !ownership
            .parent
            .permits_current(historical_target_parent_identity(&parent)?)
    {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "Workspace target parent identity changed",
        ));
    }
    Ok((parent, name, ownership))
}

fn verify_target_directory(
    parent: &ValidatedDirectory,
    directory: &ValidatedDirectory,
    name: &OsStr,
    ownership: &WorkspaceOwnership,
) -> Result<(), PortError> {
    revalidate_target_child(parent, directory, name)?;
    if volume_id_for_directory(directory)? != ownership.volume_id
        || !ownership
            .target
            .permits_current(historical_directory_identity(directory)?)
    {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "Workspace target historical identity changed",
        ));
    }
    Ok(())
}

fn isolated_name(ownership: &WorkspaceOwnership) -> String {
    format!(".thinws-remove-{}", ownership.workspace_id)
}

fn locate_target(
    parent: &ValidatedDirectory,
    target_name: &OsStr,
    ownership: &WorkspaceOwnership,
) -> Result<Option<(ValidatedDirectory, bool)>, PortError> {
    let active = open_optional_owned_child(parent, target_name)?;
    let isolated_name = isolated_name(ownership);
    let isolated = open_optional_owned_child(parent, OsStr::new(&isolated_name))?;
    if isolated.is_some()
        && ownership.isolated_path.as_ref().is_none_or(|path| {
            path.as_bytes() != parent.path.join(&isolated_name).as_os_str().as_bytes()
        })
    {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "unregistered isolated Workspace target exists",
        ));
    }
    match (active, isolated) {
        (Some(_), Some(_)) => Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "Workspace target exists at active and isolated paths",
        )),
        (Some(directory), None) => {
            verify_target_directory(parent, &directory, target_name, ownership)?;
            Ok(Some((directory, false)))
        }
        (None, Some(directory)) => {
            verify_target_directory(parent, &directory, OsStr::new(&isolated_name), ownership)?;
            Ok(Some((directory, true)))
        }
        (None, None) => Ok(None),
    }
}

fn require_operation_directories_absent(
    parent: &ValidatedDirectory,
    ownership: &WorkspaceOwnership,
) -> Result<(), PortError> {
    for evidence in [&ownership.staging, &ownership.trash] {
        let name = evidence.path.as_bytes();
        let leaf = name
            .rsplit(|byte| *byte == b'/')
            .next()
            .expect("absolute path has a leaf");
        require_missing_entry(parent, OsStr::from_bytes(leaf))?;
    }
    Ok(())
}

fn remove_operation_directory(
    adapter: &MacOsHostAdapter,
    lock: &MacOsLockGuard,
    layout: &MacOsDataRootLayout,
    parent: &ValidatedDirectory,
    evidence: &OperationDirectoryOwnership,
) -> Result<(), PortError> {
    let bytes = evidence.path.as_bytes();
    let leaf = bytes
        .rsplit(|byte| *byte == b'/')
        .next()
        .expect("absolute path has a leaf");
    let name = OsStr::from_bytes(leaf);
    let Some(directory) = open_optional_owned_child(parent, name)? else {
        return Ok(());
    };
    revalidate_target_child(parent, &directory, name)?;
    if volume_id_for_directory(&directory)? != volume_id_for_directory(parent)?
        || !evidence
            .identity
            .permits_current(historical_directory_identity(&directory)?)
    {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "operation directory identity changed",
        ));
    }
    let verify_scope = || {
        adapter
            .validate_data_root_lock(lock, layout)
            .and_then(|()| revalidate_target_child(parent, &directory, name))
            .map_err(|_| std::io::Error::from_raw_os_error(libc::ESTALE))
    };
    remove_root_contents(&directory.fd, directory.identity.as_core(), &verify_scope)
        .map_err(classify_root_removal_error)?;
    revalidate_target_child(parent, &directory, name)?;
    rustix::fs::unlinkat(&parent.fd, name, AtFlags::REMOVEDIR)
        .map_err(|error| io_error("remove Workspace operation directory", error))?;
    sync_directory(&parent.fd)
}

fn persist_workspace_ownership(
    layout: &MacOsDataRootLayout,
    desired: &WorkspaceOwnership,
) -> Result<(), PortError> {
    let metadata = &layout.controlled_directories[0];
    let current = read_workspace_ownership(metadata, desired.workspace_id)?;
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
    let mut temporary = PrivateTemp::create(&metadata.fd, "workspace-ownership-update", &bytes)?;
    temporary.exchange_with(OsStr::new(&ownership_name(desired.workspace_id)))?;
    sync_directory(&metadata.fd)?;
    if read_workspace_ownership(metadata, desired.workspace_id)? != *desired {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "isolated ownership publication changed",
        ));
    }
    unlink_entry(&metadata.fd, temporary.name())?;
    sync_directory(&metadata.fd)
}

fn open_optional_owned_child(
    parent: &ValidatedDirectory,
    name: &OsStr,
) -> Result<Option<ValidatedDirectory>, PortError> {
    match rustix::fs::statat(&parent.fd, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => open_owned_child_directory(parent, name).map(Some),
        Err(rustix::io::Errno::NOENT) => Ok(None),
        Err(error) => Err(io_error("inspect Workspace owned child", error)),
    }
}

fn require_missing_entry(parent: &ValidatedDirectory, name: &OsStr) -> Result<(), PortError> {
    match rustix::fs::statat(&parent.fd, name, AtFlags::SYMLINK_NOFOLLOW) {
        Err(rustix::io::Errno::NOENT) => Ok(()),
        Ok(_) => Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "Workspace container path reappeared during removal",
        )),
        Err(error) => Err(io_error("verify removed Workspace path", error)),
    }
}

fn require_unclaimed_control_root(directory: &ValidatedDirectory) -> Result<(), PortError> {
    let names = crate::ffi::read_directory(&directory.fd)
        .map_err(|error| io_error("inspect unclaimed control root", error))?;
    if names
        .iter()
        .any(|name| name != OsStr::new("lifecycle.lock"))
    {
        return Err(PortError::new(
            PortErrorKind::NotEmpty,
            "unclaimed control root contains another entry",
        ));
    }
    revalidate_directory(directory)
}

fn remove_exact_entry(
    directory: &crate::filesystem::ValidatedDirectory,
    name: &OsStr,
    expected: FileIdentity,
) {
    if entry_identity(&directory.fd, name).ok() == Some(expected) {
        let _ = unlink_entry(&directory.fd, name);
        let _ = sync_directory(&directory.fd);
    }
}

fn exchanged_pair_matches(
    observed: (Option<FileIdentity>, Option<FileIdentity>),
    expected: (FileIdentity, FileIdentity),
) -> bool {
    observed == (Some(expected.0), Some(expected.1))
}

fn config_is_current(
    current: Option<&InstallationIdentity>,
    requested: &InstallationIdentity,
) -> bool {
    current == Some(requested)
}

fn document_error(error: DocumentError) -> PortError {
    let kind = if error == DocumentError::UnsupportedVersion {
        PortErrorKind::UnsupportedVersion
    } else {
        PortErrorKind::InvalidData
    };
    PortError::new(kind, "decode bootstrap document").with_source(error)
}

fn classify_root_removal_error(error: std::io::Error) -> PortError {
    match error.raw_os_error() {
        Some(libc::ESTALE | libc::EXDEV | libc::ELOOP) => PortError::new(
            PortErrorKind::InvalidLayout,
            "Workspace root changed during removal",
        )
        .with_source(error),
        _ => io_error("remove Workspace root contents", error),
    }
}

fn classify_layout_revalidation(error: PortError) -> PortError {
    if error.kind() == PortErrorKind::InvalidLayout {
        return error;
    }
    if matches!(
        error.kind(),
        PortErrorKind::InvalidData
            | PortErrorKind::NotFound
            | PortErrorKind::Conflict
            | PortErrorKind::CapabilityUnavailable
    ) {
        PortError::new(
            PortErrorKind::InvalidLayout,
            "revalidate control-root layout",
        )
        .with_source(error)
    } else {
        error
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::str::FromStr;

    use tempfile::Builder;
    use thinws_core::{AbsolutePath, InstanceId, VolumeId};

    use super::*;

    #[test]
    fn root_removal_classifies_identity_and_depth_failures_as_layout_errors() {
        for errno in [libc::ESTALE, libc::EXDEV, libc::ELOOP] {
            assert_eq!(
                classify_root_removal_error(std::io::Error::from_raw_os_error(errno)).kind(),
                PortErrorKind::InvalidLayout
            );
        }
        assert_eq!(
            classify_root_removal_error(std::io::Error::from_raw_os_error(libc::EIO)).kind(),
            PortErrorKind::Io
        );
    }

    #[test]
    fn exact_entry_cleanup_and_document_error_mapping_are_observable() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-02-macos-tests");
        fs::create_dir_all(&root).unwrap();
        let temp = Builder::new()
            .prefix("exact-entry-cleanup-")
            .tempdir_in(fs::canonicalize(root).unwrap())
            .unwrap();
        let directory_path = temp.path().join("private");
        fs::create_dir(&directory_path).unwrap();
        fs::set_permissions(&directory_path, fs::Permissions::from_mode(0o700)).unwrap();
        let directory = open_private_directory(&directory_path).unwrap();
        for name in ["target", "other"] {
            fs::write(directory_path.join(name), name.as_bytes()).unwrap();
            fs::set_permissions(directory_path.join(name), fs::Permissions::from_mode(0o600))
                .unwrap();
        }
        let target_identity = entry_identity(&directory.fd, OsStr::new("target")).unwrap();
        let other_identity = entry_identity(&directory.fd, OsStr::new("other")).unwrap();

        assert!(exchanged_pair_matches(
            (Some(target_identity), Some(other_identity)),
            (target_identity, other_identity)
        ));
        assert!(!exchanged_pair_matches(
            (Some(other_identity), Some(target_identity)),
            (target_identity, other_identity)
        ));

        remove_exact_entry(&directory, OsStr::new("target"), other_identity);
        assert!(directory_path.join("target").exists());
        remove_exact_entry(&directory, OsStr::new("target"), target_identity);
        assert!(!directory_path.join("target").exists());
        require_missing_entry(&directory, OsStr::new("target")).unwrap();
        assert_eq!(
            require_missing_entry(&directory, OsStr::new("other"))
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );

        assert_eq!(
            document_error(DocumentError::UnsupportedVersion).kind(),
            PortErrorKind::UnsupportedVersion
        );
        assert_eq!(
            document_error(DocumentError::InvalidToml).kind(),
            PortErrorKind::InvalidData
        );

        let requested = InstallationIdentity::new(
            InstanceId::from_str("01890a5d-ac96-774b-bd5b-55c7b8d09f33").unwrap(),
            AbsolutePath::try_from_bytes(b"/Volumes/data/thinws".to_vec()).unwrap(),
            VolumeId::from_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
        );
        let conflicting = InstallationIdentity::new(
            InstanceId::from_str("01890a5d-ac96-774b-bd5b-55c7b8d09f34").unwrap(),
            AbsolutePath::try_from_bytes(b"/Volumes/data/thinws".to_vec()).unwrap(),
            VolumeId::from_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
        );
        assert!(config_is_current(Some(&requested), &requested));
        assert!(!config_is_current(Some(&conflicting), &requested));
        assert!(!config_is_current(None, &requested));
    }

    #[test]
    fn terminal_removal_check_rejects_reappeared_active_and_isolated_entries() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-12-macos-tests");
        fs::create_dir_all(&root).unwrap();
        let temp = Builder::new()
            .prefix("terminal-removal-")
            .tempdir_in(fs::canonicalize(root).unwrap())
            .unwrap();
        for (directory_name, entry_name) in [
            ("workspaces", "ws_01890a5d-ac96-774b-bd5b-55c7b8d09f42"),
            ("trash", "remove-ws_01890a5d-ac96-774b-bd5b-55c7b8d09f42"),
        ] {
            let directory_path = temp.path().join(directory_name);
            fs::create_dir(&directory_path).unwrap();
            fs::set_permissions(&directory_path, fs::Permissions::from_mode(0o700)).unwrap();
            let directory = open_private_directory(&directory_path).unwrap();
            require_missing_entry(&directory, OsStr::new(entry_name)).unwrap();
            let foreign = directory_path.join(entry_name);
            fs::create_dir(&foreign).unwrap();
            assert_eq!(
                require_missing_entry(&directory, OsStr::new(entry_name))
                    .unwrap_err()
                    .kind(),
                PortErrorKind::InvalidLayout
            );
            fs::remove_dir(&foreign).unwrap();
        }
    }

    #[test]
    fn volume_uuid_bytes_require_a_nonzero_value_and_use_canonical_lowercase() {
        assert_eq!(
            decode_volume_id(None).unwrap_err().kind(),
            PortErrorKind::CapabilityUnavailable
        );
        assert_eq!(
            decode_volume_id(Some([0; 16])).unwrap_err().kind(),
            PortErrorKind::CapabilityUnavailable
        );
        assert_eq!(
            decode_volume_id(Some([
                0x1a, 0x42, 0xc8, 0x88, 0x32, 0xe3, 0x48, 0x9c, 0x9b, 0xfa, 0x67, 0xfd, 0x64, 0x0a,
                0x94, 0xe8,
            ]))
            .unwrap()
            .to_string(),
            "1a42c888-32e3-489c-9bfa-67fd640a94e8"
        );
    }

    #[test]
    fn layout_revalidation_classification_preserves_true_io_failures() {
        let io = classify_layout_revalidation(PortError::new(
            PortErrorKind::Io,
            "injected filesystem failure",
        ));
        assert_eq!(io.kind(), PortErrorKind::Io);

        for kind in [
            PortErrorKind::InvalidData,
            PortErrorKind::NotFound,
            PortErrorKind::Conflict,
            PortErrorKind::CapabilityUnavailable,
        ] {
            let error = classify_layout_revalidation(PortError::new(kind, "injected drift"));
            assert_eq!(error.kind(), PortErrorKind::InvalidLayout);
        }
    }

    #[test]
    fn prepared_workspace_revalidates_each_persisted_directory_identity() {
        let test_root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-layout-v2-tests");
        fs::create_dir_all(&test_root).unwrap();
        let temp = Builder::new()
            .prefix("prepared-ownership-")
            .tempdir_in(fs::canonicalize(test_root).unwrap())
            .unwrap();
        let parent_path = temp.path().join("parent");
        fs::create_dir(&parent_path).unwrap();
        let target_root = crate::filesystem::absolute_from_path(&parent_path.join("copy")).unwrap();
        let (parent, root_name) = open_target_parent(&target_root).unwrap();
        let root = create_target_child_directory(&parent, &root_name).unwrap();
        let staging = create_target_child_directory(&parent, OsStr::new("staging")).unwrap();
        let trash = create_target_child_directory(&parent, OsStr::new("trash")).unwrap();
        let staging_root = crate::filesystem::absolute_from_path(&staging.path).unwrap();
        let trash_root = crate::filesystem::absolute_from_path(&trash.path).unwrap();
        let metadata_path = temp.path().join("metadata");
        fs::create_dir(&metadata_path).unwrap();
        fs::set_permissions(&metadata_path, fs::Permissions::from_mode(0o700)).unwrap();
        let ownership_metadata = open_private_directory(&metadata_path).unwrap();
        let (ownership_file, ownership_identity) =
            create_private_file(&ownership_metadata, OsStr::new("proof")).unwrap();
        let workspace_id =
            WorkspaceId::from_str("ws_01890a5d-ac96-774b-bd5b-55c7b8d09f48").unwrap();
        let volume_id = volume_id_for_directory(&parent).unwrap();
        let ownership = WorkspaceOwnership {
            instance_id: InstanceId::from_str("01890a5d-ac96-774b-bd5b-55c7b8d09f33").unwrap(),
            workspace_id,
            volume_id,
            target_path: target_root.clone(),
            parent: historical_target_parent_identity(&parent).unwrap(),
            target: historical_directory_identity(&root).unwrap(),
            staging: OperationDirectoryOwnership {
                path: staging_root.clone(),
                identity: historical_directory_identity(&staging).unwrap(),
            },
            trash: OperationDirectoryOwnership {
                path: trash_root.clone(),
                identity: historical_directory_identity(&trash).unwrap(),
            },
            isolated_path: None,
        };
        let mut prepared = MacOsPreparedWorkspace {
            parent,
            root,
            staging,
            trash,
            ownership_metadata,
            ownership_file,
            ownership_identity,
            ownership: ownership.clone(),
            target_root,
            staging_root,
            trash_root,
            volume_id,
        };
        prepared.revalidate_directories().unwrap();

        prepared.ownership.staging.path =
            crate::filesystem::absolute_from_path(&parent_path.join("wrong-staging")).unwrap();
        assert!(prepared.revalidate_directories().is_err());
        prepared.ownership = ownership.clone();
        prepared.ownership.staging.identity.inode += 1;
        assert!(prepared.revalidate_directories().is_err());
        prepared.ownership = ownership.clone();
        prepared.ownership.target_path =
            crate::filesystem::absolute_from_path(&parent_path.join("wrong-target")).unwrap();
        assert!(prepared.revalidate_directories().is_err());
        prepared.ownership = ownership.clone();
        prepared.ownership.parent.inode += 1;
        assert!(prepared.revalidate_directories().is_err());
        prepared.ownership = ownership.clone();
        prepared.ownership.target.inode += 1;
        assert!(prepared.revalidate_directories().is_err());
        prepared.ownership = ownership;
        prepared.ownership.isolated_path =
            Some(crate::filesystem::absolute_from_path(&parent_path.join("isolated")).unwrap());
        assert!(prepared.revalidate_directories().is_err());
    }

    #[test]
    #[ignore = "requires THINWS_P1_CROSS_VOLUME_ROOT on an APFS volume distinct from system temp"]
    fn layout_revalidation_rejects_a_real_different_apfs_volume() {
        let repository_root = std::env::var_os("THINWS_P1_CROSS_VOLUME_ROOT")
            .expect("THINWS_P1_CROSS_VOLUME_ROOT must name the prepared APFS mount");
        let repository_root = PathBuf::from(repository_root);
        fs::create_dir_all(&repository_root).unwrap();
        let data_root = Builder::new()
            .prefix("registered-root-")
            .tempdir_in(fs::canonicalize(repository_root).unwrap())
            .unwrap();
        let foreign = Builder::new().prefix("foreign-volume-").tempdir().unwrap();
        let data_root_path = fs::canonicalize(data_root.path()).unwrap();
        let foreign_path = fs::canonicalize(foreign.path()).unwrap();
        fs::set_permissions(&data_root_path, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&foreign_path, fs::Permissions::from_mode(0o700)).unwrap();
        let data_root = open_private_directory(&data_root_path).unwrap();
        let foreign = open_private_directory(&foreign_path).unwrap();
        let registered_volume = volume_id_for_directory(&data_root).unwrap();
        let foreign_volume = volume_id_for_directory(&foreign).unwrap();
        assert_ne!(registered_volume, foreign_volume);

        let (database_file, database_identity) =
            create_private_file(&foreign, OsStr::new(STATE_DATABASE_NAME)).unwrap();
        let database_path =
            crate::filesystem::absolute_from_path(&foreign_path.join(STATE_DATABASE_NAME)).unwrap();
        let layout = MacOsDataRootLayout {
            data_root,
            controlled_directories: vec![foreign],
            database_file,
            database_identity,
            database_path,
            volume_id: registered_volume,
            instance_id: InstanceId::from_str("01890a5d-ac96-774b-bd5b-55c7b8d09f33").unwrap(),
        };

        assert_eq!(
            layout.revalidate().unwrap_err().kind(),
            PortErrorKind::InvalidLayout
        );
    }
}
