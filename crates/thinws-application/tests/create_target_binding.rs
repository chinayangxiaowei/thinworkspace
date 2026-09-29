#![cfg(target_os = "macos")]

use std::ffi::OsString;
use std::fs::{self, File, FileTimes};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use tempfile::Builder;
use thinws_adapter_macos::{
    ApfsCloneMaterializer, FullCopyMaterializer, MacOsDataRootLayout, MacOsHostAdapter,
    MacOsInitializingProof, MacOsLockGuard, MacOsPreparedDataRoot, MacOsPreparedWorkspace,
};
use thinws_application::{CreateRequest, InitRequest, ThinWorkspaceService};
use thinws_core::{
    AbsolutePath, ErrorCode, FallbackPolicy, HostCapabilityReport, InstallationIdentity,
    MaterializationAttemptEvidence, MaterializationFailureKind, MaterializationPathReport,
    MaterializationPlan, MaterializationReceipt, MaterializeRequest, MaterializerKind,
    PathCapabilityReport, RollbackEvidence, RollbackStatus, RootMarker, TreeDigest, UnixMillis,
    WorkspaceId, WorkspaceName, WorkspaceState,
};
use thinws_metadata_sqlite::SqliteMetadataStoreFactory;
use thinws_ports::{
    BootstrapStore, LifecycleLock, MaterializationFailure, MaterializationPathProbeRequest,
    PlatformProbe, PortError, PortErrorKind, PublishResult, WorkspaceMaterializer,
};

fn absolute(path: &Path) -> AbsolutePath {
    AbsolutePath::try_from_bytes(path.as_os_str().as_bytes().to_vec()).unwrap()
}

fn make_request(
    source: AbsolutePath,
    name: WorkspaceName,
    allow_copy: bool,
    now: UnixMillis,
) -> CreateRequest {
    let bytes = source.as_bytes();
    let parent_end = bytes.iter().rposition(|byte| *byte == b'/').unwrap();
    let mut target = if parent_end == 0 {
        b"/".to_vec()
    } else {
        bytes[..parent_end].to_vec()
    };
    if target != b"/" {
        target.push(b'/');
    }
    target.extend_from_slice(format!("thinws-test-{}", name.as_str()).as_bytes());
    CreateRequest::new(
        source,
        AbsolutePath::try_from_bytes(target).unwrap(),
        name,
        allow_copy,
        now,
    )
}

struct SwapBeforeTargetProbe {
    inner: MacOsHostAdapter,
    swap_on_probe: usize,
    restore_after_probe: bool,
    probe_count: AtomicUsize,
}

impl SwapBeforeTargetProbe {
    fn new(inner: MacOsHostAdapter, swap_on_probe: usize) -> Self {
        Self {
            inner,
            swap_on_probe,
            restore_after_probe: false,
            probe_count: AtomicUsize::new(0),
        }
    }

    fn restoring(inner: MacOsHostAdapter, swap_on_probe: usize) -> Self {
        Self {
            inner,
            swap_on_probe,
            restore_after_probe: true,
            probe_count: AtomicUsize::new(0),
        }
    }
}

impl LifecycleLock for SwapBeforeTargetProbe {
    type Guard = MacOsLockGuard;

    fn acquire_bootstrap(&self, timeout: Duration) -> Result<Self::Guard, PortError> {
        self.inner.acquire_bootstrap(timeout)
    }

    fn acquire_data_root(
        &self,
        data_root: &AbsolutePath,
        timeout: Duration,
    ) -> Result<Self::Guard, PortError> {
        self.inner.acquire_data_root(data_root, timeout)
    }
}

impl BootstrapStore for SwapBeforeTargetProbe {
    type LockGuard = MacOsLockGuard;
    type PreparedDataRoot = MacOsPreparedDataRoot;
    type InitializingProof = MacOsInitializingProof;
    type DataRootLayout = MacOsDataRootLayout;
    type PreparedWorkspace = MacOsPreparedWorkspace;

    fn prepare_bootstrap(&self) -> Result<(), PortError> {
        self.inner.prepare_bootstrap()
    }

    fn prepare_data_root(
        &self,
        data_root: &AbsolutePath,
    ) -> Result<Self::PreparedDataRoot, PortError> {
        self.inner.prepare_data_root(data_root)
    }

    fn read_config(&self) -> Result<Option<InstallationIdentity>, PortError> {
        self.inner.read_config()
    }

    fn read_root_marker(&self, data_root: &AbsolutePath) -> Result<Option<RootMarker>, PortError> {
        self.inner.read_root_marker(data_root)
    }

    fn create_initializing(
        &self,
        lock: &Self::LockGuard,
        prepared: Self::PreparedDataRoot,
        identity: &InstallationIdentity,
    ) -> Result<Self::InitializingProof, PortError> {
        self.inner.create_initializing(lock, prepared, identity)
    }

    fn initialize_layout(
        &self,
        lock: &Self::LockGuard,
        proof: &Self::InitializingProof,
    ) -> Result<Self::DataRootLayout, PortError> {
        self.inner.initialize_layout(lock, proof)
    }

    fn validate_layout(
        &self,
        identity: &InstallationIdentity,
    ) -> Result<Self::DataRootLayout, PortError> {
        self.inner.validate_layout(identity)
    }

    fn prepare_workspace(
        &self,
        lock: &Self::LockGuard,
        layout: &Self::DataRootLayout,
        workspace_id: WorkspaceId,
        target: &AbsolutePath,
    ) -> Result<Self::PreparedWorkspace, PortError> {
        self.inner
            .prepare_workspace(lock, layout, workspace_id, target)
    }

    fn clear_workspace_incomplete(
        &self,
        lock: &Self::LockGuard,
        layout: &Self::DataRootLayout,
        prepared: Self::PreparedWorkspace,
    ) -> Result<(), PortError> {
        self.inner
            .clear_workspace_incomplete(lock, layout, prepared)
    }

    fn validate_ready_workspace(
        &self,
        layout: &Self::DataRootLayout,
        reservation: &thinws_core::WorkspaceReservation,
    ) -> Result<AbsolutePath, PortError> {
        self.inner.validate_ready_workspace(layout, reservation)
    }

    fn measure_ready_workspace_space(
        &self,
        layout: &Self::DataRootLayout,
        reservation: &thinws_core::WorkspaceReservation,
    ) -> Result<thinws_ports::WorkspaceSpace, PortError> {
        self.inner
            .measure_ready_workspace_space(layout, reservation)
    }

    fn inspect_removal_container(
        &self,
        lock: &Self::LockGuard,
        layout: &Self::DataRootLayout,
        reservation: &thinws_core::WorkspaceReservation,
    ) -> Result<Option<AbsolutePath>, PortError> {
        self.inner
            .inspect_removal_container(lock, layout, reservation)
    }

    fn append_removal_log(
        &self,
        lock: &Self::LockGuard,
        layout: &Self::DataRootLayout,
        record: &thinws_ports::RemovalLogRecord<'_>,
    ) -> Result<AbsolutePath, PortError> {
        self.inner.append_removal_log(lock, layout, record)
    }

    fn remove_workspace(
        &self,
        lock: &Self::LockGuard,
        layout: &Self::DataRootLayout,
        reservation: &thinws_core::WorkspaceReservation,
    ) -> Result<thinws_ports::WorkspaceRemoval, PortError> {
        self.inner.remove_workspace(lock, layout, reservation)
    }

    fn publish_ready(
        &self,
        lock: &Self::LockGuard,
        proof: Self::InitializingProof,
    ) -> Result<RootMarker, PortError> {
        self.inner.publish_ready(lock, proof)
    }

    fn publish_config(
        &self,
        lock: &Self::LockGuard,
        identity: &InstallationIdentity,
    ) -> Result<PublishResult, PortError> {
        self.inner.publish_config(lock, identity)
    }
}

impl PlatformProbe for SwapBeforeTargetProbe {
    fn inspect_host(&self) -> Result<HostCapabilityReport, PortError> {
        self.inner.inspect_host()
    }

    fn inspect_path(&self, path: &AbsolutePath) -> Result<PathCapabilityReport, PortError> {
        self.inner.inspect_path(path)
    }

    fn inspect_materialization_paths(
        &self,
        request: &MaterializationPathProbeRequest,
    ) -> Result<MaterializationPathReport, PortError> {
        let call = self.probe_count.fetch_add(1, Ordering::SeqCst) + 1;
        let target = PathBuf::from(OsString::from_vec(
            request.target_root().as_bytes().to_vec(),
        ));
        if call == self.swap_on_probe {
            let displaced = target.with_file_name("root-original");
            fs::rename(&target, &displaced).unwrap();
            fs::create_dir(&target).unwrap();
            fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
            File::open(&target)
                .unwrap()
                .set_times(
                    FileTimes::new().set_modified(UNIX_EPOCH + Duration::from_secs(946_684_800)),
                )
                .unwrap();
        }
        let report = self.inner.inspect_materialization_paths(request)?;
        if call == self.swap_on_probe && self.restore_after_probe {
            fs::rename(&target, target.with_file_name("root-replacement")).unwrap();
            fs::rename(target.with_file_name("root-original"), &target).unwrap();
        }
        Ok(report)
    }
}

struct CleanCowUnavailable {
    source_digest: TreeDigest,
}

impl WorkspaceMaterializer for CleanCowUnavailable {
    fn kind(&self) -> MaterializerKind {
        MaterializerKind::ApfsFileClone
    }

    fn materialize(
        &self,
        _request: &MaterializeRequest,
        plan: &MaterializationPlan,
    ) -> Result<MaterializationReceipt, MaterializationFailure> {
        Err(MaterializationFailure::new(
            PortError::new(
                PortErrorKind::CapabilityUnavailable,
                "injected CoW unavailable",
            ),
            MaterializationReceipt::failed_cow_clone(
                plan,
                MaterializationFailureKind::CowUnavailable,
                Vec::new(),
                false,
                RollbackEvidence::new(RollbackStatus::NotNeeded, Vec::new(), Vec::new()),
                MaterializationAttemptEvidence::new(
                    Some(0),
                    Some(0),
                    Some(0),
                    0,
                    Some(self.source_digest),
                    None,
                ),
                0,
            ),
        ))
    }
}

fn source_digest(adapter: &MacOsHostAdapter, source: &Path, temp: &Path) -> TreeDigest {
    let target = temp.join("probe-target");
    let staging = temp.join("probe-staging");
    let trash = temp.join("probe-trash");
    for directory in [&target, &staging, &trash] {
        fs::create_dir(directory).unwrap();
    }
    let request = MaterializeRequest::new(
        absolute(source),
        absolute(&target),
        absolute(&staging),
        absolute(&trash),
    );
    let report = adapter
        .inspect_materialization_paths(&MaterializationPathProbeRequest::from(&request))
        .unwrap();
    let plan = MaterializationPlan::for_cow_clone(&report, FallbackPolicy::Deny).unwrap();
    ApfsCloneMaterializer::new(adapter.clone())
        .materialize(&request, &plan)
        .unwrap()
        .source_manifest_digest()
        .unwrap()
}

#[test]
fn p1_12_rejects_replaced_root_before_initial_plan_without_writing_it() {
    let controlled =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-12-target-binding-tests");
    fs::create_dir_all(&controlled).unwrap();
    let temp = Builder::new()
        .prefix("root-before-plan-")
        .tempdir_in(fs::canonicalize(controlled).unwrap())
        .unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("note.txt"), b"must not write the replacement").unwrap();
    let data_root = temp.path().join("data-root");
    let inner = MacOsHostAdapter::new(temp.path().join("data-root")).unwrap();
    let service = ThinWorkspaceService::new(
        SwapBeforeTargetProbe::new(inner.clone(), 2),
        SqliteMetadataStoreFactory,
        Duration::from_secs(1),
        Duration::from_secs(1),
    );
    service
        .init(InitRequest::new(
            absolute(&data_root),
            UnixMillis::new(1_700_000_000_000).unwrap(),
        ))
        .unwrap();

    let error = service
        .create(
            make_request(
                absolute(&source),
                WorkspaceName::from_str("replaced-root").unwrap(),
                false,
                UnixMillis::new(1_700_000_000_100).unwrap(),
            ),
            &ApfsCloneMaterializer::new(inner.clone()),
            &FullCopyMaterializer::new(inner),
        )
        .unwrap_err();
    assert_eq!(error.diagnostic().code(), ErrorCode::TargetLayout);
    let id = error.diagnostic().context()["workspace_id"].user_value();
    let replacement = temp.path().join("thinws-test-replaced-root");
    assert!(
        data_root
            .join("metadata")
            .join(format!("ownership-{id}.toml"))
            .exists()
    );
    let metadata = fs::metadata(&replacement).unwrap();
    assert_eq!(metadata.mode() & 0o7777, 0o700);
    assert_eq!(metadata.mtime(), 946_684_800);
    assert_eq!(fs::read_dir(&replacement).unwrap().count(), 0);
    assert_eq!(service.doctor().unwrap().incomplete_workspaces(), 1);
    let record = service
        .list_workspaces()
        .unwrap()
        .into_iter()
        .find(|item| item.record().reservation().name().as_str() == "replaced-root")
        .unwrap();
    assert_eq!(record.record().state(), WorkspaceState::Error);
}

#[test]
fn p1_12_rejects_replaced_root_before_fallback_plan_without_writing_it() {
    let controlled =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-12-target-binding-tests");
    fs::create_dir_all(&controlled).unwrap();
    let temp = Builder::new()
        .prefix("root-before-fallback-")
        .tempdir_in(fs::canonicalize(controlled).unwrap())
        .unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("note.txt"), b"must not write the replacement").unwrap();
    let data_root = temp.path().join("data-root");
    let inner = MacOsHostAdapter::new(temp.path().join("data-root")).unwrap();
    let digest = source_digest(&inner, &source, temp.path());
    let service = ThinWorkspaceService::new(
        SwapBeforeTargetProbe::restoring(inner.clone(), 3),
        SqliteMetadataStoreFactory,
        Duration::from_secs(1),
        Duration::from_secs(1),
    );
    service
        .init(InitRequest::new(
            absolute(&data_root),
            UnixMillis::new(1_700_000_000_000).unwrap(),
        ))
        .unwrap();

    let error = service
        .create(
            make_request(
                absolute(&source),
                WorkspaceName::from_str("replaced-fallback-root").unwrap(),
                true,
                UnixMillis::new(1_700_000_000_100).unwrap(),
            ),
            &CleanCowUnavailable {
                source_digest: digest,
            },
            &FullCopyMaterializer::new(inner),
        )
        .unwrap_err();
    assert_eq!(error.diagnostic().code(), ErrorCode::TargetLayout);
    assert_eq!(error.partial_receipts().len(), 1);
    let id = error.diagnostic().context()["workspace_id"].user_value();
    let replacement = temp.path().join("root-replacement");
    assert!(
        data_root
            .join("metadata")
            .join(format!("ownership-{id}.toml"))
            .exists()
    );
    let metadata = fs::metadata(&replacement).unwrap();
    assert_eq!(metadata.mode() & 0o7777, 0o700);
    assert_eq!(metadata.mtime(), 946_684_800);
    assert_eq!(fs::read_dir(&replacement).unwrap().count(), 0);
    assert_eq!(
        fs::read_dir(temp.path().join("thinws-test-replaced-fallback-root"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(service.doctor().unwrap().incomplete_workspaces(), 1);
}
