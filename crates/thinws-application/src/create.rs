//! Workspace creation orchestration over existing platform and metadata Ports.

use std::str::FromStr;

use thinws_core::{
    AbsolutePath, ErrorCode, Evidence, FallbackPolicy, FallbackReason, InstallationRecord,
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
    target: AbsolutePath,
    name: WorkspaceName,
    allow_full_copy: bool,
    now: UnixMillis,
}

impl CreateRequest {
    /// Parses CLI path/name bytes and a caller-supplied Unix-millisecond clock.
    pub fn try_from_raw(
        source: Vec<u8>,
        target: Vec<u8>,
        name: &str,
        allow_full_copy: bool,
        now_ms: i64,
    ) -> Result<Self, UseCaseError> {
        let source = AbsolutePath::try_from_bytes(source).map_err(|_| {
            semantic_error(ErrorCode::Usage, "source must be a canonical absolute path")
        })?;
        let target = AbsolutePath::try_from_bytes(target).map_err(|_| {
            semantic_error(ErrorCode::Usage, "target must be a canonical absolute path")
        })?;
        let name = WorkspaceName::from_str(name)
            .map_err(|_| semantic_error(ErrorCode::Usage, "invalid Workspace name"))?;
        let now = UnixMillis::new(now_ms).map_err(|_| {
            semantic_error(ErrorCode::Usage, "system clock predates the Unix epoch")
        })?;
        Ok(Self::new(source, target, name, allow_full_copy, now))
    }

    /// Creates intent from already validated domain values.
    #[must_use]
    pub const fn new(
        source: AbsolutePath,
        target: AbsolutePath,
        name: WorkspaceName,
        allow_full_copy: bool,
        now: UnixMillis,
    ) -> Self {
        Self {
            source,
            target,
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

    /// Returns the exact requested final target path.
    #[must_use]
    pub const fn target(&self) -> &AbsolutePath {
        &self.target
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
    target: AbsolutePath,
    source_volume_id: VolumeId,
    target_volume_id: VolumeId,
    effective_mode: MaterializationMode,
    selected_adapter: MaterializerKind,
    fallback_reason: Option<FallbackReason>,
}

impl CreatePreview {
    /// Returns the exact final target requested by the caller.
    #[must_use]
    pub const fn target(&self) -> &AbsolutePath {
        &self.target
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
    /// Previews current platform materialization facts without any product-state write.
    pub fn preview_create(&self, request: &CreateRequest) -> Result<CreatePreview, UseCaseError> {
        let identity = self
            .bootstrap
            .read_config()
            .map_err(|error| map_port(Stage::Control, error))?
            .ok_or_else(|| {
                semantic_error(
                    ErrorCode::NotInitialized,
                    "ThinWorkspace is not initialized",
                )
            })?;
        if requested_paths_overlap(request.source(), request.target(), identity.data_root()) {
            return Err(semantic_error(
                ErrorCode::TargetLayout,
                "source, target, and control root must not overlap",
            ));
        }
        let layout = self
            .bootstrap
            .validate_layout(&identity)
            .map_err(|error| map_port(Stage::Control, error))?;
        self.require_ready_marker(&identity)?;
        let snapshot = self
            .metadata
            .inspect(
                &layout,
                &InstallationRecord::new(identity.clone(), request.now()),
                self.sqlite_timeout,
            )
            .map_err(|error| map_port(Stage::Metadata, error))?;
        require_no_active_workspace_path_overlap(request.target(), snapshot.workspaces())?;
        let source = self
            .bootstrap
            .inspect_path(request.source())
            .map_err(|error| map_port(Stage::Source, error))?;
        require_existing_source(&source)?;
        let target_parent = target_parent(request.target())?;
        // This nonce only probes possible private siblings. Dry-run neither
        // reserves nor returns a Workspace ID.
        let paths = provisional_materialization_paths(
            request.source(),
            request.target(),
            &target_parent,
            WorkspaceId::new(),
        )?;
        let report = self
            .bootstrap
            .inspect_materialization_paths(&paths)
            .map_err(|error| map_port(Stage::Layout, error))?;
        require_existing_source(report.source())?;
        if let Evidence::Known(volume) = report.target_root().filesystem().volume_id() {
            require_no_active_workspace_identity_overlap(
                &self.bootstrap,
                report.target_root(),
                *volume,
                snapshot.workspaces(),
            )?;
        }
        require_missing_target(report.target_root(), &target_parent)?;
        let data_root_report = self
            .bootstrap
            .inspect_path(identity.data_root())
            .map_err(|error| map_port(Stage::Layout, error))?;
        if materialization_overlaps_control(
            report.source(),
            report.target_root(),
            &data_root_report,
        ) {
            return Err(semantic_error(
                ErrorCode::TargetLayout,
                "source or target overlaps the control root",
            ));
        }
        let plan = select_plan(&report, request.allow_full_copy())?;
        layout
            .revalidate()
            .map_err(|error| map_port(Stage::Layout, error))?;
        self.require_ready_marker(&identity)?;
        Ok(CreatePreview {
            target: request.target().clone(),
            source_volume_id: plan.source_volume_id(),
            target_volume_id: plan.target_volume_id(),
            effective_mode: plan.effective_mode(),
            selected_adapter: plan.selected_adapter(),
            fallback_reason: plan.fallback_reason(),
        })
    }

    /// Creates an ordinary Workspace directory using injected CoW and Full Copy backends.
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
        self.create_impl(request, clone_materializer, Some(copy_materializer))
    }

    /// Creates only with the selected CoW backend; a platform without a Full
    /// Copy executor cannot silently take that fallback even when requested.
    pub fn create_cow_only<C>(
        &self,
        request: CreateRequest,
        clone_materializer: &C,
    ) -> Result<CreateOutcome, UseCaseError>
    where
        C: WorkspaceMaterializer,
    {
        self.create_impl(request, clone_materializer, None)
    }

    fn create_impl<C>(
        &self,
        request: CreateRequest,
        clone_materializer: &C,
        copy_materializer: Option<&dyn WorkspaceMaterializer>,
    ) -> Result<CreateOutcome, UseCaseError>
    where
        C: WorkspaceMaterializer,
    {
        if !matches!(
            clone_materializer.kind(),
            MaterializerKind::ApfsFileClone | MaterializerKind::BtrfsReflink
        ) || copy_materializer.is_some_and(|copy| copy.kind() != MaterializerKind::FullCopy)
        {
            return Err(semantic_error(
                ErrorCode::CapabilityUnavailable,
                "required materialization backend is unavailable",
            ));
        }
        let identity = self
            .bootstrap
            .read_config()
            .map_err(|error| map_port(Stage::Control, error))?
            .ok_or_else(|| {
                semantic_error(
                    ErrorCode::NotInitialized,
                    "ThinWorkspace is not initialized",
                )
            })?;
        if requested_paths_overlap(request.source(), request.target(), identity.data_root()) {
            return Err(semantic_error(
                ErrorCode::TargetLayout,
                "source, target, and control root must not overlap",
            ));
        }
        // Classify an already missing or invalid registered root before the
        // lock's metadata parent can turn that condition into a generic error.
        self.bootstrap
            .validate_layout(&identity)
            .map_err(|error| map_port(Stage::Control, error))?;
        let lock = self
            .bootstrap
            .acquire_data_root(identity.data_root(), self.lock_timeout)
            .map_err(|error| {
                // A registered root may disappear between the preflight and
                // the lock attempt; reclassify from a fresh layout fact.
                match self.bootstrap.validate_layout(&identity) {
                    Err(layout_error) => map_port(Stage::Control, layout_error),
                    Ok(_) => map_port(Stage::Lock, error),
                }
            })?;
        lock.revalidate()
            .map_err(|error| map_port(Stage::Layout, error))?;
        let layout = self
            .bootstrap
            .validate_layout(&identity)
            .map_err(|error| map_port(Stage::Control, error))?;
        self.require_ready_marker(&identity)?;
        let mut metadata = self
            .metadata
            .open_existing(
                &layout,
                &InstallationRecord::new(identity.clone(), request.now()),
                self.sqlite_timeout,
            )
            .map_err(|error| map_port(Stage::Metadata, error))?;
        let active = metadata
            .workspaces()
            .map_err(|error| map_port(Stage::Metadata, error))?;
        if let Some(existing) = active
            .iter()
            .find(|record| record.reservation().name() == request.name())
            .cloned()
        {
            if existing.reservation().source_path() != request.source()
                || existing.reservation().target_path() != request.target()
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
                .validate_ready_workspace(&layout, existing.reservation())
                .map_err(|error| map_port(Stage::Layout, error))?;
            if &verified_path != existing.reservation().target_path() {
                return Err(semantic_error(
                    ErrorCode::TargetLayout,
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
        require_no_active_workspace_path_overlap(request.target(), &active)?;

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
                ErrorCode::TargetLayout,
                "source and control root must not overlap",
            ));
        }
        let target_parent = target_parent(request.target())?;
        let workspace_id = WorkspaceId::new();
        let preliminary = self
            .bootstrap
            .inspect_materialization_paths(&provisional_materialization_paths(
                request.source(),
                request.target(),
                &target_parent,
                workspace_id,
            )?)
            .map_err(|error| map_port(Stage::Layout, error))?;
        require_existing_source(preliminary.source())?;
        if let Evidence::Known(volume) = preliminary.target_root().filesystem().volume_id() {
            require_no_active_workspace_identity_overlap(
                &self.bootstrap,
                preliminary.target_root(),
                *volume,
                &active,
            )?;
        }
        require_missing_target(preliminary.target_root(), &target_parent)?;
        if target_enters_existing_root(preliminary.target_root(), &data_root_report) {
            return Err(semantic_error(
                ErrorCode::TargetLayout,
                "target overlaps the control root",
            ));
        }
        let preliminary_plan = select_plan(&preliminary, request.allow_full_copy())?;
        let target = request.target().clone();
        let reservation = WorkspaceReservation::new(
            workspace_id,
            identity.instance_id(),
            request.name().clone(),
            request.source().clone(),
            target.clone(),
            preliminary_plan.source_volume_id(),
            preliminary_plan.target_volume_id(),
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
    fn create_reserved<C>(
        &self,
        request: &CreateRequest,
        lock: &<B as BootstrapStore>::LockGuard,
        layout: &<B as BootstrapStore>::DataRootLayout,
        data_root: &AbsolutePath,
        metadata: &mut dyn MetadataStore,
        reservation: &WorkspaceReservation,
        clone_materializer: &C,
        copy_materializer: Option<&dyn WorkspaceMaterializer>,
    ) -> Result<CreateOutcome, UseCaseError>
    where
        C: WorkspaceMaterializer,
    {
        let prepared = self
            .bootstrap
            .prepare_workspace(
                lock,
                layout,
                reservation.workspace_id(),
                reservation.target_path(),
            )
            .map_err(|error| map_port(Stage::Layout, error))?;
        if prepared.target_root() != reservation.target_path() {
            return Err(semantic_error(
                ErrorCode::TargetLayout,
                "prepared target differs from reserved target",
            ));
        }
        let materialize = MaterializeRequest::new(
            request.source().clone(),
            reservation.target_path().clone(),
            prepared.staging_root().clone(),
            prepared.trash_root().clone(),
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
        if materialization_overlaps_control(
            report.source(),
            report.target_root(),
            &data_root_report,
        ) {
            return Err(semantic_error(
                ErrorCode::TargetLayout,
                "source or target overlaps the control root",
            ));
        }
        let mut plan = select_plan(&report, request.allow_full_copy())?;
        if !matches_reserved_volumes(
            plan.source_volume_id(),
            plan.target_volume_id(),
            reservation,
        ) {
            return Err(semantic_error(
                ErrorCode::TargetLayout,
                "materialization volume differs from reservation",
            ));
        }
        if plan.selected_adapter() != MaterializerKind::FullCopy
            && plan.selected_adapter() != clone_materializer.kind()
        {
            return Err(semantic_error(
                ErrorCode::CapabilityUnavailable,
                "selected CoW backend does not match the injected materializer",
            ));
        }
        let receipt = if plan.selected_adapter() == MaterializerKind::FullCopy {
            let copy_materializer = copy_materializer.ok_or_else(|| {
                semantic_error(
                    ErrorCode::CapabilityUnavailable,
                    "Full Copy executor is unavailable on this platform",
                )
            })?;
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
                                "CoW clone is unavailable and Full Copy was not authorized",
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
                    let copy_materializer = copy_materializer.ok_or_else(|| {
                        semantic_error(
                            ErrorCode::CapabilityUnavailable,
                            "Full Copy executor is unavailable on this platform",
                        )
                        .with_partial_receipt(partial.clone())
                    })?;
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
            ErrorCode::TargetLayout,
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
    if report.cow_clone().state() == SupportState::Unsupported && allow_full_copy {
        MaterializationPlan::for_full_copy_after_preflight(report, policy).map_err(map_plan_error)
    } else {
        MaterializationPlan::for_cow_clone(report, policy).map_err(map_plan_error)
    }
}

fn active_workspace_protected_paths(
    record: &WorkspaceRecord,
) -> Result<[AbsolutePath; 4], UseCaseError> {
    let registered = record.reservation();
    let parent = target_parent(registered.target_path())?;
    let operation = provisional_materialization_paths(
        registered.source_path(),
        registered.target_path(),
        &parent,
        registered.workspace_id(),
    )?;
    let isolated_name = format!(".thinws-remove-{}", registered.workspace_id());
    Ok([
        registered.target_path().clone(),
        operation.staging().clone(),
        operation.trash().clone(),
        derived_path(&parent, &[isolated_name.as_bytes()])?,
    ])
}

fn require_no_active_workspace_path_overlap(
    target: &AbsolutePath,
    active: &[WorkspaceRecord],
) -> Result<(), UseCaseError> {
    for record in active {
        if active_workspace_protected_paths(record)?
            .iter()
            .any(|path| paths_overlap(path, target))
        {
            return Err(semantic_error(
                ErrorCode::TargetConflict,
                "Workspace target overlaps an active Workspace path",
            ));
        }
    }
    Ok(())
}

fn require_no_active_workspace_identity_overlap(
    probe: &impl PlatformProbe,
    target: &PathCapabilityReport,
    target_volume_id: VolumeId,
    active: &[WorkspaceRecord],
) -> Result<(), UseCaseError> {
    for record in active {
        let registered = record.reservation();
        if registered.target_volume_id() != target_volume_id {
            // A different APFS volume cannot own this target's existing parent.
            // In particular, an offline target on another volume must not block
            // creation on the available volume.
            continue;
        }
        for path in active_workspace_protected_paths(record)? {
            let existing = probe
                .inspect_path(&path)
                .map_err(|error| map_port(Stage::Layout, error))?;
            if existing.resolution() == PathResolution::ExistingDirectory
                && target_enters_existing_root(target, &existing)
            {
                return Err(semantic_error(
                    ErrorCode::TargetConflict,
                    "Workspace target enters an active Workspace path",
                ));
            }
        }
    }
    Ok(())
}

fn paths_overlap(left: &AbsolutePath, right: &AbsolutePath) -> bool {
    fn contains(parent: &[u8], child: &[u8]) -> bool {
        parent == b"/"
            || child == parent
            || (child.starts_with(parent) && child.get(parent.len()) == Some(&b'/'))
    }
    contains(left.as_bytes(), right.as_bytes()) || contains(right.as_bytes(), left.as_bytes())
}

fn requested_paths_overlap(
    source: &AbsolutePath,
    target: &AbsolutePath,
    control: &AbsolutePath,
) -> bool {
    paths_overlap(source, control)
        || paths_overlap(target, control)
        || paths_overlap(source, target)
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

fn materialization_overlaps_control(
    source: &PathCapabilityReport,
    target: &PathCapabilityReport,
    control: &PathCapabilityReport,
) -> bool {
    paths_overlap_with_identity(source, control) || target_enters_existing_root(target, control)
}

fn target_enters_existing_root(
    target: &PathCapabilityReport,
    existing: &PathCapabilityReport,
) -> bool {
    // The target can be missing while its parent is an APFS case/Unicode alias
    // of an existing control or Workspace root. Spelling alone misses that edge.
    let existing_identity = existing
        .ancestry()
        .last()
        .expect("a path report has a nonempty ancestry")
        .identity();
    target
        .ancestry()
        .iter()
        .any(|entry| entry.identity() == existing_identity)
}

fn matches_reserved_volumes(
    source: VolumeId,
    target: VolumeId,
    reservation: &WorkspaceReservation,
) -> bool {
    source == reservation.source_volume_id() && target == reservation.target_volume_id()
}

fn target_parent(target: &AbsolutePath) -> Result<AbsolutePath, UseCaseError> {
    let bytes = target.as_bytes();
    if bytes == b"/" {
        return Err(semantic_error(
            ErrorCode::TargetLayout,
            "filesystem root cannot be a Workspace target",
        ));
    }
    let last_slash = bytes
        .iter()
        .rposition(|byte| *byte == b'/')
        .expect("validated absolute path has a separator");
    let parent = if last_slash == 0 {
        b"/".as_slice()
    } else {
        &bytes[..last_slash]
    };
    AbsolutePath::try_from_bytes(parent.to_vec())
        .map_err(|_| semantic_error(ErrorCode::TargetLayout, "invalid Workspace target parent"))
}

fn require_missing_target(
    target: &PathCapabilityReport,
    expected_parent: &AbsolutePath,
) -> Result<(), UseCaseError> {
    if target.resolution() == PathResolution::ExistingDirectory {
        return Err(semantic_error(
            ErrorCode::TargetExists,
            "Workspace target already exists",
        ));
    }
    if target.resolution() != PathResolution::MissingTarget
        || target.nearest_existing_ancestor() != expected_parent
        || target.missing_components().len() != 1
    {
        return Err(semantic_error(
            ErrorCode::TargetLayout,
            "Workspace target parent must exist",
        ));
    }
    Ok(())
}

fn provisional_materialization_paths(
    source: &AbsolutePath,
    target: &AbsolutePath,
    parent: &AbsolutePath,
    workspace_id: WorkspaceId,
) -> Result<MaterializationPathProbeRequest, UseCaseError> {
    let staging_name = format!(".thinws-staging-{workspace_id}");
    let trash_name = format!(".thinws-trash-{workspace_id}");
    Ok(MaterializationPathProbeRequest::new(
        source.clone(),
        target.clone(),
        derived_path(parent, &[staging_name.as_bytes()])?,
        derived_path(parent, &[trash_name.as_bytes()])?,
    ))
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
        .map_err(|_| semantic_error(ErrorCode::TargetLayout, "invalid derived Workspace path"))
}

fn map_plan_error(error: MaterializationPlanError) -> UseCaseError {
    match error {
        MaterializationPlanError::CandidateUnsupported => semantic_error(
            ErrorCode::CowUnavailable,
            "CoW clone is unavailable and Full Copy was not authorized",
        ),
        MaterializationPlanError::UnknownVolume => semantic_error(
            ErrorCode::CapabilityUnavailable,
            "materialization volume identity is unknown",
        ),
        MaterializationPlanError::WrongFilesystem
        | MaterializationPlanError::DifferentVolume
        | MaterializationPlanError::DifferentMount => semantic_error(
            ErrorCode::TargetLayout,
            "materialization paths do not satisfy the selected filesystem layout",
        ),
        MaterializationPlanError::UnknownMount => semantic_error(
            ErrorCode::CapabilityUnavailable,
            "materialization mount identity is unknown",
        ),
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
                "CoW clone is unavailable and Full Copy was not authorized",
            ),
            source: Some(Box::new(error)),
            partial_receipts: Box::default(),
            git_inspection: None,
        }
    } else {
        map_port(Stage::Layout, error)
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;
    use std::str::FromStr;

    use tempfile::Builder;
    use thinws_adapter_macos::MacOsHostAdapter;
    use thinws_core::{
        DirectoryIdentityEvidence, Evidence, FileIdentity, FileSystemIdentity, InstanceId,
        MountEvidence, VolumeId,
    };
    use thinws_ports::PlatformProbe;

    use super::*;

    fn path(value: &str) -> AbsolutePath {
        AbsolutePath::try_from_bytes(value.as_bytes()).unwrap()
    }

    fn missing_target_report(
        requested: &str,
        nearest_existing_ancestor: &str,
        missing_components: &[&str],
    ) -> PathCapabilityReport {
        let ancestor = path(nearest_existing_ancestor);
        PathCapabilityReport::new(
            path(requested),
            PathResolution::MissingTarget,
            ancestor.clone(),
            missing_components
                .iter()
                .map(|component| component.as_bytes().to_vec())
                .collect(),
            vec![DirectoryIdentityEvidence::new(
                ancestor,
                FileIdentity::new(1, 2),
            )],
            FileSystemIdentity::new(
                "apfs",
                [1, 2],
                Evidence::Known(
                    VolumeId::from_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
                ),
            ),
            MountEvidence::new(0, true),
            SupportState::Supported,
            SupportState::Supported,
            SupportState::Supported,
        )
        .unwrap()
    }

    #[test]
    fn missing_target_guard_rejects_each_inconsistent_port_fact() {
        let expected_parent = path("/data/workspaces");
        let valid = missing_target_report("/data/workspaces/clone", "/data/workspaces", &["clone"]);
        assert!(require_missing_target(&valid, &expected_parent).is_ok());

        // PathCapabilityReport permits a Port to supply these inconsistent
        // combinations; creation must not authorize either one as a target.
        let wrong_ancestor =
            missing_target_report("/data/workspaces/clone", "/data/other", &["clone"]);
        assert_eq!(
            require_missing_target(&wrong_ancestor, &expected_parent)
                .unwrap_err()
                .diagnostic()
                .code(),
            ErrorCode::TargetLayout
        );
        let wrong_suffix = missing_target_report(
            "/data/workspaces/clone",
            "/data/workspaces",
            &["extra", "clone"],
        );
        assert_eq!(
            require_missing_target(&wrong_suffix, &expected_parent)
                .unwrap_err()
                .diagnostic()
                .code(),
            ErrorCode::TargetLayout
        );
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
    fn requested_path_overlap_rejects_each_independent_pair() {
        let control = path("/control");
        assert!(requested_paths_overlap(
            &path("/control/source"),
            &path("/target"),
            &control
        ));
        assert!(requested_paths_overlap(
            &path("/source"),
            &path("/control/target"),
            &control
        ));
        assert!(requested_paths_overlap(
            &path("/source"),
            &path("/source/target"),
            &control
        ));
        assert!(!requested_paths_overlap(
            &path("/source"),
            &path("/target"),
            &control
        ));
    }

    #[test]
    fn materialization_overlap_checks_source_and_target_identity_independently() {
        fn absolute(path: &Path) -> AbsolutePath {
            AbsolutePath::try_from_bytes(path.as_os_str().as_bytes().to_vec()).unwrap()
        }

        let temp = Builder::new().prefix("create-overlap-").tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let control = root.join(".thinws");
        let source = root.join("source");
        let source_inside_control = control.join("source");
        fs::create_dir(&control).unwrap();
        fs::create_dir(&source).unwrap();
        fs::create_dir(&source_inside_control).unwrap();
        let adapter = MacOsHostAdapter::new(&control).unwrap();
        let control = adapter.inspect_path(&absolute(&control)).unwrap();
        let source = adapter.inspect_path(&absolute(&source)).unwrap();
        let source_inside_control = adapter
            .inspect_path(&absolute(&source_inside_control))
            .unwrap();
        let ordinary_target = adapter
            .inspect_path(&absolute(&root.join("target")))
            .unwrap();
        let control_target = adapter
            .inspect_path(&absolute(&root.join(".thinws/target")))
            .unwrap();

        assert!(!materialization_overlaps_control(
            &source,
            &ordinary_target,
            &control
        ));
        assert!(materialization_overlaps_control(
            &source_inside_control,
            &ordinary_target,
            &control
        ));
        assert!(materialization_overlaps_control(
            &source,
            &control_target,
            &control
        ));
    }

    #[test]
    fn existing_workspace_identity_detects_an_apfs_spelling_alias() {
        fn absolute(path: &Path) -> AbsolutePath {
            AbsolutePath::try_from_bytes(path.as_os_str().as_bytes().to_vec()).unwrap()
        }

        let temp = Builder::new()
            .prefix("create-active-alias-")
            .tempdir()
            .unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let active_path = root.join("active");
        fs::create_dir(&active_path).unwrap();
        fs::create_dir(root.join("control")).unwrap();
        let adapter = MacOsHostAdapter::new(root.join("control")).unwrap();
        let active = adapter.inspect_path(&absolute(&active_path)).unwrap();
        let child = adapter
            .inspect_path(&absolute(&active_path.join("child")))
            .unwrap();
        // Model an APFS case/Unicode alias: the caller's spelling differs,
        // while no-follow ancestry still identifies the same parent object.
        let alias = PathCapabilityReport::new(
            absolute(&root.join("alias/child")),
            child.resolution(),
            child.nearest_existing_ancestor().clone(),
            child.missing_components().to_vec(),
            child.ancestry().to_vec(),
            child.filesystem().clone(),
            child.mount(),
            child.readability(),
            child.writability(),
            child.cow_clone(),
        )
        .unwrap();
        assert!(!paths_overlap(
            alias.requested_path(),
            active.requested_path()
        ));
        assert!(target_enters_existing_root(&alias, &active));

        let volume = match active.filesystem().volume_id() {
            Evidence::Known(volume) => *volume,
            other => panic!("real APFS path needs a known volume: {other:?}"),
        };
        let now = UnixMillis::new(1).unwrap();
        let reservation = WorkspaceReservation::new(
            WorkspaceId::new(),
            InstanceId::new(),
            WorkspaceName::from_str("active").unwrap(),
            absolute(&root.join("source")),
            absolute(&active_path),
            volume,
            volume,
            false,
            now,
        );
        let active_record =
            WorkspaceRecord::new(reservation, WorkspaceState::Ready, None, now).unwrap();
        assert_eq!(
            require_no_active_workspace_identity_overlap(
                &adapter,
                &alias,
                volume,
                &[active_record]
            )
            .unwrap_err()
            .diagnostic()
            .code(),
            ErrorCode::TargetConflict
        );
    }

    #[test]
    fn an_offline_target_on_another_volume_does_not_block_creation() {
        let temp = Builder::new()
            .prefix("create-other-volume-")
            .tempdir()
            .unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let adapter = MacOsHostAdapter::new(root.join("control")).unwrap();
        let candidate = missing_target_report("/available/clone", "/available", &["clone"]);
        let available = VolumeId::from_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let unavailable = VolumeId::from_str("550e8400-e29b-41d4-a716-446655440001").unwrap();
        let now = UnixMillis::new(1).unwrap();
        let reservation = WorkspaceReservation::new(
            WorkspaceId::new(),
            InstanceId::new(),
            WorkspaceName::from_str("offline").unwrap(),
            path("/offline/source"),
            path("/offline/target"),
            unavailable,
            unavailable,
            false,
            now,
        );
        let active = WorkspaceRecord::new(reservation, WorkspaceState::Ready, None, now).unwrap();
        assert!(
            require_no_active_workspace_identity_overlap(
                &adapter,
                &candidate,
                available,
                &[active]
            )
            .is_ok()
        );
    }

    #[test]
    fn active_workspace_protects_all_paths_that_its_cleanup_can_remove() {
        let id = WorkspaceId::new();
        let volume = VolumeId::from_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let now = UnixMillis::new(1).unwrap();
        let reservation = WorkspaceReservation::new(
            id,
            InstanceId::new(),
            WorkspaceName::from_str("parent").unwrap(),
            path("/source"),
            path("/parent/target"),
            volume,
            volume,
            false,
            now,
        );
        let record = WorkspaceRecord::new(reservation, WorkspaceState::Ready, None, now).unwrap();
        let protected = active_workspace_protected_paths(&record).unwrap();
        assert_eq!(protected[0], path("/parent/target"));
        assert_eq!(protected[1], path(&format!("/parent/.thinws-staging-{id}")));
        assert_eq!(protected[2], path(&format!("/parent/.thinws-trash-{id}")));
        assert_eq!(protected[3], path(&format!("/parent/.thinws-remove-{id}")));
        for location in protected {
            let nested = derived_path(&location, &[b"child"]).unwrap();
            assert_eq!(
                require_no_active_workspace_path_overlap(&nested, std::slice::from_ref(&record))
                    .unwrap_err()
                    .diagnostic()
                    .code(),
                ErrorCode::TargetConflict
            );
        }
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
            target: path("/data/workspaces/clone"),
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
        assert_eq!(
            map_plan_error(MaterializationPlanError::UnknownMount)
                .diagnostic()
                .code(),
            ErrorCode::CapabilityUnavailable
        );
        for cause in [
            MaterializationPlanError::WrongFilesystem,
            MaterializationPlanError::DifferentVolume,
            MaterializationPlanError::DifferentMount,
        ] {
            assert_eq!(
                map_plan_error(cause).diagnostic().code(),
                ErrorCode::TargetLayout
            );
        }
    }
}
