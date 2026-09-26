use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use thinws_adapter_git_cli::SystemGitInspector;
use thinws_core::{AbsolutePath, DiscoveryCompleteness, GitState, RepositoryState};
use thinws_ports::{GitInspectionIssue, GitInspector};

const CHILD_ROOT: &str = "THINWS_P1_16_TEST_COPY_ROOT";

fn fixture_git(cwd: &Path, home: &Path, args: &[&str]) {
    let output = Command::new("/usr/bin/git")
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .env("HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("run fixture Git");
    assert!(
        output.status.success(),
        "fixture Git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn absolute(path: &std::path::Path) -> AbsolutePath {
    AbsolutePath::try_from_bytes(path.as_os_str().as_bytes()).expect("temporary path is canonical")
}

#[test]
fn directory_without_local_git_marker_is_not_a_parent_repository() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory
        .path()
        .canonicalize()
        .expect("canonical temporary root");
    let report = SystemGitInspector.inspect(&absolute(&root));
    assert_eq!(report.discovery(), DiscoveryCompleteness::Complete);
    assert_eq!(report.aggregate(), GitState::NotApplicable);
    assert!(report.repositories().is_empty());
}

#[test]
fn missing_copy_root_is_an_incomplete_report() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let missing = directory
        .path()
        .canonicalize()
        .expect("canonical temporary root")
        .join("missing");
    let report = SystemGitInspector.inspect(&absolute(&missing));
    assert_eq!(report.discovery(), DiscoveryCompleteness::Incomplete);
    assert_eq!(report.aggregate(), GitState::Unknown);
    assert_eq!(report.issues(), &[GitInspectionIssue::InvalidCopyRoot]);
}

#[test]
fn real_repository_port_distinguishes_untracked_from_tracked_changes() {
    let fixture = tempfile::tempdir().expect("temporary fixture");
    let root = fixture
        .path()
        .canonicalize()
        .expect("canonical fixture root");
    let home = root.join("home");
    let copy_root = root.join("copy");
    fs::create_dir(&home).expect("private Git home");
    fs::create_dir(&copy_root).expect("controlled copy");
    fixture_git(&copy_root, &home, &["init", "--quiet"]);
    fixture_git(&copy_root, &home, &["config", "user.name", "Fixture"]);
    fixture_git(
        &copy_root,
        &home,
        &["config", "user.email", "fixture@example.invalid"],
    );
    fs::write(copy_root.join("tracked.txt"), b"baseline\n").expect("tracked baseline");
    fixture_git(&copy_root, &home, &["add", "tracked.txt"]);
    fixture_git(&copy_root, &home, &["commit", "--quiet", "-m", "baseline"]);

    let output = Command::new(std::env::current_exe().expect("integration test executable"))
        .args(["--exact", "isolated_git_inspection_child", "--nocapture"])
        .env_clear()
        .env("HOME", &home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env(CHILD_ROOT, &copy_root)
        .output()
        .expect("isolated inspector child");
    assert!(
        output.status.success(),
        "inspector child failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn isolated_git_inspection_child() {
    let Some(root) = std::env::var_os(CHILD_ROOT) else {
        return;
    };
    let copy_root = Path::new(OsStr::new(&root));
    let inspect = || SystemGitInspector.inspect(&absolute(copy_root));
    let clean = inspect();
    assert_eq!(clean.discovery(), DiscoveryCompleteness::Complete);
    assert_eq!(clean.aggregate(), GitState::Clean);
    assert_eq!(clean.repositories().len(), 1);
    assert_eq!(clean.repositories()[0].state(), RepositoryState::Clean);

    fs::write(copy_root.join("untracked.txt"), b"untracked\n").expect("untracked fixture");
    assert_eq!(inspect().aggregate(), GitState::Clean);

    fs::write(copy_root.join("tracked.txt"), b"changed\n").expect("tracked change");
    let dirty = inspect();
    assert_eq!(dirty.aggregate(), GitState::Dirty);
    assert_eq!(
        dirty.repositories()[0].state().tracked_change_count(),
        Some(1)
    );
    assert!(dirty.repositories()[0].issues().is_empty());

    fs::write(copy_root.join("tracked.txt"), b"baseline\n").expect("restore tracked baseline");
    assert_eq!(inspect().aggregate(), GitState::Clean);
    fixture_git(copy_root, copy_root, &["config", "core.filemode", "false"]);
    let tracked = copy_root.join("tracked.txt");
    let mut permissions = fs::metadata(&tracked)
        .expect("tracked metadata")
        .permissions();
    assert_eq!(
        permissions.mode() & 0o111,
        0,
        "fixture starts non-executable"
    );
    permissions.set_mode(permissions.mode() | 0o111);
    fs::set_permissions(&tracked, permissions).expect("tracked executable-bit change");
    let mode_dirty = inspect();
    assert_eq!(mode_dirty.aggregate(), GitState::Dirty);
    assert_eq!(
        mode_dirty.repositories()[0].state().tracked_change_count(),
        Some(1)
    );
}
