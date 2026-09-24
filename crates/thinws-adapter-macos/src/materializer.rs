use std::collections::BTreeMap;
use std::ffi::{CString, OsString};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use thinws_core::{
    AbsolutePath, CreatedObjectEvidence, FallbackReason, FileIdentity,
    MaterializationAttemptEvidence, MaterializationFailureKind, MaterializationMode,
    MaterializationPathReport, MaterializationPlan, MaterializationReceipt, MaterializeRequest,
    MaterializedEntryKind, MaterializerKind, PathCapabilityReport, PathResolution, RelativePath,
    RollbackEvidence, RollbackStatus, SupportState, TreeDigest,
};
use thinws_ports::{
    MaterializationFailure, MaterializationPathProbeRequest, PlatformProbe, PortError,
    PortErrorKind, WorkspaceMaterializer,
};

use crate::MacOsHostAdapter;
use crate::ffi::{
    RawFileKind, RawNodeMetadata, c_string, clone_file_at, create_directory_at, create_file_at,
    create_symlink_at, file_system_metadata, node_metadata, node_metadata_at, open_directory_at,
    open_file_read_at, open_root_directory, read_directory, read_link_at, remove_staged_at,
    rename_exclusive_at, set_mode, set_modified_time, volume_uuid,
};
use crate::volume::decode_volume_id;

/// macOS APFS implementation of the Phase 1 clone-only materializer.
pub struct ApfsCloneMaterializer {
    probe: MacOsHostAdapter,
}

/// macOS implementation of the Phase 1 explicit byte-copy fallback.
pub struct FullCopyMaterializer {
    probe: MacOsHostAdapter,
}

static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

impl FullCopyMaterializer {
    /// Creates a Full Copy materializer using the same host Adapter for revalidation.
    #[must_use]
    pub const fn new(probe: MacOsHostAdapter) -> Self {
        Self { probe }
    }
}

impl WorkspaceMaterializer for FullCopyMaterializer {
    fn kind(&self) -> MaterializerKind {
        MaterializerKind::FullCopy
    }

    fn materialize(
        &self,
        request: &MaterializeRequest,
        plan: &MaterializationPlan,
    ) -> Result<MaterializationReceipt, MaterializationFailure> {
        materialize_with_backend(&self.probe, Backend::FullCopy, request, plan, &NoopHook)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Backend {
    ApfsClone,
    FullCopy,
}

impl Backend {
    const fn kind(self) -> MaterializerKind {
        match self {
            Self::ApfsClone => MaterializerKind::ApfsFileClone,
            Self::FullCopy => MaterializerKind::FullCopy,
        }
    }

    const fn mode(self) -> MaterializationMode {
        match self {
            Self::ApfsClone => MaterializationMode::CowClone,
            Self::FullCopy => MaterializationMode::FullCopy,
        }
    }

    fn candidate_is_executable(self, report: &MaterializationPathReport) -> bool {
        match self {
            Self::ApfsClone => report.apfs_clone().state() != SupportState::Unsupported,
            Self::FullCopy => report.full_copy().state() == SupportState::Supported,
        }
    }
}

impl ApfsCloneMaterializer {
    /// Creates a clone materializer using the same host Adapter for revalidation.
    #[must_use]
    pub const fn new(probe: MacOsHostAdapter) -> Self {
        Self { probe }
    }
}

impl WorkspaceMaterializer for ApfsCloneMaterializer {
    fn kind(&self) -> MaterializerKind {
        MaterializerKind::ApfsFileClone
    }

    fn materialize(
        &self,
        request: &MaterializeRequest,
        plan: &MaterializationPlan,
    ) -> Result<MaterializationReceipt, MaterializationFailure> {
        self.materialize_with_hook(request, plan, &NoopHook)
    }
}

impl ApfsCloneMaterializer {
    fn materialize_with_hook(
        &self,
        request: &MaterializeRequest,
        plan: &MaterializationPlan,
        hook: &dyn ExecutionHook,
    ) -> Result<MaterializationReceipt, MaterializationFailure> {
        materialize_with_backend(&self.probe, Backend::ApfsClone, request, plan, hook)
    }
}

fn materialize_with_backend(
    probe: &MacOsHostAdapter,
    backend: Backend,
    request: &MaterializeRequest,
    plan: &MaterializationPlan,
    hook: &dyn ExecutionHook,
) -> Result<MaterializationReceipt, MaterializationFailure> {
    let started = Instant::now();
    match materialize_inner(probe, backend, request, plan, hook, started) {
        Ok(receipt) => Ok(receipt),
        Err(mut failed) => {
            let rollback = match (
                failed.target.as_ref(),
                failed.trash.as_ref(),
                failed.target_report.as_ref(),
                failed.trash_report.as_ref(),
            ) {
                (Some(target), Some(trash), Some(target_report), Some(trash_report))
                    if failed.target_modified =>
                {
                    rollback_created(
                        RollbackRoot {
                            path: request.target(),
                            directory: target,
                            report: target_report,
                        },
                        RollbackRoot {
                            path: request.trash(),
                            directory: trash,
                            report: trash_report,
                        },
                        failed.target_baseline,
                        &failed.created,
                        hook,
                    )
                }
                _ => RollbackEvidence::new(
                    if failed.target_modified {
                        RollbackStatus::Incomplete
                    } else {
                        RollbackStatus::NotNeeded
                    },
                    Vec::new(),
                    failed
                        .created
                        .iter()
                        .map(TrackedCreated::evidence)
                        .collect(),
                ),
            };
            let created = failed
                .created
                .drain(..)
                .map(|entry| entry.evidence())
                .collect();
            let failure_kind = failed.error.kind;
            let port_error = failed.error.into_port_error();
            let receipt = match backend {
                Backend::ApfsClone => MaterializationReceipt::failed_apfs_clone(
                    plan,
                    failure_kind,
                    created,
                    failed.target_modified,
                    rollback,
                    failed.evidence,
                    elapsed_millis(started),
                ),
                Backend::FullCopy => MaterializationReceipt::failed_full_copy(
                    plan,
                    failure_kind,
                    created,
                    failed.target_modified,
                    rollback,
                    failed.evidence,
                    elapsed_millis(started),
                ),
            };
            Err(MaterializationFailure::new(port_error, receipt))
        }
    }
}

fn materialize_inner(
    probe: &MacOsHostAdapter,
    backend: Backend,
    request: &MaterializeRequest,
    plan: &MaterializationPlan,
    hook: &dyn ExecutionHook,
    started: Instant,
) -> Result<MaterializationReceipt, Box<AttemptFailure>> {
    validate_request_plan(request, plan, backend).map_err(AttemptFailure::before_write)?;
    let fresh = probe
        .inspect_materialization_paths(&MaterializationPathProbeRequest::from(request))
        .map_err(|error| {
            AttemptFailure::before_write(Failure::with_source(
                MaterializationFailureKind::PlanStale,
                PortErrorKind::InvalidLayout,
                "revalidate materialization paths",
                error,
            ))
        })?;
    if fresh.evidence_digest() != plan.probe_evidence_digest()
        || !backend.candidate_is_executable(&fresh)
    {
        return Err(AttemptFailure::before_write(Failure::new(
            MaterializationFailureKind::PlanStale,
            PortErrorKind::InvalidLayout,
            "reject stale materialization plan",
        )));
    }

    hook.after_probe().map_err(AttemptFailure::before_write)?;

    let source = open_bound_directory(request.source(), fresh.source())
        .map_err(AttemptFailure::before_write)?;
    let target = open_bound_directory(request.target(), fresh.target_root())
        .map_err(AttemptFailure::before_write)?;
    let staging = open_bound_directory(request.staging(), fresh.staging())
        .map_err(AttemptFailure::before_write)?;
    let trash = open_bound_directory(request.trash(), fresh.trash())
        .map_err(AttemptFailure::before_write)?;
    let target_baseline = node_metadata(&target).map_err(|error| {
        AttemptFailure::before_write(Failure::io(
            "inspect target root before materialization",
            error,
        ))
    })?;
    ensure_directory(target_baseline, "require target root directory")
        .map_err(AttemptFailure::before_write)?;
    ensure_empty(&target).map_err(AttemptFailure::before_write)?;

    let snapshot = snapshot_tree(&source).map_err(AttemptFailure::before_write)?;
    if backend == Backend::FullCopy
        && plan.fallback_reason() == Some(FallbackReason::CloneUnavailableAtRuntime)
        && plan
            .failed_attempts()
            .first()
            .and_then(|attempt| attempt.evidence().source_manifest_digest())
            != Some(snapshot.manifest.digest())
    {
        return Err(AttemptFailure::before_write(source_changed()));
    }
    let mut context = ExecutionContext {
        created: Vec::new(),
        clone_calls: 0,
        backend,
    };
    if let Err(error) = materialize_directory(
        &source,
        &target,
        &staging,
        &[],
        snapshot.root.device,
        &snapshot.entries,
        &mut context,
        hook,
    ) {
        let target_modified = !context.created.is_empty();
        return Err(AttemptFailure::after_write(
            error,
            FailedExecution {
                source: &source,
                target,
                trash,
                target_baseline,
                context,
                target_modified,
                source_snapshot: &snapshot,
                path_report: &fresh,
            },
        ));
    }

    if let Err(error) = hook.before_root_metadata(&target) {
        return Err(AttemptFailure::after_write(
            error,
            FailedExecution {
                source: &source,
                target,
                trash,
                target_baseline,
                context,
                target_modified: true,
                source_snapshot: &snapshot,
                path_report: &fresh,
            },
        ));
    }
    if let Err(error) = set_preserved_metadata(&target, target_baseline.identity(), snapshot.root) {
        return Err(AttemptFailure::after_write(
            error,
            FailedExecution {
                source: &source,
                target,
                trash,
                target_baseline,
                context,
                target_modified: true,
                source_snapshot: &snapshot,
                path_report: &fresh,
            },
        ));
    }

    if let Err(error) = hook.before_final_validation() {
        return Err(AttemptFailure::after_write(
            error,
            FailedExecution {
                source: &source,
                target,
                trash,
                target_baseline,
                context,
                target_modified: true,
                source_snapshot: &snapshot,
                path_report: &fresh,
            },
        ));
    }

    let source_after = match snapshot_tree(&source) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return Err(AttemptFailure::after_write(
                error.into_source_revalidation(),
                FailedExecution {
                    source: &source,
                    target,
                    trash,
                    target_baseline,
                    context,
                    target_modified: true,
                    source_snapshot: &snapshot,
                    path_report: &fresh,
                },
            ));
        }
    };
    if source_after != snapshot {
        return Err(AttemptFailure::after_write(
            source_changed(),
            FailedExecution {
                source: &source,
                target,
                trash,
                target_baseline,
                context,
                target_modified: true,
                source_snapshot: &snapshot,
                path_report: &fresh,
            },
        ));
    }
    let target_snapshot = match snapshot_tree(&target) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return Err(AttemptFailure::after_write(
                error.into_target_revalidation(),
                FailedExecution {
                    source: &source,
                    target,
                    trash,
                    target_baseline,
                    context,
                    target_modified: true,
                    source_snapshot: &snapshot,
                    path_report: &fresh,
                },
            ));
        }
    };
    if !target_snapshot
        .manifest
        .matches_promised(&snapshot.manifest)
    {
        return Err(AttemptFailure::after_write(
            Failure::new(
                MaterializationFailureKind::ManifestMismatch,
                PortErrorKind::InvalidData,
                "verify materialized target manifest",
            ),
            FailedExecution {
                source: &source,
                target,
                trash,
                target_baseline,
                context,
                target_modified: true,
                source_snapshot: &snapshot,
                path_report: &fresh,
            },
        ));
    }
    if !target_snapshot_matches_created(&target_snapshot, &context.created) {
        return Err(AttemptFailure::after_write(
            target_changed(),
            FailedExecution {
                source: &source,
                target,
                trash,
                target_baseline,
                context,
                target_modified: true,
                source_snapshot: &snapshot,
                path_report: &fresh,
            },
        ));
    }
    let final_paths = revalidate_bound_path(request.source(), fresh.source(), &source)
        .and_then(|()| revalidate_bound_path(request.target(), fresh.target_root(), &target))
        .and_then(|()| revalidate_bound_path(request.staging(), fresh.staging(), &staging))
        .and_then(|()| revalidate_bound_path(request.trash(), fresh.trash(), &trash));
    if let Err(error) = final_paths {
        return Err(AttemptFailure::after_write(
            error,
            FailedExecution {
                source: &source,
                target,
                trash,
                target_baseline,
                context,
                target_modified: true,
                source_snapshot: &snapshot,
                path_report: &fresh,
            },
        ));
    }

    let created = context
        .created
        .iter()
        .map(TrackedCreated::evidence)
        .collect();
    let receipt = match backend {
        Backend::ApfsClone => MaterializationReceipt::successful_apfs_clone(
            plan,
            snapshot.manifest.regular_files,
            context.clone_calls,
            created,
            snapshot.manifest.digest(),
            target_snapshot.manifest.digest(),
            elapsed_millis(started),
            snapshot.manifest.logical_bytes,
            Some(target_snapshot.manifest.physical_bytes),
        ),
        Backend::FullCopy => MaterializationReceipt::successful_full_copy(
            plan,
            snapshot.manifest.regular_files,
            created,
            snapshot.manifest.digest(),
            target_snapshot.manifest.digest(),
            elapsed_millis(started),
            snapshot.manifest.logical_bytes,
            Some(target_snapshot.manifest.physical_bytes),
        ),
    };
    match receipt {
        Ok(receipt) => Ok(receipt),
        Err(error) => Err(AttemptFailure::after_write(
            Failure::with_source(
                MaterializationFailureKind::ManifestMismatch,
                PortErrorKind::InvalidData,
                "construct materialization receipt",
                error,
            ),
            FailedExecution {
                source: &source,
                target,
                trash,
                target_baseline,
                context,
                target_modified: true,
                source_snapshot: &snapshot,
                path_report: &fresh,
            },
        )),
    }
}

fn validate_request_plan(
    request: &MaterializeRequest,
    plan: &MaterializationPlan,
    backend: Backend,
) -> Result<(), Failure> {
    if plan.selected_adapter() != backend.kind()
        || plan.effective_mode() != backend.mode()
        || request.source() != plan.source_path()
        || request.target() != plan.target_path()
        || request.staging() != plan.staging_path()
        || request.trash() != plan.trash_path()
    {
        return Err(Failure::new(
            MaterializationFailureKind::PlanStale,
            PortErrorKind::InvalidLayout,
            "validate materialization request against plan",
        ));
    }
    Ok(())
}

#[cfg(test)]
fn open_absolute_directory(path: &AbsolutePath) -> Result<OwnedFd, Failure> {
    let mut current =
        open_root_directory().map_err(|error| Failure::io("open APFS path root", error))?;
    for component in path.as_bytes()[1..].split(|byte| *byte == b'/') {
        if component.is_empty() {
            continue;
        }
        let name = c_string(component)
            .map_err(|error| Failure::invalid("encode APFS path component", error))?;
        current = open_directory_at(&current, &name)
            .map_err(|error| Failure::path_open("open APFS path component", error))?;
    }
    Ok(current)
}

fn open_bound_directory(
    path: &AbsolutePath,
    report: &PathCapabilityReport,
) -> Result<OwnedFd, Failure> {
    if report.requested_path() != path || report.resolution() != PathResolution::ExistingDirectory {
        return Err(stale_path_binding());
    }
    let components = path.as_bytes()[1..]
        .split(|byte| *byte == b'/')
        .filter(|component| !component.is_empty())
        .collect::<Vec<_>>();
    if report.ancestry().len() != components.len() + 1 {
        return Err(stale_path_binding());
    }

    let mut current =
        open_root_directory().map_err(|error| Failure::io("open APFS path root", error))?;
    verify_ancestry_entry(&current, b"/", &report.ancestry()[0])?;
    let mut current_path = Vec::from(b"/".as_slice());
    for (index, component) in components.into_iter().enumerate() {
        let name = c_string(component)
            .map_err(|error| Failure::invalid("encode APFS path component", error))?;
        current = open_directory_at(&current, &name)
            .map_err(|error| Failure::path_open("open APFS path component", error))?;
        if current_path.len() > 1 {
            current_path.push(b'/');
        }
        current_path.extend_from_slice(component);
        verify_ancestry_entry(&current, &current_path, &report.ancestry()[index + 1])?;
    }
    verify_directory_evidence(&current, report)?;
    Ok(current)
}

fn verify_ancestry_entry(
    directory: &OwnedFd,
    path: &[u8],
    expected: &thinws_core::DirectoryIdentityEvidence,
) -> Result<(), Failure> {
    let metadata = node_metadata(directory)
        .map_err(|error| Failure::io("inspect APFS path ancestry", error))?;
    if expected.path().as_bytes() != path
        || metadata.kind != RawFileKind::Directory
        || metadata.identity() != expected.identity()
    {
        return Err(stale_path_binding());
    }
    Ok(())
}

fn revalidate_bound_path(
    path: &AbsolutePath,
    report: &PathCapabilityReport,
    held: &OwnedFd,
) -> Result<(), Failure> {
    let reopened = open_bound_directory(path, report)?;
    let held_metadata =
        node_metadata(held).map_err(|error| Failure::io("inspect held APFS directory", error))?;
    let reopened_metadata = node_metadata(&reopened)
        .map_err(|error| Failure::io("reinspect APFS directory path", error))?;
    if held_metadata.identity() != reopened_metadata.identity()
        || held_metadata.kind != RawFileKind::Directory
        || reopened_metadata.kind != RawFileKind::Directory
    {
        return Err(stale_path_binding());
    }
    Ok(())
}

fn verify_directory_evidence(
    directory: &OwnedFd,
    report: &PathCapabilityReport,
) -> Result<(), Failure> {
    let expected_identity = report
        .ancestry()
        .last()
        .filter(|entry| entry.path() == report.requested_path())
        .map(|entry| entry.identity())
        .ok_or_else(stale_path_binding)?;
    let metadata = node_metadata(directory)
        .map_err(|error| Failure::io("inspect APFS directory identity", error))?;
    let filesystem = file_system_metadata(directory)
        .map_err(|error| Failure::io("inspect APFS directory filesystem", error))?;
    let volume = volume_uuid(directory)
        .map_err(|error| Failure::io("inspect APFS directory volume UUID", error))?;
    let volume = decode_volume_id(volume).map_err(|error| {
        Failure::with_source(
            MaterializationFailureKind::PlanStale,
            PortErrorKind::InvalidLayout,
            "decode APFS directory volume UUID",
            error,
        )
    })?;
    let expected_volume = report
        .filesystem()
        .volume_id()
        .known()
        .ok_or_else(stale_path_binding)?;
    if metadata.kind != RawFileKind::Directory
        || metadata.identity() != expected_identity
        || filesystem.type_name != report.filesystem().type_name()
        || filesystem.fsid != report.filesystem().fsid()
        || filesystem.mount_flags != report.mount().raw_flags()
        || &volume != expected_volume
    {
        return Err(stale_path_binding());
    }
    Ok(())
}

fn stale_path_binding() -> Failure {
    Failure::new(
        MaterializationFailureKind::PlanStale,
        PortErrorKind::InvalidLayout,
        "bind APFS directory descriptors to plan evidence",
    )
}

fn ensure_empty(target: &OwnedFd) -> Result<(), Failure> {
    let names =
        read_directory(target).map_err(|error| Failure::io("enumerate target root", error))?;
    if names.is_empty() {
        Ok(())
    } else {
        Err(Failure::new(
            MaterializationFailureKind::InvalidLayout,
            PortErrorKind::NotEmpty,
            "require empty target root",
        ))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TreeSnapshot {
    root: RawNodeMetadata,
    entries: BTreeMap<Vec<u8>, SnapshotEntry>,
    manifest: TreeManifest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SnapshotEntry {
    metadata: RawNodeMetadata,
    link_text: Option<Vec<u8>>,
    content_digest: Option<[u8; 32]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TreeManifest {
    root_mode: u32,
    root_mtime: (i64, i64),
    entries: Vec<ManifestEntry>,
    regular_files: u64,
    logical_bytes: u64,
    physical_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ManifestEntry {
    path: Vec<u8>,
    kind: RawFileKind,
    mode: Option<u32>,
    mtime: Option<(i64, i64)>,
    length: u64,
    content_digest: Option<[u8; 32]>,
}

impl TreeManifest {
    fn matches_promised(&self, source: &Self) -> bool {
        self.root_mode == source.root_mode
            && self.root_mtime == source.root_mtime
            && self.entries == source.entries
            && self.regular_files == source.regular_files
            && self.logical_bytes == source.logical_bytes
    }

    fn digest(&self) -> TreeDigest {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"thinws-tree-manifest-v1\0");
        hasher.update(&self.root_mode.to_le_bytes());
        update_time(&mut hasher, self.root_mtime);
        for entry in &self.entries {
            update_bytes(&mut hasher, &entry.path);
            hasher.update(&[kind_byte(entry.kind)]);
            match entry.mode {
                Some(mode) => {
                    hasher.update(&[1]);
                    hasher.update(&mode.to_le_bytes());
                }
                None => {
                    hasher.update(&[0]);
                }
            }
            match entry.mtime {
                Some(time) => {
                    hasher.update(&[1]);
                    update_time(&mut hasher, time);
                }
                None => {
                    hasher.update(&[0]);
                }
            }
            hasher.update(&entry.length.to_le_bytes());
            match entry.content_digest {
                Some(digest) => {
                    hasher.update(&[1]);
                    hasher.update(&digest);
                }
                None => {
                    hasher.update(&[0]);
                }
            }
        }
        TreeDigest::new(*hasher.finalize().as_bytes())
    }
}

fn snapshot_tree(root: &OwnedFd) -> Result<TreeSnapshot, Failure> {
    let root_before =
        node_metadata(root).map_err(|error| Failure::io("inspect manifest root", error))?;
    ensure_directory(root_before, "require manifest root directory")?;
    let mut snapshot = TreeSnapshot {
        root: root_before,
        entries: BTreeMap::new(),
        manifest: TreeManifest {
            root_mode: root_before.mode,
            root_mtime: root_before.mtime(),
            entries: Vec::new(),
            regular_files: 0,
            logical_bytes: 0,
            physical_bytes: 0,
        },
    };
    snapshot_directory(root, &[], root_before.device, &mut snapshot)?;
    let root_after =
        node_metadata(root).map_err(|error| Failure::io("reinspect manifest root", error))?;
    if root_after != root_before {
        return Err(source_changed());
    }
    Ok(snapshot)
}

fn snapshot_directory(
    directory: &OwnedFd,
    relative: &[OsString],
    root_device: u64,
    snapshot: &mut TreeSnapshot,
) -> Result<(), Failure> {
    let mut names = read_directory(directory)
        .map_err(|error| Failure::io("enumerate manifest directory", error))?;
    names.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    for name in names {
        let component = component_name(&name)?;
        let mut child = relative.to_vec();
        child.push(name);
        let path = relative_bytes(&child);
        let before = node_metadata_at(directory, &component)
            .map_err(|error| Failure::io("inspect manifest entry", error))?;
        if before.device != root_device {
            return Err(Failure::new(
                MaterializationFailureKind::UnsupportedSourceEntry,
                PortErrorKind::InvalidLayout,
                "reject source submount",
            ));
        }

        let mut entry = SnapshotEntry {
            metadata: before,
            link_text: None,
            content_digest: None,
        };
        let (mode, mtime, length, digest) = match before.kind {
            RawFileKind::Directory => {
                let child_fd = open_directory_at(directory, &component)
                    .map_err(|error| Failure::path_open("open manifest directory", error))?;
                if node_metadata(&child_fd)
                    .map_err(|error| Failure::io("reinspect manifest directory", error))?
                    != before
                {
                    return Err(source_changed());
                }
                snapshot_directory(&child_fd, &child, root_device, snapshot)?;
                (Some(before.mode), Some(before.mtime()), 0, None)
            }
            RawFileKind::RegularFile => {
                let file = open_file_read_at(directory, &component)
                    .map_err(|error| Failure::path_open("open manifest file", error))?;
                let (length, digest) = digest_file(file, before)?;
                entry.content_digest = Some(digest);
                snapshot.manifest.regular_files += 1;
                snapshot.manifest.logical_bytes =
                    snapshot.manifest.logical_bytes.saturating_add(length);
                snapshot.manifest.physical_bytes = snapshot
                    .manifest
                    .physical_bytes
                    .saturating_add(before.allocated_bytes);
                (
                    Some(before.mode),
                    Some(before.mtime()),
                    length,
                    Some(digest),
                )
            }
            RawFileKind::SymbolicLink => {
                let text = read_link_at(directory, &component)
                    .map_err(|error| Failure::io("read manifest symlink", error))?;
                if node_metadata_at(directory, &component)
                    .map_err(|error| Failure::io("reinspect manifest symlink", error))?
                    != before
                {
                    return Err(source_changed());
                }
                let digest = *blake3::hash(&text).as_bytes();
                let length = u64::try_from(text.len()).unwrap_or(u64::MAX);
                entry.link_text = Some(text);
                entry.content_digest = Some(digest);
                (None, None, length, Some(digest))
            }
            RawFileKind::Fifo
            | RawFileKind::Socket
            | RawFileKind::CharacterDevice
            | RawFileKind::BlockDevice
            | RawFileKind::Unknown => {
                return Err(Failure::new(
                    MaterializationFailureKind::UnsupportedSourceEntry,
                    PortErrorKind::InvalidData,
                    "reject unsupported source entry",
                ));
            }
        };
        snapshot.entries.insert(path.clone(), entry);
        snapshot.manifest.entries.push(ManifestEntry {
            path,
            kind: before.kind,
            mode,
            mtime,
            length,
            content_digest: digest,
        });
    }
    Ok(())
}

fn digest_file(file: OwnedFd, expected: RawNodeMetadata) -> Result<(u64, [u8; 32]), Failure> {
    let held =
        node_metadata(&file).map_err(|error| Failure::io("inspect opened manifest file", error))?;
    if held != expected {
        return Err(source_changed());
    }
    let mut file = File::from(file);
    let mut hasher = blake3::Hasher::new();
    let mut length = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| Failure::io("read manifest file", error))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        length = length.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
    }
    let after =
        node_metadata(&file).map_err(|error| Failure::io("reinspect manifest file", error))?;
    if after != expected || length != expected.size {
        return Err(source_changed());
    }
    Ok((length, *hasher.finalize().as_bytes()))
}

#[allow(clippy::too_many_arguments)]
fn materialize_directory(
    source: &OwnedFd,
    target: &OwnedFd,
    staging: &OwnedFd,
    relative: &[OsString],
    source_root_device: u64,
    expected: &BTreeMap<Vec<u8>, SnapshotEntry>,
    context: &mut ExecutionContext,
    hook: &dyn ExecutionHook,
) -> Result<(), Failure> {
    let mut names = read_directory(source)
        .map_err(|error| Failure::io("enumerate source during clone", error))?;
    names.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    for name in names {
        let component = component_name(&name)?;
        let mut child = relative.to_vec();
        child.push(name);
        let path = relative_bytes(&child);
        let expected_entry = expected.get(&path).ok_or_else(source_changed)?;
        let before = node_metadata_at(source, &component)
            .map_err(|error| Failure::io("inspect source entry before clone", error))?;
        if before != expected_entry.metadata || before.device != source_root_device {
            return Err(source_changed());
        }

        match before.kind {
            RawFileKind::Directory => {
                let source_child = open_directory_at(source, &component)
                    .map_err(|error| Failure::path_open("open source directory", error))?;
                if node_metadata(&source_child)
                    .map_err(|error| Failure::io("inspect held source directory", error))?
                    != before
                {
                    return Err(source_changed());
                }
                let identity = stage_and_publish(
                    staging,
                    target,
                    &component,
                    &child,
                    before.kind,
                    context,
                    |stage, name| create_directory_at(stage, name, 0o700),
                    Failure::io,
                    "create staged target directory",
                    || hook.after_directory_published_before_validation(&child),
                )?;
                let target_child = open_directory_at(target, &component)
                    .map_err(|error| Failure::path_open("open created target directory", error))?;
                ensure_identity(
                    node_metadata(&target_child)
                        .map_err(|error| Failure::io("inspect held target directory", error))?,
                    identity,
                    before.kind,
                )?;
                materialize_directory(
                    &source_child,
                    &target_child,
                    staging,
                    &child,
                    source_root_device,
                    expected,
                    context,
                    hook,
                )?;
                set_preserved_metadata(&target_child, identity, before)?;
                hook.after_created(&child, context.created.len())?;
                if node_metadata(&source_child)
                    .map_err(|error| Failure::io("reinspect source directory", error))?
                    != before
                {
                    return Err(source_changed());
                }
            }
            RawFileKind::RegularFile => {
                let source_file = open_file_read_at(source, &component)
                    .map_err(|error| Failure::path_open("open source file", error))?;
                if node_metadata(&source_file)
                    .map_err(|error| Failure::io("inspect held source file", error))?
                    != before
                {
                    return Err(source_changed());
                }
                let (identity, target_file) = match context.backend {
                    Backend::ApfsClone => {
                        let identity = stage_and_publish(
                            staging,
                            target,
                            &component,
                            &child,
                            before.kind,
                            context,
                            |stage, name| clone_file_at(&source_file, stage, name),
                            Failure::clone_io,
                            "clone staged target file",
                            || hook.after_clone_published_before_validation(&child),
                        )?;
                        context.clone_calls = context.clone_calls.saturating_add(1);
                        let target_file =
                            open_file_read_at(target, &component).map_err(|error| {
                                Failure::path_open("open cloned target file", error)
                            })?;
                        (identity, target_file)
                    }
                    Backend::FullCopy => {
                        let target_file = create_file_at(target, &component, 0o600)
                            .map_err(|error| Failure::io("create Full Copy target file", error))?;
                        hook.after_file_created_before_registration(&child);
                        let identity = register_created(
                            target,
                            &component,
                            &child,
                            before.kind,
                            Some(expected),
                            Some(&target_file),
                            context,
                        )?;
                        ensure_identity(
                            node_metadata(&target_file).map_err(|error| {
                                Failure::io("inspect held Full Copy target file", error)
                            })?,
                            identity,
                            before.kind,
                        )?;
                        copy_file_bytes(&source_file, &target_file, &child, hook)?;
                        (identity, target_file)
                    }
                };
                ensure_identity(
                    node_metadata(&target_file)
                        .map_err(|error| Failure::io("inspect held materialized file", error))?,
                    identity,
                    before.kind,
                )?;
                set_preserved_metadata(&target_file, identity, before)?;
                hook.after_created(&child, context.created.len())?;
                if node_metadata(&source_file)
                    .map_err(|error| Failure::io("reinspect source file", error))?
                    != before
                {
                    return Err(source_changed());
                }
            }
            RawFileKind::SymbolicLink => {
                let text = read_link_at(source, &component)
                    .map_err(|error| Failure::io("read source symlink", error))?;
                if expected_entry.link_text.as_deref() != Some(text.as_slice())
                    || node_metadata_at(source, &component)
                        .map_err(|error| Failure::io("reinspect source symlink", error))?
                        != before
                {
                    return Err(source_changed());
                }
                let text = CString::new(text)
                    .map_err(|error| Failure::invalid("encode source symlink", error))?;
                stage_and_publish(
                    staging,
                    target,
                    &component,
                    &child,
                    before.kind,
                    context,
                    |stage, name| create_symlink_at(&text, stage, name),
                    Failure::io,
                    "create staged target symlink",
                    || hook.after_symlink_published_before_validation(&child),
                )?;
                hook.after_created(&child, context.created.len())?;
            }
            RawFileKind::Fifo
            | RawFileKind::Socket
            | RawFileKind::CharacterDevice
            | RawFileKind::BlockDevice
            | RawFileKind::Unknown => {
                return Err(Failure::new(
                    MaterializationFailureKind::UnsupportedSourceEntry,
                    PortErrorKind::InvalidData,
                    "reject unsupported source entry during clone",
                ));
            }
        }
    }
    Ok(())
}

fn copy_file_bytes(
    source: &OwnedFd,
    target: &OwnedFd,
    relative: &[OsString],
    hook: &dyn ExecutionHook,
) -> Result<(), Failure> {
    let mut source = File::from(
        source
            .try_clone()
            .map_err(|error| Failure::io("duplicate Full Copy source file", error))?,
    );
    let mut target = File::from(
        target
            .try_clone()
            .map_err(|error| Failure::io("duplicate Full Copy target file", error))?,
    );
    let mut buffer = [0_u8; 64 * 1024];
    let mut written_total = 0_u64;
    loop {
        let read = source
            .read(&mut buffer)
            .map_err(|error| Failure::io("read Full Copy source file", error))?;
        if read == 0 {
            break;
        }
        target
            .write_all(&buffer[..read])
            .map_err(|error| Failure::io("write Full Copy target file", error))?;
        written_total = written_total.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        hook.after_copy_chunk(relative, written_total)?;
    }
    target
        .flush()
        .map_err(|error| Failure::io("flush Full Copy target file", error))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn stage_and_publish(
    staging: &OwnedFd,
    target: &OwnedFd,
    target_name: &CString,
    relative: &[OsString],
    kind: RawFileKind,
    context: &mut ExecutionContext,
    mut create: impl FnMut(&OwnedFd, &CString) -> io::Result<()>,
    classify_create_error: fn(&'static str, io::Error) -> Failure,
    create_operation: &'static str,
    after_publish: impl FnOnce() -> Result<(), Failure>,
) -> Result<FileIdentity, Failure> {
    let mut staged_name = None;
    for _ in 0..128 {
        let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let name = CString::new(format!(
            ".thinws-materialize-{}-{sequence}",
            std::process::id()
        ))
        .expect("staging names contain no NUL");
        match create(staging, &name) {
            Ok(()) => {
                staged_name = Some(name);
                break;
            }
            Err(error) if error.raw_os_error() == Some(libc::EEXIST) => continue,
            Err(error) => return Err(classify_create_error(create_operation, error)),
        }
    }
    let staged_name = staged_name.ok_or_else(|| {
        Failure::new(
            MaterializationFailureKind::Filesystem,
            PortErrorKind::Io,
            "reserve unique staging entry",
        )
    })?;
    // The instance-private staging namespace has no external writer while an
    // operation runs; published target names remain concurrently mutable.
    let staged = match node_metadata_at(staging, &staged_name) {
        Ok(staged) => staged,
        Err(error) => {
            remove_staged_at(staging, &staged_name, kind)
                .map_err(|cleanup| Failure::io("remove unverified staged object", cleanup))?;
            return Err(Failure::io("inspect staged object identity", error));
        }
    };
    if staged.kind != kind {
        return Err(target_changed());
    }
    let identity = staged.identity();
    context.created.push(TrackedCreated {
        relative: relative.to_vec(),
        kind,
        identity: Some(identity),
    });
    if let Err(error) = rename_exclusive_at(staging, &staged_name, target, target_name) {
        cleanup_staged(staging, &staged_name, kind, identity)?;
        context.created.pop();
        return if error.raw_os_error() == Some(libc::EEXIST) {
            Err(target_changed())
        } else {
            Err(Failure::io("publish staged target object", error))
        };
    }
    after_publish()?;
    let published = node_metadata_at(target, target_name)
        .map_err(|error| Failure::io("inspect published target identity", error))?;
    ensure_identity(published, identity, kind)?;
    Ok(identity)
}

fn cleanup_staged(
    staging: &OwnedFd,
    name: &CString,
    kind: RawFileKind,
    identity: FileIdentity,
) -> Result<(), Failure> {
    let observed = node_metadata_at(staging, name)
        .map_err(|error| Failure::io("inspect failed staged object", error))?;
    ensure_identity(observed, identity, kind)?;
    remove_staged_at(staging, name, kind)
        .map_err(|error| Failure::io("remove failed staged object", error))
}

fn register_created(
    parent: &OwnedFd,
    component: &CString,
    relative: &[OsString],
    kind: RawFileKind,
    forbidden_source_files: Option<&BTreeMap<Vec<u8>, SnapshotEntry>>,
    created_file: Option<&OwnedFd>,
    context: &mut ExecutionContext,
) -> Result<FileIdentity, Failure> {
    context.created.push(TrackedCreated {
        relative: relative.to_vec(),
        kind,
        identity: None,
    });
    let held_identity = created_file
        .map(|file| {
            node_metadata(file)
                .map(|metadata| metadata.identity())
                .map_err(|error| Failure::io("inspect new Full Copy target file", error))
        })
        .transpose()?;
    let metadata = node_metadata_at(parent, component)
        .map_err(|error| Failure::io("register created target identity", error))?;
    if metadata.kind != kind {
        return Err(target_changed());
    }
    let identity = metadata.identity();
    if held_identity.is_some_and(|held| held != identity) {
        return Err(target_changed());
    }
    if forbidden_source_files.is_some_and(|entries| {
        entries.values().any(|entry| {
            entry.metadata.kind == RawFileKind::RegularFile && entry.metadata.identity() == identity
        })
    }) {
        return Err(target_changed());
    }
    context
        .created
        .last_mut()
        .expect("the just-created target is recorded")
        .identity = Some(identity);
    Ok(identity)
}

fn set_preserved_metadata(
    target: &OwnedFd,
    expected_identity: FileIdentity,
    source: RawNodeMetadata,
) -> Result<(), Failure> {
    ensure_identity(
        node_metadata(target)
            .map_err(|error| Failure::io("inspect target before metadata update", error))?,
        expected_identity,
        source.kind,
    )?;
    set_modified_time(target, source.modified_seconds, source.modified_nanoseconds)
        .map_err(|error| Failure::io("preserve target modification time", error))?;
    set_mode(target, source.mode)
        .map_err(|error| Failure::io("preserve target permissions", error))?;
    let after = node_metadata(target)
        .map_err(|error| Failure::io("inspect target after metadata update", error))?;
    ensure_identity(after, expected_identity, source.kind)?;
    if after.mode != source.mode || after.mtime() != source.mtime() {
        return Err(Failure::new(
            MaterializationFailureKind::ManifestMismatch,
            PortErrorKind::InvalidData,
            "verify preserved target metadata",
        ));
    }
    Ok(())
}

trait ExecutionHook {
    fn after_probe(&self) -> Result<(), Failure> {
        Ok(())
    }

    fn after_created(&self, _relative: &[OsString], _created_count: usize) -> Result<(), Failure> {
        Ok(())
    }

    fn after_clone_published_before_validation(
        &self,
        _relative: &[OsString],
    ) -> Result<(), Failure> {
        Ok(())
    }

    fn after_file_created_before_registration(&self, _relative: &[OsString]) {}

    fn after_directory_published_before_validation(
        &self,
        _relative: &[OsString],
    ) -> Result<(), Failure> {
        Ok(())
    }

    fn after_symlink_published_before_validation(
        &self,
        _relative: &[OsString],
    ) -> Result<(), Failure> {
        Ok(())
    }

    fn after_copy_chunk(&self, _relative: &[OsString], _written_total: u64) -> Result<(), Failure> {
        Ok(())
    }

    fn before_root_metadata(&self, _target: &OwnedFd) -> Result<(), Failure> {
        Ok(())
    }

    fn before_final_validation(&self) -> Result<(), Failure> {
        Ok(())
    }

    fn before_rollback_detach(&self, _relative: &[OsString]) -> Result<(), Failure> {
        Ok(())
    }

    fn after_rollback_detach(&self, _relative: &[OsString]) {}
}

struct NoopHook;

impl ExecutionHook for NoopHook {}

struct ExecutionContext {
    created: Vec<TrackedCreated>,
    clone_calls: u64,
    backend: Backend,
}

#[derive(Clone)]
struct TrackedCreated {
    relative: Vec<OsString>,
    kind: RawFileKind,
    identity: Option<FileIdentity>,
}

impl TrackedCreated {
    fn evidence(&self) -> CreatedObjectEvidence {
        let path = RelativePath::try_from_bytes(relative_bytes(&self.relative))
            .expect("filesystem components form a safe relative path");
        CreatedObjectEvidence::new(path, materialized_kind(self.kind), self.identity)
    }
}

struct RollbackRoot<'a> {
    path: &'a AbsolutePath,
    directory: &'a OwnedFd,
    report: &'a PathCapabilityReport,
}

fn rollback_created(
    target: RollbackRoot<'_>,
    trash: RollbackRoot<'_>,
    baseline: Option<RawNodeMetadata>,
    created: &[TrackedCreated],
    hook: &dyn ExecutionHook,
) -> RollbackEvidence {
    let Some(baseline) = baseline else {
        return RollbackEvidence::new(
            RollbackStatus::Incomplete,
            Vec::new(),
            created.iter().map(TrackedCreated::evidence).collect(),
        );
    };
    let current_root = match node_metadata(target.directory) {
        Ok(metadata)
            if metadata.identity() == baseline.identity()
                && metadata.kind == RawFileKind::Directory =>
        {
            metadata
        }
        _ => {
            return RollbackEvidence::new(
                RollbackStatus::Incomplete,
                Vec::new(),
                created.iter().map(TrackedCreated::evidence).collect(),
            );
        }
    };
    if !path_points_to_bound(target.path, target.report, target.directory)
        || !path_points_to_bound(trash.path, trash.report, trash.directory)
    {
        return RollbackEvidence::new(
            RollbackStatus::Incomplete,
            Vec::new(),
            created.iter().map(TrackedCreated::evidence).collect(),
        );
    }
    if current_root.mode != baseline.mode && set_mode(target.directory, baseline.mode).is_err() {
        return RollbackEvidence::new(
            RollbackStatus::Incomplete,
            Vec::new(),
            created.iter().map(TrackedCreated::evidence).collect(),
        );
    }

    let mut removed = Vec::new();
    let mut quarantined = Vec::new();
    let mut unconfirmed_quarantined = Vec::new();
    let mut active_count = created.len();
    for entry in created.iter().rev() {
        if !rollback_set_matches(target.directory, &created[..active_count]) {
            break;
        }
        if !path_points_to_bound(target.path, target.report, target.directory)
            || !path_points_to_bound(trash.path, trash.report, trash.directory)
        {
            break;
        }
        let Some(expected_identity) = entry.identity else {
            break;
        };
        let Ok((parent, component)) = open_rollback_parent(target.directory, entry, created) else {
            break;
        };
        let Ok(current) = node_metadata_at(&parent, &component) else {
            break;
        };
        if current.identity() != expected_identity || current.kind != entry.kind {
            break;
        }
        if hook.before_rollback_detach(&entry.relative).is_err() {
            break;
        }
        match detach_created_to_trash(
            &parent,
            &component,
            trash.directory,
            entry,
            removed.len(),
            hook,
        ) {
            DetachOutcome::Confirmed(quarantine) => {
                quarantined.push(quarantine);
                removed.push(entry.evidence().path().clone());
                active_count -= 1;
            }
            DetachOutcome::UnconfirmedQuarantine(quarantine) => {
                unconfirmed_quarantined.push(quarantine);
                break;
            }
            DetachOutcome::NotDetached => break,
        }
    }

    let exact_remaining = rollback_set_matches(target.directory, &created[..active_count]);
    let root_restored = exact_remaining
        && path_points_to_bound(target.path, target.report, target.directory)
        && path_points_to_bound(trash.path, trash.report, trash.directory)
        && node_metadata(target.directory).is_ok_and(|metadata| {
            metadata.identity() == baseline.identity()
                && metadata.kind == RawFileKind::Directory
                && set_modified_time(
                    target.directory,
                    baseline.modified_seconds,
                    baseline.modified_nanoseconds,
                )
                .is_ok()
                && set_mode(target.directory, baseline.mode).is_ok()
                && node_metadata(target.directory).is_ok_and(|after| {
                    after.identity() == baseline.identity()
                        && after.kind == RawFileKind::Directory
                        && after.mode == baseline.mode
                        && after.mtime() == baseline.mtime()
                })
        });
    let remaining = created[..active_count]
        .iter()
        .map(TrackedCreated::evidence)
        .collect::<Vec<_>>();
    let status = if remaining.is_empty() && root_restored {
        RollbackStatus::ConfirmedBaseline
    } else {
        RollbackStatus::Incomplete
    };
    RollbackEvidence::new(status, removed, remaining)
        .with_quarantined(quarantined)
        .with_unconfirmed_quarantined(unconfirmed_quarantined)
}

fn path_points_to_bound(
    path: &AbsolutePath,
    report: &PathCapabilityReport,
    held: &OwnedFd,
) -> bool {
    let Ok(reopened) = open_bound_directory(path, report) else {
        return false;
    };
    let Ok(expected) = node_metadata(held) else {
        return false;
    };
    node_metadata(&reopened).is_ok_and(|observed| {
        observed.kind == RawFileKind::Directory
            && expected.kind == RawFileKind::Directory
            && observed.identity() == expected.identity()
    })
}

fn detach_created_to_trash(
    parent: &OwnedFd,
    component: &CString,
    trash: &OwnedFd,
    entry: &TrackedCreated,
    sequence: usize,
    hook: &dyn ExecutionHook,
) -> DetachOutcome {
    let Some(expected_identity) = entry.identity else {
        return DetachOutcome::NotDetached;
    };
    for collision in 0..128_u32 {
        let quarantine = CString::new(format!(
            ".thinws-rollback-{}-{}-{sequence}-{collision}",
            expected_identity.device(),
            expected_identity.inode(),
        ))
        .expect("rollback quarantine names contain no NUL");
        match rename_exclusive_at(parent, component, trash, &quarantine) {
            Ok(()) => {
                hook.after_rollback_detach(&entry.relative);
                let quarantine_path = RelativePath::try_from_bytes(quarantine.as_bytes().to_vec())
                    .expect("rollback quarantine names are safe relative paths");
                let moved_matches =
                    quarantined_entry_matches(trash, &quarantine, expected_identity, entry.kind);
                if moved_matches {
                    return DetachOutcome::Confirmed(quarantine_path);
                }
                return if rename_exclusive_at(trash, &quarantine, parent, component).is_ok() {
                    DetachOutcome::NotDetached
                } else {
                    DetachOutcome::UnconfirmedQuarantine(quarantine_path)
                };
            }
            Err(error) if error.raw_os_error() == Some(libc::EEXIST) => continue,
            Err(_) => return DetachOutcome::NotDetached,
        }
    }
    DetachOutcome::NotDetached
}

enum DetachOutcome {
    Confirmed(RelativePath),
    UnconfirmedQuarantine(RelativePath),
    NotDetached,
}

fn quarantined_entry_matches(
    trash: &OwnedFd,
    quarantine: &CString,
    expected_identity: FileIdentity,
    expected_kind: RawFileKind,
) -> bool {
    let moved_matches = node_metadata_at(trash, quarantine)
        .is_ok_and(|moved| moved.identity() == expected_identity && moved.kind == expected_kind);
    if !moved_matches || expected_kind != RawFileKind::Directory {
        return moved_matches;
    }
    let Ok(directory) = open_directory_at(trash, quarantine) else {
        return false;
    };
    node_metadata(&directory).is_ok_and(|held| {
        held.identity() == expected_identity && held.kind == RawFileKind::Directory
    }) && read_directory(&directory).is_ok_and(|entries| entries.is_empty())
}

fn rollback_set_matches(target: &OwnedFd, created: &[TrackedCreated]) -> bool {
    let Ok(root) = node_metadata(target) else {
        return false;
    };
    let mut observed = BTreeMap::new();
    if observe_target_entries(target, &[], root.device, &mut observed).is_err()
        || observed.len() != created.len()
    {
        return false;
    }
    created.iter().all(|entry| {
        let Some(identity) = entry.identity else {
            return false;
        };
        observed.get(&relative_bytes(&entry.relative)) == Some(&(identity, entry.kind))
    })
}

fn target_snapshot_matches_created(snapshot: &TreeSnapshot, created: &[TrackedCreated]) -> bool {
    snapshot.entries.len() == created.len()
        && created.iter().all(|entry| {
            let Some(identity) = entry.identity else {
                return false;
            };
            snapshot
                .entries
                .get(&relative_bytes(&entry.relative))
                .is_some_and(|observed| {
                    observed.metadata.identity() == identity && observed.metadata.kind == entry.kind
                })
        })
}

fn observe_target_entries(
    directory: &OwnedFd,
    relative: &[OsString],
    root_device: u64,
    observed: &mut BTreeMap<Vec<u8>, (FileIdentity, RawFileKind)>,
) -> io::Result<()> {
    let mut names = read_directory(directory)?;
    names.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    for name in names {
        let component = component_name_io(&name)?;
        let mut child = relative.to_vec();
        child.push(name);
        let metadata = node_metadata_at(directory, &component)?;
        if metadata.device != root_device {
            return Err(io::Error::from_raw_os_error(libc::EXDEV));
        }
        observed.insert(relative_bytes(&child), (metadata.identity(), metadata.kind));
        if metadata.kind == RawFileKind::Directory {
            let child_fd = open_directory_at(directory, &component)?;
            let held = node_metadata(&child_fd)?;
            if held.identity() != metadata.identity() || held.kind != RawFileKind::Directory {
                return Err(io::Error::from_raw_os_error(libc::ESTALE));
            }
            observe_target_entries(&child_fd, &child, root_device, observed)?;
        }
    }
    Ok(())
}

fn open_rollback_parent(
    target: &OwnedFd,
    entry: &TrackedCreated,
    created: &[TrackedCreated],
) -> Result<(OwnedFd, CString), io::Error> {
    let (name, parents) = entry
        .relative
        .split_last()
        .ok_or_else(|| io::Error::from_raw_os_error(libc::EINVAL))?;
    let mut current = target.try_clone()?;
    let mut prefix = Vec::new();
    for parent_name in parents {
        prefix.push(parent_name.clone());
        let component = component_name_io(parent_name)?;
        current = open_directory_at(&current, &component)?;
        let expected = created
            .iter()
            .find(|candidate| candidate.relative == prefix)
            .and_then(|candidate| {
                (candidate.kind == RawFileKind::Directory)
                    .then_some(candidate.identity)
                    .flatten()
            })
            .ok_or_else(|| io::Error::from_raw_os_error(libc::ESTALE))?;
        let observed = node_metadata(&current)?;
        if observed.identity() != expected || observed.kind != RawFileKind::Directory {
            return Err(io::Error::from_raw_os_error(libc::ESTALE));
        }
    }
    Ok((current, component_name_io(name)?))
}

struct AttemptFailure {
    error: Failure,
    target: Option<OwnedFd>,
    trash: Option<OwnedFd>,
    target_report: Option<PathCapabilityReport>,
    trash_report: Option<PathCapabilityReport>,
    target_baseline: Option<RawNodeMetadata>,
    created: Vec<TrackedCreated>,
    target_modified: bool,
    evidence: MaterializationAttemptEvidence,
}

struct FailedExecution<'a> {
    source: &'a OwnedFd,
    target: OwnedFd,
    trash: OwnedFd,
    path_report: &'a MaterializationPathReport,
    target_baseline: RawNodeMetadata,
    context: ExecutionContext,
    target_modified: bool,
    source_snapshot: &'a TreeSnapshot,
}

impl AttemptFailure {
    fn before_write(error: Failure) -> Box<Self> {
        Box::new(Self {
            error,
            target: None,
            trash: None,
            target_report: None,
            trash_report: None,
            target_baseline: None,
            created: Vec::new(),
            target_modified: false,
            evidence: MaterializationAttemptEvidence::default(),
        })
    }

    fn after_write(error: Failure, failed: FailedExecution<'_>) -> Box<Self> {
        let source_observation = snapshot_tree(failed.source).ok();
        let error = if error.kind == MaterializationFailureKind::CowUnavailable
            && source_observation.as_ref() != Some(failed.source_snapshot)
        {
            source_changed()
        } else {
            error
        };
        let target_snapshot = snapshot_tree(&failed.target).ok();
        let evidence = MaterializationAttemptEvidence::new(
            Some(failed.source_snapshot.manifest.logical_bytes),
            target_snapshot
                .as_ref()
                .map(|snapshot| snapshot.manifest.physical_bytes),
            Some(failed.source_snapshot.manifest.regular_files),
            failed.context.clone_calls,
            source_observation
                .as_ref()
                .map(|snapshot| snapshot.manifest.digest()),
            target_snapshot
                .as_ref()
                .map(|snapshot| snapshot.manifest.digest()),
        );
        Box::new(Self {
            error,
            target: Some(failed.target),
            trash: Some(failed.trash),
            target_report: Some(failed.path_report.target_root().clone()),
            trash_report: Some(failed.path_report.trash().clone()),
            target_baseline: Some(failed.target_baseline),
            created: failed.context.created,
            target_modified: failed.target_modified,
            evidence,
        })
    }
}

#[derive(Debug)]
struct Failure {
    kind: MaterializationFailureKind,
    port_kind: PortErrorKind,
    operation: &'static str,
    source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}

impl Failure {
    const fn new(
        kind: MaterializationFailureKind,
        port_kind: PortErrorKind,
        operation: &'static str,
    ) -> Self {
        Self {
            kind,
            port_kind,
            operation,
            source: None,
        }
    }

    fn with_source(
        kind: MaterializationFailureKind,
        port_kind: PortErrorKind,
        operation: &'static str,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            kind,
            port_kind,
            operation,
            source: Some(Box::new(source)),
        }
    }

    fn io(operation: &'static str, error: io::Error) -> Self {
        let kind = if error.raw_os_error() == Some(libc::ENOSPC) {
            MaterializationFailureKind::NoSpace
        } else {
            MaterializationFailureKind::Filesystem
        };
        Self::with_source(kind, PortErrorKind::Io, operation, error)
    }

    fn clone_io(operation: &'static str, error: io::Error) -> Self {
        let (kind, port_kind) = match error.raw_os_error() {
            Some(libc::ENOTSUP) => (
                MaterializationFailureKind::CowUnavailable,
                PortErrorKind::CapabilityUnavailable,
            ),
            Some(libc::EXDEV) => (
                MaterializationFailureKind::PlanStale,
                PortErrorKind::InvalidLayout,
            ),
            Some(libc::ENOSPC) => (MaterializationFailureKind::NoSpace, PortErrorKind::Io),
            _ => (MaterializationFailureKind::Filesystem, PortErrorKind::Io),
        };
        Self::with_source(kind, port_kind, operation, error)
    }

    fn invalid(
        operation: &'static str,
        error: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::with_source(
            MaterializationFailureKind::InvalidLayout,
            PortErrorKind::InvalidLayout,
            operation,
            error,
        )
    }

    fn path_open(operation: &'static str, error: io::Error) -> Self {
        if matches!(
            error.raw_os_error(),
            Some(libc::ENOENT) | Some(libc::ESTALE) | Some(libc::ELOOP) | Some(libc::ENOTDIR)
        ) {
            Self::with_source(
                MaterializationFailureKind::PlanStale,
                PortErrorKind::InvalidLayout,
                operation,
                error,
            )
        } else {
            Self::io(operation, error)
        }
    }

    fn into_port_error(self) -> PortError {
        let error = PortError::new(self.port_kind, self.operation);
        match self.source {
            Some(source) => error.with_boxed_source(source),
            None => error,
        }
    }

    fn into_source_revalidation(mut self) -> Self {
        if matches!(
            self.kind,
            MaterializationFailureKind::InvalidLayout
                | MaterializationFailureKind::UnsupportedSourceEntry
        ) {
            self.kind = MaterializationFailureKind::SourceChanged;
            self.port_kind = PortErrorKind::Conflict;
            self.operation = "source changed during final APFS revalidation";
        }
        self
    }

    fn into_target_revalidation(mut self) -> Self {
        if matches!(
            self.kind,
            MaterializationFailureKind::InvalidLayout
                | MaterializationFailureKind::SourceChanged
                | MaterializationFailureKind::UnsupportedSourceEntry
        ) {
            self.kind = MaterializationFailureKind::TargetChanged;
            self.port_kind = PortErrorKind::Conflict;
            self.operation = "target changed during final APFS revalidation";
        }
        self
    }
}

fn ensure_directory(metadata: RawNodeMetadata, operation: &'static str) -> Result<(), Failure> {
    if metadata.kind == RawFileKind::Directory {
        Ok(())
    } else {
        Err(Failure::new(
            MaterializationFailureKind::InvalidLayout,
            PortErrorKind::InvalidLayout,
            operation,
        ))
    }
}

fn ensure_identity(
    metadata: RawNodeMetadata,
    expected: FileIdentity,
    kind: RawFileKind,
) -> Result<(), Failure> {
    if metadata.identity() == expected && metadata.kind == kind {
        Ok(())
    } else {
        Err(target_changed())
    }
}

fn component_name(name: &OsString) -> Result<CString, Failure> {
    component_name_io(name)
        .map_err(|error| Failure::invalid("encode materialization path component", error))
}

fn component_name_io(name: &OsString) -> io::Result<CString> {
    c_string(name.as_bytes())
}

fn relative_bytes(components: &[OsString]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (index, component) in components.iter().enumerate() {
        if index != 0 {
            bytes.push(b'/');
        }
        bytes.extend_from_slice(component.as_bytes());
    }
    bytes
}

fn materialized_kind(kind: RawFileKind) -> MaterializedEntryKind {
    match kind {
        RawFileKind::Directory => MaterializedEntryKind::Directory,
        RawFileKind::RegularFile => MaterializedEntryKind::RegularFile,
        RawFileKind::SymbolicLink => MaterializedEntryKind::SymbolicLink,
        RawFileKind::Fifo
        | RawFileKind::Socket
        | RawFileKind::CharacterDevice
        | RawFileKind::BlockDevice
        | RawFileKind::Unknown => unreachable!("unsupported entries are never registered"),
    }
}

fn source_changed() -> Failure {
    Failure::new(
        MaterializationFailureKind::SourceChanged,
        PortErrorKind::Conflict,
        "source changed during APFS materialization",
    )
}

fn target_changed() -> Failure {
    Failure::new(
        MaterializationFailureKind::TargetChanged,
        PortErrorKind::Conflict,
        "target changed during APFS materialization",
    )
}

fn update_bytes(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn update_time(hasher: &mut blake3::Hasher, time: (i64, i64)) {
    hasher.update(&time.0.to_le_bytes());
    hasher.update(&time.1.to_le_bytes());
}

const fn kind_byte(kind: RawFileKind) -> u8 {
    match kind {
        RawFileKind::Directory => 1,
        RawFileKind::RegularFile => 2,
        RawFileKind::SymbolicLink => 3,
        RawFileKind::Fifo => 4,
        RawFileKind::Socket => 5,
        RawFileKind::CharacterDevice => 6,
        RawFileKind::BlockDevice => 7,
        RawFileKind::Unknown => 8,
    }
}

fn elapsed_millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

impl RawNodeMetadata {
    const fn identity(self) -> FileIdentity {
        FileIdentity::new(self.device, self.inode)
    }

    const fn mtime(self) -> (i64, i64) {
        (self.modified_seconds, self.modified_nanoseconds)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::error::Error;
    use std::fs::{self, FileTimes};
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::{Path, PathBuf};

    use tempfile::{Builder, TempDir};
    use thinws_core::{FallbackPolicy, MaterializationOutcome, MaterializationPlan};

    use super::*;

    fn absolute(path: &Path) -> AbsolutePath {
        AbsolutePath::try_from_bytes(path.as_os_str().as_bytes().to_vec()).unwrap()
    }

    fn fixture(
        prefix: &str,
    ) -> (
        TempDir,
        MaterializeRequest,
        MaterializationPlan,
        MacOsHostAdapter,
    ) {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/p1-06-materializer-unit-tests");
        fs::create_dir_all(&root).unwrap();
        let temp = Builder::new()
            .prefix(prefix)
            .tempdir_in(fs::canonicalize(root).unwrap())
            .unwrap();
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        let staging = temp.path().join("staging");
        let trash = temp.path().join("trash");
        for directory in [&source, &target, &staging, &trash] {
            fs::create_dir(directory).unwrap();
        }
        fs::write(source.join("file.txt"), b"source bytes").unwrap();
        let adapter = MacOsHostAdapter::new(temp.path().join("bootstrap")).unwrap();
        let request = MaterializeRequest::new(
            absolute(&source),
            absolute(&target),
            absolute(&staging),
            absolute(&trash),
        );
        let report = adapter
            .inspect_materialization_paths(&MaterializationPathProbeRequest::from(&request))
            .unwrap();
        let plan = MaterializationPlan::for_apfs_clone(&report, FallbackPolicy::Deny).unwrap();
        (temp, request, plan, adapter)
    }

    fn runtime_copy_plan(
        adapter: &MacOsHostAdapter,
        request: &MaterializeRequest,
    ) -> MaterializationPlan {
        let report = adapter
            .inspect_materialization_paths(&MaterializationPathProbeRequest::from(request))
            .unwrap();
        let clone_plan = MaterializationPlan::for_apfs_clone(
            &report,
            FallbackPolicy::AllowFullCopyOnCowUnsupported,
        )
        .unwrap();
        let source = open_bound_directory(request.source(), report.source()).unwrap();
        let source_digest = snapshot_tree(&source).unwrap().manifest.digest();
        let failed_clone = MaterializationReceipt::failed_apfs_clone(
            &clone_plan,
            MaterializationFailureKind::CowUnavailable,
            Vec::new(),
            false,
            RollbackEvidence::new(RollbackStatus::NotNeeded, Vec::new(), Vec::new()),
            MaterializationAttemptEvidence::new(None, None, None, 0, Some(source_digest), None),
            1,
        );
        MaterializationPlan::for_full_copy_after_cow_unavailable(
            &report,
            &clone_plan,
            &failed_clone,
        )
        .unwrap()
    }

    #[test]
    fn request_plan_guard_rejects_each_role_and_wrong_backend_independently() {
        let (_, request, plan, _) = fixture("request-plan-guard-");
        let other = AbsolutePath::try_from_bytes(b"/different".to_vec()).unwrap();
        let mismatches = [
            MaterializeRequest::new(
                other.clone(),
                request.target().clone(),
                request.staging().clone(),
                request.trash().clone(),
            ),
            MaterializeRequest::new(
                request.source().clone(),
                other.clone(),
                request.staging().clone(),
                request.trash().clone(),
            ),
            MaterializeRequest::new(
                request.source().clone(),
                request.target().clone(),
                other.clone(),
                request.trash().clone(),
            ),
            MaterializeRequest::new(
                request.source().clone(),
                request.target().clone(),
                request.staging().clone(),
                other,
            ),
        ];
        for mismatch in &mismatches {
            assert_eq!(
                validate_request_plan(mismatch, &plan, Backend::ApfsClone)
                    .unwrap_err()
                    .kind,
                MaterializationFailureKind::PlanStale
            );
        }
        assert_eq!(
            validate_request_plan(&request, &plan, Backend::FullCopy)
                .unwrap_err()
                .kind,
            MaterializationFailureKind::PlanStale
        );
        assert!(validate_request_plan(&request, &plan, Backend::ApfsClone).is_ok());
    }

    #[test]
    fn backend_candidate_guard_rejects_unsupported_fact() {
        let (_, request, _, adapter) = fixture("backend-candidate-guard-");
        let report = adapter
            .inspect_materialization_paths(&MaterializationPathProbeRequest::from(&request))
            .unwrap();
        let supported = MaterializationPathReport::new(
            report.source().clone(),
            report.target_root().clone(),
            report.staging().clone(),
            report.trash().clone(),
            thinws_core::CandidateEvidence::new(
                MaterializerKind::ApfsFileClone,
                SupportState::Supported,
                Vec::new(),
            ),
            thinws_core::CandidateEvidence::new(
                MaterializerKind::FullCopy,
                SupportState::Supported,
                Vec::new(),
            ),
            report.evidence_digest(),
        );
        assert!(Backend::ApfsClone.candidate_is_executable(&supported));
        assert!(Backend::FullCopy.candidate_is_executable(&supported));
        let unsupported_clone = MaterializationPathReport::new(
            report.source().clone(),
            report.target_root().clone(),
            report.staging().clone(),
            report.trash().clone(),
            thinws_core::CandidateEvidence::new(
                MaterializerKind::ApfsFileClone,
                SupportState::Unsupported,
                Vec::new(),
            ),
            report.full_copy().clone(),
            report.evidence_digest(),
        );
        let unsupported_copy = MaterializationPathReport::new(
            report.source().clone(),
            report.target_root().clone(),
            report.staging().clone(),
            report.trash().clone(),
            report.apfs_clone().clone(),
            thinws_core::CandidateEvidence::new(
                MaterializerKind::FullCopy,
                SupportState::Unsupported,
                Vec::new(),
            ),
            report.evidence_digest(),
        );
        assert!(!Backend::ApfsClone.candidate_is_executable(&unsupported_clone));
        assert!(!Backend::FullCopy.candidate_is_executable(&unsupported_copy));
    }

    struct FailAfterFirstCopyChunk;

    impl ExecutionHook for FailAfterFirstCopyChunk {
        fn after_copy_chunk(
            &self,
            _relative: &[OsString],
            _written_total: u64,
        ) -> Result<(), Failure> {
            Err(Failure::io(
                "inject Full Copy write failure",
                io::Error::from_raw_os_error(libc::EIO),
            ))
        }
    }

    #[test]
    fn partial_full_copy_write_registers_the_file_and_restores_the_empty_target() {
        let (temp, request, _, adapter) = fixture("copy-partial-write-");
        let source_file = temp.path().join("source/file.txt");
        let source_bytes = vec![0x5a_u8; 128 * 1024 + 17];
        fs::write(&source_file, &source_bytes).unwrap();
        let plan = runtime_copy_plan(&adapter, &request);

        let failure = materialize_with_backend(
            &adapter,
            Backend::FullCopy,
            &request,
            &plan,
            &FailAfterFirstCopyChunk,
        )
        .unwrap_err();

        assert_eq!(failure.receipt().outcome(), MaterializationOutcome::Partial);
        assert_eq!(
            failure.receipt().failure_kind(),
            Some(MaterializationFailureKind::Filesystem)
        );
        assert_eq!(
            failure.receipt().cow_evidence(),
            thinws_core::CowEvidence::NotUsed
        );
        assert_eq!(failure.receipt().clone_calls_succeeded(), 0);
        assert_eq!(failure.receipt().created().len(), 1);
        assert!(failure.receipt().created()[0].identity().is_some());
        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::ConfirmedBaseline
        );
        assert!(
            fs::read_dir(temp.path().join("target"))
                .unwrap()
                .next()
                .is_none()
        );
        assert_eq!(fs::read(&source_file).unwrap(), source_bytes);
        assert_eq!(failure.receipt().failed_attempts().len(), 1);
    }

    struct ReplaceCopyWithSourceHardLinkHook {
        source_file: PathBuf,
        target_file: PathBuf,
        fired: Cell<bool>,
    }

    impl ExecutionHook for ReplaceCopyWithSourceHardLinkHook {
        fn after_copy_chunk(
            &self,
            _relative: &[OsString],
            _written_total: u64,
        ) -> Result<(), Failure> {
            if !self.fired.replace(true) {
                fs::remove_file(&self.target_file).unwrap();
                fs::hard_link(&self.source_file, &self.target_file).unwrap();
            }
            Ok(())
        }
    }

    #[test]
    fn full_copy_never_writes_through_a_replacement_source_hard_link() {
        let (temp, request, _, adapter) = fixture("copy-source-hard-link-");
        let source_file = temp.path().join("source/file.txt");
        let target_file = temp.path().join("target/file.txt");
        let source_bytes = vec![0x41_u8; 128 * 1024 + 17];
        fs::write(&source_file, &source_bytes).unwrap();
        let plan = runtime_copy_plan(&adapter, &request);
        let hook = ReplaceCopyWithSourceHardLinkHook {
            source_file: source_file.clone(),
            target_file: target_file.clone(),
            fired: Cell::new(false),
        };

        let failure = materialize_with_backend(&adapter, Backend::FullCopy, &request, &plan, &hook)
            .unwrap_err();

        assert!(hook.fired.get());
        assert_eq!(
            failure.receipt().failure_kind(),
            Some(MaterializationFailureKind::TargetChanged)
        );
        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::Incomplete
        );
        assert_eq!(fs::read(&source_file).unwrap(), source_bytes);
        assert_eq!(fs::read(&target_file).unwrap(), source_bytes);
        assert_eq!(
            fs::metadata(&source_file).unwrap().ino(),
            fs::metadata(&target_file).unwrap().ino()
        );
    }

    struct ReplaceUnregisteredCopyWithSourceHardLinkHook {
        source_file: PathBuf,
        target_file: PathBuf,
        fired: Cell<bool>,
    }

    impl ExecutionHook for ReplaceUnregisteredCopyWithSourceHardLinkHook {
        fn after_file_created_before_registration(&self, _relative: &[OsString]) {
            if !self.fired.replace(true) {
                fs::remove_file(&self.target_file).unwrap();
                fs::hard_link(&self.source_file, &self.target_file).unwrap();
            }
        }
    }

    #[test]
    fn full_copy_rejects_a_source_hard_link_before_first_identity_registration() {
        let (temp, request, _, adapter) = fixture("copy-unregistered-hard-link-");
        let source_file = temp.path().join("source/file.txt");
        let target_file = temp.path().join("target/file.txt");
        let source_bytes = fs::read(&source_file).unwrap();
        let plan = runtime_copy_plan(&adapter, &request);
        let hook = ReplaceUnregisteredCopyWithSourceHardLinkHook {
            source_file: source_file.clone(),
            target_file: target_file.clone(),
            fired: Cell::new(false),
        };

        let failure = materialize_with_backend(&adapter, Backend::FullCopy, &request, &plan, &hook)
            .unwrap_err();

        assert!(hook.fired.get());
        assert_eq!(
            failure.receipt().failure_kind(),
            Some(MaterializationFailureKind::TargetChanged)
        );
        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::Incomplete
        );
        assert_eq!(failure.receipt().created().len(), 1);
        assert_eq!(failure.receipt().created()[0].identity(), None);
        assert_eq!(fs::read(&source_file).unwrap(), source_bytes);
        assert_eq!(fs::read(&target_file).unwrap(), source_bytes);
        assert_eq!(
            fs::metadata(&source_file).unwrap().ino(),
            fs::metadata(&target_file).unwrap().ino()
        );
    }

    struct ReplaceUnregisteredCopyWithForeignFileHook {
        replacement_file: PathBuf,
        target_file: PathBuf,
        fired: Cell<bool>,
    }

    impl ExecutionHook for ReplaceUnregisteredCopyWithForeignFileHook {
        fn after_file_created_before_registration(&self, _relative: &[OsString]) {
            if !self.fired.replace(true) {
                fs::rename(&self.replacement_file, &self.target_file).unwrap();
            }
        }
    }

    #[test]
    fn full_copy_does_not_adopt_a_foreign_file_before_registration() {
        let (temp, request, _, adapter) = fixture("copy-unregistered-foreign-");
        let source_file = temp.path().join("source/file.txt");
        let target_file = temp.path().join("target/file.txt");
        let replacement_file = temp.path().join("foreign.txt");
        let source_bytes = fs::read(&source_file).unwrap();
        let foreign_bytes = b"foreign target data must survive";
        fs::write(&replacement_file, foreign_bytes).unwrap();
        let plan = runtime_copy_plan(&adapter, &request);
        let hook = ReplaceUnregisteredCopyWithForeignFileHook {
            replacement_file,
            target_file: target_file.clone(),
            fired: Cell::new(false),
        };

        let failure = materialize_with_backend(&adapter, Backend::FullCopy, &request, &plan, &hook)
            .unwrap_err();

        assert!(hook.fired.get());
        assert_eq!(
            failure.receipt().failure_kind(),
            Some(MaterializationFailureKind::TargetChanged)
        );
        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::Incomplete
        );
        assert_eq!(failure.receipt().created().len(), 1);
        assert_eq!(failure.receipt().created()[0].identity(), None);
        assert_eq!(fs::read(&source_file).unwrap(), source_bytes);
        assert_eq!(fs::read(&target_file).unwrap(), foreign_bytes);
    }

    struct ReplacePublishedDirectoryHook {
        target: PathBuf,
        foreign: PathBuf,
    }

    impl ExecutionHook for ReplacePublishedDirectoryHook {
        fn after_directory_published_before_validation(
            &self,
            _relative: &[OsString],
        ) -> Result<(), Failure> {
            fs::remove_dir(&self.target).unwrap();
            fs::rename(&self.foreign, &self.target).unwrap();
            Ok(())
        }
    }

    #[test]
    fn clone_does_not_adopt_or_write_into_foreign_directory_before_validation() {
        let (temp, request, plan, adapter) = fixture("unregistered-foreign-directory-");
        let source_child = temp.path().join("source/child");
        fs::create_dir(&source_child).unwrap();
        fs::write(source_child.join("nested.txt"), b"nested bytes").unwrap();
        let target = temp.path().join("target/child");
        let foreign = temp.path().join("foreign-directory");
        fs::create_dir(&foreign).unwrap();
        let foreign_inode = fs::symlink_metadata(&foreign).unwrap().ino();
        let hook = ReplacePublishedDirectoryHook {
            target: target.clone(),
            foreign,
        };

        let failure = ApfsCloneMaterializer::new(adapter)
            .materialize_with_hook(&request, &plan, &hook)
            .unwrap_err();

        assert_eq!(
            failure.receipt().failure_kind(),
            Some(MaterializationFailureKind::TargetChanged)
        );
        assert_eq!(fs::symlink_metadata(&target).unwrap().ino(), foreign_inode);
        assert!(fs::read_dir(&target).unwrap().next().is_none());
    }

    struct ReplacePublishedSymlinkHook {
        target: PathBuf,
        foreign: PathBuf,
    }

    impl ExecutionHook for ReplacePublishedSymlinkHook {
        fn after_symlink_published_before_validation(
            &self,
            _relative: &[OsString],
        ) -> Result<(), Failure> {
            fs::remove_file(&self.target).unwrap();
            fs::rename(&self.foreign, &self.target).unwrap();
            Ok(())
        }
    }

    #[test]
    fn clone_does_not_adopt_foreign_symlink_before_validation() {
        let (temp, request, plan, adapter) = fixture("unregistered-foreign-symlink-");
        let source = temp.path().join("source/link");
        std::os::unix::fs::symlink("../outside", &source).unwrap();
        let target = temp.path().join("target/link");
        let foreign = temp.path().join("foreign-link");
        std::os::unix::fs::symlink("../outside", &foreign).unwrap();
        let foreign_inode = fs::symlink_metadata(&foreign).unwrap().ino();
        let hook = ReplacePublishedSymlinkHook {
            target: target.clone(),
            foreign,
        };

        let failure = ApfsCloneMaterializer::new(adapter)
            .materialize_with_hook(&request, &plan, &hook)
            .unwrap_err();

        assert_eq!(
            failure.receipt().failure_kind(),
            Some(MaterializationFailureKind::TargetChanged)
        );
        assert_eq!(fs::symlink_metadata(&target).unwrap().ino(), foreign_inode);
        assert_eq!(fs::read_link(&target).unwrap(), PathBuf::from("../outside"));
    }

    struct ReplacePublishedCloneWithForeignFileHook {
        target: PathBuf,
        foreign: PathBuf,
    }

    impl ExecutionHook for ReplacePublishedCloneWithForeignFileHook {
        fn after_clone_published_before_validation(
            &self,
            _relative: &[OsString],
        ) -> Result<(), Failure> {
            fs::remove_file(&self.target).unwrap();
            fs::rename(&self.foreign, &self.target).unwrap();
            Ok(())
        }
    }

    #[test]
    fn clone_does_not_confirm_cow_for_matching_foreign_file_before_validation() {
        let (temp, request, plan, adapter) = fixture("unregistered-foreign-clone-");
        let source = temp.path().join("source/file.txt");
        let target = temp.path().join("target/file.txt");
        let foreign = temp.path().join("foreign-file.txt");
        fs::copy(&source, &foreign).unwrap();
        let source_mtime = fs::metadata(&source).unwrap().modified().unwrap();
        fs::File::open(&foreign)
            .unwrap()
            .set_times(FileTimes::new().set_modified(source_mtime))
            .unwrap();
        let foreign_inode = fs::symlink_metadata(&foreign).unwrap().ino();
        let hook = ReplacePublishedCloneWithForeignFileHook {
            target: target.clone(),
            foreign,
        };

        let failure = ApfsCloneMaterializer::new(adapter)
            .materialize_with_hook(&request, &plan, &hook)
            .unwrap_err();

        assert_eq!(
            failure.receipt().failure_kind(),
            Some(MaterializationFailureKind::TargetChanged)
        );
        assert_eq!(fs::symlink_metadata(&target).unwrap().ino(), foreign_inode);
        assert_eq!(fs::read(&target).unwrap(), fs::read(&source).unwrap());
        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::Incomplete
        );
    }

    #[test]
    fn staging_name_collision_retries_but_other_creation_errors_do_not() {
        let (_temp, request, _, _) = fixture("stage-create-errors-");
        let staging = open_absolute_directory(request.staging()).unwrap();
        let target = open_absolute_directory(request.target()).unwrap();
        let name = CString::new("published-link").unwrap();
        let mut context = ExecutionContext {
            created: Vec::new(),
            clone_calls: 0,
            backend: Backend::ApfsClone,
        };
        let calls = Cell::new(0);
        stage_and_publish(
            &staging,
            &target,
            &name,
            &[OsString::from("published-link")],
            RawFileKind::SymbolicLink,
            &mut context,
            |parent, candidate| {
                calls.set(calls.get() + 1);
                if calls.get() == 1 {
                    Err(io::Error::from_raw_os_error(libc::EEXIST))
                } else {
                    create_symlink_at(c"../outside", parent, candidate)
                }
            },
            Failure::io,
            "create staged test symlink",
            || Ok(()),
        )
        .unwrap();
        assert_eq!(calls.get(), 2);
        assert_eq!(context.created.len(), 1);
        assert!(read_directory(&staging).unwrap().is_empty());

        calls.set(0);
        let error = stage_and_publish(
            &staging,
            &target,
            &CString::new("never-published").unwrap(),
            &[OsString::from("never-published")],
            RawFileKind::SymbolicLink,
            &mut context,
            |_, _| {
                calls.set(calls.get() + 1);
                Err(io::Error::from_raw_os_error(libc::EACCES))
            },
            Failure::io,
            "reject staged test symlink",
            || Ok(()),
        )
        .unwrap_err();
        assert_eq!(error.kind, MaterializationFailureKind::Filesystem);
        assert_eq!(calls.get(), 1);
        assert_eq!(context.created.len(), 1);
        assert!(read_directory(&staging).unwrap().is_empty());
    }

    #[test]
    fn rejected_staged_publication_cleans_only_its_own_directory() {
        let (temp, request, _, _) = fixture("stage-target-collision-");
        let staging = open_absolute_directory(request.staging()).unwrap();
        let target = open_absolute_directory(request.target()).unwrap();
        let occupied = temp.path().join("target/occupied");
        fs::create_dir(&occupied).unwrap();
        fs::write(occupied.join("sentinel"), b"foreign data").unwrap();
        let occupied_inode = fs::symlink_metadata(&occupied).unwrap().ino();
        let mut context = ExecutionContext {
            created: Vec::new(),
            clone_calls: 0,
            backend: Backend::ApfsClone,
        };

        let error = stage_and_publish(
            &staging,
            &target,
            &CString::new("occupied").unwrap(),
            &[OsString::from("occupied")],
            RawFileKind::Directory,
            &mut context,
            |parent, candidate| create_directory_at(parent, candidate, 0o700),
            Failure::io,
            "create staged test directory",
            || Ok(()),
        )
        .unwrap_err();

        assert_eq!(error.kind, MaterializationFailureKind::TargetChanged);
        assert!(context.created.is_empty());
        assert!(read_directory(&staging).unwrap().is_empty());
        assert_eq!(
            fs::symlink_metadata(&occupied).unwrap().ino(),
            occupied_inode
        );
        assert_eq!(
            fs::read(occupied.join("sentinel")).unwrap(),
            b"foreign data"
        );
    }

    struct ChangeSourceThenReportCloneUnavailableHook {
        source_file: PathBuf,
        fired: Cell<bool>,
    }

    impl ExecutionHook for ChangeSourceThenReportCloneUnavailableHook {
        fn after_created(
            &self,
            _relative: &[OsString],
            _created_count: usize,
        ) -> Result<(), Failure> {
            if !self.fired.replace(true) {
                fs::write(&self.source_file, b"changed after first clone").unwrap();
                return Err(Failure::clone_io(
                    "inject clone unavailable after source change",
                    io::Error::from_raw_os_error(libc::ENOTSUP),
                ));
            }
            Ok(())
        }
    }

    #[test]
    fn changed_source_takes_precedence_over_clone_unavailable_for_fallback() {
        let (temp, request, _, adapter) = fixture("clone-unavailable-changed-source-");
        let source_file = temp.path().join("source/file.txt");
        let report = adapter
            .inspect_materialization_paths(&MaterializationPathProbeRequest::from(&request))
            .unwrap();
        let plan = MaterializationPlan::for_apfs_clone(
            &report,
            FallbackPolicy::AllowFullCopyOnCowUnsupported,
        )
        .unwrap();
        let hook = ChangeSourceThenReportCloneUnavailableHook {
            source_file,
            fired: Cell::new(false),
        };

        let failure =
            materialize_with_backend(&adapter, Backend::ApfsClone, &request, &plan, &hook)
                .unwrap_err();

        assert!(hook.fired.get());
        assert_eq!(
            failure.receipt().failure_kind(),
            Some(MaterializationFailureKind::SourceChanged)
        );
        assert_eq!(
            MaterializationPlan::for_full_copy_after_cow_unavailable(
                &report,
                &plan,
                failure.receipt(),
            ),
            Err(thinws_core::MaterializationPlanError::FallbackNotEligible)
        );
    }

    #[test]
    fn runtime_full_copy_rejects_source_changed_after_the_clone_attempt() {
        let (temp, request, _, adapter) = fixture("copy-runtime-source-drift-");
        let source_file = temp.path().join("source/file.txt");
        let plan = runtime_copy_plan(&adapter, &request);
        fs::write(&source_file, b"source changed before fallback execution").unwrap();

        let failure =
            materialize_with_backend(&adapter, Backend::FullCopy, &request, &plan, &NoopHook)
                .unwrap_err();

        assert_eq!(
            failure.receipt().failure_kind(),
            Some(MaterializationFailureKind::SourceChanged)
        );
        assert_eq!(failure.receipt().outcome(), MaterializationOutcome::Failed);
        assert!(failure.receipt().created().is_empty());
        assert!(
            fs::read_dir(temp.path().join("target"))
                .unwrap()
                .next()
                .is_none()
        );
    }

    struct ReportCloneUnavailableAfterFirstFile;

    impl ExecutionHook for ReportCloneUnavailableAfterFirstFile {
        fn after_created(
            &self,
            _relative: &[OsString],
            _created_count: usize,
        ) -> Result<(), Failure> {
            Err(Failure::clone_io(
                "inject clone unavailable after first file",
                io::Error::from_raw_os_error(libc::ENOTSUP),
            ))
        }
    }

    #[test]
    fn runtime_fallback_copies_only_after_a_real_clean_clone_rollback() {
        let (temp, request, _, adapter) = fixture("copy-after-clone-rollback-");
        let initial = adapter
            .inspect_materialization_paths(&MaterializationPathProbeRequest::from(&request))
            .unwrap();
        let clone_plan = MaterializationPlan::for_apfs_clone(
            &initial,
            FallbackPolicy::AllowFullCopyOnCowUnsupported,
        )
        .unwrap();
        let failed = materialize_with_backend(
            &adapter,
            Backend::ApfsClone,
            &request,
            &clone_plan,
            &ReportCloneUnavailableAfterFirstFile,
        )
        .unwrap_err();
        assert_eq!(
            failed.receipt().failure_kind(),
            Some(MaterializationFailureKind::CowUnavailable)
        );
        assert_eq!(
            failed.receipt().rollback().status(),
            RollbackStatus::ConfirmedBaseline
        );
        let fresh = adapter
            .inspect_materialization_paths(&MaterializationPathProbeRequest::from(&request))
            .unwrap();
        let copy_plan = MaterializationPlan::for_full_copy_after_cow_unavailable(
            &fresh,
            &clone_plan,
            failed.receipt(),
        )
        .unwrap();

        let receipt =
            materialize_with_backend(&adapter, Backend::FullCopy, &request, &copy_plan, &NoopHook)
                .unwrap();

        assert_eq!(receipt.outcome(), MaterializationOutcome::Succeeded);
        assert_eq!(receipt.failed_attempts().len(), 1);
        assert_eq!(receipt.actual_mode(), MaterializationMode::FullCopy);
        assert_eq!(receipt.cow_evidence(), thinws_core::CowEvidence::NotUsed);
        assert_eq!(receipt.clone_calls_succeeded(), 0);
        assert_eq!(
            fs::read(temp.path().join("target/file.txt")).unwrap(),
            fs::read(temp.path().join("source/file.txt")).unwrap()
        );
    }

    #[test]
    fn full_copy_detects_source_change_and_rolls_back_created_content() {
        let (temp, request, _, adapter) = fixture("copy-source-change-");
        let source_file = temp.path().join("source/file.txt");
        let plan = runtime_copy_plan(&adapter, &request);
        let hook = MutateSourceHook {
            source_file: source_file.clone(),
            fired: Cell::new(false),
        };

        let failure = materialize_with_backend(&adapter, Backend::FullCopy, &request, &plan, &hook)
            .unwrap_err();

        assert!(hook.fired.get());
        assert_eq!(
            failure.receipt().failure_kind(),
            Some(MaterializationFailureKind::SourceChanged)
        );
        assert_eq!(failure.receipt().outcome(), MaterializationOutcome::Partial);
        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::ConfirmedBaseline
        );
        assert!(
            fs::read_dir(temp.path().join("target"))
                .unwrap()
                .next()
                .is_none()
        );
        assert_eq!(
            fs::read(&source_file).unwrap(),
            b"source changed while cloning"
        );
    }

    struct MutateSourceHook {
        source_file: PathBuf,
        fired: Cell<bool>,
    }

    struct ChangeNextSourceEntryHook {
        next_file: PathBuf,
        fired: Cell<bool>,
    }

    impl ExecutionHook for ChangeNextSourceEntryHook {
        fn after_created(
            &self,
            _relative: &[OsString],
            _created_count: usize,
        ) -> Result<(), Failure> {
            if !self.fired.replace(true) {
                fs::write(&self.next_file, b"changed before its own clone").unwrap();
            }
            Ok(())
        }
    }

    #[test]
    fn source_entry_change_stops_before_cloning_that_entry() {
        let (temp, request, plan, adapter) = fixture("source-next-entry-change-");
        fs::rename(
            temp.path().join("source/file.txt"),
            temp.path().join("source/a.txt"),
        )
        .unwrap();
        let next_file = temp.path().join("source/b.txt");
        fs::write(&next_file, b"original second file").unwrap();
        let hook = ChangeNextSourceEntryHook {
            next_file,
            fired: Cell::new(false),
        };

        let failure =
            materialize_with_backend(&adapter, Backend::ApfsClone, &request, &plan, &hook)
                .unwrap_err();

        assert!(hook.fired.get());
        assert_eq!(
            failure.receipt().failure_kind(),
            Some(MaterializationFailureKind::SourceChanged)
        );
        assert_eq!(failure.receipt().clone_calls_succeeded(), 1);
        assert_eq!(failure.receipt().created().len(), 1);
        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::ConfirmedBaseline
        );
    }

    #[test]
    fn symlink_text_mismatch_is_rejected_even_when_identity_is_stable() {
        let (temp, request, _, _) = fixture("symlink-text-mismatch-");
        fs::remove_file(temp.path().join("source/file.txt")).unwrap();
        std::os::unix::fs::symlink("original", temp.path().join("source/link")).unwrap();
        let source = open_absolute_directory(request.source()).unwrap();
        let target = open_absolute_directory(request.target()).unwrap();
        let staging = open_absolute_directory(request.staging()).unwrap();
        let mut snapshot = snapshot_tree(&source).unwrap();
        snapshot
            .entries
            .get_mut(b"link".as_slice())
            .unwrap()
            .link_text = Some(b"different".to_vec());
        let mut context = ExecutionContext {
            created: Vec::new(),
            clone_calls: 0,
            backend: Backend::FullCopy,
        };

        let result = materialize_directory(
            &source,
            &target,
            &staging,
            &[],
            snapshot.root.device,
            &snapshot.entries,
            &mut context,
            &NoopHook,
        );

        assert_eq!(
            result.unwrap_err().kind,
            MaterializationFailureKind::SourceChanged
        );
        assert!(context.created.is_empty());
        assert!(!temp.path().join("target/link").exists());
    }

    impl ExecutionHook for MutateSourceHook {
        fn after_created(
            &self,
            _relative: &[OsString],
            _created_count: usize,
        ) -> Result<(), Failure> {
            if !self.fired.replace(true) {
                fs::write(&self.source_file, b"source changed while cloning").unwrap();
            }
            Ok(())
        }
    }

    #[test]
    fn source_change_after_clone_returns_partial_receipt_and_rolls_back() {
        let (temp, request, plan, adapter) = fixture("source-change-");
        let hook = MutateSourceHook {
            source_file: temp.path().join("source/file.txt"),
            fired: Cell::new(false),
        };

        let failure = ApfsCloneMaterializer::new(adapter)
            .materialize_with_hook(&request, &plan, &hook)
            .unwrap_err();

        assert_eq!(
            failure.receipt().failure_kind(),
            Some(MaterializationFailureKind::SourceChanged)
        );
        assert_eq!(failure.receipt().created().len(), 1);
        assert_eq!(failure.receipt().regular_file_count(), Some(1));
        assert_eq!(failure.receipt().clone_calls_succeeded(), 1);
        assert!(failure.receipt().logical_bytes().is_some());
        assert!(failure.receipt().source_manifest_digest().is_some());
        assert!(failure.receipt().target_manifest_digest().is_some());
        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::ConfirmedBaseline
        );
        assert_eq!(failure.receipt().rollback().quarantined().len(), 1);
        assert!(
            fs::read_dir(temp.path().join("target"))
                .unwrap()
                .next()
                .is_none()
        );
    }

    struct ReplaceAfterProbeHook {
        target: PathBuf,
        displaced: PathBuf,
    }

    impl ExecutionHook for ReplaceAfterProbeHook {
        fn after_probe(&self) -> Result<(), Failure> {
            fs::rename(&self.target, &self.displaced).unwrap();
            fs::create_dir(&self.target).unwrap();
            Ok(())
        }
    }

    #[test]
    fn target_replacement_after_probe_is_rejected_before_writes() {
        let (temp, request, plan, adapter) = fixture("post-probe-root-replacement-");
        let target = temp.path().join("target");
        let displaced = temp.path().join("displaced-target");
        let hook = ReplaceAfterProbeHook {
            target: target.clone(),
            displaced: displaced.clone(),
        };

        let failure = ApfsCloneMaterializer::new(adapter)
            .materialize_with_hook(&request, &plan, &hook)
            .unwrap_err();

        assert_eq!(
            failure.receipt().failure_kind(),
            Some(MaterializationFailureKind::PlanStale)
        );
        assert_eq!(failure.receipt().outcome(), MaterializationOutcome::Failed);
        assert!(fs::read_dir(target).unwrap().next().is_none());
        assert!(fs::read_dir(displaced).unwrap().next().is_none());
    }

    struct RemoveSourceAfterProbeHook {
        source: PathBuf,
    }

    impl ExecutionHook for RemoveSourceAfterProbeHook {
        fn after_probe(&self) -> Result<(), Failure> {
            fs::remove_dir_all(&self.source).unwrap();
            Ok(())
        }
    }

    #[test]
    fn source_disappearance_after_fresh_probe_is_plan_stale_without_writes() {
        let (temp, request, plan, adapter) = fixture("post-probe-source-disappearance-");
        let hook = RemoveSourceAfterProbeHook {
            source: temp.path().join("source"),
        };

        let failure = ApfsCloneMaterializer::new(adapter)
            .materialize_with_hook(&request, &plan, &hook)
            .unwrap_err();

        assert_eq!(
            failure.receipt().failure_kind(),
            Some(MaterializationFailureKind::PlanStale)
        );
        assert_eq!(failure.receipt().outcome(), MaterializationOutcome::Failed);
        assert!(failure.receipt().created().is_empty());
        assert!(
            fs::read_dir(temp.path().join("target"))
                .unwrap()
                .next()
                .is_none()
        );
    }

    struct MakeTargetUnwritableAfterProbeHook {
        target: PathBuf,
    }

    impl ExecutionHook for MakeTargetUnwritableAfterProbeHook {
        fn after_probe(&self) -> Result<(), Failure> {
            fs::set_permissions(&self.target, fs::Permissions::from_mode(0o500)).unwrap();
            Ok(())
        }
    }

    #[test]
    fn clone_failure_before_the_first_write_does_not_claim_rollback_work() {
        let (temp, request, plan, adapter) = fixture("prewrite-clone-failure-");
        let target = temp.path().join("target");
        let hook = MakeTargetUnwritableAfterProbeHook {
            target: target.clone(),
        };

        let failure = ApfsCloneMaterializer::new(adapter)
            .materialize_with_hook(&request, &plan, &hook)
            .unwrap_err();

        assert_eq!(failure.receipt().outcome(), MaterializationOutcome::Failed);
        assert!(failure.receipt().created().is_empty());
        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::NotNeeded
        );
        assert!(fs::read_dir(&target).unwrap().next().is_none());
        fs::set_permissions(target, fs::Permissions::from_mode(0o700)).unwrap();
    }

    struct ReplaceCommonParentKeepLeafIdentitiesHook {
        root: PathBuf,
        displaced: PathBuf,
    }

    impl ExecutionHook for ReplaceCommonParentKeepLeafIdentitiesHook {
        fn after_probe(&self) -> Result<(), Failure> {
            fs::rename(&self.root, &self.displaced).unwrap();
            fs::create_dir(&self.root).unwrap();
            for name in ["source", "target", "staging", "trash"] {
                fs::rename(self.displaced.join(name), self.root.join(name)).unwrap();
            }
            Ok(())
        }
    }

    #[test]
    fn changed_ancestor_is_rejected_even_when_every_leaf_identity_is_preserved() {
        let (temp, request, plan, adapter) = fixture("ancestor-replacement-");
        let root = temp.path().to_path_buf();
        let displaced = root.with_extension("displaced");
        let hook = ReplaceCommonParentKeepLeafIdentitiesHook {
            root,
            displaced: displaced.clone(),
        };

        let failure = ApfsCloneMaterializer::new(adapter)
            .materialize_with_hook(&request, &plan, &hook)
            .unwrap_err();

        assert_eq!(
            failure.receipt().failure_kind(),
            Some(MaterializationFailureKind::PlanStale)
        );
        assert_eq!(failure.receipt().outcome(), MaterializationOutcome::Failed);
        fs::remove_dir(displaced).unwrap();
    }

    struct MoveTargetBeforeFinalValidationHook {
        target: PathBuf,
        displaced: PathBuf,
    }

    impl ExecutionHook for MoveTargetBeforeFinalValidationHook {
        fn before_final_validation(&self) -> Result<(), Failure> {
            fs::rename(&self.target, &self.displaced).unwrap();
            fs::create_dir(&self.target).unwrap();
            Ok(())
        }
    }

    #[test]
    fn target_root_move_during_execution_cannot_return_success_or_touch_replacement() {
        let (temp, request, plan, adapter) = fixture("mid-execution-root-move-");
        let target = temp.path().join("target");
        let displaced = temp.path().join("displaced-target");
        fs::set_permissions(
            temp.path().join("source"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
        let hook = MoveTargetBeforeFinalValidationHook {
            target: target.clone(),
            displaced: displaced.clone(),
        };

        let failure = ApfsCloneMaterializer::new(adapter)
            .materialize_with_hook(&request, &plan, &hook)
            .unwrap_err();

        assert_eq!(
            failure.receipt().failure_kind(),
            Some(MaterializationFailureKind::PlanStale)
        );
        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::Incomplete
        );
        assert!(fs::read_dir(target).unwrap().next().is_none());
        assert_eq!(
            fs::read(displaced.join("file.txt")).unwrap(),
            b"source bytes"
        );
        assert_eq!(
            fs::metadata(displaced).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    struct ReplaceCloneWithSourceHardLinkHook {
        source_file: PathBuf,
        target_file: PathBuf,
        fired: Cell<bool>,
    }

    impl ExecutionHook for ReplaceCloneWithSourceHardLinkHook {
        fn after_created(
            &self,
            _relative: &[OsString],
            _created_count: usize,
        ) -> Result<(), Failure> {
            if !self.fired.replace(true) {
                fs::remove_file(&self.target_file).unwrap();
                fs::hard_link(&self.source_file, &self.target_file).unwrap();
            }
            Ok(())
        }
    }

    struct ReplacePublishedCloneWithSourceHardLinkHook {
        source_file: PathBuf,
        target_file: PathBuf,
        fired: Cell<bool>,
    }

    impl ExecutionHook for ReplacePublishedCloneWithSourceHardLinkHook {
        fn after_clone_published_before_validation(
            &self,
            _relative: &[OsString],
        ) -> Result<(), Failure> {
            if !self.fired.replace(true) {
                fs::remove_file(&self.target_file).unwrap();
                fs::hard_link(&self.source_file, &self.target_file).unwrap();
            }
            Ok(())
        }
    }

    #[test]
    fn source_hard_link_after_clone_publication_is_rejected_and_preserved() {
        let (temp, request, plan, adapter) = fixture("unregistered-source-hard-link-");
        let source_file = temp.path().join("source/file.txt");
        let target_file = temp.path().join("target/file.txt");
        let hook = ReplacePublishedCloneWithSourceHardLinkHook {
            source_file: source_file.clone(),
            target_file: target_file.clone(),
            fired: Cell::new(false),
        };

        let failure = ApfsCloneMaterializer::new(adapter)
            .materialize_with_hook(&request, &plan, &hook)
            .unwrap_err();

        assert_eq!(
            failure.receipt().failure_kind(),
            Some(MaterializationFailureKind::TargetChanged)
        );
        assert_eq!(
            failure.receipt().cow_evidence(),
            thinws_core::CowEvidence::Unknown
        );
        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::Incomplete
        );
        assert_eq!(
            fs::metadata(source_file).unwrap().ino(),
            fs::metadata(target_file).unwrap().ino()
        );
    }

    #[test]
    fn source_hard_link_replacement_is_detected_without_claiming_cow_success() {
        let (temp, request, plan, adapter) = fixture("source-hard-link-");
        let source_file = temp.path().join("source/file.txt");
        let target_file = temp.path().join("target/file.txt");
        let hook = ReplaceCloneWithSourceHardLinkHook {
            source_file: source_file.clone(),
            target_file: target_file.clone(),
            fired: Cell::new(false),
        };

        let failure = ApfsCloneMaterializer::new(adapter)
            .materialize_with_hook(&request, &plan, &hook)
            .unwrap_err();

        assert_eq!(
            failure.receipt().failure_kind(),
            Some(MaterializationFailureKind::TargetChanged)
        );
        assert_eq!(
            failure.receipt().cow_evidence(),
            thinws_core::CowEvidence::Unknown
        );
        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::Incomplete
        );
        assert_eq!(
            fs::metadata(source_file).unwrap().ino(),
            fs::metadata(target_file).unwrap().ino()
        );
    }

    struct ReplaceTargetHook {
        target_file: PathBuf,
        fired: Cell<bool>,
    }

    impl ExecutionHook for ReplaceTargetHook {
        fn after_created(
            &self,
            _relative: &[OsString],
            _created_count: usize,
        ) -> Result<(), Failure> {
            if !self.fired.replace(true) {
                fs::remove_file(&self.target_file).unwrap();
                fs::write(&self.target_file, b"replacement owned by another actor").unwrap();
                return Err(target_changed());
            }
            Ok(())
        }
    }

    #[test]
    fn rollback_preserves_a_replacement_object_and_reports_incomplete() {
        let (temp, request, plan, adapter) = fixture("replacement-");
        let target_file = temp.path().join("target/file.txt");
        let hook = ReplaceTargetHook {
            target_file: target_file.clone(),
            fired: Cell::new(false),
        };

        let failure = ApfsCloneMaterializer::new(adapter)
            .materialize_with_hook(&request, &plan, &hook)
            .unwrap_err();

        assert_eq!(
            failure.receipt().failure_kind(),
            Some(MaterializationFailureKind::TargetChanged)
        );
        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::Incomplete
        );
        assert_eq!(failure.receipt().rollback().remaining().len(), 1);
        assert_eq!(
            fs::read(target_file).unwrap(),
            b"replacement owned by another actor"
        );
    }

    struct AddUnknownTargetHook {
        unknown_file: PathBuf,
        fired: Cell<bool>,
    }

    impl ExecutionHook for AddUnknownTargetHook {
        fn after_created(
            &self,
            _relative: &[OsString],
            _created_count: usize,
        ) -> Result<(), Failure> {
            if !self.fired.replace(true) {
                fs::write(&self.unknown_file, b"external object").unwrap();
                return Err(target_changed());
            }
            Ok(())
        }
    }

    #[test]
    fn rollback_does_not_delete_anything_when_an_unknown_object_appears() {
        let (temp, request, plan, adapter) = fixture("unknown-object-");
        let hook = AddUnknownTargetHook {
            unknown_file: temp.path().join("target/unknown.txt"),
            fired: Cell::new(false),
        };

        let failure = ApfsCloneMaterializer::new(adapter)
            .materialize_with_hook(&request, &plan, &hook)
            .unwrap_err();

        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::Incomplete
        );
        assert!(temp.path().join("target/file.txt").exists());
        assert!(temp.path().join("target/unknown.txt").exists());
        assert!(failure.receipt().rollback().removed().is_empty());
    }

    struct ReplaceBetweenRollbackCheckAndDetachHook {
        target_file: PathBuf,
        fired: Cell<bool>,
        detached: Cell<bool>,
    }

    impl ExecutionHook for ReplaceBetweenRollbackCheckAndDetachHook {
        fn after_created(
            &self,
            _relative: &[OsString],
            _created_count: usize,
        ) -> Result<(), Failure> {
            Err(target_changed())
        }

        fn before_rollback_detach(&self, _relative: &[OsString]) -> Result<(), Failure> {
            if !self.fired.replace(true) {
                fs::remove_file(&self.target_file).unwrap();
                fs::write(&self.target_file, b"replacement during rollback").unwrap();
            }
            Ok(())
        }

        fn after_rollback_detach(&self, _relative: &[OsString]) {
            if !self.detached.replace(true) {
                fs::write(&self.target_file, b"object occupying restored name").unwrap();
            }
        }
    }

    #[test]
    fn rollback_never_deletes_an_object_replaced_after_its_last_identity_check() {
        let (temp, request, plan, adapter) = fixture("rollback-detach-race-");
        let target_file = temp.path().join("target/file.txt");
        let hook = ReplaceBetweenRollbackCheckAndDetachHook {
            target_file: target_file.clone(),
            fired: Cell::new(false),
            detached: Cell::new(false),
        };

        let failure = ApfsCloneMaterializer::new(adapter)
            .materialize_with_hook(&request, &plan, &hook)
            .unwrap_err();

        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::Incomplete
        );
        assert!(failure.receipt().rollback().removed().is_empty());
        assert_eq!(
            failure.receipt().rollback().unconfirmed_quarantined().len(),
            1
        );
        assert_eq!(
            fs::read(target_file).unwrap(),
            b"object occupying restored name"
        );
        let quarantine = OsString::from_vec(
            failure.receipt().rollback().unconfirmed_quarantined()[0]
                .as_bytes()
                .to_vec(),
        );
        assert_eq!(
            fs::read(temp.path().join("trash").join(quarantine)).unwrap(),
            b"replacement during rollback"
        );
    }

    struct AddUnknownChildBeforeDirectoryDetachHook {
        target_directory: PathBuf,
    }

    impl ExecutionHook for AddUnknownChildBeforeDirectoryDetachHook {
        fn before_root_metadata(&self, _target: &OwnedFd) -> Result<(), Failure> {
            Err(Failure::io(
                "injected failure before directory rollback",
                io::Error::from_raw_os_error(libc::EIO),
            ))
        }

        fn before_rollback_detach(&self, relative: &[OsString]) -> Result<(), Failure> {
            if relative_bytes(relative) == b"dir" {
                fs::write(self.target_directory.join("unknown.txt"), b"external child").unwrap();
            }
            Ok(())
        }
    }

    #[test]
    fn directory_with_a_late_unknown_child_is_restored_and_never_confirmed() {
        let (temp, request, plan, adapter) = fixture("directory-rollback-race-");
        fs::remove_file(temp.path().join("source/file.txt")).unwrap();
        fs::create_dir(temp.path().join("source/dir")).unwrap();
        fs::write(temp.path().join("source/dir/file.txt"), b"nested").unwrap();
        let hook = AddUnknownChildBeforeDirectoryDetachHook {
            target_directory: temp.path().join("target/dir"),
        };

        let failure = ApfsCloneMaterializer::new(adapter)
            .materialize_with_hook(&request, &plan, &hook)
            .unwrap_err();

        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::Incomplete
        );
        assert_eq!(
            fs::read(temp.path().join("target/dir/unknown.txt")).unwrap(),
            b"external child"
        );
        assert_eq!(failure.receipt().rollback().removed().len(), 1);
    }

    struct RootMetadataFailureHook;

    impl ExecutionHook for RootMetadataFailureHook {
        fn before_root_metadata(&self, target: &OwnedFd) -> Result<(), Failure> {
            let before = node_metadata(target).unwrap();
            set_modified_time(
                target,
                before.modified_seconds.saturating_sub(1),
                before.modified_nanoseconds,
            )
            .unwrap();
            Err(Failure::io(
                "injected target root metadata failure",
                io::Error::from_raw_os_error(libc::EIO),
            ))
        }
    }

    #[test]
    fn empty_tree_root_metadata_failure_is_partial_and_restores_the_baseline() {
        let (temp, request, plan, adapter) = fixture("root-metadata-");
        fs::remove_file(temp.path().join("source/file.txt")).unwrap();
        let target = open_absolute_directory(&absolute(&temp.path().join("target"))).unwrap();
        let baseline = node_metadata(&target).unwrap();

        let failure = ApfsCloneMaterializer::new(adapter)
            .materialize_with_hook(&request, &plan, &RootMetadataFailureHook)
            .unwrap_err();

        assert_eq!(failure.receipt().outcome(), MaterializationOutcome::Partial);
        assert!(failure.receipt().created().is_empty());
        assert_eq!(
            failure.receipt().rollback().status(),
            RollbackStatus::ConfirmedBaseline
        );
        assert_eq!(node_metadata(&target).unwrap(), baseline);
    }

    #[test]
    fn submount_guard_rejects_a_directory_entry_from_another_device() {
        let (temp, _, _, _) = fixture("submount-guard-");
        let source = open_absolute_directory(&absolute(&temp.path().join("source"))).unwrap();
        let mut snapshot = TreeSnapshot {
            root: node_metadata(&source).unwrap(),
            entries: BTreeMap::new(),
            manifest: TreeManifest {
                root_mode: 0,
                root_mtime: (0, 0),
                entries: Vec::new(),
                regular_files: 0,
                logical_bytes: 0,
                physical_bytes: 0,
            },
        };

        let error = snapshot_directory(&source, &[], u64::MAX, &mut snapshot).unwrap_err();
        assert_eq!(
            error.kind,
            MaterializationFailureKind::UnsupportedSourceEntry
        );
    }

    #[test]
    fn manifest_digest_binds_each_relative_path() {
        let mut first = TreeManifest {
            root_mode: 0o700,
            root_mtime: (1, 2),
            entries: vec![ManifestEntry {
                path: b"a".to_vec(),
                kind: RawFileKind::RegularFile,
                mode: Some(0o600),
                mtime: Some((3, 4)),
                length: 5,
                content_digest: Some([6; 32]),
            }],
            regular_files: 1,
            logical_bytes: 5,
            physical_bytes: 8,
        };
        let second = first.clone();
        first.entries[0].path = b"b".to_vec();

        assert_ne!(first.digest(), second.digest());
    }

    #[test]
    fn port_error_chain_preserves_original_errno_values() {
        for (failure, expected_errno, expected_kind, expected_port_kind) in [
            (
                Failure::clone_io("injected EIO", io::Error::from_raw_os_error(libc::EIO)),
                libc::EIO,
                MaterializationFailureKind::Filesystem,
                PortErrorKind::Io,
            ),
            (
                Failure::clone_io(
                    "injected ENOSPC",
                    io::Error::from_raw_os_error(libc::ENOSPC),
                ),
                libc::ENOSPC,
                MaterializationFailureKind::NoSpace,
                PortErrorKind::Io,
            ),
            (
                Failure::clone_io("injected EXDEV", io::Error::from_raw_os_error(libc::EXDEV)),
                libc::EXDEV,
                MaterializationFailureKind::PlanStale,
                PortErrorKind::InvalidLayout,
            ),
            (
                Failure::clone_io(
                    "injected ENOTSUP",
                    io::Error::from_raw_os_error(libc::ENOTSUP),
                ),
                libc::ENOTSUP,
                MaterializationFailureKind::CowUnavailable,
                PortErrorKind::CapabilityUnavailable,
            ),
        ] {
            assert_eq!(failure.kind, expected_kind);
            let error = failure.into_port_error();
            assert_eq!(error.kind(), expected_port_kind);
            let source = error.source().unwrap().downcast_ref::<io::Error>().unwrap();
            assert_eq!(source.raw_os_error(), Some(expected_errno));
        }
    }
}
