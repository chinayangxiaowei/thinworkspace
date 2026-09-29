#![cfg(target_os = "linux")]

use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

use tempfile::Builder;
use thinws_p0_linux_reflink::{BtrfsPairPreflight, clone_file_data, inspect_btrfs_pair};

fn configured_btrfs_root() -> PathBuf {
    let root = PathBuf::from(
        env::var_os("THINWS_LINUX_BTRFS_TEST_ROOT")
            .expect("set THINWS_LINUX_BTRFS_TEST_ROOT for the ignored real Btrfs test"),
    );
    assert!(root.is_absolute(), "test root must be absolute");
    assert_eq!(
        fs::canonicalize(&root).expect("test root must exist"),
        root,
        "test root must not contain symlinks"
    );
    let mount = Command::new("findmnt")
        .args(["--noheadings", "--output", "FSTYPE", "--target"])
        .arg(&root)
        .output()
        .expect("findmnt must be installed for this real filesystem test");
    assert!(mount.status.success(), "test root mount lookup failed");
    assert_eq!(mount.stdout, b"btrfs\n", "test root must be on Btrfs");
    root
}

#[test]
#[ignore = "requires THINWS_LINUX_BTRFS_TEST_ROOT on a writable Btrfs mount"]
fn ficlone_shares_blocks_and_later_writes_stay_private() {
    let root = configured_btrfs_root();
    let scratch = Builder::new()
        .prefix("thinws-linux-ficlone-")
        .tempdir_in(root)
        .expect("create private Btrfs fixture root");
    let source_path = scratch.path().join("source");
    let clone_path = scratch.path().join("clone");
    let initial = b"thinws Linux Btrfs FICLONE test\n";
    let mut source = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&source_path)
        .expect("create source only inside the private fixture root");
    source.write_all(initial).expect("write source fixture");
    source.sync_all().expect("persist source fixture");
    let mut clone = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&clone_path)
        .expect("create clone only inside the private fixture root");

    assert_eq!(
        inspect_btrfs_pair(&source, &clone).expect("inspect both held file descriptors"),
        BtrfsPairPreflight::EligibleSameMount
    );

    clone_file_data(&source, &clone).expect("FICLONE must succeed on the configured Btrfs mount");
    assert_eq!(fs::read(&clone_path).unwrap(), initial);
    clone.write_all(b"changed").expect("write into cloned file");
    clone.sync_all().expect("persist cloned write");
    assert_eq!(fs::read(&source_path).unwrap(), initial);
    assert!(fs::read(&clone_path).unwrap().starts_with(b"changed"));
}

#[test]
#[ignore = "requires THINWS_LINUX_BTRFS_TEST_ROOT on a writable Btrfs mount"]
fn ficlone_reports_bad_destination_mode_without_hiding_errno() {
    let root = configured_btrfs_root();
    let scratch = Builder::new()
        .prefix("thinws-linux-ficlone-error-")
        .tempdir_in(root)
        .expect("create private Btrfs fixture root");
    let source_path = scratch.path().join("source");
    let destination_path = scratch.path().join("destination");
    fs::write(&source_path, b"source data").expect("write private source fixture");
    fs::write(&destination_path, b"").expect("create private destination fixture");
    let source = OpenOptions::new().read(true).open(&source_path).unwrap();
    let destination = OpenOptions::new()
        .read(true)
        .open(&destination_path)
        .unwrap();

    let error = clone_file_data(&source, &destination).expect_err("read-only target must fail");
    assert_eq!(error.raw_os_error(), Some(libc::EBADF));
}

#[test]
#[ignore = "requires THINWS_LINUX_BTRFS_TEST_ROOT on a writable Btrfs mount"]
fn preflight_rejects_a_directory_even_on_the_same_btrfs_mount() {
    let root = configured_btrfs_root();
    let scratch = Builder::new()
        .prefix("thinws-linux-pair-type-")
        .tempdir_in(root)
        .expect("create private Btrfs fixture root");
    let regular_path = scratch.path().join("regular");
    fs::write(&regular_path, b"regular file").expect("write private fixture");
    let directory = OpenOptions::new().read(true).open(scratch.path()).unwrap();
    let regular = OpenOptions::new().read(true).open(regular_path).unwrap();

    assert_eq!(
        inspect_btrfs_pair(&directory, &regular).unwrap(),
        BtrfsPairPreflight::UnsupportedFileType
    );
}

#[test]
#[ignore = "requires THINWS_LINUX_BTRFS_TEST_ROOT and THINWS_LINUX_OTHER_TEST_FILE"]
fn preflight_rejects_a_file_on_another_filesystem() {
    let root = configured_btrfs_root();
    let scratch = Builder::new()
        .prefix("thinws-linux-pair-fs-")
        .tempdir_in(root)
        .expect("create private Btrfs fixture root");
    let source_path = scratch.path().join("source");
    fs::write(&source_path, b"source").expect("write private Btrfs fixture");
    let source = OpenOptions::new().read(true).open(source_path).unwrap();
    let other_path = PathBuf::from(
        env::var_os("THINWS_LINUX_OTHER_TEST_FILE")
            .expect("set THINWS_LINUX_OTHER_TEST_FILE to an existing regular file"),
    );
    assert!(other_path.is_absolute(), "other test file must be absolute");
    let other = OpenOptions::new()
        .read(true)
        .open(other_path)
        .expect("open other test file read-only");

    assert_eq!(
        inspect_btrfs_pair(&source, &other).unwrap(),
        BtrfsPairPreflight::UnsupportedFilesystem
    );
}
