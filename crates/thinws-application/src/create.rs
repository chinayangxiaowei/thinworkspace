//! Workspace creation orchestration over existing platform and metadata Ports.

use std::str::FromStr;

use thinws_core::{
    AbsolutePath, ErrorCode, FallbackPolicy, FallbackReason, InstallationRecord,
    MaterializationFailureKind, MaterializationMode, MaterializationPathReport,
    MaterializationPlan, MaterializationPlanError, MaterializeRequest, MaterializerKind,
    PathCapabilityReport, PathResolution, SupportState, UnixMillis, VolumeId, WorkspaceId,
    WorkspaceName, WorkspaceRecord, WorkspaceReservation, WorkspaceState,
};
use thinws_ports::{
    BootstrapStore, DataRootLayoutEvidence, FinalMaterializationSummary, LifecycleLock,
    LifecycleLockGuard, MaterializationPathProbeRequest, MetadataStore, MetadataStoreFactory,
    PlatformProbe, PortErrorKind, PreparedWorkspaceEvidence, WorkspaceMaterializer,
};

use crate::{Stage, ThinWorkspaceService, UseCaseError, map_port, semantic_error};

/// Validated intent for one new or idempotent Workspace creation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateRequest {
    source: AbsolutePath,
    name: WorkspaceName,
    allow_full_copy: bool,
    now: UnixMillis,
}

impl CreateRequest {
    /// Parses CLI path/name bytes and a caller-supplied Unix-millisecond clock.
    pub fn try_from_raw(
        source: Vec<u8>,
        name: &str,
        allow_full_copy: bool,
        now_ms: i64,
    ) -> Result<Self, UseCaseError> {
        let source = AbsolutePath::try_from_bytes(source).map_err(|_| {
            semantic_error(ErrorCode::Usage, "source must be a canonical absolute path")
        })?;
        let name = WorkspaceName::from_str(name)
            .map_err(|_| semantic_error(ErrorCode::Usage, "invalid Workspace name"))?;
        let now = UnixMillis::new(now_ms).map_err(|_| {
            semantic_error(ErrorCode::Usage, "system clock predates the Unix epoch")
        })?;
        Ok(Self::new(source, name, allow_full_copy, now))
    }

    /// Creates intent from already validated domain values.
    #[must_use]
    pub const fn new(
        source: AbsolutePath,
        name: WorkspaceName,
        allow_full_copy: bool,
        now: UnixMillis,
    ) -> Self {
        Self {
            source,
            name,
            allow_full_copy,
            now,
        }
    }

    /// Returns the canonical source path.
    #[must_use]
    pub const fn source(&self) -> &AbsolutePath {
        &self.source
    }

    /// Returns the globally unique requested Workspace name.
    #[must_use]
    pub const fn name(&self) -> &WorkspaceName {
        &self.name
    }

    /// Returns whether Full Copy was explicitly authorized if CoW is unavailable.
    #[must_use]
    pub const fn allow_full_copy(&self) -> bool {
        self.allow_full_copy
    }

    /// Returns the caller-supplied operation timestamp.
    #[must_use]
    pub const fn now(&self) -> UnixMillis {
        self.now
    }
}

/// Ready Workspace and durable materialization facts returned by create.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateOutcome {
    record: WorkspaceRecord,
    materialization: FinalMaterializationSummary,
    created: bool,
}

impl CreateOutcome {
    /// Combines a verified Ready record and its immutable final receipt summary.
    #[must_use]
    pub const fn new(
        record: WorkspaceRecord,
        materialization: FinalMaterializationSummary,
        created: bool,
    ) -> Self {
        Self {
            record,
            materialization,
            created,
        }
    }

    /// Returns the durable Workspace record.
    #[must_use]
    pub const fn record(&self) -> &WorkspaceRecord {
        &self.record
    }

    /// Returns actual materialization facts, including any fallback.
    #[must_use]
    pub const fn materialization(&self) -> &FinalMaterializationSummary {
        &self.materialization
    }

    /// Reports whether this request materialized a new Workspace.
    #[must_use]
    pub const fn created(&self) -> bool {
        self.created
    }
}

/// Read-only, non-executable creation facts without a Workspace identifier.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreatePreview {
    target_parent: AbsolutePath,
    source_volume_id: VolumeId,
    target_volume_id: VolumeId,
    effective_mode: MaterializationMode,
    selected_adapter: MaterializerKind,
    fallback_reason: Option<FallbackReason>,
}

impl CreatePreview {
    /// Returns the private parent under which an ID-derived target would be placed.
    #[must_use]
    pub const fn target_parent(&self) -> &AbsolutePath {
        &self.target_parent
    }

    /// Returns the source volume observed by the current probe.
    #[must_use]
    pub const fn source_volume_id(&self) -> VolumeId {
        self.source_volume_id
    }

    /// Returns the registered target volume observed by the current probe.
    #[must_use]
    pub const fn target_volume_id(&self) -> VolumeId {
        self.target_volume_id
    }

    /// Returns the mode that a new create would currently attempt.
    #[must_use]
    pub const fn effective_mode(&self) -> MaterializationMode {
        self.effective_mode
    }

    /// Returns the backend selected by current read-only evidence.
    #[must_use]
    pub const fn selected_adapter(&self) -> MaterializerKind {
        self.selected_adapter
    }

    /// Returns a preflight fallback reason, if one is currently required.
    #[must_use]
    pub const fn fallback_reason(&self) -> Option<FallbackReason> {
        self.fallback_reason
    }
}

impl<B, M> ThinWorkspaceService<B, M>
where
    B: BootstrapStore + LifecycleLock<Guard = <B as BootstrapStore>::LockGuard> + PlatformProbe,
    M: MetadataStoreFactory<<B as BootstrapStore>::DataRootLayout>,
{
    /// Previews current APFS materialization facts without any product-state write.
    pub fn preview_create(&self, request: &CreateRequest) -> Result<CreatePreview, UseCaseError> {
        let identity = self
            .bootstrap
            .read_config()
            .map_err(|error| map_port(Stage::Bootstrap, error))?
            .ok_or_else(|| {
                semantic_error(
                    ErrorCode::NotInitialized,
                    "ThinWorkspace is not initialized",
                )
            })?;
        if paths_overlap(request.source(), identity.data_root()) {
            return Err(semantic_error(
                ErrorCode::DataRootLayout,
                "source and data root must not overlap",
            ));
        }
        let layout = self
            .bootstrap
            .validate_layout(&identity)
            .map_err(|error| map_port(Stage::Layout, error))?;
        self.require_ready_marker(&identity)?;
        self.metadata
            .inspect(
                &layout,
                &InstallationRecord::new(identity.clone(), request.now()),
                self.sqlite_timeout,
            )
            .map_err(|error| map_port(Stage::Metadata, error))?;
        let source = self
            .bootstrap
            .inspect_path(request.source())
            .map_err(|error| map_port(Stage::Source, error))?;
        require_existing_source(&source)?;
        let target_parent = derived_path(identity.data_root(), &[b"workspaces"])?;
        let paths = MaterializationPathProbeRequest::new(
            request.source().clone(),
            target_parent.clone(),
            derived_path(identity.data_root(), &[b"staging"])?,
            derived_path(identity.data_root(), &[b"trash"])?,
        );
        let report = self
            .bootstrap
            .inspect_materialization_paths(&paths)
            .map_err(|error| map_port(Stage::Layout, error))?;
        require_existing_source(report.source())?;
        let data_root_report = self
            .bootstrap
            .inspect_path(identity.data_root())
            .map_err(|error| map_port(Stage::Layout, error))?;
        if paths_overlap_with_identity(report.source(), &data_root_report) {
            return Err(semantic_error(
                ErrorCode::DataRootLayout,
                "source and data root must not overlap",
            ));
        }
        let plan = select_plan(&report, request.allow_full_copy())?;
        if plan.target_volume_id() != identity.volume_id() {
            return Err(semantic_error(
                ErrorCode::DataRootLayout,
                "preview target volume differs from registered data root",
            ));
        }
        layout
            .revalidate()
            .map_err(|error| map_port(Stage::Layout, error))?;
        self.require_ready_marker(&identity)?;
        Ok(CreatePreview {
            target_parent,
            source_volume_id: plan.source_volume_id(),
            target_volume_id: plan.target_volume_id(),
            effective_mode: plan.effective_mode(),
            selected_adapter: plan.selected_adapter(),
            fallback_reason: plan.fallback_reason(),
        })
    }

    /// Creates an ordinary Workspace directory using injected APFS and Full Copy backends.
    pub fn create<C, F>(
        &self,
        request: CreateRequest,
        clone_materializer: &C,
        copy_materializer: &F,
    ) -> Result<CreateOutcome, UseCaseError>
    where
        C: WorkspaceMaterializer,
        F: WorkspaceMaterializer,
    {
        if clone_materializer.kind() != MaterializerKind::ApfsFileClone
            || copy_materializer.kind() != MaterializerKind::FullCopy
        {
            return Err(semantic_error(
                ErrorCode::CapabilityUnavailable,
                "required materialization backend is unavailable",
            ));
        }
        let identity = self
            .bootstrap
            .read_config()
            .map_err(|error| map_port(Stage::Bootstrap, error))?
            .ok_or_else(|| {
                semantic_error(
                    ErrorCode::NotInitialized,
                    "ThinWorkspace is not initialized",
                )
            })?;
        if paths_overlap(request.source(), identity.data_root()) {
            return Err(semantic_error(
                ErrorCode::DataRootLayout,
                "source and data root must not overlap",
            ));
        }
        // Classify an already missing or invalid registered root before the
        // lock's metadata parent can turn that condition into a generic error.
        self.bootstrap
            .validate_layout(&identity)
            .map_err(|error| map_port(Stage::Layout, error))?;
        let lock = self
            .bootstrap
            .acquire_data_root(identity.data_root(), self.lock_timeout)
            .map_err(|error| {
                // A registered root may disappear between the preflight and
                // the lock attempt; reclassify from a fresh layout fact.
                match self.bootstrap.validate_layout(&identity) {
                    Err(layout_error) => map_port(Stage::Layout, layout_error),
                    Ok(_) => map_port(Stage::Lock, error),
                }
            })?;
        lock.revalidate()
            .map_err(|error| map_port(Stage::Layout, error))?;
        let layout = self
            .bootstrap
            .validate_layout(&identity)
            .map_err(|error| map_port(Stage::Layout, error))?;
        self.require_ready_marker(&identity)?;
        let mut metadata = self
            .metadata
            .open_existing(
                &layout,
                &InstallationRecord::new(identity.clone(), request.now()),
                self.sqlite_timeout,
            )
            .map_err(|error| map_port(Stage::Metadata, error))?;
        if let Some(existing) = metadata
            .workspaces()
            .map_err(|error| map_port(Stage::Metadata, error))?
            .into_iter()
            .find(|record| record.reservation().name() == request.name())
        {
            if existing.reservation().source_path() != request.source()
                || existing.reservation().allow_full_copy() != request.allow_full_copy()
            {
                return Err(semantic_error(
                    ErrorCode::NameConflict,
                    "Workspace name is already used with different creation inputs",
                ));
            }
            if existing.state() != WorkspaceState::Ready {
                return Err(semantic_error(
                    ErrorCode::WorkspaceIncomplete,
                    "Workspace creation is incomplete",
                ));
            }
            lock.revalidate()
                .map_err(|error| map_port(Stage::Layout, error))?;
            let verified_path = self
                .bootstrap
                .validate_ready_workspace(&layout, existing.reservation().workspace_id())
                .map_err(|error| map_port(Stage::Layout, error))?;
            if &verified_path != existing.reservation().target_path() {
                return Err(semantic_error(
                    ErrorCode::DataRootLayout,
                    "Ready Workspace path does not match metadata",
                ));
            }
            let summary = metadata
                .final_materialization(existing.reservation().workspace_id())
                .map_err(|error| map_port(Stage::Metadata, error))?
                .ok_or_else(|| {
                    semantic_error(ErrorCode::Metadata, "Ready Workspace has no final receipt")
                })?;
            return Ok(CreateOutcome::new(existing, summary, false));
        }

        // Only this preliminary source observation precedes Creating: its actual
        // APFS Volume UUID is an immutable reservation field, not a caller guess.
        let source = self
            .bootstrap
            .inspect_path(request.source())
            .map_err(|error| map_port(Stage::Source, error))?;
        require_existing_source(&source)?;
        let data_root_report = self
            .bootstrap
            .inspect_path(identity.data_root())
            .map_err(|error| map_port(Stage::Layout, error))?;
        if paths_overlap_with_identity(&source, &data_root_report) {
            return Err(semantic_error(
                ErrorCode::DataRootLayout,
                "source and data root must not overlap",
            ));
        }
        if source.filesystem().type_name() != "apfs" {
            return Err(semantic_error(
                ErrorCode::DataRootLayout,
                "source must be on the registered APFS volume",
            ));
        }
        let source_volume = source
            .filesystem()
            .volume_id()
            .known()
            .copied()
            .ok_or_else(|| {
                semantic_error(
                    ErrorCode::CapabilityUnavailable,
                    "source volume identity is unknown",
                )
            })?;
        if source_volume != identity.volume_id() {
            return Err(semantic_error(
                ErrorCode::DataRootLayout,
                "source and data root are on different APFS volumes",
            ));
        }
        let workspace_id = WorkspaceId::new();
        let target = derived_path(
            identity.data_root(),
            &[b"workspaces", workspace_id.to_string().as_bytes(), b"root"],
        )?;
        let staging = derived_path(identity.data_root(), &[b"staging"])?;
        let trash = derived_path(identity.data_root(), &[b"trash"])?;
        let reservation = WorkspaceReservation::new(
            workspace_id,
            identity.instance_id(),
            request.name().clone(),
            request.source().clone(),
            target.clone(),
            source_volume,
            identity.volume_id(),
            request.allow_full_copy(),
            request.now(),
        );
        metadata
            .reserve_workspace(&reservation)
            .map_err(|error| map_port(Stage::Metadata, error))?;
        let result = self
            .create_reserved(
                &request,
                &lock,
                &layout,
                identity.data_root(),
                metadata.as_mut(),
                &reservation,
                staging,
                trash,
                clone_materializer,
                copy_materializer,
            )
            .map_err(|error| error.with_workspace_id(workspace_id));
        if let Err(error) = &result
            && let Err(record_error) = metadata.record_failure(
                workspace_id,
                WorkspaceState::Creating,
                error.diagnostic().code(),
                request.now(),
            )
        {
            let mut mapped =
                map_port(Stage::Metadata, record_error).with_workspace_id(workspace_id);
            for receipt in error.partial_receipts.iter().cloned() {
                mapped = mapped.with_partial_receipt(receipt);
            }
            return Err(mapped);
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn create_reserved<C, F>(
        &self,
        request: &CreateRequest,
        lock: &<B as BootstrapStore>::LockGuard,
        layout: &<B as BootstrapStore>::DataRootLayout,
        data_root: &AbsolutePath,
        metadata: &mut dyn MetadataStore,
        reservation: &WorkspaceReservation,
        staging: AbsolutePath,
        trash: AbsolutePath,
        clone_materializer: &C,
        copy_materializer: &F,
    ) -> Result<CreateOutcome, UseCaseError>
    where
        C: WorkspaceMaterializer,
        F: WorkspaceMaterializer,
    {
        let prepared = self
            .bootstrap
            .prepare_workspace(lock, layout, reservation.workspace_id())
            .map_err(|error| map_port(Stage::Layout, error))?;
        if prepared.target_root() != reservation.target_path() {
            return Err(semantic_error(
                ErrorCode::DataRootLayout,
                "prepared target differs from reserved target",
            ));
        }
        let materialize = MaterializeRequest::new(
            request.source().clone(),
            reservation.target_path().clone(),
            staging,
            trash,
        );
        let probe = MaterializationPathProbeRequest::from(&materialize);
        let report = self
            .bootstrap
            .inspect_materialization_paths(&probe)
            .map_err(|error| map_port(Stage::Layout, error))?;
        require_prepared_target(&prepared, &report)?;
        require_existing_source(report.source())?;
        let data_root_report = self
            .bootstrap
            .inspect_path(data_root)
            .map_err(|error| map_port(Stage::Layout, error))?;
        if paths_overlap_with_identity(report.source(), &data_root_report) {
            return Err(semantic_error(
                ErrorCode::DataRootLayout,
                "source and data root must not overlap",
            ));
        }
        let mut plan = select_plan(&report, request.allow_full_copy())?;
        if !matches_reserved_volumes(
            plan.source_volume_id(),
            plan.target_volume_id(),
            reservation,
        ) {
            return Err(semantic_error(
                ErrorCode::DataRootLayout,
                "materialization volume differs from reservation",
            ));
        }
        let receipt = if plan.selected_adapter() == MaterializerKind::FullCopy {
            copy_materializer
                .materialize(&materialize, &plan)
                .map_err(|failure| {
                    let (error, partial) = failure.into_parts();
                    map_port(Stage::Layout, error).with_partial_receipt(partial)
                })?
        } else {
            match clone_materializer.materialize(&materialize, &plan) {
                Ok(receipt) => receipt,
                Err(failure) => {
                    let (error, partial) = failure.into_parts();
                    if partial.failure_kind() != Some(MaterializationFailureKind::CowUnavailable) {
                        return Err(map_materialization_error(error.kind(), error)
                            .with_partial_receipt(partial));
                    }
                    if !request.allow_full_copy() {
                        return Err(UseCaseError {
                            diagnostic: thinws_core::CoreError::new(
                                ErrorCode::CowUnavailable,
                                "APFS clone is unavailable and Full Copy was not authorized",
                            ),
                            source: Some(Box::new(error)),
                            partial_receipts: Box::default(),
                            git_inspection: None,
                        }
                        .with_partial_receipt(partial));
                    }
                    let fresh = self
                        .bootstrap
                        .inspect_materialization_paths(&probe)
                        .map_err(|error| {
                            map_port(Stage::Layout, error).with_partial_receipt(partial.clone())
                        })?;
                    require_prepared_target(&prepared, &fresh)
                        .map_err(|error| error.with_partial_receipt(partial.clone()))?;
                    plan = MaterializationPlan::for_full_copy_after_cow_unavailable(
                        &fresh, &plan, &partial,
                    )
                    .map_err(|error| map_plan_error(error).with_partial_receipt(partial.clone()))?;
                    copy_materializer
                        .materialize(&materialize, &plan)
                        .map_err(|failure| {
                            let (error, copy_partial) = failure.into_parts();
                            map_port(Stage::Layout, error)
                                .with_partial_receipt(partial)
                                .with_partial_receipt(copy_partial)
                        })?
                }
            }
        };
        prepared
            .revalidate()
            .map_err(|error| map_port(Stage::Layout, error))?;
        layout
            .revalidate()
            .map_err(|error| map_port(Stage::Layout, error))?;
        lock.revalidate()
            .map_err(|error| map_port(Stage::Layout, error))?;
        self.bootstrap
            .clear_workspace_incomplete(lock, layout, prepared)
            .map_err(|error| map_port(Stage::Layout, error))?;
        let record = metadata
            .complete_materialization(reservation.workspace_id(), &plan, &receipt, request.now())
            .map_err(|error| map_port(Stage::Metadata, error))?;
        let summary = FinalMaterializationSummary::new(
            receipt.requested_mode(),
            receipt.effective_mode(),
            receipt.actual_mode(),
            receipt.actual_adapter(),
            receipt.cow_evidence(),
            receipt.fallback_reason(),
            receipt.failed_attempts().len(),
        );
        Ok(CreateOutcome::new(record, summary, true))
    }
}

fn require_prepared_target(
    prepared: &impl PreparedWorkspaceEvidence,
    report: &MaterializationPathReport,
) -> Result<(), UseCaseError> {
    prepared
        .revalidate()
        .map_err(|error| map_port(Stage::Layout, error))?;
    let target = report.target_root();
    let matches = target.resolution() == PathResolution::ExistingDirectory
        && target.requested_path() == prepared.target_root()
        && target.ancestry().last().is_some_and(|entry| {
            entry.path() == prepared.target_root() && entry.identity() == prepared.target_identity()
        });
    if !matches {
        return Err(semantic_error(
            ErrorCode::DataRootLayout,
            "materialization target differs from prepared Workspace root",
        ));
    }
    Ok(())
}

fn select_plan(
    report: &MaterializationPathReport,
    allow_full_copy: bool,
) -> Result<MaterializationPlan, UseCaseError> {
    let policy = if allow_full_copy {
        FallbackPolicy::AllowFullCopyOnCowUnsupported
    } else {
        FallbackPolicy::Deny
    };
    if report.apfs_clone().state() == SupportState::Unsupported && allow_full_copy {
        MaterializationPlan::for_full_copy_after_preflight(report, policy).map_err(map_plan_error)
    } else {
        MaterializationPlan::for_apfs_clone(report, policy).map_err(map_plan_error)
    }
}

fn paths_overlap(left: &AbsolutePath, right: &AbsolutePath) -> bool {
    fn contains(parent: &[u8], child: &[u8]) -> bool {
        parent == b"/"
            || child == parent
            || (child.starts_with(parent) && child.get(parent.len()) == Some(&b'/'))
    }
    contains(left.as_bytes(), right.as_bytes()) || contains(right.as_bytes(), left.as_bytes())
}

fn require_existing_source(source: &PathCapabilityReport) -> Result<(), UseCaseError> {
    if source.resolution() != PathResolution::ExistingDirectory {
        return Err(semantic_error(
            ErrorCode::Filesystem,
            "source directory does not exist",
        ));
    }
    match source.readability() {
        SupportState::Supported => Ok(()),
        SupportState::Unsupported => Err(semantic_error(
            ErrorCode::Filesystem,
            "source directory is not readable and searchable",
        )),
        SupportState::Unknown => Err(semantic_error(
            ErrorCode::CapabilityUnavailable,
            "source directory access could not be determined",
        )),
    }
}

fn paths_overlap_with_identity(
    source: &PathCapabilityReport,
    controlled: &PathCapabilityReport,
) -> bool {
    if paths_overlap(source.requested_path(), controlled.requested_path()) {
        return true;
    }
    // Source existence is checked by the caller; the registered root is bound
    // by the layout proof and revalidated before success. Compare descriptor
    // identities, not spelling: APFS may fold case or Unicode.
    let source_leaf = source
        .ancestry()
        .last()
        .expect("a path report has a nonempty ancestry")
        .identity();
    let controlled_leaf = controlled
        .ancestry()
        .last()
        .expect("a path report has a nonempty ancestry")
        .identity();
    controlled
        .ancestry()
        .iter()
        .any(|entry| entry.identity() == source_leaf)
        || source
            .ancestry()
            .iter()
            .any(|entry| entry.identity() == controlled_leaf)
}

fn matches_reserved_volumes(
    source: VolumeId,
    target: VolumeId,
    reservation: &WorkspaceReservation,
) -> bool {
    source == reservation.source_volume_id() && target == reservation.data_volume_id()
}

fn derived_path(root: &AbsolutePath, components: &[&[u8]]) -> Result<AbsolutePath, UseCaseError> {
    let mut bytes = root.as_bytes().to_vec();
    for component in components {
        if bytes != b"/" {
            bytes.push(b'/');
        }
        bytes.extend_from_slice(component);
    }
    AbsolutePath::try_from_bytes(bytes)
        .map_err(|_| semantic_error(ErrorCode::DataRootLayout, "invalid derived Workspace path"))
}

fn map_plan_error(error: MaterializationPlanError) -> UseCaseError {
    match error {
        MaterializationPlanError::CandidateUnsupported => semantic_error(
            ErrorCode::CowUnavailable,
            "APFS clone is unavailable and Full Copy was not authorized",
        ),
        MaterializationPlanError::UnknownVolume => semantic_error(
            ErrorCode::CapabilityUnavailable,
            "materialization volume identity is unknown",
        ),
        MaterializationPlanError::NotApfs | MaterializationPlanError::DifferentVolume => {
            semantic_error(
                ErrorCode::DataRootLayout,
                "materialization paths are not on one APFS volume",
            )
        }
        _ => semantic_error(
            ErrorCode::Filesystem,
            "materialization plan cannot be completed",
        ),
    }
}

fn map_materialization_error(kind: PortErrorKind, error: thinws_ports::PortError) -> UseCaseError {
    if kind == PortErrorKind::CapabilityUnavailable {
        UseCaseError {
            diagnostic: thinws_core::CoreError::new(
                ErrorCode::CowUnavailable,
                "APFS clone is unavailable and Full Copy was not authorized",
            ),
            source: Some(Box::new(error)),
            partial_receipts: Box::default(),
            git_inspection: None,
        }
    } else {
        map_port(Stage::Layout, error)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use thinws_core::{InstanceId, VolumeId};

    use super::*;

    fn path(value: &str) -> AbsolutePath {
        AbsolutePath::try_from_bytes(value.as_bytes()).unwrap()
    }

    #[test]
    fn overlap_is_component_aware_in_both_directions() {
        assert!(paths_overlap(&path("/a/b"), &path("/a/b")));
        assert!(paths_overlap(&path("/a"), &path("/a/b")));
        assert!(paths_overlap(&path("/a/b"), &path("/a")));
        assert!(paths_overlap(&path("/"), &path("/a")));
        assert!(!paths_overlap(&path("/a/b"), &path("/a/bc")));
        assert!(!paths_overlap(&path("/a/b"), &path("/a/c")));
    }

    #[test]
    fn reservation_requires_both_independent_volume_identites() {
        let registered = VolumeId::from_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let other = VolumeId::from_str("550e8400-e29b-41d4-a716-446655440001").unwrap();
        let reservation = WorkspaceReservation::new(
            WorkspaceId::new(),
            InstanceId::new(),
            WorkspaceName::from_str("volume-check").unwrap(),
            path("/source"),
            path("/data/workspaces/root"),
            registered,
            registered,
            false,
            UnixMillis::new(1).unwrap(),
        );
        assert!(matches_reserved_volumes(
            registered,
            registered,
            &reservation
        ));
        assert!(!matches_reserved_volumes(other, registered, &reservation));
        assert!(!matches_reserved_volumes(registered, other, &reservation));
    }

    #[test]
    fn preview_exposes_the_selected_preflight_fallback_reason() {
        let volume = VolumeId::from_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let preview = CreatePreview {
            target_parent: path("/data/workspaces"),
            source_volume_id: volume,
            target_volume_id: volume,
            effective_mode: MaterializationMode::FullCopy,
            selected_adapter: MaterializerKind::FullCopy,
            fallback_reason: Some(FallbackReason::CloneUnsupportedAtPreflight),
        };

        assert_eq!(
            preview.fallback_reason(),
            Some(FallbackReason::CloneUnsupportedAtPreflight)
        );
    }

    #[test]
    fn plan_error_classes_keep_distinct_public_codes() {
        assert_eq!(
            map_plan_error(MaterializationPlanError::CandidateUnsupported)
                .diagnostic()
                .code(),
            ErrorCode::CowUnavailable
        );
        assert_eq!(
            map_plan_error(MaterializationPlanError::UnknownVolume)
                .diagnostic()
                .code(),
            ErrorCode::CapabilityUnavailable
        );
        for cause in [
            MaterializationPlanError::NotApfs,
            MaterializationPlanError::DifferentVolume,
        ] {
            assert_eq!(
                map_plan_error(cause).diagnostic().code(),
                ErrorCode::DataRootLayout
            );
        }
    }
}
