#![cfg(target_os = "linux")]

use std::env;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::fs::symlink;
use std::time::{Duration, Instant};

use thinws_adapter_linux::LinuxHostAdapter;
use thinws_core::AbsolutePath;
use thinws_ports::{LifecycleLock, LifecycleLockGuard, LifecycleScope, PortErrorKind};

#[test]
fn lock_is_bounded_and_revalidates_its_parent_and_leaf() {
    let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
        .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to the dedicated Btrfs test mount");
    let fixture = tempfile::Builder::new()
        .prefix("thinws-linux-lock-")
        .tempdir_in(root)
        .unwrap();
    let control = fixture.path().join("control");
    std::fs::create_dir(&control).unwrap();
    std::fs::set_permissions(&control, std::fs::Permissions::from_mode(0o700)).unwrap();
    let adapter = LinuxHostAdapter::new(&control).unwrap();
    let guard = adapter
        .acquire_bootstrap(Duration::from_millis(100))
        .unwrap();
    assert_eq!(guard.scope(), LifecycleScope::Bootstrap);
    guard.revalidate().unwrap();
    let timeout = Duration::from_millis(30);
    let started = Instant::now();
    assert_eq!(
        adapter.acquire_bootstrap(timeout).unwrap_err().kind(),
        PortErrorKind::Timeout,
    );
    assert!(started.elapsed() >= timeout);
    drop(guard);
    let control_path =
        AbsolutePath::try_from_bytes(control.as_os_str().as_encoded_bytes().to_vec()).unwrap();
    let guard = adapter
        .acquire_data_root(&control_path, Duration::from_millis(100))
        .unwrap();
    assert_eq!(guard.scope(), LifecycleScope::DataRoot);
    let displaced = fixture.path().join("displaced");
    std::fs::rename(&control, &displaced).unwrap();
    std::fs::create_dir(&control).unwrap();
    std::fs::set_permissions(&control, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        guard.revalidate().unwrap_err().kind(),
        PortErrorKind::InvalidLayout
    );
}

#[test]
fn lock_refuses_replaced_leaf_and_nonprivate_control_directory() {
    let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
        .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to the dedicated Btrfs test mount");
    let fixture = tempfile::Builder::new()
        .prefix("thinws-linux-lock-leaf-")
        .tempdir_in(root)
        .unwrap();
    let control = fixture.path().join("control");
    std::fs::create_dir(&control).unwrap();
    std::fs::set_permissions(&control, std::fs::Permissions::from_mode(0o755)).unwrap();
    let adapter = LinuxHostAdapter::new(&control).unwrap();
    assert_eq!(
        adapter
            .acquire_bootstrap(Duration::from_millis(10))
            .unwrap_err()
            .kind(),
        PortErrorKind::InvalidLayout,
    );
    std::fs::set_permissions(&control, std::fs::Permissions::from_mode(0o700)).unwrap();
    let guard = adapter
        .acquire_bootstrap(Duration::from_millis(100))
        .unwrap();
    std::fs::rename(
        control.join("lifecycle.lock"),
        control.join("displaced.lock"),
    )
    .unwrap();
    assert_eq!(
        guard.revalidate().unwrap_err().kind(),
        PortErrorKind::InvalidLayout
    );
    std::fs::write(control.join("lifecycle.lock"), b"new").unwrap();
    assert_eq!(
        guard.revalidate().unwrap_err().kind(),
        PortErrorKind::InvalidLayout
    );
}

#[test]
fn lock_refuses_symlinked_control_path() {
    let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
        .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to the dedicated Btrfs test mount");
    let fixture = tempfile::Builder::new()
        .prefix("thinws-linux-lock-symlink-")
        .tempdir_in(root)
        .unwrap();
    let actual = fixture.path().join("actual");
    std::fs::create_dir(&actual).unwrap();
    std::fs::set_permissions(&actual, std::fs::Permissions::from_mode(0o700)).unwrap();
    let alias = fixture.path().join("alias");
    symlink(&actual, &alias).unwrap();
    let adapter = LinuxHostAdapter::new(alias).unwrap();
    assert_eq!(
        adapter
            .acquire_bootstrap(Duration::from_millis(10))
            .unwrap_err()
            .kind(),
        PortErrorKind::InvalidLayout,
    );
}
