use std::ffi::OsStr;
use std::fs::File;

use rustix::fs::{AtFlags, RenameFlags};

use thinws_core::{
    AbsolutePath, InstallationIdentity, RootMarker, RootMarkerState, VolumeId, WorkspaceId,
};
use thinws_ports::{
    BootstrapStore, DataRootLayoutEvidence, LifecycleLockGuard, LifecycleScope, PortConflict,
    PortError, PortErrorKind, PreparedDataRootEvidence, PreparedWorkspaceEvidence, PublishResult,
    RemovalLogRecord, WorkspaceRemoval,
};

use crate::destroy::remove_root_contents;
use crate::document::{
    DocumentError, WorkspaceOwnership, decode_bootstrap_config, decode_root_marker,
    decode_workspace_ownership, encode_config, encode_marker, encode_workspace_ownership,
};
use crate::filesystem::{
    FileIdentity, NoReplaceError, PrivateTemp, ValidatedDirectory, create_private_child_directory,
    create_private_file, duplicate_validated_directory, entry_identity,
    historical_directory_identity, io_error, open_owned_child_directory,
    open_private_child_directory, open_private_directory, open_private_directory_optional,
    open_private_file, path_from_absolute, prepare_private_directory, read_private_file,
    require_empty_directory, revalidate_attached_directory, revalidate_directory, sync_directory,
    unlink_entry, validate_file_entry,
};
use crate::volume::decode_volume_id;
use crate::{MacOsHostAdapter, MacOsLockGuard};

const CONFIG_NAME: &str = "config.toml";
const MARKER_NAME: &str = ".thinws-root.toml";
const STATE_DATABASE_NAME: &str = "state.db";
const CONTROLLED_DIRECTORIES: [&str; 5] = ["metadata", "logs", "workspaces", "staging", "trash"];

/// Non-copyable proof that this process published one exact initializing marker.
pub struct MacOsInitializingProof {
    data_root: ValidatedDirectory,
    marker_file: File,
    marker_identity: FileIdentity,
    marker: RootMarker,
}

/// Descriptor-backed evidence for one prepared APFS data root.
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

/// Descriptor-backed evidence for one controlled data-root layout.
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
    container: ValidatedDirectory,
    state: ValidatedDirectory,
    root: ValidatedDirectory,
    incomplete_file: File,
    incomplete_identity: FileIdentity,
    ownership_metadata: ValidatedDirectory,
    ownership_file: File,
    ownership_identity: FileIdentity,
    ownership: WorkspaceOwnership,
    target_root: AbsolutePath,
    volume_id: VolumeId,
}

impl PreparedWorkspaceEvidence for MacOsPreparedWorkspace {
    fn target_root(&self) -> &AbsolutePath {
        &self.target_root
    }

    fn target_identity(&self) -> thinws_core::FileIdentity {
        self.root.identity.as_core()
    }

    fn revalidate(&self) -> Result<(), PortError> {
        self.revalidate_directories()?;
        validate_file_entry(
            &self.state.fd,
            OsStr::new("incomplete"),
            &self.incomplete_file,
            self.incomplete_identity,
        )?;
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
        for directory in [&self.container, &self.state] {
            revalidate_directory(directory)?;
            if volume_id_for_directory(directory)? != self.volume_id {
                return Err(PortError::new(
                    PortErrorKind::InvalidLayout,
                    "revalidate Workspace container volume",
                ));
            }
        }
        revalidate_attached_directory(&self.container, &self.root, OsStr::new("root"))?;
        if volume_id_for_directory(&self.root)? != self.volume_id {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "revalidate Workspace root volume",
            ));
        }
        if !self
            .ownership
            .container
            .permits_current(historical_directory_identity(&self.container)?)
            || !self
                .ownership
                .root
                .permits_current(historical_directory_identity(&self.root)?)
        {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "Workspace historical directory identity changed",
            ));
        }
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
                "revalidate data-root volume identity",
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
        let directory = prepare_private_directory(&path_from_absolute(data_root), true)?;
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
                "validate prepared data root",
                PortConflict::InstallationIdentity,
            ));
        }
        let directory = prepared.directory;
        revalidate_directory(&directory)?;
        if volume_id_for_directory(&directory)? != identity.volume_id() {
            return Err(PortError::conflict(
                "revalidate prepared data-root volume",
                PortConflict::InstallationIdentity,
            ));
        }
        require_empty_directory(&directory)?;
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
                "revalidate initializing data-root volume",
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
        let Some(data_root) =
            open_private_directory_optional(&path_from_absolute(identity.data_root()))?
        else {
            return Err(PortError::new(
                PortErrorKind::Unavailable,
                "open registered data root",
            ));
        };
        if volume_id_for_directory(&data_root)? != identity.volume_id() {
            return Err(PortError::new(
                PortErrorKind::InvalidData,
                "validate registered data-root volume",
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
    ) -> Result<Self::PreparedWorkspace, PortError> {
        self.validate_data_root_lock(lock, layout)?;
        layout.revalidate()?;
        let workspaces = &layout.controlled_directories[2];
        let name = workspace_id.to_string();
        let container = create_private_child_directory(workspaces, OsStr::new(&name))?;
        let state = create_private_child_directory(&container, OsStr::new(".state"))?;
        let (incomplete_file, incomplete_identity) =
            create_private_file(&state, OsStr::new("incomplete"))?;
        let root = create_private_child_directory(&container, OsStr::new("root"))?;
        require_empty_directory(&root)?;
        let ownership = WorkspaceOwnership {
            instance_id: layout.instance_id,
            workspace_id,
            volume_id: layout.volume_id,
            container: historical_directory_identity(&container)?,
            root: historical_directory_identity(&root)?,
        };
        let ownership_metadata = duplicate_validated_directory(&layout.controlled_directories[0])?;
        let ownership_bytes = encode_workspace_ownership(ownership).map_err(document_error)?;
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
        let target_root = crate::filesystem::absolute_from_path(&root.path).map_err(|error| {
            PortError::new(PortErrorKind::InvalidData, "derive Workspace target path")
                .with_source(error)
        })?;
        let prepared = MacOsPreparedWorkspace {
            container,
            state,
            root,
            incomplete_file,
            incomplete_identity,
            ownership_metadata,
            ownership_file,
            ownership_identity,
            ownership,
            target_root,
            volume_id: layout.volume_id,
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
        let workspaces = &layout.controlled_directories[2];
        if prepared.container.path.parent() != Some(workspaces.path.as_path()) {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "verify prepared Workspace parent",
            ));
        }
        unlink_entry(&prepared.state.fd, OsStr::new("incomplete"))?;
        sync_directory(&prepared.state.fd)?;
        prepared.revalidate_directories()?;
        if read_private_file(&prepared.state.fd, OsStr::new("incomplete"))?.is_some() {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "verify incomplete marker removal",
            ));
        }
        layout.revalidate()?;
        Ok(())
    }

    fn validate_ready_workspace(
        &self,
        layout: &Self::DataRootLayout,
        workspace_id: WorkspaceId,
    ) -> Result<AbsolutePath, PortError> {
        layout.revalidate()?;
        let workspaces = &layout.controlled_directories[2];
        let container =
            open_private_child_directory(workspaces, OsStr::new(&workspace_id.to_string()))?;
        let state = open_private_child_directory(&container, OsStr::new(".state"))?;
        if read_private_file(&state.fd, OsStr::new("incomplete"))?.is_some() {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "Ready Workspace still has an incomplete marker",
            ));
        }
        let root = open_owned_child_directory(&container, OsStr::new("root"))?;
        revalidate_attached_directory(&container, &root, OsStr::new("root"))?;
        let ownership = read_workspace_ownership(&layout.controlled_directories[0], workspace_id)?;
        if ownership.instance_id != layout.instance_id
            || ownership.volume_id != layout.volume_id
            || !ownership
                .container
                .permits_current(historical_directory_identity(&container)?)
            || !ownership
                .root
                .permits_current(historical_directory_identity(&root)?)
        {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "Ready Workspace historical ownership changed",
            ));
        }
        for directory in [&container, &state, &root] {
            if volume_id_for_directory(directory)? != layout.volume_id {
                return Err(PortError::new(
                    PortErrorKind::InvalidLayout,
                    "Ready Workspace volume changed",
                ));
            }
        }
        let path = crate::filesystem::absolute_from_path(&root.path).map_err(|error| {
            PortError::new(PortErrorKind::InvalidData, "derive Ready Workspace path")
                .with_source(error)
        })?;
        revalidate_attached_directory(
            workspaces,
            &container,
            OsStr::new(&workspace_id.to_string()),
        )?;
        revalidate_attached_directory(&container, &state, OsStr::new(".state"))?;
        revalidate_attached_directory(&container, &root, OsStr::new("root"))?;
        if read_workspace_ownership(&layout.controlled_directories[0], workspace_id)? != ownership {
            return Err(PortError::new(
                PortErrorKind::InvalidLayout,
                "Ready Workspace ownership proof changed",
            ));
        }
        layout.revalidate()?;
        Ok(path)
    }

    fn inspect_removal_container(
        &self,
        lock: &Self::LockGuard,
        layout: &Self::DataRootLayout,
        workspace_id: WorkspaceId,
    ) -> Result<Option<AbsolutePath>, PortError> {
        self.validate_data_root_lock(lock, layout)?;
        layout.revalidate()?;
        let workspaces = &layout.controlled_directories[2];
        let trash = &layout.controlled_directories[4];
        let active_name = workspace_id.to_string();
        let isolated_name = format!("remove-{workspace_id}");
        let active = open_optional_private_child(workspaces, OsStr::new(&active_name))?;
        let isolated = open_optional_private_child(trash, OsStr::new(&isolated_name))?;
        let located = match (active, isolated) {
            (None, None) => None,
            (Some(_), Some(_)) => {
                return Err(PortError::new(
                    PortErrorKind::InvalidLayout,
                    "Workspace exists at both active and isolated paths",
                ));
            }
            (Some(container), None) => Some((workspaces, active_name.as_str(), container)),
            (None, Some(container)) => Some((trash, isolated_name.as_str(), container)),
        };
        let result = if let Some((parent, name, container)) = located {
            let _ = validate_removal_layout(layout, workspace_id, &container)?;
            revalidate_attached_directory(parent, &container, OsStr::new(name))?;
            Some(
                crate::filesystem::absolute_from_path(&container.path).map_err(|error| {
                    PortError::new(PortErrorKind::InvalidData, "derive removal container path")
                        .with_source(error)
                })?,
            )
        } else {
            None
        };
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
        workspace_id: WorkspaceId,
    ) -> Result<WorkspaceRemoval, PortError> {
        self.validate_data_root_lock(lock, layout)?;
        layout.revalidate()?;
        let workspaces = &layout.controlled_directories[2];
        let trash = &layout.controlled_directories[4];
        let name = workspace_id.to_string();
        let isolated_name = format!("remove-{workspace_id}");
        let source = open_optional_private_child(workspaces, OsStr::new(&name))?;
        let isolated = open_optional_private_child(trash, OsStr::new(&isolated_name))?;
        let container = match (source, isolated) {
            (None, None) => return Ok(WorkspaceRemoval::AlreadyAbsent),
            (Some(_), Some(_)) => {
                return Err(PortError::new(
                    PortErrorKind::InvalidLayout,
                    "Workspace exists at both active and isolated paths",
                ));
            }
            (None, Some(container)) => container,
            (Some(source), None) => {
                let _ = validate_removal_layout(layout, workspace_id, &source)?;
                revalidate_attached_directory(workspaces, &source, OsStr::new(&name))?;
                self.validate_data_root_lock(lock, layout)?;
                rustix::fs::renameat_with(
                    &workspaces.fd,
                    name.as_str(),
                    &trash.fd,
                    isolated_name.as_str(),
                    RenameFlags::NOREPLACE,
                )
                .map_err(|error| io_error("isolate Workspace without replacement", error))?;
                sync_directory(&workspaces.fd)?;
                sync_directory(&trash.fd)?;
                let moved = open_private_child_directory(trash, OsStr::new(&isolated_name));
                let matches_source = moved
                    .as_ref()
                    .is_ok_and(|moved| moved.identity == source.identity);
                if !matches_source {
                    return Err(PortError::new(
                        PortErrorKind::InvalidLayout,
                        "isolated Workspace identity changed; unsafe to restore or remove",
                    ));
                }
                moved.expect("matched isolated Workspace is open")
            }
        };
        let post_isolation = self
            .validate_data_root_lock(lock, layout)
            .and_then(|()| {
                revalidate_attached_directory(trash, &container, OsStr::new(&isolated_name))
            })
            .and_then(|()| validate_removal_layout(layout, workspace_id, &container));
        // A failed recheck leaves the container at its isolated name. A later
        // explicit request may retry only after proving that same container.
        let (state, root) = post_isolation?;
        revalidate_attached_directory(trash, &container, OsStr::new(&isolated_name))?;
        let mut root_entries = 0;
        if let Some(root) = &root {
            revalidate_attached_directory(&container, root, OsStr::new("root"))?;
            let verify_scope = || {
                self.validate_data_root_lock(lock, layout)
                    .and_then(|()| {
                        revalidate_attached_directory(trash, &container, OsStr::new(&isolated_name))
                    })
                    .and_then(|()| {
                        revalidate_attached_directory(&container, root, OsStr::new("root"))
                    })
                    .map_err(|_| std::io::Error::from_raw_os_error(libc::ESTALE))
            };
            root_entries = remove_root_contents(&root.fd, root.identity.as_core(), &verify_scope)
                .map_err(classify_root_removal_error)?;
            revalidate_attached_directory(&container, root, OsStr::new("root"))?;
            rustix::fs::unlinkat(&container.fd, "root", AtFlags::REMOVEDIR)
                .map_err(|error| io_error("remove Workspace root directory", error))?;
            sync_directory(&container.fd)?;
        }
        if let Some(state) = &state {
            revalidate_attached_directory(&container, state, OsStr::new(".state"))?;
            if read_private_file(&state.fd, OsStr::new("incomplete"))?.is_some() {
                unlink_entry(&state.fd, OsStr::new("incomplete"))?;
                sync_directory(&state.fd)?;
            }
            require_empty_directory(state)?;
            revalidate_attached_directory(&container, state, OsStr::new(".state"))?;
            rustix::fs::unlinkat(&container.fd, ".state", AtFlags::REMOVEDIR)
                .map_err(|error| io_error("remove Workspace state directory", error))?;
            sync_directory(&container.fd)?;
        }
        require_empty_directory(&container)?;
        revalidate_attached_directory(trash, &container, OsStr::new(&isolated_name))?;
        rustix::fs::unlinkat(&trash.fd, isolated_name.as_str(), AtFlags::REMOVEDIR)
            .map_err(|error| io_error("remove Workspace container", error))?;
        sync_directory(&trash.fd)?;
        self.validate_data_root_lock(lock, layout)?;
        layout.revalidate()?;
        require_missing_entry(workspaces, OsStr::new(&name))?;
        require_missing_entry(trash, OsStr::new(&isolated_name))?;
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
        .map_err(|error| io_error("inspect data-root filesystem", error))?;
    if filesystem != "apfs" {
        return Err(PortError::new(
            PortErrorKind::CapabilityUnavailable,
            "require APFS data root",
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
            lock.protects_directory(&layout.controlled_directories[0]),
        ) != (LifecycleScope::DataRoot, true, true)
        {
            return Err(PortError::new(
                PortErrorKind::InvalidData,
                "require matching data-root lifecycle lock",
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

fn require_removal_ownership(
    layout: &MacOsDataRootLayout,
    workspace_id: WorkspaceId,
    container: &ValidatedDirectory,
) -> Result<WorkspaceOwnership, PortError> {
    let ownership = read_workspace_ownership(&layout.controlled_directories[0], workspace_id)?;
    if ownership.instance_id != layout.instance_id
        || ownership.volume_id != layout.volume_id
        || !ownership
            .container
            .permits_current(historical_directory_identity(container)?)
        || volume_id_for_directory(container)? != layout.volume_id
    {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "Workspace container historical ownership changed",
        ));
    }
    Ok(ownership)
}

fn validate_removal_layout(
    layout: &MacOsDataRootLayout,
    workspace_id: WorkspaceId,
    container: &ValidatedDirectory,
) -> Result<(Option<ValidatedDirectory>, Option<ValidatedDirectory>), PortError> {
    let ownership = require_removal_ownership(layout, workspace_id, container)?;
    let state = open_optional_private_child(container, OsStr::new(".state"))?;
    let root = open_optional_owned_child(container, OsStr::new("root"))?;
    if let Some(root) = &root
        && (!ownership
            .root
            .permits_current(historical_directory_identity(root)?)
            || volume_id_for_directory(root)? != layout.volume_id)
    {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "Workspace root historical ownership changed",
        ));
    }
    if let Some(state) = &state
        && volume_id_for_directory(state)? != layout.volume_id
    {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "Workspace state volume changed",
        ));
    }
    require_only_entries(container, &[".state", "root"])?;
    if let Some(state) = &state {
        require_only_entries(state, &["incomplete"])?;
        let _ = read_private_file(&state.fd, OsStr::new("incomplete"))?;
    }
    Ok((state, root))
}

fn open_optional_private_child(
    parent: &ValidatedDirectory,
    name: &OsStr,
) -> Result<Option<ValidatedDirectory>, PortError> {
    match rustix::fs::statat(&parent.fd, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => open_private_child_directory(parent, name).map(Some),
        Err(rustix::io::Errno::NOENT) => Ok(None),
        Err(error) => Err(io_error("inspect Workspace private child", error)),
    }
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

fn require_only_entries(directory: &ValidatedDirectory, allowed: &[&str]) -> Result<(), PortError> {
    let names = crate::ffi::read_directory(&directory.fd)
        .map_err(|error| io_error("inspect Workspace platform entries", error))?;
    if names
        .iter()
        .any(|name| !allowed.iter().any(|allowed| name == OsStr::new(allowed)))
    {
        return Err(PortError::new(
            PortErrorKind::InvalidLayout,
            "Workspace container has unrecognized platform entries",
        ));
    }
    Ok(())
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
            "revalidate controlled data-root layout",
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
    #[ignore = "requires THINWS_P1_CROSS_VOLUME_ROOT on an APFS volume distinct from system temp"]
    fn layout_revalidation_rejects_a_real_different_apfs_volume() {
        let Some(repository_root) = std::env::var_os("THINWS_P1_CROSS_VOLUME_ROOT") else {
            eprintln!("skipped: THINWS_P1_CROSS_VOLUME_ROOT is not set");
            return;
        };
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
