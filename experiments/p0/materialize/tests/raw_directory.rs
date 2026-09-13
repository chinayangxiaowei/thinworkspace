#![deny(unsafe_code)]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use thinws_p0_materialize::{
    AttemptEvidence, AttemptOutcome, Backend, CopyWriteFault, CowEvidence, CreatedIdentity,
    FaultInjection, ManifestEntryKind, MaterializationError, MaterializeRequest, RollbackStatus,
    SystemOperation, execute_prepared, materialize_once, prepare_attempt,
};

mod support;

use support::{ControlledTree, TrackedKind};

struct Fixture {
    tree: ControlledTree,
    root: PathBuf,
    source: PathBuf,
    target: PathBuf,
    staging: PathBuf,
    trash: PathBuf,
}

impl Fixture {
    fn create(label: &str) -> io::Result<Self> {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("the experiment crate is nested below the repository root")
            .canonicalize()?;
        let parent = repository_root.join("target/p0-materialize-tests");
        fs::create_dir_all(&parent)?;

        let tree = ControlledTree::create_in(&parent, label)?;
        let root = tree.root_path().to_path_buf();
        let mut fixture = Self {
            tree,
            source: root.join("source"),
            target: root.join("target"),
            staging: root.join("staging"),
            trash: root.join("trash"),
            root,
        };
        for directory in ["source", "target", "staging", "trash"] {
            fixture.tree.create_directory(directory, 0o700)?;
        }
        Ok(fixture)
    }

    fn request(&self) -> MaterializeRequest<'_> {
        MaterializeRequest {
            source: &self.source,
            target: &self.target,
            staging: &self.staging,
            trash: &self.trash,
        }
    }

    fn adopt_created(&mut self, receipt: &AttemptEvidence) -> io::Result<()> {
        let target_relative = self
            .target
            .strip_prefix(&self.root)
            .map_err(|_| io::Error::other("fixture target escaped its controlled root"))?;
        for entry in &receipt.created {
            let CreatedIdentity::Confirmed(identity) = entry.identity else {
                return Err(io::Error::other(
                    "receipt did not confirm a created identity",
                ));
            };
            let kind = match entry.kind {
                ManifestEntryKind::Directory => TrackedKind::Directory,
                ManifestEntryKind::RegularFile => TrackedKind::RegularFile,
                ManifestEntryKind::SymbolicLink => TrackedKind::SymbolicLink,
            };
            self.tree.adopt_confirmed(
                &target_relative.join(&entry.path.display),
                identity.device,
                identity.inode,
                kind,
            )?;
        }
        Ok(())
    }

    fn adopt_remaining(&mut self, receipt: &AttemptEvidence) -> io::Result<()> {
        let target_relative = self
            .target
            .strip_prefix(&self.root)
            .map_err(|_| io::Error::other("fixture target escaped its controlled root"))?;
        for entry in &receipt.created {
            if !receipt
                .rollback
                .remaining
                .iter()
                .any(|remaining| remaining == &entry.path)
            {
                continue;
            }
            let CreatedIdentity::Confirmed(identity) = entry.identity else {
                return Err(io::Error::other(
                    "remaining receipt entry did not confirm its identity",
                ));
            };
            let kind = match entry.kind {
                ManifestEntryKind::Directory => TrackedKind::Directory,
                ManifestEntryKind::RegularFile => TrackedKind::RegularFile,
                ManifestEntryKind::SymbolicLink => TrackedKind::SymbolicLink,
            };
            self.tree.adopt_confirmed(
                &target_relative.join(&entry.path.display),
                identity.device,
                identity.inode,
                kind,
            )?;
        }
        Ok(())
    }

    fn cleanup(self) -> io::Result<()> {
        self.tree.cleanup()
    }
}

#[test]
fn full_copy_materializes_source_git_directory_as_ordinary_content() {
    let mut fixture = Fixture::create("raw-git-directory").expect("create controlled fixture");
    fixture
        .tree
        .create_directory("source/.git", 0o700)
        .expect("create source .git directory");
    fixture
        .tree
        .create_file("source/.git/config", b"[core]\n\tbare = false\n", 0o600)
        .expect("create nested .git file");

    let result = materialize_once(&fixture.request(), Backend::FullCopy);
    let outcome = match result {
        Ok(receipt) => {
            fixture
                .adopt_created(&receipt)
                .expect("adopt materialized identities");
            fs::read(fixture.target.join(".git/config")).map_err(|error| error.to_string())
        }
        Err(failure) => Err(failure.error.to_string()),
    };
    fixture.cleanup().expect("clean controlled fixture");

    assert_eq!(
        outcome.expect("raw .git directory must be treated as ordinary source content"),
        b"[core]\n\tbare = false\n"
    );
}

#[test]
fn apfs_clone_materializes_git_directory_and_nested_files() {
    let mut fixture = Fixture::create("clone-git-directory").expect("create controlled fixture");
    fixture
        .tree
        .create_directory("source/.git", 0o700)
        .expect("create source .git directory");
    fixture
        .tree
        .create_file("source/.git/HEAD", b"ref: refs/heads/main\n", 0o600)
        .expect("create .git HEAD");
    fixture
        .tree
        .create_directory("source/.git/objects", 0o700)
        .expect("create .git objects directory");
    fixture
        .tree
        .create_file("source/.git/objects/object", b"git object bytes\n", 0o600)
        .expect("create nested .git object");

    let receipt = materialize_once(&fixture.request(), Backend::ApfsFileClone)
        .expect("real APFS clone must treat .git as ordinary source content");
    fixture
        .adopt_created(&receipt)
        .expect("adopt materialized identities");
    assert_eq!(receipt.outcome, AttemptOutcome::Succeeded);
    assert_eq!(receipt.cow_evidence, CowEvidence::Confirmed);
    assert_eq!(receipt.clone_calls_succeeded, 2);
    assert_eq!(receipt.ordinary_files_materialized, 2);
    assert!(
        receipt
            .ordinary_files
            .iter()
            .all(|file| file.real_clone_call_succeeded)
    );
    assert_eq!(receipt.source_manifest, receipt.target_manifest);
    assert_eq!(
        fs::read(fixture.target.join(".git/HEAD")).unwrap(),
        b"ref: refs/heads/main\n"
    );
    assert_eq!(
        fs::read(fixture.target.join(".git/objects/object")).unwrap(),
        b"git object bytes\n"
    );
    fixture.cleanup().expect("clean controlled fixture");
}

#[test]
fn both_backends_materialize_non_git_ignored_untracked_and_build_content() {
    for backend in [Backend::ApfsFileClone, Backend::FullCopy] {
        let label = match backend {
            Backend::ApfsFileClone => "raw-non-git-clone",
            Backend::FullCopy => "raw-non-git-copy",
        };
        let mut fixture = Fixture::create(label).expect("create controlled fixture");
        fixture
            .tree
            .create_file("outside-sentinel", b"outside stays unchanged\n", 0o600)
            .expect("create external sentinel");
        fixture
            .tree
            .create_file("source/.gitignore", b"target/\n*.ignored\n", 0o640)
            .expect("create ignore rules as ordinary content");
        fixture
            .tree
            .create_file("source/cache.ignored", b"ignored bytes\n", 0o600)
            .expect("create ignored-named file");
        fixture
            .tree
            .create_file("source/untracked.txt", b"untracked bytes\n", 0o644)
            .expect("create untracked-named file");
        fixture
            .tree
            .create_directory("source/target", 0o750)
            .expect("create build directory");
        fixture
            .tree
            .create_directory("source/target/debug", 0o750)
            .expect("create nested build directory");
        fixture
            .tree
            .create_file("source/target/debug/build.bin", b"build artifact\n", 0o700)
            .expect("create build artifact");
        fixture
            .tree
            .create_symlink("source/link-to-sentinel", b"../outside-sentinel")
            .expect("create external symlink");
        let sentinel_before = fixture
            .tree
            .identity_at(Path::new("outside-sentinel"))
            .expect("record sentinel identity");

        let receipt = materialize_once(&fixture.request(), backend)
            .expect("raw non-Git content must materialize without name filtering");
        fixture
            .adopt_created(&receipt)
            .expect("adopt materialized identities");
        assert_eq!(receipt.source_manifest, receipt.target_manifest);
        assert_eq!(receipt.ordinary_files_materialized, 4);
        assert_eq!(
            receipt.cow_evidence,
            match backend {
                Backend::ApfsFileClone => CowEvidence::Confirmed,
                Backend::FullCopy => CowEvidence::NotUsed,
            }
        );
        assert_eq!(
            receipt.clone_calls_succeeded,
            if backend == Backend::ApfsFileClone {
                4
            } else {
                0
            }
        );
        assert_eq!(
            receipt
                .source_manifest
                .as_ref()
                .expect("source manifest is recorded")
                .entries
                .iter()
                .map(|entry| entry.path.display.as_str())
                .collect::<Vec<_>>(),
            vec![
                ".gitignore",
                "cache.ignored",
                "link-to-sentinel",
                "target",
                "target/debug",
                "target/debug/build.bin",
                "untracked.txt",
            ]
        );
        assert_eq!(
            fs::read(fixture.target.join("cache.ignored")).unwrap(),
            b"ignored bytes\n"
        );
        assert_eq!(
            fs::read(fixture.target.join("untracked.txt")).unwrap(),
            b"untracked bytes\n"
        );
        assert_eq!(
            fs::read(fixture.target.join("target/debug/build.bin")).unwrap(),
            b"build artifact\n"
        );
        assert_eq!(
            fs::read_link(fixture.target.join("link-to-sentinel")).unwrap(),
            PathBuf::from("../outside-sentinel")
        );
        assert_eq!(
            fixture
                .tree
                .identity_at(Path::new("outside-sentinel"))
                .expect("sentinel identity remains readable"),
            sentinel_before
        );
        assert_eq!(
            fs::read(fixture.root.join("outside-sentinel")).unwrap(),
            b"outside stays unchanged\n"
        );
        fixture.cleanup().expect("clean controlled fixture");
    }
}

#[test]
fn both_backends_copy_plain_git_pointer_without_accessing_its_target() {
    for backend in [Backend::ApfsFileClone, Backend::FullCopy] {
        let label = match backend {
            Backend::ApfsFileClone => "git-pointer-clone",
            Backend::FullCopy => "git-pointer-copy",
        };
        let mut fixture = Fixture::create(label).expect("create controlled fixture");
        fixture
            .tree
            .create_file(
                "outside-sentinel",
                b"external git metadata sentinel\n",
                0o600,
            )
            .expect("create external pointer target sentinel");
        fixture
            .tree
            .create_file("source/.git", b"gitdir: ../outside-sentinel\n", 0o600)
            .expect("create ordinary .git pointer text");
        let sentinel_before = fixture
            .tree
            .identity_at(Path::new("outside-sentinel"))
            .expect("record pointer target identity");

        let receipt = materialize_once(&fixture.request(), backend)
            .expect("ordinary .git pointer text must be copied as bytes");
        fixture
            .adopt_created(&receipt)
            .expect("adopt materialized identities");
        assert_eq!(receipt.source_manifest, receipt.target_manifest);
        assert_eq!(receipt.ordinary_files_materialized, 1);
        assert_eq!(
            receipt.cow_evidence,
            match backend {
                Backend::ApfsFileClone => CowEvidence::Confirmed,
                Backend::FullCopy => CowEvidence::NotUsed,
            }
        );
        assert_eq!(
            fs::read(fixture.target.join(".git")).unwrap(),
            b"gitdir: ../outside-sentinel\n"
        );
        assert_eq!(
            fixture
                .tree
                .identity_at(Path::new("outside-sentinel"))
                .expect("pointer target remains readable"),
            sentinel_before
        );
        assert_eq!(
            fs::read(fixture.root.join("outside-sentinel")).unwrap(),
            b"external git metadata sentinel\n"
        );
        fixture.cleanup().expect("clean controlled fixture");
    }
}

#[test]
fn clone_empty_and_directory_symlink_only_trees_report_cow_not_used() {
    let empty = Fixture::create("empty-raw-tree").expect("create controlled empty fixture");
    let receipt = materialize_once(&empty.request(), Backend::ApfsFileClone)
        .expect("empty tree materialization succeeds");
    assert_eq!(receipt.outcome, AttemptOutcome::Succeeded);
    assert_eq!(receipt.cow_evidence, CowEvidence::NotUsed);
    assert_eq!(receipt.clone_calls_succeeded, 0);
    assert_eq!(receipt.ordinary_files_materialized, 0);
    assert!(receipt.created.is_empty());
    empty.cleanup().expect("clean controlled empty fixture");

    let mut links = Fixture::create("directory-link-tree")
        .expect("create controlled directory and symlink fixture");
    links
        .tree
        .create_file("outside-sentinel", b"outside stays unchanged\n", 0o600)
        .expect("create external sentinel");
    links
        .tree
        .create_directory("source/empty", 0o750)
        .expect("create empty source directory");
    links
        .tree
        .create_directory("source/nested", 0o700)
        .expect("create nested source directory");
    links
        .tree
        .create_symlink("source/nested/link", b"../../outside-sentinel")
        .expect("create external symlink");
    let receipt = materialize_once(&links.request(), Backend::ApfsFileClone)
        .expect("directory and symlink-only tree materialization succeeds");
    links
        .adopt_created(&receipt)
        .expect("adopt materialized identities");
    assert_eq!(receipt.outcome, AttemptOutcome::Succeeded);
    assert_eq!(receipt.cow_evidence, CowEvidence::NotUsed);
    assert_eq!(receipt.clone_calls_succeeded, 0);
    assert_eq!(receipt.ordinary_files_materialized, 0);
    assert_eq!(receipt.created.len(), 3);
    assert_eq!(
        fs::read_link(links.target.join("nested/link")).unwrap(),
        PathBuf::from("../../outside-sentinel")
    );
    assert_eq!(
        fs::read(links.root.join("outside-sentinel")).unwrap(),
        b"outside stays unchanged\n"
    );
    links.cleanup().expect("clean controlled fixture");
}

#[test]
fn full_copy_failure_inside_git_rolls_back_the_registered_git_tree() {
    let mut fixture = Fixture::create("git-rollback").expect("create controlled fixture");
    fixture
        .tree
        .create_directory("source/.git", 0o700)
        .expect("create source .git directory");
    fixture
        .tree
        .create_file("source/.git/HEAD", b"ref: refs/heads/main\n", 0o600)
        .expect("create .git HEAD");
    fixture
        .tree
        .create_directory("source/.git/objects", 0o700)
        .expect("create .git objects directory");
    fixture
        .tree
        .create_file("source/.git/objects/pack.bin", b"pack bytes\n", 0o600)
        .expect("create nested .git payload");
    let prepared = prepare_attempt(&fixture.request(), Backend::FullCopy)
        .expect("prepare Full Copy for raw .git tree");

    let failure = execute_prepared(
        &prepared,
        &FaultInjection {
            copy_write: Some(CopyWriteFault {
                ordinary_file_index: 1,
                after_bytes: 1,
                errno: libc::ENOSPC,
            }),
            ..FaultInjection::default()
        },
    )
    .expect_err("injected write failure must interrupt copying inside .git");
    assert!(matches!(
        failure.error,
        MaterializationError::SystemCall {
            ref operation,
            errno: Some(libc::ENOSPC),
            injected: true,
            ..
        } if operation == &SystemOperation::Named("write target file".to_owned())
    ));
    assert_eq!(failure.evidence.outcome, AttemptOutcome::Partial);
    assert_eq!(
        failure.evidence.rollback.status,
        RollbackStatus::ConfirmedBaseline
    );
    assert_eq!(
        failure
            .evidence
            .created
            .iter()
            .map(|entry| entry.path.display.as_str())
            .collect::<Vec<_>>(),
        vec![".git", ".git/HEAD", ".git/objects", ".git/objects/pack.bin"]
    );
    assert_eq!(
        failure
            .evidence
            .rollback
            .removed
            .iter()
            .map(|path| path.display.as_str())
            .collect::<Vec<_>>(),
        vec![".git/objects/pack.bin", ".git/objects", ".git/HEAD", ".git"]
    );
    assert!(failure.evidence.rollback.remaining.is_empty());
    assert!(failure.evidence.rollback.problems.is_empty());
    assert_eq!(fs::read_dir(&fixture.target).unwrap().count(), 0);
    fixture.cleanup().expect("clean controlled fixture");
}

#[test]
fn both_backends_reject_nonempty_target_without_overwriting() {
    for backend in [Backend::ApfsFileClone, Backend::FullCopy] {
        for name in [".git", "existing.txt"] {
            let label = format!("nonempty-{backend:?}-{name}");
            let mut fixture = Fixture::create(&label).expect("create controlled fixture");
            fixture
                .tree
                .create_file("source/payload", b"new source bytes\n", 0o600)
                .expect("create source payload");
            let target_relative = PathBuf::from("target").join(name);
            fixture
                .tree
                .create_file(&target_relative, b"existing target bytes\n", 0o640)
                .expect("create existing target entry");
            let target_before = fixture
                .tree
                .identity_at(&target_relative)
                .expect("record existing target identity");

            let failure = materialize_once(&fixture.request(), backend)
                .expect_err("every nonempty target must be rejected");
            assert!(matches!(
                failure.error,
                MaterializationError::UnknownTargetEntry { ref path } if path.display == name
            ));
            assert!(failure.evidence.created.is_empty());
            assert_eq!(failure.evidence.rollback.status, RollbackStatus::NotNeeded);
            assert_eq!(
                fixture
                    .tree
                    .identity_at(&target_relative)
                    .expect("existing target identity remains"),
                target_before
            );
            assert_eq!(
                fs::read(fixture.target.join(name)).unwrap(),
                b"existing target bytes\n"
            );
            assert!(!fixture.target.join("payload").exists());
            fixture.cleanup().expect("clean controlled fixture");
        }
    }
}

#[test]
fn apfs_clone_preserves_root_directory_and_regular_file_mtime_and_permissions() {
    assert_backend_preserves_mtime_and_permissions(Backend::ApfsFileClone);
}

#[test]
fn full_copy_preserves_root_directory_and_regular_file_mtime_and_permissions() {
    assert_backend_preserves_mtime_and_permissions(Backend::FullCopy);
}

fn assert_backend_preserves_mtime_and_permissions(backend: Backend) {
    let label = match backend {
        Backend::ApfsFileClone => "metadata-clone",
        Backend::FullCopy => "metadata-copy",
    };
    let mut fixture = Fixture::create(label).expect("create controlled fixture");
    fixture
        .tree
        .create_directory("source/nested", 0o700)
        .expect("create nested source directory");
    fixture
        .tree
        .create_directory("source/empty", 0o700)
        .expect("create empty source directory");
    fixture
        .tree
        .create_file("source/root-file", b"root bytes\n", 0o400)
        .expect("create source root file");
    fixture
        .tree
        .create_file("source/nested/leaf", b"nested bytes\n", 0o440)
        .expect("create nested source file");
    fixture
        .tree
        .set_mode(Path::new("source/nested"), 0o550)
        .expect("set nested source directory permissions");
    fixture
        .tree
        .set_mode(Path::new("source/empty"), 0o510)
        .expect("set empty source directory permissions");

    for (path, seconds, nanoseconds) in [
        ("source/root-file", 1_651_000_001, 123_456_789),
        ("source/nested/leaf", 1_652_000_002, 234_567_890),
        ("source/empty", 1_653_000_003, 345_678_901),
        ("source/nested", 1_654_000_004, 456_789_012),
        ("source", 1_655_000_005, 567_890_123),
    ] {
        fixture
            .tree
            .set_modified_time(Path::new(path), seconds, nanoseconds)
            .expect("set controlled source mtime");
    }
    fixture
        .tree
        .set_mode(Path::new("source"), 0o555)
        .expect("make source root read-only but searchable");

    let result = materialize_once(&fixture.request(), backend);
    let outcome = match result {
        Ok(receipt) => {
            fixture
                .adopt_created(&receipt)
                .expect("adopt materialized identities");
            let source_metadata = [
                "source",
                "source/empty",
                "source/nested",
                "source/nested/leaf",
                "source/root-file",
            ]
            .map(|path| {
                fixture
                    .tree
                    .mode_and_modified_time_at(Path::new(path))
                    .expect("observe source metadata")
            });
            let target_metadata = [
                "target",
                "target/empty",
                "target/nested",
                "target/nested/leaf",
                "target/root-file",
            ]
            .map(|path| {
                fixture
                    .tree
                    .mode_and_modified_time_at(Path::new(path))
                    .expect("observe target metadata")
            });
            for path in ["target", "target/empty", "target/nested"] {
                fixture
                    .tree
                    .set_mode(Path::new(path), 0o700)
                    .expect("restore writable target directory for controlled cleanup");
            }
            Ok((source_metadata, target_metadata))
        }
        Err(failure) => Err(failure.error.to_string()),
    };
    for path in ["source", "source/empty", "source/nested"] {
        fixture
            .tree
            .set_mode(Path::new(path), 0o700)
            .expect("restore writable source directory for controlled cleanup");
    }
    fixture.cleanup().expect("clean controlled fixture");

    let (source_metadata, target_metadata) =
        outcome.expect("metadata-preserving materialization must succeed");
    assert_eq!(
        source_metadata[0].0, 0o555,
        "source root test is meaningful"
    );
    assert_eq!(target_metadata, source_metadata, "backend {backend:?}");
}

#[test]
fn prepared_attempt_rejects_root_directory_and_regular_file_mtime_changes() {
    for (label, changed_path) in [
        ("source-root-mtime-change", "source"),
        ("source-directory-mtime-change", "source/nested"),
        ("source-file-mtime-change", "source/nested/file"),
    ] {
        let mut fixture = Fixture::create(label).expect("create controlled fixture");
        fixture
            .tree
            .create_directory("source/nested", 0o700)
            .expect("create source directory");
        fixture
            .tree
            .create_file("source/nested/file", b"source bytes\n", 0o600)
            .expect("create source file");
        let prepared = prepare_attempt(&fixture.request(), Backend::FullCopy)
            .expect("prepare metadata-observing attempt");
        fixture
            .tree
            .set_modified_time(Path::new(changed_path), 1_670_000_001, 456_789_012)
            .expect("change guaranteed source metadata after preparation");

        let failure = execute_prepared(&prepared, &FaultInjection::default())
            .expect_err("guaranteed source metadata change must stale the prepared attempt");
        assert!(matches!(
            failure.error,
            MaterializationError::SourceChanged { ref path } if path == &fixture.source
        ));
        assert_eq!(failure.evidence.outcome, AttemptOutcome::Failed);
        assert!(!failure.evidence.target_root_metadata_started);
        assert!(failure.evidence.created.is_empty());
        assert_eq!(
            failure.evidence.rollback.status,
            RollbackStatus::ConfirmedBaseline
        );
        assert!(failure.evidence.rollback.problems.is_empty());
        assert_eq!(fs::read_dir(&fixture.target).unwrap().count(), 0);
        fixture.cleanup().expect("clean controlled fixture");
    }
}

#[test]
fn mtime_failure_before_root_metadata_safely_stops_at_read_only_created_parent() {
    let mut fixture = Fixture::create("mtime-rollback-read-only")
        .expect("create controlled metadata-failure fixture");
    fixture
        .tree
        .create_directory("source/a-locked", 0o700)
        .expect("create source directory");
    fixture
        .tree
        .create_file("source/a-locked/child", b"child bytes\n", 0o600)
        .expect("create child in source directory");
    fixture
        .tree
        .set_mode(Path::new("source/a-locked"), 0o555)
        .expect("make source directory non-writable");
    fixture
        .tree
        .create_file("source/z-file", b"later bytes\n", 0o600)
        .expect("create later source file");
    let prepared = prepare_attempt(&fixture.request(), Backend::FullCopy)
        .expect("prepare metadata failure attempt");

    let failure = execute_prepared(
        &prepared,
        &FaultInjection {
            mtime_errno_at: Some((2, libc::ENOSPC)),
            ..FaultInjection::default()
        },
    )
    .expect_err("injected mtime failure must abort materialization");
    assert!(matches!(
        failure.error,
        MaterializationError::SystemCall {
            ref operation,
            errno: Some(libc::ENOSPC),
            injected: true,
            ..
        } if operation == &SystemOperation::Named("futimens target mtime".to_owned())
    ));
    assert_eq!(failure.evidence.outcome, AttemptOutcome::Partial);
    assert!(!failure.evidence.target_root_metadata_started);
    assert_eq!(failure.evidence.rollback.status, RollbackStatus::Incomplete);
    assert_eq!(
        failure
            .evidence
            .rollback
            .removed
            .iter()
            .map(|path| path.display.as_str())
            .collect::<Vec<_>>(),
        vec!["z-file"]
    );
    assert_eq!(
        failure
            .evidence
            .rollback
            .remaining
            .iter()
            .map(|path| path.display.as_str())
            .collect::<Vec<_>>(),
        vec!["a-locked", "a-locked/child"]
    );
    assert!(failure.evidence.rollback.problems.iter().any(|problem| {
        problem.operation == "unlinkat rollback entry" && problem.errno == Some(libc::EACCES)
    }));

    fixture
        .adopt_remaining(&failure.evidence)
        .expect("adopt explicitly retained target identities");
    fixture
        .tree
        .set_mode(Path::new("source/a-locked"), 0o700)
        .expect("restore source directory for controlled cleanup");
    fixture
        .tree
        .set_mode(Path::new("target/a-locked"), 0o700)
        .expect("restore retained target directory for controlled cleanup");
    fixture.cleanup().expect("clean controlled fixture");
}

#[test]
fn empty_tree_root_mtime_failure_is_partial_and_preserves_the_target() {
    let fixture = Fixture::create("empty-root-mtime-failure")
        .expect("create controlled empty metadata fixture");
    let prepared = prepare_attempt(&fixture.request(), Backend::FullCopy)
        .expect("prepare empty tree materialization");

    let failure = execute_prepared(
        &prepared,
        &FaultInjection {
            mtime_errno_at: Some((0, libc::ENOSPC)),
            ..FaultInjection::default()
        },
    )
    .expect_err("injected root mtime failure must abort empty-tree materialization");
    assert!(matches!(
        failure.error,
        MaterializationError::SystemCall {
            ref operation,
            errno: Some(libc::ENOSPC),
            injected: true,
            ..
        } if operation == &SystemOperation::Named("futimens target mtime".to_owned())
    ));
    assert_eq!(failure.evidence.outcome, AttemptOutcome::Partial);
    assert!(failure.evidence.target_root_metadata_started);
    assert!(failure.evidence.created.is_empty());
    assert_eq!(failure.evidence.rollback.status, RollbackStatus::Incomplete);
    assert!(failure.evidence.rollback.removed.is_empty());
    assert!(failure.evidence.rollback.remaining.is_empty());
    assert_eq!(failure.evidence.rollback.problems.len(), 1);
    assert_eq!(
        failure.evidence.rollback.problems[0].operation,
        "preserve target after root metadata stage failure"
    );
    assert!(failure.evidence.target_manifest.is_some());
    assert_eq!(fs::read_dir(&fixture.target).unwrap().count(), 0);
    fixture.cleanup().expect("clean controlled fixture");
}
