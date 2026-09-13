#![deny(unsafe_code)]

use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use thinws_p0_materialize::{Backend, MaterializationError, MaterializeRequest, prepare_attempt};

mod support;

use support::ControlledTree;

#[derive(Debug, Eq, PartialEq)]
struct BorrowedSourceObservation {
    source_device: u64,
    source_inode: u64,
    mounted_device: u64,
    mounted_inode: u64,
    names: Vec<OsString>,
}

struct Fixture {
    tree: ControlledTree,
    target: PathBuf,
    staging: PathBuf,
    trash: PathBuf,
}

impl Fixture {
    fn create(label: &str) -> io::Result<Self> {
        let mut tree = ControlledTree::create_in(Path::new("/private/tmp"), label)?;
        let root = tree.root_path().to_path_buf();
        for directory in ["target", "staging", "trash"] {
            tree.create_directory(directory, 0o700)?;
        }
        Ok(Self {
            tree,
            target: root.join("target"),
            staging: root.join("staging"),
            trash: root.join("trash"),
        })
    }

    fn request<'a>(&'a self, source: &'a Path) -> MaterializeRequest<'a> {
        MaterializeRequest {
            source,
            target: &self.target,
            staging: &self.staging,
            trash: &self.trash,
        }
    }

    fn cleanup(self) -> io::Result<()> {
        self.tree.cleanup()
    }
}

fn observe_borrowed_source(source: &Path) -> io::Result<BorrowedSourceObservation> {
    let source_metadata = fs::symlink_metadata(source)?;
    if !source_metadata.file_type().is_dir() {
        return Err(io::Error::other("borrowed source is not a directory"));
    }
    let mounted = source.join("mounted");
    let mounted_metadata = fs::symlink_metadata(&mounted)?;
    if !mounted_metadata.file_type().is_dir() {
        return Err(io::Error::other(
            "borrowed mounted child is not a directory",
        ));
    }
    let mut names = fs::read_dir(source)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<io::Result<Vec<_>>>()?;
    names.sort();
    Ok(BorrowedSourceObservation {
        source_device: source_metadata.dev(),
        source_inode: source_metadata.ino(),
        mounted_device: mounted_metadata.dev(),
        mounted_inode: mounted_metadata.ino(),
        names,
    })
}

fn submount_source() -> (PathBuf, BorrowedSourceObservation) {
    let source = std::env::var_os("THINWS_P0_SUBMOUNT_SOURCE")
        .map(PathBuf::from)
        .expect("THINWS_P0_SUBMOUNT_SOURCE must name the dedicated borrowed source");
    assert!(source.is_absolute(), "borrowed source must be absolute");
    let observation = observe_borrowed_source(&source).expect("inspect dedicated borrowed source");
    assert_eq!(observation.names, [OsString::from("mounted")]);
    assert_ne!(
        observation.source_device, observation.mounted_device,
        "mounted child must be a different device from its source parent"
    );
    (source, observation)
}

fn assert_submount_rejected_before_writing(backend: Backend, label: &str) {
    let (source, source_before) = submount_source();
    let fixture = Fixture::create(label).expect("create controlled target fixture in /private/tmp");

    let controlled_devices = [&fixture.target, &fixture.staging, &fixture.trash]
        .into_iter()
        .map(|path| fs::symlink_metadata(path).map(|metadata| metadata.dev()))
        .collect::<io::Result<Vec<_>>>();
    let controlled_devices = match controlled_devices {
        Ok(devices) => devices,
        Err(error) => {
            fixture
                .cleanup()
                .expect("clean controlled fixture after layout observation failure");
            panic!("inspect controlled layout devices: {error}");
        }
    };
    if controlled_devices
        .iter()
        .any(|device| *device != source_before.source_device)
    {
        fixture
            .cleanup()
            .expect("clean controlled fixture after same-volume precondition failure");
        panic!(
            "controlled roots must share source device {}; observed {controlled_devices:?}",
            source_before.source_device
        );
    }

    let target_before = fixture
        .tree
        .observed_tree()
        .expect("snapshot controlled target fixture before prepare");
    let rejection = prepare_attempt(&fixture.request(&source), backend).err();
    let target_entries = fs::read_dir(&fixture.target).and_then(|entries| {
        entries
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<io::Result<Vec<_>>>()
    });
    let target_after = fixture.tree.observed_tree();
    let source_after = observe_borrowed_source(&source);

    fixture
        .cleanup()
        .expect("clean only the exact registered controlled target fixture");

    assert_eq!(
        source_after.expect("reinspect borrowed source after read-only prepare"),
        source_before,
        "prepare must leave the borrowed source and mountpoint identities unchanged"
    );
    assert!(
        target_entries
            .expect("enumerate target after read-only prepare")
            .is_empty(),
        "prepare must not create target entries"
    );
    assert_eq!(
        target_after.expect("snapshot controlled target fixture after prepare"),
        target_before,
        "prepare must not change the controlled target tree"
    );

    match rejection.expect("a real source submount must be rejected during prepare") {
        MaterializationError::UnsupportedSourceEntry { path, kind } => {
            assert_eq!(path, source.join("mounted"));
            assert_eq!(kind, "submount");
        }
        other => panic!("unexpected source-submount rejection: {other:?}"),
    }
}

#[test]
#[ignore = "requires THINWS_P0_SUBMOUNT_SOURCE on a dedicated mounted APFS volume"]
fn apfs_clone_prepare_rejects_real_source_submount_before_writing() {
    assert_submount_rejected_before_writing(Backend::ApfsFileClone, "submount-apfs-clone");
}

#[test]
#[ignore = "requires THINWS_P0_SUBMOUNT_SOURCE on a dedicated mounted APFS volume"]
fn full_copy_prepare_rejects_real_source_submount_before_writing() {
    assert_submount_rejected_before_writing(Backend::FullCopy, "submount-full-copy");
}
