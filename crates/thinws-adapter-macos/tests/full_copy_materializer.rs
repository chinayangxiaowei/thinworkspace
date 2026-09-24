use std::ffi::OsStr;
use std::fs::{self, File, FileTimes};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use tempfile::{Builder, TempDir};
use thinws_adapter_macos::{FullCopyMaterializer, MacOsHostAdapter};
use thinws_core::{
    AbsolutePath, CandidateEvidence, CowEvidence, FallbackPolicy, FallbackReason,
    MaterializationMode, MaterializationOutcome, MaterializationPathReport, MaterializationPlan,
    MaterializeRequest, MaterializerKind, RollbackStatus, SupportState,
};
use thinws_ports::{
    MaterializationPathProbeRequest, PlatformProbe, PortErrorKind, WorkspaceMaterializer,
};

fn absolute(path: &Path) -> AbsolutePath {
    AbsolutePath::try_from_bytes(path.as_os_str().as_bytes().to_vec()).unwrap()
}

fn fixture(prefix: &str) -> (TempDir, MacOsHostAdapter) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-07-copy-tests");
    fs::create_dir_all(&root).unwrap();
    let root = fs::canonicalize(root).unwrap();
    let temp = Builder::new().prefix(prefix).tempdir_in(&root).unwrap();
    let host = MacOsHostAdapter::new(root.join("bootstrap")).unwrap();
    (temp, host)
}

fn request_and_full_copy_plan(
    host: &MacOsHostAdapter,
    source: &Path,
    target: &Path,
    staging: &Path,
    trash: &Path,
) -> (MaterializeRequest, MaterializationPlan) {
    let request = MaterializeRequest::new(
        absolute(source),
        absolute(target),
        absolute(staging),
        absolute(trash),
    );
    let report = host
        .inspect_materialization_paths(&MaterializationPathProbeRequest::from(&request))
        .unwrap();
    // The host supports clone. Retain the real path evidence/digest while
    // selecting a synthetic preflight fallback to test the Full Copy backend.
    let copy_report = MaterializationPathReport::new(
        report.source().clone(),
        report.target_root().clone(),
        report.staging().clone(),
        report.trash().clone(),
        CandidateEvidence::new(
            MaterializerKind::ApfsFileClone,
            SupportState::Unsupported,
            vec!["clone_capability_unsupported".to_owned()],
        ),
        report.full_copy().clone(),
        report.evidence_digest(),
    );
    let copy_plan = MaterializationPlan::for_full_copy_after_preflight(
        &copy_report,
        FallbackPolicy::AllowFullCopyOnCowUnsupported,
    )
    .unwrap();
    (request, copy_plan)
}

fn four_roots(temp: &TempDir) -> [PathBuf; 4] {
    let roots = ["source", "target", "staging", "trash"].map(|name| temp.path().join(name));
    for root in &roots {
        fs::create_dir(root).unwrap();
    }
    roots
}

fn set_fixed_mtime(path: &Path, seconds: u64, nanoseconds: u32) {
    let modified = UNIX_EPOCH
        .checked_add(Duration::new(seconds, nanoseconds))
        .unwrap();
    File::open(path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(modified))
        .unwrap();
}

#[test]
fn real_full_copy_preserves_tree_and_isolates_each_regular_file() {
    let (temp, host) = fixture("success-");
    let [source, target, staging, trash] = four_roots(&temp);
    let outside = temp.path().join("outside");
    fs::write(&outside, b"outside unchanged").unwrap();
    fs::create_dir(source.join(".git")).unwrap();
    fs::create_dir(source.join("nested")).unwrap();
    fs::write(source.join(".git/HEAD"), b"ref: refs/heads/main\n").unwrap();
    fs::write(source.join(".gitignore"), b"*.ignored\n").unwrap();
    fs::write(source.join("cache.ignored"), b"ignored bytes").unwrap();
    fs::write(source.join("untracked.txt"), b"untracked bytes").unwrap();
    fs::write(source.join("nested/build.cache"), b"cached bytes").unwrap();
    fs::write(source.join("plain.txt"), b"original").unwrap();
    fs::hard_link(source.join("plain.txt"), source.join("alias.txt")).unwrap();
    fs::write(source.join("name with spaces-雪.txt"), b"raw name").unwrap();
    symlink(
        OsStr::from_bytes(b"../../outside"),
        source.join("nested/link"),
    )
    .unwrap();
    fs::set_permissions(source.join("plain.txt"), fs::Permissions::from_mode(0o640)).unwrap();
    fs::set_permissions(source.join("nested"), fs::Permissions::from_mode(0o550)).unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o555)).unwrap();
    set_fixed_mtime(&source.join("plain.txt"), 1_700_000_001, 123_456_789);
    set_fixed_mtime(&source.join("nested"), 1_700_000_002, 234_567_890);
    set_fixed_mtime(&source, 1_700_000_003, 345_678_901);

    let (request, plan) = request_and_full_copy_plan(&host, &source, &target, &staging, &trash);
    let receipt = FullCopyMaterializer::new(host)
        .materialize(&request, &plan)
        .unwrap();

    assert_eq!(receipt.outcome(), MaterializationOutcome::Succeeded);
    assert_eq!(receipt.requested_mode(), MaterializationMode::CowClone);
    assert_eq!(receipt.effective_mode(), MaterializationMode::FullCopy);
    assert_eq!(receipt.actual_mode(), MaterializationMode::FullCopy);
    assert_eq!(receipt.actual_adapter(), MaterializerKind::FullCopy);
    assert_eq!(receipt.cow_evidence(), CowEvidence::NotUsed);
    assert_eq!(receipt.clone_calls_succeeded(), 0);
    assert_eq!(receipt.regular_file_count(), Some(8));
    assert_eq!(
        receipt.source_manifest_digest(),
        receipt.target_manifest_digest()
    );
    assert_eq!(
        receipt.fallback_reason(),
        Some(FallbackReason::CloneUnsupportedAtPreflight)
    );
    assert!(receipt.failed_attempts().is_empty());
    assert_eq!(
        fs::read(target.join(".git/HEAD")).unwrap(),
        b"ref: refs/heads/main\n"
    );
    assert_eq!(
        fs::read(target.join("cache.ignored")).unwrap(),
        b"ignored bytes"
    );
    assert_eq!(
        fs::read(target.join("untracked.txt")).unwrap(),
        b"untracked bytes"
    );
    assert_eq!(
        fs::read(target.join("name with spaces-雪.txt")).unwrap(),
        b"raw name"
    );
    assert_eq!(
        fs::read_link(target.join("nested/link"))
            .unwrap()
            .as_os_str()
            .as_bytes(),
        b"../../outside"
    );
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o7777,
        0o555
    );
    assert_eq!(
        fs::metadata(target.join("nested"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o550
    );
    assert_eq!(
        fs::metadata(target.join("plain.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o640
    );
    for relative in ["", "nested", "plain.txt"] {
        let source_metadata = fs::metadata(source.join(relative)).unwrap();
        let target_metadata = fs::metadata(target.join(relative)).unwrap();
        assert_eq!(target_metadata.mtime(), source_metadata.mtime());
        assert_eq!(target_metadata.mtime_nsec(), source_metadata.mtime_nsec());
    }
    let source_inode = fs::metadata(source.join("plain.txt")).unwrap().ino();
    let target_inode = fs::metadata(target.join("plain.txt")).unwrap().ino();
    let alias_inode = fs::metadata(target.join("alias.txt")).unwrap().ino();
    assert_ne!(source_inode, target_inode);
    assert_ne!(source_inode, alias_inode);
    assert_ne!(target_inode, alias_inode);

    fs::write(target.join("plain.txt"), b"target changed").unwrap();
    assert_eq!(fs::read(source.join("plain.txt")).unwrap(), b"original");
    assert_eq!(fs::read(target.join("alias.txt")).unwrap(), b"original");
    fs::write(source.join("plain.txt"), b"source changed").unwrap();
    assert_eq!(
        fs::read(target.join("plain.txt")).unwrap(),
        b"target changed"
    );

    fs::set_permissions(&source, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(source.join("nested"), fs::Permissions::from_mode(0o700)).unwrap();
    fs::remove_dir_all(&source).unwrap();
    assert_eq!(
        fs::read(target.join("plain.txt")).unwrap(),
        b"target changed"
    );
    assert_eq!(fs::read(&outside).unwrap(), b"outside unchanged");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(target.join("nested"), fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn full_copy_rejects_a_nonempty_target_without_touching_caller_data() {
    let (temp, host) = fixture("nonempty-");
    let [source, target, staging, trash] = four_roots(&temp);
    fs::write(source.join("source.txt"), b"source").unwrap();
    fs::write(target.join("caller.txt"), b"keep").unwrap();
    let (request, plan) = request_and_full_copy_plan(&host, &source, &target, &staging, &trash);

    let failure = FullCopyMaterializer::new(host)
        .materialize(&request, &plan)
        .unwrap_err();
    assert_eq!(failure.error().kind(), PortErrorKind::NotEmpty);
    assert_eq!(failure.receipt().outcome(), MaterializationOutcome::Failed);
    assert_eq!(failure.receipt().cow_evidence(), CowEvidence::NotUsed);
    assert_eq!(
        failure.receipt().rollback().status(),
        RollbackStatus::NotNeeded
    );
    assert_eq!(fs::read(target.join("caller.txt")).unwrap(), b"keep");
    assert!(!target.join("source.txt").exists());
}
