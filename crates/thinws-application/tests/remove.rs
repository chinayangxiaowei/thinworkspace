use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tempfile::{Builder, TempDir};
use thinws_adapter_macos::{ApfsCloneMaterializer, FullCopyMaterializer, MacOsHostAdapter};
use thinws_application::{
    CreateRequest, InitRequest, RemoveRequest, RemoveResult, ThinWorkspaceService,
};
use thinws_core::{
    AbsolutePath, DiscoveryCompleteness, ErrorCode, ProcessUse, RemovalWarning, RepositoryState,
    UnixMillis, WorkspaceId, WorkspaceName, WorkspaceState,
};
use thinws_metadata_sqlite::SqliteMetadataStoreFactory;
use thinws_ports::{
    BootstrapStore, GitInspection, GitInspector, LifecycleLock, MetadataStoreFactory, PortError,
    PortErrorKind, ProcessObservation, ProcessProbe,
};

fn absolute(path: &Path) -> AbsolutePath {
    AbsolutePath::try_from_bytes(path.as_os_str().as_bytes().to_vec()).unwrap()
}

struct NoRepositories;

impl GitInspector for NoRepositories {
    fn inspect(&self, _copy_root: &AbsolutePath) -> GitInspection {
        GitInspection::new(DiscoveryCompleteness::Complete, Vec::new(), Vec::new())
    }
}

struct NoExternalUse;

impl ProcessProbe for NoExternalUse {
    fn inspect_workspace(
        &self,
        _workspace_container: &AbsolutePath,
    ) -> Result<ProcessObservation, PortError> {
        Ok(ProcessObservation {
            observed_at: UnixMillis::new(1_700_000_000_000).unwrap(),
            use_state: ProcessUse::NoEvidence,
        })
    }
}

fn ready_fixture() -> (
    TempDir,
    ThinWorkspaceService<MacOsHostAdapter, SqliteMetadataStoreFactory>,
    WorkspaceId,
) {
    let controlled =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-12-application-tests");
    fs::create_dir_all(&controlled).unwrap();
    let temp = Builder::new()
        .prefix("remove-")
        .tempdir_in(fs::canonicalize(controlled).unwrap())
        .unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("note.txt"), b"source survives removal").unwrap();
    let data_root = temp.path().join("data-root");
    let adapter = MacOsHostAdapter::new(temp.path().join("bootstrap")).unwrap();
    let service = ThinWorkspaceService::new(
        adapter.clone(),
        SqliteMetadataStoreFactory,
        Duration::from_secs(1),
        Duration::from_secs(1),
    );
    let now = UnixMillis::new(1_700_000_000_000).unwrap();
    service
        .init(InitRequest::new(absolute(&data_root), now))
        .unwrap();
    let name: WorkspaceName = "remove-case".parse().unwrap();
    let created = service
        .create(
            CreateRequest::new(absolute(&source), name, false, now),
            &ApfsCloneMaterializer::new(adapter.clone()),
            &FullCopyMaterializer::new(adapter),
        )
        .unwrap();
    let workspace_id = created.record().reservation().workspace_id();
    (temp, service, workspace_id)
}

fn target(temp: &TempDir, id: WorkspaceId) -> PathBuf {
    temp.path()
        .join("data-root/workspaces")
        .join(id.to_string())
}

fn log_events(temp: &TempDir) -> Vec<serde_json::Value> {
    let log = fs::read_to_string(temp.path().join("data-root/logs/operations.jsonl")).unwrap();
    log.lines()
        .map(serde_json::from_str::<serde_json::Value>)
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

#[test]
fn p1_12_normal_remove_deletes_only_registered_copy_and_records_durable_events() {
    let (temp, service, workspace_id) = ready_fixture();
    let target = target(&temp, workspace_id);
    assert!(target.join("root/note.txt").is_file());

    let outcome = service
        .remove(
            RemoveRequest::try_from_raw("remove-case", false, 1_700_000_000_001).unwrap(),
            &NoRepositories,
            &NoExternalUse,
        )
        .unwrap();
    assert_eq!(outcome.result(), RemoveResult::Removed);
    assert_eq!(outcome.result().as_str(), "removed");
    assert_eq!(outcome.workspace_id(), workspace_id);
    assert!(!outcome.forced());
    assert_eq!(outcome.warning(), None);
    assert!(!target.exists());
    assert_eq!(
        fs::read(temp.path().join("source/note.txt")).unwrap(),
        b"source survives removal"
    );
    let events = log_events(&temp);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["event"], "started");
    assert_eq!(events[0]["git_state"], "not-applicable");
    assert_eq!(events[0]["git_check_complete"], true);
    assert_eq!(events[1]["event"], "completed");
}

#[test]
fn p1_12_tracked_dirty_refuses_ordinary_remove_but_explicit_force_succeeds() {
    let (temp, service, id) = ready_fixture();
    let dirty = FixedGit(GitInspection::new(
        DiscoveryCompleteness::Complete,
        vec![thinws_ports::RepositoryInspection::new(
            PathBuf::new(),
            thinws_core::RepositoryState::Dirty {
                tracked_changes: std::num::NonZeroUsize::new(1).unwrap(),
            },
            Vec::new(),
        )],
        Vec::new(),
    ));
    let error = service
        .remove(
            RemoveRequest::try_from_raw("remove-case", false, 1_700_000_000_001).unwrap(),
            &dirty,
            &NoExternalUse,
        )
        .unwrap_err();
    assert_eq!(error.diagnostic().code(), ErrorCode::WorkspaceDirty);
    assert_eq!(
        error.diagnostic().message(),
        "Workspace has tracked changes"
    );
    assert_eq!(
        error.diagnostic().remediation(),
        Some(
            "Preserve and commit required work, or use workspace remove <name-or-id> --force to discard the copy."
        )
    );
    assert!(target(&temp, id).join("root/note.txt").is_file());
    let refused = log_events(&temp);
    assert_eq!(refused[0]["event"], "refused");
    assert_eq!(refused[0]["git_check_complete"], true);

    let outcome = service
        .remove(
            RemoveRequest::try_from_raw("remove-case", true, 1_700_000_000_002).unwrap(),
            &NeverInspectGit,
            &NoExternalUse,
        )
        .unwrap();
    assert_eq!(outcome.result(), RemoveResult::Removed);
    assert!(outcome.forced());
    assert!(!target(&temp, id).exists());
    let events = log_events(&temp);
    assert_eq!(events.len(), 3);
    assert_eq!(events[1]["event"], "started");
    assert_eq!(events[2]["event"], "completed");
}

#[test]
fn p1_12_incomplete_git_check_refuses_ordinary_remove_but_force_can_continue() {
    let (temp, service, id) = ready_fixture();
    let unknown = FixedGit(GitInspection::new(
        DiscoveryCompleteness::Incomplete,
        Vec::new(),
        Vec::new(),
    ));
    let error = service
        .remove(
            RemoveRequest::try_from_raw("remove-case", false, 1_700_000_000_001).unwrap(),
            &unknown,
            &NoExternalUse,
        )
        .unwrap_err();
    assert_eq!(error.diagnostic().code(), ErrorCode::GitCheckIncomplete);
    assert!(target(&temp, id).join("root/note.txt").is_file());
    let refused = log_events(&temp);
    assert_eq!(refused[0]["git_state"], "unknown");
    assert_eq!(refused[0]["git_check_complete"], false);

    let result = service
        .remove(
            RemoveRequest::try_from_raw("remove-case", true, 1_700_000_000_002).unwrap(),
            &unknown,
            &NoExternalUse,
        )
        .unwrap();
    assert_eq!(result.result(), RemoveResult::Removed);
    assert!(!target(&temp, id).exists());
}

struct FixedGit(GitInspection);

impl GitInspector for FixedGit {
    fn inspect(&self, _copy_root: &AbsolutePath) -> GitInspection {
        self.0.clone()
    }
}

struct NeverInspectGit;

impl GitInspector for NeverInspectGit {
    fn inspect(&self, _copy_root: &AbsolutePath) -> GitInspection {
        panic!("force must not run Git inspection")
    }
}

struct FixedProcess(ProcessUse);

impl ProcessProbe for FixedProcess {
    fn inspect_workspace(
        &self,
        _workspace_container: &AbsolutePath,
    ) -> Result<ProcessObservation, PortError> {
        Ok(ProcessObservation {
            observed_at: UnixMillis::new(1_700_000_000_000).unwrap(),
            use_state: self.0,
        })
    }
}

#[test]
fn p1_12_confirmed_process_use_blocks_force_without_deletion() {
    let (temp, service, id) = ready_fixture();
    let error = service
        .remove(
            RemoveRequest::try_from_raw("remove-case", true, 1_700_000_000_001).unwrap(),
            &NoRepositories,
            &FixedProcess(ProcessUse::ConfirmedInUse),
        )
        .unwrap_err();
    assert_eq!(error.diagnostic().code(), ErrorCode::WorkspaceBusy);
    assert!(target(&temp, id).join("root/note.txt").is_file());
    let events = log_events(&temp);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event"], "refused");
    assert_eq!(events[0]["protection_reason"], "confirmed-in-use");
}

#[test]
fn p1_12_repeated_exact_id_returns_tombstone_without_reselecting_a_name() {
    let (temp, service, id) = ready_fixture();
    service
        .remove(
            RemoveRequest::try_from_raw("remove-case", true, 1_700_000_000_001).unwrap(),
            &NoRepositories,
            &NoExternalUse,
        )
        .unwrap();
    let repeated = service
        .remove(
            RemoveRequest::try_from_raw(&id.to_string(), false, 1_700_000_000_002).unwrap(),
            &NoRepositories,
            &NoExternalUse,
        )
        .unwrap();
    assert_eq!(repeated.workspace_id(), id);
    assert_eq!(repeated.result(), RemoveResult::AlreadyRemoved);
    assert_eq!(repeated.result().as_str(), "already-removed");
    assert!(!repeated.forced());
    assert!(!target(&temp, id).exists());
    let events = log_events(&temp);
    assert_eq!(events.len(), 4);
    assert_eq!(events[3]["result"], "already-absent");
}

#[test]
fn missing_registered_target_keeps_the_active_record_even_with_force() {
    let (temp, service, id) = ready_fixture();
    fs::remove_dir_all(target(&temp, id)).unwrap();

    for force in [false, true] {
        let error = service
            .remove(
                RemoveRequest::try_from_raw("remove-case", force, 1_700_000_000_002).unwrap(),
                &NoRepositories,
                &NoExternalUse,
            )
            .unwrap_err();
        assert_eq!(error.diagnostic().code().as_str(), "E_TARGET_MISSING");
        let active = service.list_workspaces().unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].record().reservation().workspace_id(), id);
        assert_eq!(active[0].record().state(), WorkspaceState::Ready);
    }
    let events = log_events(&temp);
    assert_eq!(events.len(), 2);
    assert!(events.iter().all(|event| event["event"] == "failed"));
}

#[test]
fn missing_registered_root_inside_old_container_keeps_the_active_record() {
    let (temp, service, id) = ready_fixture();
    let container = target(&temp, id);
    fs::remove_dir_all(container.join("root")).unwrap();

    let error = service
        .remove(
            RemoveRequest::try_from_raw("remove-case", true, 1_700_000_000_002).unwrap(),
            &NoRepositories,
            &NoExternalUse,
        )
        .unwrap_err();
    assert_eq!(error.diagnostic().code(), ErrorCode::TargetMissing);
    assert!(container.exists());
    let active = service.list_workspaces().unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].record().reservation().workspace_id(), id);
}

#[test]
fn p1_12_complete_discovery_with_unknown_repository_is_not_logged_as_a_complete_check() {
    let (temp, service, id) = ready_fixture();
    let unknown = FixedGit(GitInspection::new(
        DiscoveryCompleteness::Complete,
        vec![thinws_ports::RepositoryInspection::new(
            PathBuf::new(),
            RepositoryState::Unknown,
            Vec::new(),
        )],
        Vec::new(),
    ));
    let error = service
        .remove(
            RemoveRequest::try_from_raw("remove-case", false, 1_700_000_000_001).unwrap(),
            &unknown,
            &FixedProcess(ProcessUse::ScanIncomplete),
        )
        .unwrap_err();
    assert_eq!(error.diagnostic().code(), ErrorCode::GitCheckIncomplete);
    assert_eq!(
        error.diagnostic().message(),
        "tracked-change check is incomplete"
    );
    assert_eq!(
        error
            .diagnostic()
            .context()
            .get("process_use")
            .map(|value| value.user_value()),
        Some("scan-incomplete")
    );
    assert_eq!(log_events(&temp)[0]["git_check_complete"], false);
    assert_eq!(log_events(&temp)[0]["process_use"], "scan-incomplete");
    assert!(target(&temp, id).join("root/note.txt").is_file());
}

#[test]
fn p1_12_incomplete_process_scan_warns_without_claiming_no_evidence() {
    let (temp, service, id) = ready_fixture();
    let outcome = service
        .remove(
            RemoveRequest::try_from_raw("remove-case", false, 1_700_000_000_001).unwrap(),
            &NoRepositories,
            &FixedProcess(ProcessUse::ScanIncomplete),
        )
        .unwrap();
    assert_eq!(outcome.workspace_id(), id);
    assert_eq!(
        outcome.warning(),
        Some(RemovalWarning::ProcessScanIncomplete)
    );
    let events = log_events(&temp);
    assert_eq!(events[0]["process_use"], "scan-incomplete");
    assert_eq!(events[1]["process_use"], "scan-incomplete");
}

#[test]
fn p1_12_unwritable_log_stops_force_before_deleting_or_entering_deleting() {
    let (temp, service, id) = ready_fixture();
    let external = temp.path().join("outside-log");
    fs::write(&external, b"unchanged").unwrap();
    symlink(
        &external,
        temp.path().join("data-root/logs/operations.jsonl"),
    )
    .unwrap();
    let error = service
        .remove(
            RemoveRequest::try_from_raw("remove-case", true, 1_700_000_000_001).unwrap(),
            &NoRepositories,
            &NoExternalUse,
        )
        .unwrap_err();
    assert_eq!(error.diagnostic().code(), ErrorCode::Filesystem);
    assert!(target(&temp, id).join("root/note.txt").is_file());
    assert_eq!(fs::read(external).unwrap(), b"unchanged");
    assert!(service.workspace_path("remove-case").is_ok());
}

#[test]
fn p1_12_error_workspace_requires_new_explicit_force_without_replaying_creation() {
    let (temp, service, id) = ready_fixture();
    let adapter = MacOsHostAdapter::new(temp.path().join("bootstrap")).unwrap();
    let identity = adapter.read_config().unwrap().unwrap();
    let lock = adapter
        .acquire_data_root(identity.data_root(), Duration::from_secs(1))
        .unwrap();
    let layout = adapter.validate_layout(&identity).unwrap();
    let mut metadata = SqliteMetadataStoreFactory
        .open_existing(
            &layout,
            &thinws_core::InstallationRecord::new(
                identity,
                UnixMillis::new(1_700_000_000_001).unwrap(),
            ),
            Duration::from_secs(1),
        )
        .unwrap();
    metadata
        .record_failure(
            id,
            WorkspaceState::Ready,
            ErrorCode::Filesystem,
            UnixMillis::new(1_700_000_000_001).unwrap(),
        )
        .unwrap();
    drop(metadata);
    drop(lock);

    let error = service
        .remove(
            RemoveRequest::try_from_raw("remove-case", false, 1_700_000_000_002).unwrap(),
            &NoRepositories,
            &FixedProcess(ProcessUse::ScanIncomplete),
        )
        .unwrap_err();
    assert_eq!(error.diagnostic().code(), ErrorCode::WorkspaceIncomplete);
    assert_eq!(
        error
            .diagnostic()
            .context()
            .get("process_use")
            .map(|value| value.user_value()),
        Some("scan-incomplete")
    );
    assert!(target(&temp, id).join("root/note.txt").is_file());
    let forced = service
        .remove(
            RemoveRequest::try_from_raw("remove-case", true, 1_700_000_000_003).unwrap(),
            &NoRepositories,
            &NoExternalUse,
        )
        .unwrap();
    assert_eq!(forced.result(), RemoveResult::Removed);
    assert!(!target(&temp, id).exists());
}

struct ReplaceRootDuringProcessScan {
    container: PathBuf,
    external: PathBuf,
}

impl ProcessProbe for ReplaceRootDuringProcessScan {
    fn inspect_workspace(
        &self,
        workspace_container: &AbsolutePath,
    ) -> Result<ProcessObservation, PortError> {
        assert_eq!(
            workspace_container.as_bytes(),
            self.container.as_os_str().as_bytes()
        );
        fs::remove_dir_all(self.container.join("root")).unwrap();
        symlink(&self.external, self.container.join("root")).unwrap();
        Ok(ProcessObservation {
            observed_at: UnixMillis::new(1_700_000_000_000).unwrap(),
            use_state: ProcessUse::NoEvidence,
        })
    }
}

#[test]
fn p1_12_root_replacement_before_delete_fails_and_logs_without_touching_external_data() {
    let (temp, service, id) = ready_fixture();
    let external = temp.path().join("external");
    fs::create_dir(&external).unwrap();
    fs::write(external.join("precious.txt"), b"untouched").unwrap();
    let container = target(&temp, id);
    let error = service
        .remove(
            RemoveRequest::try_from_raw("remove-case", true, 1_700_000_000_001).unwrap(),
            &NoRepositories,
            &ReplaceRootDuringProcessScan {
                container: container.clone(),
                external: external.clone(),
            },
        )
        .unwrap_err();
    assert_eq!(error.diagnostic().code().as_str(), "E_TARGET_IDENTITY");
    assert_eq!(
        fs::read(external.join("precious.txt")).unwrap(),
        b"untouched"
    );
    assert!(container.join("root").is_symlink());
    let events = log_events(&temp);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["event"], "started");
    assert_eq!(events[1]["event"], "failed");
    let listed = service.list_workspaces().unwrap();
    assert_eq!(listed[0].record().state(), WorkspaceState::Error);

    fs::remove_file(container.join("root")).unwrap();
    let retried = service
        .remove(
            RemoveRequest::try_from_raw("remove-case", true, 1_700_000_000_002).unwrap(),
            &NoRepositories,
            &NoExternalUse,
        )
        .unwrap_err();
    assert_eq!(retried.diagnostic().code(), ErrorCode::TargetMissing);
    assert!(container.exists());
    assert_eq!(service.list_workspaces().unwrap().len(), 1);
    assert_eq!(
        fs::read(external.join("precious.txt")).unwrap(),
        b"untouched"
    );
}

struct ProcessProbeFails;

impl ProcessProbe for ProcessProbeFails {
    fn inspect_workspace(
        &self,
        _workspace_container: &AbsolutePath,
    ) -> Result<ProcessObservation, PortError> {
        Err(PortError::new(
            PortErrorKind::Io,
            "injected process preflight failure",
        ))
    }
}

#[test]
fn p1_12_process_preflight_failure_is_logged_before_any_removal_state_change() {
    let (temp, service, id) = ready_fixture();
    let error = service
        .remove(
            RemoveRequest::try_from_raw("remove-case", true, 1_700_000_000_001).unwrap(),
            &NoRepositories,
            &ProcessProbeFails,
        )
        .unwrap_err();
    assert_eq!(error.diagnostic().code(), ErrorCode::Filesystem);
    assert!(target(&temp, id).join("root/note.txt").is_file());
    assert!(service.workspace_path("remove-case").is_ok());
    let events = log_events(&temp);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event"], "failed");
    assert_eq!(events[0]["error_code"], "E_FILESYSTEM");
}

#[test]
fn p1_12_unproven_root_refuses_force_and_persists_a_failed_preflight_event() {
    let (temp, service, id) = ready_fixture();
    let external = temp.path().join("external");
    fs::create_dir(&external).unwrap();
    fs::write(external.join("precious.txt"), b"untouched").unwrap();
    let container = target(&temp, id);
    fs::remove_dir_all(container.join("root")).unwrap();
    symlink(&external, container.join("root")).unwrap();
    let error = service
        .remove(
            RemoveRequest::try_from_raw("remove-case", true, 1_700_000_000_001).unwrap(),
            &NoRepositories,
            &NoExternalUse,
        )
        .unwrap_err();
    assert_eq!(error.diagnostic().code().as_str(), "E_TARGET_IDENTITY");
    assert_eq!(
        fs::read(external.join("precious.txt")).unwrap(),
        b"untouched"
    );
    assert!(container.join("root").is_symlink());
    let events = log_events(&temp);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event"], "failed");
    assert_eq!(events[0]["error_code"], "E_TARGET_IDENTITY");
}

#[test]
fn p1_12_name_that_equals_another_id_cannot_silently_select_the_wrong_workspace() {
    let (temp, service, id_b) = ready_fixture();
    let adapter = MacOsHostAdapter::new(temp.path().join("bootstrap")).unwrap();
    let name_a: WorkspaceName = id_b.to_string().parse().unwrap();
    let created_a = service
        .create(
            CreateRequest::new(
                absolute(&temp.path().join("source")),
                name_a,
                false,
                UnixMillis::new(1_700_000_000_001).unwrap(),
            ),
            &ApfsCloneMaterializer::new(adapter.clone()),
            &FullCopyMaterializer::new(adapter),
        )
        .unwrap();
    let id_a = created_a.record().reservation().workspace_id();
    assert_ne!(id_a, id_b);
    let target_a = target(&temp, id_a);
    let target_b = target(&temp, id_b);
    assert!(target_a.is_dir() && target_b.is_dir());

    let error = service
        .remove(
            RemoveRequest::try_from_raw(&id_b.to_string(), false, 1_700_000_000_002).unwrap(),
            &NoRepositories,
            &NoExternalUse,
        )
        .unwrap_err();
    assert_eq!(error.diagnostic().code(), ErrorCode::Usage);
    assert!(target_a.is_dir() && target_b.is_dir());

    let selected_b = service
        .remove(
            RemoveRequest::try_from_raw(&format!("id:{id_b}"), false, 1_700_000_000_003).unwrap(),
            &NoRepositories,
            &NoExternalUse,
        )
        .unwrap();
    assert_eq!(selected_b.workspace_id(), id_b);
    assert!(target_a.is_dir() && !target_b.exists());
    let selected_a = service
        .remove(
            RemoveRequest::try_from_raw(&format!("name:{id_b}"), false, 1_700_000_000_004).unwrap(),
            &NoRepositories,
            &NoExternalUse,
        )
        .unwrap();
    assert_eq!(selected_a.workspace_id(), id_a);
    assert!(!target_a.exists());
}
