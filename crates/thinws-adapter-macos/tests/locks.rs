#![cfg(target_os = "macos")]

use std::fs::{self, OpenOptions};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use tempfile::{Builder, TempDir};
use thinws_adapter_macos::MacOsHostAdapter;
use thinws_core::AbsolutePath;
use thinws_ports::{LifecycleLock, LifecycleLockGuard, LifecycleScope, PortErrorKind};

const CHILD_BOOTSTRAP: &str = "THINWS_TEST_LOCK_BOOTSTRAP";
const CHILD_READY: &str = "THINWS_TEST_LOCK_READY";
const CHILD_RELEASE: &str = "THINWS_TEST_LOCK_RELEASE";

fn controlled_tempdir() -> TempDir {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-02-macos-tests");
    fs::create_dir_all(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    Builder::new()
        .prefix("locks-")
        .tempdir_in(fs::canonicalize(root).unwrap())
        .unwrap()
}

fn private_dir(path: &Path) {
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn absolute(path: &Path) -> AbsolutePath {
    AbsolutePath::try_from_bytes(path.as_os_str().as_bytes().to_vec()).unwrap()
}

#[test]
fn one_lifecycle_file_serializes_init_and_workspace_mutations() {
    let temp = controlled_tempdir();
    let control_root = temp.path().join(".thinws");
    private_dir(&control_root);
    let adapter = MacOsHostAdapter::new(&control_root).unwrap();

    let bootstrap_guard = adapter
        .acquire_bootstrap(Duration::from_millis(100))
        .unwrap();
    assert_eq!(bootstrap_guard.scope(), LifecycleScope::Bootstrap);
    let diagnostic_before = fs::read(control_root.join("lifecycle.lock")).unwrap();
    let started = Instant::now();
    let error = adapter
        .acquire_data_root(&absolute(&control_root), Duration::from_millis(40))
        .err()
        .expect("workspace mutation must contend with init");
    assert_eq!(error.kind(), PortErrorKind::Timeout);
    assert!(started.elapsed() >= Duration::from_millis(35));
    assert_eq!(
        fs::read(control_root.join("lifecycle.lock")).unwrap(),
        diagnostic_before
    );

    drop(bootstrap_guard);
    let data_guard = adapter
        .acquire_data_root(&absolute(&control_root), Duration::from_millis(100))
        .unwrap();
    assert_eq!(data_guard.scope(), LifecycleScope::DataRoot);
    data_guard.revalidate().unwrap();
    drop(data_guard);
    adapter
        .acquire_bootstrap(Duration::from_millis(100))
        .unwrap();
    assert!(!control_root.join("init.lock").exists());
    assert!(!control_root.join("metadata/lifecycle.lock").exists());
}

#[test]
fn concurrent_first_lifecycle_lock_open_serializes_without_spurious_not_found() {
    for _ in 0..12 {
        let temp = controlled_tempdir();
        let control_root = temp.path().join(".thinws");
        private_dir(&control_root);
        let adapter = MacOsHostAdapter::new(&control_root).unwrap();
        let barrier = Arc::new(Barrier::new(3));
        let mut threads = Vec::new();
        for _ in 0..2 {
            let adapter = adapter.clone();
            let data_root = absolute(&control_root);
            let barrier = barrier.clone();
            threads.push(thread::spawn(move || {
                barrier.wait();
                let guard = adapter
                    .acquire_data_root(&data_root, Duration::from_secs(2))
                    .unwrap();
                guard.revalidate().unwrap();
            }));
        }
        barrier.wait();
        for thread in threads {
            thread.join().unwrap();
        }
        assert!(control_root.join("lifecycle.lock").exists());
    }
}

#[test]
fn lock_symlink_and_post_acquisition_replacement_are_rejected() {
    let temp = controlled_tempdir();
    let bootstrap = temp.path().join("bootstrap");
    private_dir(&bootstrap);
    let victim = temp.path().join("victim");
    fs::write(&victim, b"victim").unwrap();
    fs::set_permissions(&victim, fs::Permissions::from_mode(0o600)).unwrap();
    symlink(&victim, bootstrap.join("lifecycle.lock")).unwrap();
    let adapter = MacOsHostAdapter::new(&bootstrap).unwrap();
    assert!(
        adapter
            .acquire_bootstrap(Duration::from_millis(20))
            .is_err()
    );
    assert_eq!(fs::read(&victim).unwrap(), b"victim");
    fs::remove_file(bootstrap.join("lifecycle.lock")).unwrap();

    private_dir(&bootstrap.join("lifecycle.lock"));
    assert!(
        adapter
            .acquire_bootstrap(Duration::from_millis(20))
            .is_err()
    );
    fs::remove_dir(bootstrap.join("lifecycle.lock")).unwrap();

    let guard = adapter
        .acquire_bootstrap(Duration::from_millis(100))
        .unwrap();
    fs::rename(
        bootstrap.join("lifecycle.lock"),
        bootstrap.join("displaced.lock"),
    )
    .unwrap();
    let replacement = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(bootstrap.join("lifecycle.lock"))
        .unwrap();
    replacement.sync_all().unwrap();
    assert_eq!(
        guard.revalidate().unwrap_err().kind(),
        PortErrorKind::InvalidData
    );
}

#[test]
fn lifecycle_lock_contends_with_a_separate_process() {
    let temp = controlled_tempdir();
    let bootstrap = temp.path().join("bootstrap");
    private_dir(&bootstrap);
    let ready = temp.path().join("child-ready");
    let release = temp.path().join("child-release");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "lifecycle_lock_holder_child",
            "--nocapture",
        ])
        .env(CHILD_BOOTSTRAP, &bootstrap)
        .env(CHILD_READY, &ready)
        .env(CHILD_RELEASE, &release)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready.exists() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("lock holder exited before readiness: {status}");
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("lock holder did not become ready");
        }
        thread::sleep(Duration::from_millis(10));
    }

    let adapter = MacOsHostAdapter::new(&bootstrap).unwrap();
    let error = adapter
        .acquire_bootstrap(Duration::from_millis(80))
        .err()
        .expect("another process owns the lock");
    assert_eq!(error.kind(), PortErrorKind::Timeout);
    fs::write(&release, b"release").unwrap();
    assert!(child.wait().unwrap().success());
    adapter
        .acquire_bootstrap(Duration::from_millis(100))
        .unwrap();
}

#[test]
#[ignore = "fixed child entry; spawned by lifecycle_lock_contends_with_a_separate_process"]
fn lifecycle_lock_holder_child() {
    let Some(bootstrap) = std::env::var_os(CHILD_BOOTSTRAP) else {
        return;
    };
    let ready = PathBuf::from(std::env::var_os(CHILD_READY).unwrap());
    let release = PathBuf::from(std::env::var_os(CHILD_RELEASE).unwrap());
    let adapter = MacOsHostAdapter::new(PathBuf::from(bootstrap)).unwrap();
    let _guard = adapter.acquire_bootstrap(Duration::from_secs(2)).unwrap();
    fs::write(ready, b"ready").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !release.exists() {
        assert!(Instant::now() < deadline, "parent did not release child");
        thread::sleep(Duration::from_millis(10));
    }
}
