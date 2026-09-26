use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use tempfile::Builder;
use thinws_adapter_macos::{ApfsCloneMaterializer, FullCopyMaterializer, MacOsHostAdapter};
use thinws_application::{CreateRequest, InitRequest, ThinWorkspaceService};
use thinws_core::{
    AbsolutePath, DeletionTombstone, ErrorCode, InstallationRecord, InstanceId,
    MaterializationPlan, MaterializationReceipt, RemovalMode, UnixMillis, WorkspaceId,
    WorkspaceName, WorkspaceRecord, WorkspaceReservation, WorkspaceState,
};
use thinws_metadata_sqlite::SqliteMetadataStoreFactory;
use thinws_ports::{
    DataRootLayoutEvidence, FinalMaterializationSummary, MetadataSnapshot, MetadataStore,
    MetadataStoreFactory, PortError, PortErrorKind,
};

fn absolute(path: &Path) -> AbsolutePath {
    AbsolutePath::try_from_bytes(path.as_os_str().as_bytes().to_vec()).unwrap()
}

struct FailFinalCommitFactory;

impl<L: DataRootLayoutEvidence> MetadataStoreFactory<L> for FailFinalCommitFactory {
    fn initialize(
        &self,
        layout: &L,
        expected: &InstallationRecord,
        busy_timeout: Duration,
    ) -> Result<InstallationRecord, PortError> {
        SqliteMetadataStoreFactory.initialize(layout, expected, busy_timeout)
    }

    fn inspect(
        &self,
        layout: &L,
        expected: &InstallationRecord,
        busy_timeout: Duration,
    ) -> Result<MetadataSnapshot, PortError> {
        SqliteMetadataStoreFactory.inspect(layout, expected, busy_timeout)
    }

    fn open_existing(
        &self,
        layout: &L,
        expected: &InstallationRecord,
        busy_timeout: Duration,
    ) -> Result<Box<dyn MetadataStore>, PortError> {
        let inner = SqliteMetadataStoreFactory.open_existing(layout, expected, busy_timeout)?;
        Ok(Box::new(FailFinalCommitStore(inner)))
    }
}

struct FailFinalCommitStore(Box<dyn MetadataStore>);

impl MetadataStore for FailFinalCommitStore {
    fn installation(&self) -> &InstallationRecord {
        self.0.installation()
    }

    fn reserve_workspace(
        &mut self,
        reservation: &WorkspaceReservation,
    ) -> Result<WorkspaceRecord, PortError> {
        self.0.reserve_workspace(reservation)
    }

    fn complete_materialization(
        &mut self,
        _workspace_id: WorkspaceId,
        _plan: &MaterializationPlan,
        _receipt: &MaterializationReceipt,
        _recorded_at: UnixMillis,
    ) -> Result<WorkspaceRecord, PortError> {
        Err(PortError::new(
            PortErrorKind::Storage,
            "injected final SQLite commit failure",
        ))
    }

    fn workspace(&self, workspace_id: WorkspaceId) -> Result<Option<WorkspaceRecord>, PortError> {
        self.0.workspace(workspace_id)
    }

    fn final_materialization(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<FinalMaterializationSummary>, PortError> {
        self.0.final_materialization(workspace_id)
    }

    fn workspaces(&self) -> Result<Vec<WorkspaceRecord>, PortError> {
        self.0.workspaces()
    }

    fn record_failure(
        &mut self,
        workspace_id: WorkspaceId,
        expected: WorkspaceState,
        error_code: ErrorCode,
        updated_at: UnixMillis,
    ) -> Result<WorkspaceRecord, PortError> {
        self.0
            .record_failure(workspace_id, expected, error_code, updated_at)
    }

    fn begin_removal(
        &mut self,
        workspace_id: WorkspaceId,
        expected: WorkspaceState,
        mode: RemovalMode,
        updated_at: UnixMillis,
    ) -> Result<WorkspaceRecord, PortError> {
        self.0
            .begin_removal(workspace_id, expected, mode, updated_at)
    }

    fn complete_deletion(
        &mut self,
        workspace_id: WorkspaceId,
        instance_id: InstanceId,
        deleted_at: UnixMillis,
    ) -> Result<DeletionTombstone, PortError> {
        self.0
            .complete_deletion(workspace_id, instance_id, deleted_at)
    }
}

#[test]
fn final_metadata_commit_failure_never_publishes_ready_even_after_clone() {
    let controlled =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-09-application-tests");
    fs::create_dir_all(&controlled).unwrap();
    let temp = Builder::new()
        .prefix("create-final-commit-failure-")
        .tempdir_in(fs::canonicalize(controlled).unwrap())
        .unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("file.txt"), b"cloned but not ready").unwrap();
    let data_root = temp.path().join("data-root");
    let adapter = MacOsHostAdapter::new(temp.path().join("bootstrap")).unwrap();
    let service = ThinWorkspaceService::new(
        adapter.clone(),
        FailFinalCommitFactory,
        Duration::from_secs(1),
        Duration::from_secs(1),
    );
    service
        .init(InitRequest::new(
            absolute(&data_root),
            UnixMillis::new(1_700_000_000_000).unwrap(),
        ))
        .unwrap();
    let clone = ApfsCloneMaterializer::new(adapter.clone());
    let copy = FullCopyMaterializer::new(adapter);
    let request = CreateRequest::new(
        absolute(&source),
        WorkspaceName::from_str("commit-failure").unwrap(),
        false,
        UnixMillis::new(1_700_000_000_100).unwrap(),
    );
    let error = service.create(request.clone(), &clone, &copy).unwrap_err();
    assert_eq!(error.diagnostic().code(), ErrorCode::Metadata);
    assert_eq!(service.doctor().unwrap().incomplete_workspaces(), 1);
    assert_eq!(
        fs::read_dir(data_root.join("workspaces")).unwrap().count(),
        1
    );
    let repeated = service.create(request, &clone, &copy).unwrap_err();
    assert_eq!(repeated.diagnostic().code(), ErrorCode::WorkspaceIncomplete);
}
