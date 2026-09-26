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
    AbsolutePath, DiscoveryCompleteness, ErrorCode, ProcessUse, UnixMillis, WorkspaceId,
    WorkspaceName,
};
use thinws_metadata_sqlite::SqliteMetadataStoreFactory;
use thinws_ports::{GitInspection, GitInspector, PortError, ProcessObservation, ProcessProbe};

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
    assert_eq!(outcome.workspace_id(), workspace_id);
    assert!(!target.exists());
    assert_eq!(
        fs::read(temp.path().join("source/note.txt")).unwrap(),
        b"source survives removal"
    );
    let events = log_events(&temp);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["event"], "started");
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
    assert!(target(&temp, id).join("root/note.txt").is_file());
    assert_eq!(log_events(&temp)[0]["event"], "refused");

    let outcome = service
        .remove(
            RemoveRequest::try_from_raw("remove-case", true, 1_700_000_000_002).unwrap(),
            &dirty,
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
    assert!(!target(&temp, id).exists());
    let events = log_events(&temp);
    assert_eq!(events.len(), 4);
    assert_eq!(events[3]["result"], "already-absent");
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
