use std::error::Error;
use std::fs;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::process::Command;
use std::str::FromStr;
use std::time::Duration;

use tempfile::Builder;
use thinws_adapter_macos::{ApfsCloneMaterializer, FullCopyMaterializer, MacOsHostAdapter};
use thinws_application::{CreateRequest, InitRequest, ThinWorkspaceService};
use thinws_core::{
    AbsolutePath, CowEvidence, ErrorCode, FallbackPolicy, FallbackReason,
    MaterializationAttemptEvidence, MaterializationFailureKind, MaterializationMode,
    MaterializationPlan, MaterializationReceipt, MaterializeRequest, MaterializerKind,
    RollbackEvidence, RollbackStatus, TreeDigest, UnixMillis, WorkspaceName, WorkspaceState,
};
use thinws_metadata_sqlite::SqliteMetadataStoreFactory;
use thinws_ports::{
    MaterializationFailure, MaterializationPathProbeRequest, PlatformProbe, PortError,
    PortErrorKind, WorkspaceMaterializer,
};

fn absolute(path: &std::path::Path) -> AbsolutePath {
    AbsolutePath::try_from_bytes(path.as_os_str().as_bytes().to_vec()).unwrap()
}

struct WrongCloneKind;

impl WorkspaceMaterializer for WrongCloneKind {
    fn kind(&self) -> MaterializerKind {
        MaterializerKind::FullCopy
    }

    fn materialize(
        &self,
        _request: &MaterializeRequest,
        _plan: &MaterializationPlan,
    ) -> Result<MaterializationReceipt, MaterializationFailure> {
        panic!("wrong backend kind must be rejected before execution")
    }
}

struct NonCowFailure;

impl WorkspaceMaterializer for NonCowFailure {
    fn kind(&self) -> MaterializerKind {
        MaterializerKind::ApfsFileClone
    }

    fn materialize(
        &self,
        _request: &MaterializeRequest,
        plan: &MaterializationPlan,
    ) -> Result<MaterializationReceipt, MaterializationFailure> {
        Err(MaterializationFailure::new(
            PortError::new(PortErrorKind::Io, "injected non-CoW clone failure"),
            MaterializationReceipt::failed_apfs_clone(
                plan,
                MaterializationFailureKind::Filesystem,
                Vec::new(),
                false,
                RollbackEvidence::new(RollbackStatus::NotNeeded, Vec::new(), Vec::new()),
                MaterializationAttemptEvidence::default(),
                0,
            ),
        ))
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
            MaterializationReceipt::failed_apfs_clone(
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

#[test]
fn p1_09_create_mirrors_a_plain_source_and_reuses_ready_workspace() {
    let controlled =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-09-application-tests");
    fs::create_dir_all(&controlled).unwrap();
    let temp = Builder::new()
        .prefix("create-")
        .tempdir_in(fs::canonicalize(controlled).unwrap())
        .unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("note.txt"), b"plain source without Git").unwrap();
    let data_root = temp.path().join("data-root");
    let adapter = MacOsHostAdapter::new(temp.path().join("bootstrap")).unwrap();
    let service = ThinWorkspaceService::new(
        adapter.clone(),
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
    let clone = ApfsCloneMaterializer::new(adapter.clone());
    let copy = FullCopyMaterializer::new(adapter);
    let request = CreateRequest::new(
        absolute(&source),
        WorkspaceName::from_str("plain-workspace").unwrap(),
        false,
        UnixMillis::new(1_700_000_000_100).unwrap(),
    );
    let created = service.create(request.clone(), &clone, &copy).unwrap();
    assert!(created.created());
    assert_eq!(created.record().state(), WorkspaceState::Ready);
    assert_eq!(
        created.materialization().actual_mode(),
        MaterializationMode::CowClone
    );
    let target = PathBuf::from(std::ffi::OsString::from_vec(
        created
            .record()
            .reservation()
            .target_path()
            .as_bytes()
            .to_vec(),
    ));
    assert_eq!(
        fs::read(target.join("note.txt")).unwrap(),
        b"plain source without Git"
    );
    assert!(!target.parent().unwrap().join(".state/incomplete").exists());

    let authorized = service
        .create(
            CreateRequest::new(
                absolute(&source),
                WorkspaceName::from_str("copy-authorized").unwrap(),
                true,
                UnixMillis::new(1_700_000_000_110).unwrap(),
            ),
            &clone,
            &copy,
        )
        .unwrap();
    assert_eq!(
        authorized.materialization().actual_mode(),
        MaterializationMode::CowClone,
        "--allow-copy must not force Full Copy when CoW works"
    );

    fs::rename(&source, temp.path().join("source-moved-away")).unwrap();
    let repeated = service.create(request, &clone, &copy).unwrap();
    assert!(!repeated.created());
    assert_eq!(
        repeated.record().reservation().workspace_id(),
        created.record().reservation().workspace_id()
    );
    assert_eq!(repeated.materialization(), created.materialization());
    assert_eq!(
        fs::read(target.join("note.txt")).unwrap(),
        b"plain source without Git"
    );

    let different_source = CreateRequest::new(
        absolute(&temp.path().join("source-moved-away")),
        WorkspaceName::from_str("plain-workspace").unwrap(),
        false,
        UnixMillis::new(1_700_000_000_200).unwrap(),
    );
    let conflict = service.create(different_source, &clone, &copy).unwrap_err();
    assert_eq!(conflict.diagnostic().code(), ErrorCode::NameConflict);
}

#[test]
fn p1_09_rejects_wrong_backend_kind_before_any_installation_write() {
    let adapter =
        MacOsHostAdapter::new(PathBuf::from("/Volumes/data/no-real-bootstrap-needed")).unwrap();
    let service = ThinWorkspaceService::new(
        adapter.clone(),
        SqliteMetadataStoreFactory,
        Duration::from_secs(1),
        Duration::from_secs(1),
    );
    let request = CreateRequest::new(
        AbsolutePath::try_from_bytes(b"/Volumes/data/source".to_vec()).unwrap(),
        WorkspaceName::from_str("wrong-backend").unwrap(),
        false,
        UnixMillis::new(1).unwrap(),
    );
    let error = service
        .create(
            request,
            &WrongCloneKind,
            &FullCopyMaterializer::new(adapter),
        )
        .unwrap_err();
    assert_eq!(error.diagnostic().code(), ErrorCode::CapabilityUnavailable);
}

#[test]
fn p1_09_non_cow_failure_does_not_become_a_copy_fallback() {
    let controlled =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-09-application-tests");
    fs::create_dir_all(&controlled).unwrap();
    let temp = Builder::new()
        .prefix("create-no-fallback-")
        .tempdir_in(fs::canonicalize(controlled).unwrap())
        .unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("file.txt"), b"source").unwrap();
    let adapter = MacOsHostAdapter::new(temp.path().join("bootstrap")).unwrap();
    let service = ThinWorkspaceService::new(
        adapter.clone(),
        SqliteMetadataStoreFactory,
        Duration::from_secs(1),
        Duration::from_secs(1),
    );
    service
        .init(InitRequest::new(
            absolute(&temp.path().join("data-root")),
            UnixMillis::new(1_700_000_000_000).unwrap(),
        ))
        .unwrap();
    let error = service
        .create(
            CreateRequest::new(
                absolute(&source),
                WorkspaceName::from_str("non-cow-failure").unwrap(),
                true,
                UnixMillis::new(1_700_000_000_100).unwrap(),
            ),
            &NonCowFailure,
            &FullCopyMaterializer::new(adapter),
        )
        .unwrap_err();
    assert_eq!(error.diagnostic().code(), ErrorCode::Filesystem);
    assert!(
        error.source().is_some(),
        "original clone error must survive"
    );
    assert_eq!(service.doctor().unwrap().incomplete_workspaces(), 1);
}

#[test]
fn p1_09_runtime_copy_requires_explicit_policy_and_a_clean_cow_failure() {
    let controlled =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-09-application-tests");
    fs::create_dir_all(&controlled).unwrap();
    let temp = Builder::new()
        .prefix("create-runtime-copy-")
        .tempdir_in(fs::canonicalize(controlled).unwrap())
        .unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let bootstrap = temp.path().join("bootstrap");
    let adapter = MacOsHostAdapter::new(bootstrap).unwrap();
    let probe_target = temp.path().join("probe-target");
    let probe_staging = temp.path().join("probe-staging");
    let probe_trash = temp.path().join("probe-trash");
    for directory in [&probe_target, &probe_staging, &probe_trash] {
        fs::create_dir(directory).unwrap();
    }
    let materialize = MaterializeRequest::new(
        absolute(&source),
        absolute(&probe_target),
        absolute(&probe_staging),
        absolute(&probe_trash),
    );
    let report = adapter
        .inspect_materialization_paths(&MaterializationPathProbeRequest::from(&materialize))
        .unwrap();
    let plan = MaterializationPlan::for_apfs_clone(&report, FallbackPolicy::Deny).unwrap();
    let source_digest = ApfsCloneMaterializer::new(adapter.clone())
        .materialize(&materialize, &plan)
        .unwrap()
        .source_manifest_digest()
        .unwrap();

    let service = ThinWorkspaceService::new(
        adapter.clone(),
        SqliteMetadataStoreFactory,
        Duration::from_secs(1),
        Duration::from_secs(1),
    );
    service
        .init(InitRequest::new(
            absolute(&temp.path().join("data-root")),
            UnixMillis::new(1_700_000_000_000).unwrap(),
        ))
        .unwrap();
    let failure = CleanCowUnavailable { source_digest };
    let copy = FullCopyMaterializer::new(adapter);
    let denied = service
        .create(
            CreateRequest::new(
                absolute(&source),
                WorkspaceName::from_str("copy-denied").unwrap(),
                false,
                UnixMillis::new(1_700_000_000_100).unwrap(),
            ),
            &failure,
            &copy,
        )
        .unwrap_err();
    assert_eq!(denied.diagnostic().code(), ErrorCode::CowUnavailable);

    let allowed = service
        .create(
            CreateRequest::new(
                absolute(&source),
                WorkspaceName::from_str("copy-allowed").unwrap(),
                true,
                UnixMillis::new(1_700_000_000_200).unwrap(),
            ),
            &failure,
            &copy,
        )
        .unwrap();
    assert_eq!(allowed.record().state(), WorkspaceState::Ready);
    assert_eq!(
        allowed.materialization().actual_mode(),
        MaterializationMode::FullCopy
    );
    assert_eq!(allowed.materialization().cow(), CowEvidence::NotUsed);
    assert_eq!(
        allowed.materialization().fallback_reason(),
        Some(FallbackReason::CloneUnavailableAtRuntime)
    );
    assert_eq!(allowed.materialization().failed_attempt_count(), 1);
}

#[test]
fn p1_09_unsupported_source_remains_incomplete_and_is_not_retried() {
    let controlled =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-09-application-tests");
    fs::create_dir_all(&controlled).unwrap();
    let temp = Builder::new()
        .prefix("create-failure-")
        .tempdir_in(fs::canonicalize(controlled).unwrap())
        .unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let fifo = source.join("unsupported.fifo");
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let adapter = MacOsHostAdapter::new(temp.path().join("bootstrap")).unwrap();
    let service = ThinWorkspaceService::new(
        adapter.clone(),
        SqliteMetadataStoreFactory,
        Duration::from_secs(1),
        Duration::from_secs(1),
    );
    service
        .init(InitRequest::new(
            absolute(&temp.path().join("data-root")),
            UnixMillis::new(1_700_000_000_000).unwrap(),
        ))
        .unwrap();
    let clone = ApfsCloneMaterializer::new(adapter.clone());
    let copy = FullCopyMaterializer::new(adapter);
    let request = CreateRequest::new(
        absolute(&source),
        WorkspaceName::from_str("unsupported-entry").unwrap(),
        false,
        UnixMillis::new(1_700_000_000_100).unwrap(),
    );
    let first = service.create(request.clone(), &clone, &copy).unwrap_err();
    assert_eq!(first.diagnostic().code(), ErrorCode::DataRootLayout);
    assert_eq!(service.doctor().unwrap().incomplete_workspaces(), 1);
    fs::remove_file(fifo).unwrap();
    let second = service.create(request, &clone, &copy).unwrap_err();
    assert_eq!(second.diagnostic().code(), ErrorCode::WorkspaceIncomplete);
}

#[test]
fn p1_09_cross_volume_and_containment_fail_before_reserving_a_workspace() {
    let controlled =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-09-application-tests");
    fs::create_dir_all(&controlled).unwrap();
    let temp = Builder::new()
        .prefix("create-layout-")
        .tempdir_in(fs::canonicalize(controlled).unwrap())
        .unwrap();
    let system_source = tempfile::tempdir().unwrap();
    let adapter = MacOsHostAdapter::new(temp.path().join("bootstrap")).unwrap();
    let service = ThinWorkspaceService::new(
        adapter.clone(),
        SqliteMetadataStoreFactory,
        Duration::from_secs(1),
        Duration::from_secs(1),
    );
    let data_root = temp.path().join("data-root");
    service
        .init(InitRequest::new(
            absolute(&data_root),
            UnixMillis::new(1_700_000_000_000).unwrap(),
        ))
        .unwrap();
    let clone = ApfsCloneMaterializer::new(adapter.clone());
    let copy = FullCopyMaterializer::new(adapter);
    let cross_volume = CreateRequest::new(
        absolute(system_source.path()),
        WorkspaceName::from_str("cross-volume").unwrap(),
        true,
        UnixMillis::new(1_700_000_000_100).unwrap(),
    );
    let error = service.create(cross_volume, &clone, &copy).unwrap_err();
    assert_eq!(error.diagnostic().code(), ErrorCode::DataRootLayout);

    let contained = CreateRequest::new(
        absolute(&data_root.join("workspaces")),
        WorkspaceName::from_str("contained").unwrap(),
        false,
        UnixMillis::new(1_700_000_000_100).unwrap(),
    );
    let error = service.create(contained, &clone, &copy).unwrap_err();
    assert_eq!(error.diagnostic().code(), ErrorCode::DataRootLayout);
    assert_eq!(service.doctor().unwrap().incomplete_workspaces(), 0);
}
