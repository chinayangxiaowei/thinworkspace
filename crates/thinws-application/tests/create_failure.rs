use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use tempfile::Builder;
use thinws_adapter_macos::{ApfsCloneMaterializer, FullCopyMaterializer, MacOsHostAdapter};
use thinws_application::{CreateRequest, InitRequest, ThinWorkspaceService};
use thinws_core::{
    AbsolutePath, CreatedObjectEvidence, DeletionTombstone, ErrorCode, InstallationRecord,
    InstanceId, MaterializationAttemptEvidence, MaterializationFailureKind, MaterializationPlan,
    MaterializationReceipt, MaterializeRequest, MaterializedEntryKind, MaterializerKind,
    RelativePath, RemovalMode, RollbackEvidence, RollbackStatus, UnixMillis, WorkspaceId,
    WorkspaceName, WorkspaceRecord, WorkspaceReservation, WorkspaceState,
};
use thinws_metadata_sqlite::SqliteMetadataStoreFactory;
use thinws_ports::{
    DataRootLayoutEvidence, FinalMaterializationSummary, MaterializationFailure, MetadataSnapshot,
    MetadataStore, MetadataStoreFactory, PortError, PortErrorKind, WorkspaceMaterializer,
};

fn absolute(path: &Path) -> AbsolutePath {
    AbsolutePath::try_from_bytes(path.as_os_str().as_bytes().to_vec()).unwrap()
}

struct FailMetadataFactory {
    fail_final_commit: bool,
    fail_record_failure: bool,
}

impl<L: DataRootLayoutEvidence> MetadataStoreFactory<L> for FailMetadataFactory {
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
        Ok(Box::new(FailMetadataStore {
            inner,
            fail_final_commit: self.fail_final_commit,
            fail_record_failure: self.fail_record_failure,
        }))
    }
}

struct FailMetadataStore {
    inner: Box<dyn MetadataStore>,
    fail_final_commit: bool,
    fail_record_failure: bool,
}

impl MetadataStore for FailMetadataStore {
    fn installation(&self) -> &InstallationRecord {
        self.inner.installation()
    }

    fn reserve_workspace(
        &mut self,
        reservation: &WorkspaceReservation,
    ) -> Result<WorkspaceRecord, PortError> {
        self.inner.reserve_workspace(reservation)
    }

    fn complete_materialization(
        &mut self,
        workspace_id: WorkspaceId,
        plan: &MaterializationPlan,
        receipt: &MaterializationReceipt,
        recorded_at: UnixMillis,
    ) -> Result<WorkspaceRecord, PortError> {
        if self.fail_final_commit {
            Err(PortError::new(
                PortErrorKind::Storage,
                "injected final SQLite commit failure",
            ))
        } else {
            self.inner
                .complete_materialization(workspace_id, plan, receipt, recorded_at)
        }
    }

    fn workspace(&self, workspace_id: WorkspaceId) -> Result<Option<WorkspaceRecord>, PortError> {
        self.inner.workspace(workspace_id)
    }

    fn deletion_tombstone(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<thinws_core::DeletionTombstone>, PortError> {
        self.inner.deletion_tombstone(workspace_id)
    }

    fn final_materialization(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Option<FinalMaterializationSummary>, PortError> {
        self.inner.final_materialization(workspace_id)
    }

    fn workspaces(&self) -> Result<Vec<WorkspaceRecord>, PortError> {
        self.inner.workspaces()
    }

    fn record_failure(
        &mut self,
        workspace_id: WorkspaceId,
        expected: WorkspaceState,
        error_code: ErrorCode,
        updated_at: UnixMillis,
    ) -> Result<WorkspaceRecord, PortError> {
        if self.fail_record_failure {
            Err(PortError::new(
                PortErrorKind::Storage,
                "injected failure-state SQLite commit failure",
            ))
        } else {
            self.inner
                .record_failure(workspace_id, expected, error_code, updated_at)
        }
    }

    fn begin_removal(
        &mut self,
        workspace_id: WorkspaceId,
        expected: WorkspaceState,
        mode: RemovalMode,
        updated_at: UnixMillis,
    ) -> Result<WorkspaceRecord, PortError> {
        self.inner
            .begin_removal(workspace_id, expected, mode, updated_at)
    }

    fn complete_deletion(
        &mut self,
        workspace_id: WorkspaceId,
        instance_id: InstanceId,
        deleted_at: UnixMillis,
    ) -> Result<DeletionTombstone, PortError> {
        self.inner
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
        FailMetadataFactory {
            fail_final_commit: true,
            fail_record_failure: false,
        },
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

struct PartialCloneFailure;

impl WorkspaceMaterializer for PartialCloneFailure {
    fn kind(&self) -> MaterializerKind {
        MaterializerKind::ApfsFileClone
    }

    fn materialize(
        &self,
        _request: &MaterializeRequest,
        plan: &MaterializationPlan,
    ) -> Result<MaterializationReceipt, MaterializationFailure> {
        Err(MaterializationFailure::new(
            PortError::new(PortErrorKind::Io, "injected clone failure"),
            MaterializationReceipt::failed_apfs_clone(
                plan,
                MaterializationFailureKind::Filesystem,
                Vec::new(),
                false,
                RollbackEvidence::new(RollbackStatus::Incomplete, Vec::new(), Vec::new()),
                MaterializationAttemptEvidence::default(),
                0,
            )
            .with_unconfirmed_staging(CreatedObjectEvidence::new(
                RelativePath::try_from_bytes(b"orphan".to_vec()).unwrap(),
                MaterializedEntryKind::RegularFile,
                None,
            )),
        ))
    }
}

#[test]
fn failure_state_commit_error_retains_partial_receipt_diagnostics() {
    let controlled =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-09-application-tests");
    fs::create_dir_all(&controlled).unwrap();
    let temp = Builder::new()
        .prefix("create-double-failure-")
        .tempdir_in(fs::canonicalize(controlled).unwrap())
        .unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("file.txt"), b"source").unwrap();
    let data_root = temp.path().join("data-root");
    let adapter = MacOsHostAdapter::new(temp.path().join("bootstrap")).unwrap();
    let service = ThinWorkspaceService::new(
        adapter.clone(),
        FailMetadataFactory {
            fail_final_commit: false,
            fail_record_failure: true,
        },
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
            CreateRequest::new(
                absolute(&source),
                WorkspaceName::from_str("double-failure").unwrap(),
                false,
                UnixMillis::new(1_700_000_000_100).unwrap(),
            ),
            &PartialCloneFailure,
            &FullCopyMaterializer::new(adapter),
        )
        .unwrap_err();
    assert_eq!(error.diagnostic().code(), ErrorCode::Metadata);
    assert_eq!(error.partial_receipts().len(), 1);
    let context = error.diagnostic().context();
    assert!(context.contains_key("workspace_id"));
    assert_eq!(context["materialization_attempt_count"].user_value(), "1");
    assert_eq!(context["rollback_incomplete"].user_value(), "true");
    assert_eq!(context["unconfirmed_staging"].user_value(), "true");
}
