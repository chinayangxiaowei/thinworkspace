use std::ffi::OsStr;
use std::fs::{self, File, FileTimes};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use tempfile::{Builder, TempDir};
use thinws_adapter_macos::{ApfsCloneMaterializer, MacOsHostAdapter};
use thinws_core::{
    AbsolutePath, CowEvidence, FallbackPolicy, MaterializationFailureKind, MaterializationOutcome,
    MaterializationPlan, MaterializeRequest, RollbackStatus,
};
use thinws_ports::{
    MaterializationPathProbeRequest, PlatformProbe, PortErrorKind, WorkspaceMaterializer,
};

fn absolute(path: &Path) -> AbsolutePath {
    AbsolutePath::try_from_bytes(path.as_os_str().as_bytes().to_vec()).unwrap()
}

fn controlled_root(prefix: &str) -> TempDir {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-06-materialize-tests");
    fs::create_dir_all(&root).unwrap();
    Builder::new()
        .prefix(prefix)
        .tempdir_in(fs::canonicalize(root).unwrap())
        .unwrap()
}

fn adapter() -> MacOsHostAdapter {
    let bootstrap =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-06-materialize-bootstrap");
    MacOsHostAdapter::new(
        fs::canonicalize(bootstrap.parent().unwrap())
            .unwrap()
            .join("p1-06-materialize-bootstrap"),
    )
    .unwrap()
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

fn request_and_plan(
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
    let plan = MaterializationPlan::for_apfs_clone(&report, FallbackPolicy::Deny).unwrap();
    (request, plan)
}

#[test]
fn real_apfs_clone_materializes_the_whole_tree_and_isolates_later_writes() {
    let temp = controlled_root("success-");
    let source = temp.path().join("source");
    let target = temp.path().join("target");
    let staging = temp.path().join("staging");
    let trash = temp.path().join("trash");
    for directory in [&source, &target, &staging, &trash] {
        fs::create_dir(directory).unwrap();
    }
    let outside_sentinel = temp.path().join("outside-sentinel");
    fs::write(&outside_sentinel, b"outside stays unchanged").unwrap();
    fs::create_dir(source.join(".git")).unwrap();
    fs::create_dir(source.join("nested")).unwrap();
    fs::write(source.join(".git/HEAD"), b"ref: refs/heads/main\n").unwrap();
    fs::write(source.join(".gitignore"), b"*.ignored\n").unwrap();
    fs::write(source.join("cache.ignored"), b"ignored bytes").unwrap();
    fs::write(source.join("untracked.txt"), b"untracked bytes").unwrap();
    fs::write(source.join("nested/build.cache"), b"cached bytes").unwrap();
    fs::write(source.join("plain.txt"), b"original").unwrap();
    fs::hard_link(source.join("plain.txt"), source.join("hardlink-alias.txt")).unwrap();
    fs::write(source.join("name with spaces-雪.txt"), b"raw name").unwrap();
    fs::set_permissions(source.join("plain.txt"), fs::Permissions::from_mode(0o640)).unwrap();
    symlink(
        OsStr::from_bytes(b"../../outside-sentinel"),
        source.join("nested/link"),
    )
    .unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o555)).unwrap();
    fs::set_permissions(source.join("nested"), fs::Permissions::from_mode(0o550)).unwrap();
    set_fixed_mtime(&source.join("plain.txt"), 1_700_000_001, 123_456_789);
    set_fixed_mtime(&source.join("nested"), 1_700_000_002, 234_567_890);
    set_fixed_mtime(&source, 1_700_000_003, 345_678_901);

    let host = adapter();
    let (request, plan) = request_and_plan(&host, &source, &target, &staging, &trash);
    let receipt = ApfsCloneMaterializer::new(host)
        .materialize(&request, &plan)
        .unwrap();

    assert_eq!(receipt.outcome(), MaterializationOutcome::Succeeded);
    assert_eq!(receipt.cow_evidence(), CowEvidence::Confirmed);
    assert_eq!(receipt.regular_file_count(), Some(8));
    assert_eq!(receipt.clone_calls_succeeded(), 8);
    assert_eq!(
        receipt.source_manifest_digest(),
        receipt.target_manifest_digest()
    );
    assert_eq!(
        fs::read(target.join(".git/HEAD")).unwrap(),
        b"ref: refs/heads/main\n"
    );
    assert_eq!(
        fs::read_link(target.join("nested/link"))
            .unwrap()
            .as_os_str()
            .as_bytes(),
        b"../../outside-sentinel"
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
    assert_ne!(
        fs::metadata(target.join("plain.txt")).unwrap().ino(),
        fs::metadata(target.join("hardlink-alias.txt"))
            .unwrap()
            .ino()
    );
    assert_eq!(
        fs::read(target.join("name with spaces-雪.txt")).unwrap(),
        b"raw name"
    );
    assert_eq!(fs::read(target.join(".gitignore")).unwrap(), b"*.ignored\n");
    assert_eq!(
        fs::read(target.join("cache.ignored")).unwrap(),
        b"ignored bytes"
    );
    assert_eq!(
        fs::read(target.join("untracked.txt")).unwrap(),
        b"untracked bytes"
    );
    assert_eq!(
        fs::read(&outside_sentinel).unwrap(),
        b"outside stays unchanged"
    );

    fs::write(target.join("plain.txt"), b"target changed").unwrap();
    assert_eq!(fs::read(source.join("plain.txt")).unwrap(), b"original");
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
    assert_eq!(
        fs::read(target.join(".git/HEAD")).unwrap(),
        b"ref: refs/heads/main\n"
    );
    assert_eq!(
        fs::read(&outside_sentinel).unwrap(),
        b"outside stays unchanged"
    );

    fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(target.join("nested"), fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn empty_tree_succeeds_without_claiming_that_cow_was_used() {
    let temp = controlled_root("empty-");
    let source = temp.path().join("source");
    let target = temp.path().join("target");
    let staging = temp.path().join("staging");
    let trash = temp.path().join("trash");
    for directory in [&source, &target, &staging, &trash] {
        fs::create_dir(directory).unwrap();
    }

    let host = adapter();
    let (request, plan) = request_and_plan(&host, &source, &target, &staging, &trash);
    let receipt = ApfsCloneMaterializer::new(host)
        .materialize(&request, &plan)
        .unwrap();

    assert_eq!(receipt.outcome(), MaterializationOutcome::Succeeded);
    assert_eq!(receipt.cow_evidence(), CowEvidence::NotUsed);
    assert_eq!(receipt.regular_file_count(), Some(0));
    assert_eq!(receipt.clone_calls_succeeded(), 0);
}

#[test]
fn directory_and_symlink_only_tree_succeeds_with_cow_not_used() {
    let temp = controlled_root("directory-link-");
    let source = temp.path().join("source");
    let target = temp.path().join("target");
    let staging = temp.path().join("staging");
    let trash = temp.path().join("trash");
    for directory in [&source, &target, &staging, &trash] {
        fs::create_dir(directory).unwrap();
    }
    fs::create_dir(source.join("directory")).unwrap();
    symlink(
        OsStr::from_bytes(b"outside-text"),
        source.join("directory/link"),
    )
    .unwrap();

    let host = adapter();
    let (request, plan) = request_and_plan(&host, &source, &target, &staging, &trash);
    let receipt = ApfsCloneMaterializer::new(host)
        .materialize(&request, &plan)
        .unwrap();

    assert_eq!(receipt.cow_evidence(), CowEvidence::NotUsed);
    assert_eq!(receipt.regular_file_count(), Some(0));
    assert_eq!(
        fs::read_link(target.join("directory/link"))
            .unwrap()
            .as_os_str()
            .as_bytes(),
        b"outside-text"
    );
}

#[test]
fn replacing_each_planned_root_is_detected_before_any_write() {
    for replaced_role in ["source", "target", "staging", "trash"] {
        let temp = controlled_root("stale-plan-");
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        let staging = temp.path().join("staging");
        let trash = temp.path().join("trash");
        for directory in [&source, &target, &staging, &trash] {
            fs::create_dir(directory).unwrap();
        }
        fs::write(source.join("source.txt"), b"source").unwrap();

        let host = adapter();
        let (request, plan) = request_and_plan(&host, &source, &target, &staging, &trash);
        let replaced = temp.path().join(replaced_role);
        let displaced = temp.path().join(format!("displaced-{replaced_role}"));
        fs::rename(&replaced, &displaced).unwrap();
        fs::create_dir(&replaced).unwrap();

        let failure = ApfsCloneMaterializer::new(host)
            .materialize(&request, &plan)
            .unwrap_err();

        assert_eq!(
            failure.receipt().failure_kind(),
            Some(MaterializationFailureKind::PlanStale),
            "role {replaced_role}"
        );
        assert!(failure.receipt().created().is_empty());
        assert!(fs::read_dir(&target).unwrap().next().is_none());
    }
}

#[test]
fn nonempty_target_is_rejected_without_modifying_its_contents() {
    let temp = controlled_root("nonempty-");
    let source = temp.path().join("source");
    let target = temp.path().join("target");
    let staging = temp.path().join("staging");
    let trash = temp.path().join("trash");
    for directory in [&source, &target, &staging, &trash] {
        fs::create_dir(directory).unwrap();
    }
    fs::write(source.join("source.txt"), b"source").unwrap();
    fs::write(target.join("owned-by-caller.txt"), b"keep").unwrap();

    let host = adapter();
    let (request, plan) = request_and_plan(&host, &source, &target, &staging, &trash);
    let failure = ApfsCloneMaterializer::new(host)
        .materialize(&request, &plan)
        .unwrap_err();

    assert_eq!(failure.error().kind(), PortErrorKind::NotEmpty);
    assert_eq!(
        failure.receipt().failure_kind(),
        Some(MaterializationFailureKind::InvalidLayout)
    );
    assert!(failure.receipt().created().is_empty());
    assert_eq!(
        failure.receipt().rollback().status(),
        RollbackStatus::NotNeeded
    );
    assert_eq!(
        fs::read(target.join("owned-by-caller.txt")).unwrap(),
        b"keep"
    );
    assert!(!target.join("source.txt").exists());
}

#[test]
fn unsupported_source_entry_is_rejected_before_the_first_target_write() {
    // Unix-domain socket paths are bounded by `sockaddr_un::sun_path`. Keep
    // this fixture independent of the checkout path so it also runs from the
    // longer temporary worktrees created by cargo-mutants.
    let temp = Builder::new()
        .prefix("tw-p106-special-")
        .permissions(fs::Permissions::from_mode(0o700))
        .tempdir_in("/private/tmp")
        .unwrap();
    assert_eq!(
        temp.path().metadata().unwrap().permissions().mode() & 0o777,
        0o700
    );
    let source = temp.path().join("source");
    let target = temp.path().join("target");
    let staging = temp.path().join("staging");
    let trash = temp.path().join("trash");
    for directory in [&source, &target, &staging, &trash] {
        fs::create_dir(directory).unwrap();
    }
    fs::write(source.join("a-ordinary.txt"), b"would otherwise clone").unwrap();
    let _socket = UnixListener::bind(source.join("z-special.sock")).unwrap();

    let host = adapter();
    let (request, plan) = request_and_plan(&host, &source, &target, &staging, &trash);
    let failure = ApfsCloneMaterializer::new(host)
        .materialize(&request, &plan)
        .unwrap_err();

    assert_eq!(
        failure.receipt().failure_kind(),
        Some(MaterializationFailureKind::UnsupportedSourceEntry)
    );
    assert!(failure.receipt().created().is_empty());
    assert!(fs::read_dir(&target).unwrap().next().is_none());
}
