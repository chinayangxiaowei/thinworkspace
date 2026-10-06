#![cfg(target_os = "linux")]

use std::env;
use std::ffi::OsString;
use std::fs::{self, File, FileTimes};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, UNIX_EPOCH};

use thinws_adapter_linux::{
    BtrfsReflinkMaterializer, LinuxFullCopyMaterializer, LinuxPlatformProbe,
};
use thinws_core::{
    AbsolutePath, CowEvidence, FallbackPolicy, FallbackReason, MaterializationAttemptEvidence,
    MaterializationFailureKind, MaterializationOutcome, MaterializationPlan,
    MaterializationReceipt, MaterializeRequest, RollbackEvidence, RollbackStatus,
};
use thinws_ports::{MaterializationPathProbeRequest, PlatformProbe, WorkspaceMaterializer};

fn absolute(path: &Path) -> AbsolutePath {
    AbsolutePath::try_from_bytes(path.as_os_str().as_bytes().to_vec()).unwrap()
}

fn fixture(prefix: &str) -> (tempfile::TempDir, [PathBuf; 4]) {
    let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
        .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs directory");
    let fixture = tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(root)
        .unwrap();
    let paths = ["source", "target", "staging", "trash"].map(|name| fixture.path().join(name));
    for path in &paths {
        fs::create_dir(path).unwrap();
    }
    (fixture, paths)
}

fn request_and_plan(paths: &[PathBuf; 4]) -> (MaterializeRequest, MaterializationPlan) {
    let request = MaterializeRequest::new(
        absolute(&paths[0]),
        absolute(&paths[1]),
        absolute(&paths[2]),
        absolute(&paths[3]),
    );
    let report = LinuxPlatformProbe
        .inspect_materialization_paths(&MaterializationPathProbeRequest::from(&request))
        .unwrap();
    let plan = MaterializationPlan::for_cow_clone(&report, FallbackPolicy::Deny).unwrap();
    (request, plan)
}

fn mtime(path: &Path, seconds: u64, nanoseconds: u32) {
    let modified = UNIX_EPOCH + Duration::new(seconds, nanoseconds);
    File::open(path)
        .unwrap()
        .set_times(FileTimes::new().set_modified(modified))
        .unwrap();
}

#[test]
fn real_btrfs_reflink_mirrors_every_entry_and_preserves_promised_metadata() {
    let (_fixture, paths) = fixture("thinws-btrfs-materialize-");
    let [source, target, staging, _trash] = &paths;
    fs::create_dir(source.join(".git")).unwrap();
    fs::write(source.join(".git/HEAD"), b"ref: refs/heads/main\n").unwrap();
    fs::create_dir(source.join("nested")).unwrap();
    fs::write(source.join("nested/cache"), b"cached bytes").unwrap();
    fs::write(source.join("ordinary"), b"source bytes").unwrap();
    fs::hard_link(source.join("ordinary"), source.join("hard-link-alias")).unwrap();
    fs::write(
        source.join(OsString::from_vec(b"raw-\xff".to_vec())),
        b"non UTF-8",
    )
    .unwrap();
    symlink("../../outside", source.join("nested/link")).unwrap();
    fs::set_permissions(source.join("ordinary"), fs::Permissions::from_mode(0o640)).unwrap();
    fs::set_permissions(source.join("nested"), fs::Permissions::from_mode(0o550)).unwrap();
    mtime(&source.join("ordinary"), 1_700_000_001, 123_456_789);
    mtime(&source.join("nested"), 1_700_000_002, 234_567_890);
    mtime(source, 1_700_000_003, 345_678_901);
    let (request, plan) = request_and_plan(&paths);
    let receipt = BtrfsReflinkMaterializer::new()
        .materialize(&request, &plan)
        .unwrap();
    assert_eq!(receipt.outcome(), MaterializationOutcome::Succeeded);
    assert_eq!(receipt.cow_evidence(), CowEvidence::Confirmed);
    assert_eq!(receipt.regular_file_count(), Some(5));
    assert_eq!(receipt.clone_calls_succeeded(), 5);
    assert_eq!(
        receipt.source_manifest_digest(),
        receipt.target_manifest_digest()
    );
    assert!(fs::read_dir(staging).unwrap().next().is_none());
    assert_eq!(
        fs::read(target.join(".git/HEAD")).unwrap(),
        b"ref: refs/heads/main\n"
    );
    assert_eq!(fs::read(target.join("ordinary")).unwrap(), b"source bytes");
    assert_eq!(
        fs::read_link(target.join("nested/link"))
            .unwrap()
            .as_os_str()
            .as_bytes(),
        b"../../outside"
    );
    assert_ne!(
        fs::metadata(target.join("ordinary")).unwrap().ino(),
        fs::metadata(target.join("hard-link-alias")).unwrap().ino()
    );
    for (original, clone) in [
        (source.to_path_buf(), target.to_path_buf()),
        (source.join("nested"), target.join("nested")),
        (source.join("ordinary"), target.join("ordinary")),
    ] {
        let original = fs::metadata(original).unwrap();
        let clone = fs::metadata(clone).unwrap();
        assert_eq!(
            original.permissions().mode() & 0o7777,
            clone.permissions().mode() & 0o7777
        );
        assert_eq!(
            (original.mtime(), original.mtime_nsec()),
            (clone.mtime(), clone.mtime_nsec())
        );
    }
    fs::write(target.join("ordinary"), b"target changed").unwrap();
    assert_eq!(fs::read(source.join("ordinary")).unwrap(), b"source bytes");
}

#[test]
fn a_replaced_target_invalidates_the_frozen_plan_before_any_write() {
    let (fixture, paths) = fixture("thinws-btrfs-stale-");
    fs::write(paths[0].join("file"), b"source").unwrap();
    let (request, plan) = request_and_plan(&paths);
    fs::rename(&paths[1], fixture.path().join("displaced")).unwrap();
    fs::create_dir(&paths[1]).unwrap();
    fs::write(paths[1].join("foreign"), b"keep").unwrap();
    let failure = BtrfsReflinkMaterializer::new()
        .materialize(&request, &plan)
        .unwrap_err();
    assert_eq!(
        failure.receipt().failure_kind(),
        Some(MaterializationFailureKind::PlanStale)
    );
    assert_eq!(
        failure.receipt().rollback().status(),
        RollbackStatus::NotNeeded
    );
    assert!(failure.receipt().created().is_empty());
    assert_eq!(fs::read(paths[1].join("foreign")).unwrap(), b"keep");
}

#[test]
fn unsupported_socket_in_source_fails_without_touching_target() {
    let (_fixture, paths) = fixture("thinws-btrfs-special-");
    let _socket = UnixListener::bind(paths[0].join("socket")).unwrap();
    let (request, plan) = request_and_plan(&paths);
    let failure = BtrfsReflinkMaterializer::new()
        .materialize(&request, &plan)
        .unwrap_err();
    assert_eq!(
        failure.receipt().failure_kind(),
        Some(MaterializationFailureKind::UnsupportedSourceEntry)
    );
    assert!(failure.receipt().created().is_empty());
    assert_eq!(
        failure.receipt().rollback().status(),
        RollbackStatus::NotNeeded
    );
    assert!(fs::read_dir(&paths[1]).unwrap().next().is_none());
}

#[test]
fn nocow_source_file_fails_closed_when_target_inherits_cow() {
    let (_fixture, paths) = fixture("thinws-btrfs-nocow-");
    let nocow = paths[0].join("nocow");
    fs::create_dir(&nocow).unwrap();
    let chattr = Command::new("chattr")
        .arg("+C")
        .arg(&nocow)
        .status()
        .expect("chattr must be installed for the Btrfs NOCOW test");
    assert!(chattr.success(), "Btrfs test root must permit NOCOW");
    fs::write(nocow.join("file"), b"NOCOW source bytes").unwrap();
    let (request, plan) = request_and_plan(&paths);
    let failure = BtrfsReflinkMaterializer::new()
        .materialize(&request, &plan)
        .unwrap_err();
    assert_eq!(
        failure.receipt().failure_kind(),
        Some(MaterializationFailureKind::CowUnavailable)
    );
    assert_eq!(
        failure.receipt().rollback().status(),
        RollbackStatus::ConfirmedBaseline
    );
    assert!(fs::read_dir(&paths[1]).unwrap().next().is_none());
    assert_eq!(fs::read(nocow.join("file")).unwrap(), b"NOCOW source bytes");
}

fn full_copy_fallback_fixture(
    prefix: &str,
    nocow_at_root: bool,
) -> (
    tempfile::TempDir,
    [PathBuf; 4],
    MaterializeRequest,
    MaterializationPlan,
    MaterializationPlan,
    MaterializationReceipt,
) {
    let (fixture, paths) = fixture(prefix);
    fs::write(paths[0].join("a-first"), b"cloned before failure").unwrap();
    if nocow_at_root {
        assert!(
            Command::new("chattr")
                .arg("+C")
                .arg(&paths[0])
                .status()
                .unwrap()
                .success()
        );
        fs::write(paths[0].join("z-nocow"), b"NOCOW bytes").unwrap();
    } else {
        let nocow = paths[0].join("z-nocow");
        fs::create_dir(&nocow).unwrap();
        assert!(
            Command::new("chattr")
                .arg("+C")
                .arg(&nocow)
                .status()
                .unwrap()
                .success()
        );
        fs::write(nocow.join("file"), b"NOCOW bytes").unwrap();
    }
    let request = MaterializeRequest::new(
        absolute(&paths[0]),
        absolute(&paths[1]),
        absolute(&paths[2]),
        absolute(&paths[3]),
    );
    let probe = LinuxPlatformProbe;
    let report = probe
        .inspect_materialization_paths(&MaterializationPathProbeRequest::from(&request))
        .unwrap();
    let cow_plan =
        MaterializationPlan::for_cow_clone(&report, FallbackPolicy::AllowFullCopyOnCowUnsupported)
            .unwrap();
    let failed = BtrfsReflinkMaterializer::new()
        .materialize(&request, &cow_plan)
        .unwrap_err();
    assert_eq!(
        failed.receipt().failure_kind(),
        Some(MaterializationFailureKind::CowUnavailable)
    );
    assert_eq!(
        failed.receipt().rollback().status(),
        RollbackStatus::ConfirmedBaseline
    );
    assert!(!failed.receipt().rollback().quarantined().is_empty());
    let fresh = probe
        .inspect_materialization_paths(&MaterializationPathProbeRequest::from(&request))
        .unwrap();
    let copy_plan = MaterializationPlan::for_full_copy_after_cow_unavailable(
        &fresh,
        &cow_plan,
        failed.receipt(),
    )
    .unwrap();
    (
        fixture,
        paths,
        request,
        copy_plan,
        cow_plan,
        failed.receipt().clone(),
    )
}

#[test]
fn full_copy_after_partial_reflink_clears_only_confirmed_quarantine() {
    let (_fixture, paths, request, plan, _, _) =
        full_copy_fallback_fixture("thinws-btrfs-copy-after-rollback-", false);
    let receipt = LinuxFullCopyMaterializer::new()
        .materialize(&request, &plan)
        .unwrap();
    assert_eq!(receipt.outcome(), MaterializationOutcome::Succeeded);
    assert_eq!(receipt.cow_evidence(), CowEvidence::NotUsed);
    assert_eq!(receipt.clone_calls_succeeded(), 0);
    assert_eq!(
        receipt.fallback_reason(),
        Some(FallbackReason::CloneUnavailableAtRuntime)
    );
    assert_eq!(receipt.failed_attempts().len(), 1);
    assert!(fs::read_dir(&paths[3]).unwrap().next().is_none());
    for name in ["a-first", "z-nocow/file"] {
        assert_eq!(
            fs::read(paths[1].join(name)).unwrap(),
            fs::read(paths[0].join(name)).unwrap()
        );
        assert_ne!(
            fs::metadata(paths[1].join(name)).unwrap().ino(),
            fs::metadata(paths[0].join(name)).unwrap().ino()
        );
    }
}

#[test]
fn full_copy_accepts_one_exact_confirmed_quarantine_entry() {
    let (_fixture, paths, request, plan, _, _) =
        full_copy_fallback_fixture("thinws-btrfs-copy-one-quarantine-", true);
    assert_eq!(plan.failed_attempts()[0].rollback().quarantined().len(), 1);
    let receipt = LinuxFullCopyMaterializer::new()
        .materialize(&request, &plan)
        .unwrap();
    assert_eq!(receipt.outcome(), MaterializationOutcome::Succeeded);
    assert_eq!(fs::read(paths[1].join("z-nocow")).unwrap(), b"NOCOW bytes");
    assert!(fs::read_dir(&paths[3]).unwrap().next().is_none());
}

#[test]
fn full_copy_rejects_inconsistent_prior_rollback_without_deleting_quarantine() {
    let (_fixture, paths, request, _plan, cow_plan, failed) =
        full_copy_fallback_fixture("thinws-btrfs-copy-inconsistent-rollback-", false);
    let mut removed = failed.rollback().removed().to_vec();
    removed.push(removed[0].clone());
    let forged_rollback =
        RollbackEvidence::new(RollbackStatus::ConfirmedBaseline, removed, Vec::new())
            .with_quarantined(failed.rollback().quarantined().to_vec());
    let evidence = MaterializationAttemptEvidence::new(
        failed.logical_bytes(),
        failed.physical_bytes(),
        failed.regular_file_count(),
        failed.clone_calls_succeeded(),
        failed.source_manifest_digest(),
        failed.target_manifest_digest(),
    );
    let forged = MaterializationReceipt::failed_cow_clone(
        &cow_plan,
        MaterializationFailureKind::CowUnavailable,
        failed.created().to_vec(),
        true,
        forged_rollback,
        evidence,
        failed.elapsed_millis(),
    );
    let report = LinuxPlatformProbe
        .inspect_materialization_paths(&MaterializationPathProbeRequest::from(&request))
        .unwrap();
    let plan =
        MaterializationPlan::for_full_copy_after_cow_unavailable(&report, &cow_plan, &forged)
            .unwrap();
    let quarantined_before = fs::read_dir(&paths[3])
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    let failure = LinuxFullCopyMaterializer::new()
        .materialize(&request, &plan)
        .unwrap_err();
    assert_eq!(
        failure.receipt().failure_kind(),
        Some(MaterializationFailureKind::PlanStale)
    );
    assert!(fs::read_dir(&paths[1]).unwrap().next().is_none());
    for entry in quarantined_before {
        assert!(fs::symlink_metadata(paths[3].join(entry)).is_ok());
    }
}

#[test]
fn full_copy_after_cow_fails_before_first_entry_accepts_not_needed_rollback() {
    let (_fixture, paths) = fixture("thinws-btrfs-copy-no-prior-output-");
    assert!(
        Command::new("chattr")
            .arg("+C")
            .arg(&paths[0])
            .status()
            .unwrap()
            .success()
    );
    let content = (0..(64 * 1024 + 257))
        .map(|i| (i % 251) as u8)
        .collect::<Vec<_>>();
    fs::write(paths[0].join("file"), &content).unwrap();
    let request = MaterializeRequest::new(
        absolute(&paths[0]),
        absolute(&paths[1]),
        absolute(&paths[2]),
        absolute(&paths[3]),
    );
    let probe = LinuxPlatformProbe;
    let report = probe
        .inspect_materialization_paths(&MaterializationPathProbeRequest::from(&request))
        .unwrap();
    let cow_plan =
        MaterializationPlan::for_cow_clone(&report, FallbackPolicy::AllowFullCopyOnCowUnsupported)
            .unwrap();
    let failed = BtrfsReflinkMaterializer::new()
        .materialize(&request, &cow_plan)
        .unwrap_err();
    assert_eq!(
        failed.receipt().failure_kind(),
        Some(MaterializationFailureKind::CowUnavailable)
    );
    assert_eq!(
        failed.receipt().rollback().status(),
        RollbackStatus::NotNeeded
    );
    assert!(failed.receipt().created().is_empty());
    let fresh = probe
        .inspect_materialization_paths(&MaterializationPathProbeRequest::from(&request))
        .unwrap();
    let copy_plan = MaterializationPlan::for_full_copy_after_cow_unavailable(
        &fresh,
        &cow_plan,
        failed.receipt(),
    )
    .unwrap();
    let receipt = LinuxFullCopyMaterializer::new()
        .materialize(&request, &copy_plan)
        .unwrap();
    assert_eq!(receipt.outcome(), MaterializationOutcome::Succeeded);
    assert_eq!(receipt.cow_evidence(), CowEvidence::NotUsed);
    assert_eq!(fs::read(paths[1].join("file")).unwrap(), content);
    assert!(fs::read_dir(&paths[3]).unwrap().next().is_none());
}

#[test]
fn full_copy_rejects_source_change_after_confirmed_reflink_rollback() {
    let (_fixture, paths, request, plan, _, _) =
        full_copy_fallback_fixture("thinws-btrfs-copy-source-change-", false);
    fs::write(paths[0].join("z-nocow/file"), b"changed after failed clone").unwrap();
    let failure = LinuxFullCopyMaterializer::new()
        .materialize(&request, &plan)
        .unwrap_err();
    assert_eq!(
        failure.receipt().failure_kind(),
        Some(MaterializationFailureKind::SourceChanged)
    );
    assert_eq!(
        failure.receipt().rollback().status(),
        RollbackStatus::NotNeeded
    );
    assert!(fs::read_dir(&paths[1]).unwrap().next().is_none());
    assert!(
        !fs::read_dir(&paths[3])
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn full_copy_keeps_foreign_trash_entry_and_does_not_publish_success() {
    let (_fixture, paths, request, plan, _, _) =
        full_copy_fallback_fixture("thinws-btrfs-copy-foreign-trash-", false);
    fs::write(paths[3].join("foreign"), b"must remain").unwrap();
    let failure = LinuxFullCopyMaterializer::new()
        .materialize(&request, &plan)
        .unwrap_err();
    assert_eq!(
        failure.receipt().failure_kind(),
        Some(MaterializationFailureKind::TargetChanged)
    );
    assert_eq!(fs::read(paths[3].join("foreign")).unwrap(), b"must remain");
    assert!(fs::read_dir(&paths[1]).unwrap().next().is_none());
}

#[test]
fn symlink_only_tree_succeeds_without_claiming_cow() {
    let (_fixture, paths) = fixture("thinws-btrfs-no-files-");
    fs::create_dir(paths[0].join("empty")).unwrap();
    symlink("missing", paths[0].join("link")).unwrap();
    let (request, plan) = request_and_plan(&paths);
    let receipt = BtrfsReflinkMaterializer::new()
        .materialize(&request, &plan)
        .unwrap();
    assert_eq!(receipt.cow_evidence(), CowEvidence::NotUsed);
    assert_eq!(receipt.regular_file_count(), Some(0));
    assert_eq!(receipt.clone_calls_succeeded(), 0);
    assert_eq!(
        fs::read_link(paths[1].join("link")).unwrap(),
        Path::new("missing")
    );
}
