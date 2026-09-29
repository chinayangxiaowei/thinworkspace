#![cfg(target_os = "linux")]

use std::env;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

use thinws_adapter_linux::LinuxHostAdapter;
use thinws_core::{AbsolutePath, ProcessUse};
use thinws_ports::{PortErrorKind, ProcessProbe};

fn absolute(path: &Path) -> AbsolutePath {
    AbsolutePath::try_from_bytes(path.as_os_str().as_encoded_bytes().to_vec()).unwrap()
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn confirmed(adapter: &LinuxHostAdapter, path: &Path) -> bool {
    for _ in 0..20 {
        if adapter
            .inspect_workspace(&absolute(path))
            .unwrap()
            .use_state
            == ProcessUse::ConfirmedInUse
        {
            return true;
        }
        thread::sleep(Duration::from_millis(25));
    }
    false
}

#[test]
fn current_user_child_cwd_is_confirmed() {
    let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
        .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs directory");
    let fixture = tempfile::Builder::new()
        .prefix("thinws-linux-process-cwd-")
        .tempdir_in(root)
        .unwrap();
    let workspace = fixture.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let adapter = LinuxHostAdapter::new(fixture.path().join("control")).unwrap();
    let child = ChildGuard(
        Command::new("/bin/sleep")
            .arg("30")
            .current_dir(&workspace)
            .spawn()
            .unwrap(),
    );
    assert!(confirmed(&adapter, &workspace));
    drop(child);
    assert_ne!(
        adapter
            .inspect_workspace(&absolute(&workspace))
            .unwrap()
            .use_state,
        ProcessUse::ConfirmedInUse
    );
}

#[test]
fn current_user_child_open_file_is_confirmed() {
    let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
        .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs directory");
    let fixture = tempfile::Builder::new()
        .prefix("thinws-linux-process-fd-")
        .tempdir_in(root)
        .unwrap();
    let workspace = fixture.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let file = workspace.join("held");
    fs::write(&file, b"held").unwrap();
    let adapter = LinuxHostAdapter::new(fixture.path().join("control")).unwrap();
    let child = ChildGuard(
        Command::new("/bin/sleep")
            .arg("30")
            .stdin(Stdio::from(fs::File::open(&file).unwrap()))
            .spawn()
            .unwrap(),
    );
    assert!(confirmed(&adapter, &workspace));
    drop(child);
}

#[test]
fn scan_rejects_symlinked_workspace_root() {
    let root = env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
        .expect("set THINWS_LINUX_BTRFS_TEST_ROOT to a writable Btrfs directory");
    let fixture = tempfile::Builder::new()
        .prefix("thinws-linux-process-symlink-")
        .tempdir_in(root)
        .unwrap();
    let workspace = fixture.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    let alias = fixture.path().join("alias");
    symlink(&workspace, &alias).unwrap();
    let adapter = LinuxHostAdapter::new(fixture.path().join("control")).unwrap();
    assert_eq!(
        adapter
            .inspect_workspace(&absolute(&alias))
            .unwrap_err()
            .kind(),
        PortErrorKind::InvalidLayout
    );
}
