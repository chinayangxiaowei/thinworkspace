#![deny(unsafe_code)]

use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};

use thinws_p0_materialize::{
    AttemptEvidence, AttemptOutcome, Backend, ClonePreflightOverride, CopyWriteFault, CowEvidence,
    CreatedIdentity, FallbackPolicy, FallbackReason, FaultInjection, ManifestEntryKind,
    MaterializationError, MaterializeRequest, PolicyExperimentOptions, RequestedMode,
    RollbackStatus, SystemOperation, execute_prepared, materialize_once, prepare_attempt,
    run_policy_experiment,
};
use thinws_p0_probe::{Evidence, FileIdentity, inspect_path};

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
    fn create_fixed_tree() -> io::Result<Self> {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("the experiment crate is nested below the repository root")
            .canonicalize()?;
        let parent = repository_root.join("target/p0-materialize-tests");
        fs::create_dir_all(&parent)?;

        Self::create_fixed_tree_in(&parent, "same-volume")
    }

    fn create_empty_tree() -> io::Result<Self> {
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("the experiment crate is nested below the repository root")
            .canonicalize()?;
        let parent = repository_root.join("target/p0-materialize-tests");
        fs::create_dir_all(&parent)?;

        Self::create_empty_tree_in(&parent, "empty-source")
    }

    fn create_fixed_tree_in(existing_parent: &Path, label: &str) -> io::Result<Self> {
        let mut fixture = Self::create_empty_tree_in(existing_parent, label)?;
        fixture.tree.create_file("source/empty", b"", 0o640)?;
        fixture
            .tree
            .create_file("source/executable.sh", b"#!/bin/sh\necho p0\n", 0o751)?;
        fixture.tree.create_directory("source/nested", 0o750)?;
        fixture.tree.create_file(
            "source/nested/payload.bin",
            b"\x00\xffP0-02 fixed payload\n",
            0o600,
        )?;
        fixture
            .tree
            .create_symlink("source/link-to-sentinel", b"../outside-sentinel")?;
        Ok(fixture)
    }

    fn create_empty_tree_in(existing_parent: &Path, label: &str) -> io::Result<Self> {
        let tree = ControlledTree::create_in(existing_parent, label)?;
        let root = tree.root_path().to_path_buf();
        let source = root.join("source");
        let target = root.join("target");
        let staging = root.join("staging");
        let trash = root.join("trash");
        let mut fixture = Self {
            tree,
            root,
            source,
            target,
            staging,
            trash,
        };
        for directory in ["source", "target", "staging", "trash"] {
            fixture.tree.create_directory(directory, 0o700)?;
        }
        fixture
            .tree
            .create_file("outside-sentinel", b"outside stays unchanged\n", 0o600)?;
        fixture
            .tree
            .create_file("target/.git", b"protected control sentinel\n", 0o600)?;
        Ok(fixture)
    }

    fn request(&self) -> MaterializeRequest<'_> {
        let target_relative = self
            .target
            .strip_prefix(&self.root)
            .expect("fixture target remains within its private root");
        let protected_git = self.tree.tracked().get(&target_relative.join(".git"));
        MaterializeRequest {
            source: &self.source,
            target: &self.target,
            staging: &self.staging,
            trash: &self.trash,
            protected_git: protected_git.map(|identity| FileIdentity {
                device: identity.device,
                inode: identity.inode,
            }),
        }
    }

    fn move_target_under_source(&mut self) -> io::Result<()> {
        self.tree.create_directory("source/target-inside", 0o700)?;
        self.target = self.source.join("target-inside");
        Ok(())
    }

    fn adopt_materialized_fixed_tree(&mut self, receipt: &AttemptEvidence) -> io::Result<()> {
        self.adopt_created(receipt)
    }

    fn adopt_created(&mut self, receipt: &AttemptEvidence) -> io::Result<()> {
        self.adopt_created_paths(receipt, receipt.created.iter().map(|entry| &entry.path))
    }

    fn adopt_remaining_created(&mut self, receipt: &AttemptEvidence) -> io::Result<()> {
        self.adopt_created_paths(receipt, receipt.rollback.remaining.iter())
    }

    fn adopt_created_paths<'a>(
        &mut self,
        receipt: &AttemptEvidence,
        paths: impl IntoIterator<Item = &'a thinws_p0_materialize::EncodedRelativePath>,
    ) -> io::Result<()> {
        let target_relative = self
            .target
            .strip_prefix(&self.root)
            .map_err(|_| io::Error::other("fixture target escaped its controlled root"))?;
        for path in paths {
            let evidence = receipt
                .created
                .iter()
                .find(|entry| entry.path == *path)
                .ok_or_else(|| io::Error::other("rollback path omitted from created ledger"))?;
            let relative = target_relative.join(&evidence.path.display);
            let CreatedIdentity::Confirmed(expected) = evidence.identity else {
                return Err(io::Error::other(
                    "receipt did not confirm a created identity",
                ));
            };
            self.tree.adopt_confirmed(
                &relative,
                expected.device,
                expected.inode,
                match evidence.kind {
                    ManifestEntryKind::Directory => TrackedKind::Directory,
                    ManifestEntryKind::RegularFile => TrackedKind::RegularFile,
                    ManifestEntryKind::SymbolicLink => TrackedKind::SymbolicLink,
                },
            )?;
        }
        Ok(())
    }

    fn cleanup(self) -> io::Result<()> {
        self.tree.cleanup()
    }

    fn verify_roots(&self) -> io::Result<()> {
        self.tree.verify_roots()
    }

    fn metadata_relative(&self, relative: &Path) -> io::Result<support::TrackedIdentity> {
        self.tree.identity_at(relative)
    }
}

fn assert_fixed_manifest(receipt: &thinws_p0_materialize::AttemptEvidence) {
    let source = receipt
        .source_manifest
        .as_ref()
        .expect("source manifest must be recorded");
    let target = receipt
        .target_manifest
        .as_ref()
        .expect("target manifest must be recorded");
    assert_eq!(source, target);
    assert_eq!(source.entries.len(), 5);
    assert_eq!(
        source
            .entries
            .iter()
            .map(|entry| (entry.path.display.as_str(), entry.kind))
            .collect::<Vec<_>>(),
        vec![
            ("empty", ManifestEntryKind::RegularFile),
            ("executable.sh", ManifestEntryKind::RegularFile),
            ("link-to-sentinel", ManifestEntryKind::SymbolicLink),
            ("nested", ManifestEntryKind::Directory),
            ("nested/payload.bin", ManifestEntryKind::RegularFile),
        ]
    );
    assert_eq!(source.entries[0].length, 0);
    assert_eq!(source.entries[0].permissions, Some(0o640));
    assert_eq!(source.entries[1].permissions, Some(0o751));
    assert_eq!(source.entries[2].permissions, None);
    assert_eq!(
        source.entries[2].length,
        b"../outside-sentinel".len() as u64
    );
    assert_eq!(source.entries[3].permissions, Some(0o750));
    assert_eq!(source.entries[4].length, 22);
    assert_eq!(
        source
            .entries
            .iter()
            .map(|entry| entry.path.bytes_hex.as_str())
            .collect::<Vec<_>>(),
        vec![
            "656d707479",
            "65786563757461626c652e7368",
            "6c696e6b2d746f2d73656e74696e656c",
            "6e6573746564",
            "6e65737465642f7061796c6f61642e62696e",
        ]
    );
    assert_eq!(
        source
            .entries
            .iter()
            .map(|entry| entry.content_digest_hex.clone())
            .collect::<Vec<_>>(),
        vec![
            Some(blake3::hash(b"").to_hex().to_string()),
            Some(blake3::hash(b"#!/bin/sh\necho p0\n").to_hex().to_string(),),
            Some(blake3::hash(b"../outside-sentinel").to_hex().to_string(),),
            None,
            Some(
                blake3::hash(b"\x00\xffP0-02 fixed payload\n")
                    .to_hex()
                    .to_string(),
            ),
        ]
    );
}

fn assert_control_and_outside_sentinels_unchanged(fixture: &Fixture) {
    assert_eq!(
        fs::read(fixture.target.join(".git")).unwrap(),
        b"protected control sentinel\n"
    );
    assert_eq!(
        fs::read(fixture.root.join("outside-sentinel")).unwrap(),
        b"outside stays unchanged\n"
    );
}

#[test]
fn actual_same_volume_clone_materializes_fixed_tree_with_complete_manifests() {
    let mut fixture = Fixture::create_fixed_tree().expect("create controlled fixed fixture");
    let result = materialize_once(&fixture.request(), Backend::ApfsFileClone);
    let target_payload = fs::read(fixture.target.join("nested/payload.bin"));
    let target_link = fs::read_link(fixture.target.join("link-to-sentinel"));
    let control = fs::read(fixture.target.join(".git"));
    let outside = fs::read(fixture.root.join("outside-sentinel"));

    let receipt = result.expect("real same-volume APFS fixed-tree clone should succeed");
    assert_eq!(receipt.backend, Backend::ApfsFileClone);
    assert_eq!(receipt.outcome, AttemptOutcome::Succeeded);
    assert_eq!(receipt.cow_evidence, CowEvidence::Confirmed);
    assert_eq!(receipt.clone_calls_succeeded, 3);
    assert_eq!(receipt.ordinary_files_materialized, 3);
    assert!(receipt.ordinary_files.iter().all(|file| {
        file.real_clone_call_succeeded && file.source_identity != file.target_identity
    }));
    assert_fixed_manifest(&receipt);
    fixture
        .adopt_materialized_fixed_tree(&receipt)
        .expect("receipt identities must match independent observations");
    assert_eq!(
        target_payload.expect("read target payload"),
        b"\x00\xffP0-02 fixed payload\n"
    );
    assert_eq!(
        target_link
            .expect("read materialized link")
            .as_os_str()
            .as_bytes(),
        b"../outside-sentinel"
    );
    assert_eq!(
        control.expect("read .git sentinel"),
        b"protected control sentinel\n"
    );
    assert_eq!(
        outside.expect("read outside sentinel"),
        b"outside stays unchanged\n"
    );

    fs::write(
        fixture.target.join("nested/payload.bin"),
        b"target-only mutation data",
    )
    .unwrap();
    assert_eq!(
        fs::read(fixture.source.join("nested/payload.bin")).unwrap(),
        b"\x00\xffP0-02 fixed payload\n"
    );
    fs::write(
        fixture.source.join("executable.sh"),
        b"source-only change!\n",
    )
    .unwrap();
    assert_eq!(
        fs::read(fixture.target.join("executable.sh")).unwrap(),
        b"#!/bin/sh\necho p0\n"
    );

    fixture.cleanup().expect("remove verified fixed fixture");
}

#[test]
fn non_ascii_utf8_relative_name_and_content_digest_are_preserved_by_full_copy() {
    let mut fixture = Fixture::create_empty_tree().expect("create controlled empty fixture");
    let raw_name = OsString::from("数据.bin");
    let source_relative = PathBuf::from("source").join(&raw_name);
    let target_relative = PathBuf::from("target").join(&raw_name);
    let contents = b"non-UTF-8 name payload\n";
    fixture
        .tree
        .create_file(&source_relative, contents, 0o641)
        .expect("create fixture file with a non-ASCII UTF-8 name");

    let receipt = materialize_once(&fixture.request(), Backend::FullCopy)
        .expect("Full Copy must preserve the raw relative name");
    assert_eq!(receipt.cow_evidence, CowEvidence::NotUsed);
    assert_eq!(receipt.clone_calls_succeeded, 0);
    let source = receipt
        .source_manifest
        .as_ref()
        .expect("source manifest is present");
    let target = receipt
        .target_manifest
        .as_ref()
        .expect("target manifest is present");
    assert_eq!(source, target);
    assert_eq!(source.entries.len(), 1);
    assert_eq!(source.entries[0].path.bytes_hex, "e695b0e68dae2e62696e");
    assert_eq!(source.entries[0].kind, ManifestEntryKind::RegularFile);
    assert_eq!(source.entries[0].permissions, Some(0o641));
    assert_eq!(source.entries[0].length, contents.len() as u64);
    assert_eq!(
        source.entries[0].content_digest_hex,
        Some(blake3::hash(contents).to_hex().to_string())
    );
    assert_eq!(receipt.created.len(), 1);
    assert_eq!(receipt.created[0].path.bytes_hex, "e695b0e68dae2e62696e");
    let CreatedIdentity::Confirmed(created) = receipt.created[0].identity else {
        panic!("created raw-name file identity must be confirmed");
    };
    fixture
        .tree
        .adopt_confirmed(
            &target_relative,
            created.device,
            created.inode,
            TrackedKind::RegularFile,
        )
        .expect("raw-name receipt identity matches the original OsString path");
    assert_eq!(
        fs::read(fixture.root.join(&target_relative)).unwrap(),
        contents
    );
    assert_control_and_outside_sentinels_unchanged(&fixture);
    fixture.cleanup().expect("remove verified raw-name fixture");
}

#[test]
fn empty_source_clone_succeeds_without_claiming_cow_confirmation() {
    let fixture = Fixture::create_empty_tree().expect("create controlled empty fixture");
    let receipt = materialize_once(&fixture.request(), Backend::ApfsFileClone)
        .expect("an empty source is a successful no-op materialization");
    assert_eq!(receipt.outcome, AttemptOutcome::Succeeded);
    assert_eq!(receipt.backend, Backend::ApfsFileClone);
    assert_eq!(receipt.clone_calls_succeeded, 0);
    assert_eq!(receipt.ordinary_files_materialized, 0);
    assert!(receipt.created.is_empty());
    assert!(receipt.ordinary_files.is_empty());
    assert_eq!(receipt.cow_evidence, CowEvidence::Unknown);
    assert!(
        receipt
            .source_manifest
            .as_ref()
            .is_some_and(|manifest| manifest.entries.is_empty())
    );
    assert_eq!(receipt.source_manifest, receipt.target_manifest);
    assert_eq!(fs::read_dir(&fixture.target).unwrap().count(), 1);
    assert_control_and_outside_sentinels_unchanged(&fixture);
    fixture.cleanup().expect("remove verified empty fixture");
}

#[test]
fn partial_clone_failure_rolls_back_exact_created_identity_set_in_reverse() {
    let fixture = Fixture::create_fixed_tree().expect("create controlled fixed fixture");
    let prepared = prepare_attempt(&fixture.request(), Backend::ApfsFileClone)
        .expect("prepare fixed clone attempt");
    let faults = FaultInjection {
        clone_errno_at: Some((2, libc::EIO)),
        ..FaultInjection::default()
    };

    let failure = execute_prepared(&prepared, &faults).expect_err("third clone must fail");
    assert!(matches!(
        failure.error,
        MaterializationError::SystemCall {
            ref operation,
            errno: Some(libc::EIO),
            injected: true,
            ..
        } if *operation == SystemOperation::CloneFileAt
    ));
    assert_eq!(failure.evidence.outcome, AttemptOutcome::Partial);
    assert_eq!(failure.evidence.cow_evidence, CowEvidence::Unknown);
    assert_eq!(failure.evidence.clone_calls_succeeded, 2);
    assert_eq!(failure.evidence.created.len(), 4);
    assert_eq!(
        failure
            .evidence
            .created
            .iter()
            .map(|entry| entry.path.display.as_str())
            .collect::<Vec<_>>(),
        vec!["empty", "executable.sh", "link-to-sentinel", "nested"]
    );
    assert_eq!(
        failure.evidence.rollback.status,
        RollbackStatus::ConfirmedBaseline,
        "rollback evidence: {:#?}",
        failure.evidence.rollback
    );
    assert_eq!(
        failure
            .evidence
            .rollback
            .removed
            .iter()
            .map(|path| path.display.as_str())
            .collect::<Vec<_>>(),
        vec!["nested", "link-to-sentinel", "executable.sh", "empty"]
    );
    assert!(failure.evidence.rollback.remaining.is_empty());
    assert!(failure.evidence.rollback.problems.is_empty());
    assert_eq!(
        fs::read_dir(&fixture.target)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>(),
        vec![".git"]
    );
    assert_eq!(
        fs::read(fixture.target.join(".git")).unwrap(),
        b"protected control sentinel\n"
    );
    assert_eq!(
        fs::read(fixture.root.join("outside-sentinel")).unwrap(),
        b"outside stays unchanged\n"
    );
    fixture.cleanup().expect("remove verified fixed fixture");
}

#[test]
fn prepare_rejects_target_nested_under_source_without_writing() {
    let mut fixture = Fixture::create_fixed_tree().expect("create controlled fixed fixture");
    fixture
        .move_target_under_source()
        .expect("create the controlled overlapping target");

    let result = prepare_attempt(&fixture.request(), Backend::ApfsFileClone);
    assert!(matches!(
        result,
        Err(MaterializationError::OverlappingRoots { .. })
    ));
    assert_eq!(
        fs::read_dir(&fixture.target).unwrap().count(),
        0,
        "prepare must not write into a target nested under the source"
    );
    fixture.cleanup().expect("remove verified fixed fixture");
}

#[test]
fn prepare_rejects_equal_roots_and_source_nested_under_target_without_writing() {
    let equal = Fixture::create_fixed_tree().expect("create controlled fixed fixture");
    let baseline = equal
        .tree
        .observed_tree()
        .expect("record controlled fixture before equal-root prepare");
    let mut equal_request = equal.request();
    equal_request.target = &equal.source;
    equal_request.protected_git = None;
    assert!(matches!(
        prepare_attempt(&equal_request, Backend::ApfsFileClone),
        Err(MaterializationError::OverlappingRoots { .. })
    ));
    assert_eq!(
        equal
            .tree
            .observed_tree()
            .expect("equal-root prepare must not mutate the fixture"),
        baseline
    );
    assert_control_and_outside_sentinels_unchanged(&equal);
    equal.cleanup().expect("remove verified fixed fixture");

    let mut nested = Fixture::create_fixed_tree().expect("create controlled fixed fixture");
    nested
        .tree
        .create_directory(Path::new("target/source-inside"), 0o700)
        .expect("create controlled source below target");
    nested.source = nested.target.join("source-inside");
    let baseline = nested
        .tree
        .observed_tree()
        .expect("record controlled fixture before nested-source prepare");
    assert!(matches!(
        prepare_attempt(&nested.request(), Backend::FullCopy),
        Err(MaterializationError::OverlappingRoots { .. })
    ));
    assert_eq!(
        nested
            .tree
            .observed_tree()
            .expect("nested-source prepare must not mutate the fixture"),
        baseline
    );
    assert_control_and_outside_sentinels_unchanged(&nested);
    nested.cleanup().expect("remove verified fixed fixture");
}

#[test]
fn fixture_cleanup_preflight_rejects_moved_parent_before_any_deletion() {
    let fixture = Fixture::create_fixed_tree().expect("create controlled fixed fixture");
    let moved_name = fixture
        .tree
        .move_fixture_parent_exclusive()
        .expect("exclusively move only this test's unique fixture parent");

    let rejection = fixture
        .verify_roots()
        .expect_err("cleanup preflight must reject a moved parent");
    assert!(rejection.to_string().contains("No such file"));
    let held_tree = fixture
        .tree
        .observed_tree()
        .expect("held root remains readable");
    assert_eq!(
        &held_tree,
        fixture.tree.tracked(),
        "failed cleanup preflight must delete nothing"
    );

    fixture
        .tree
        .restore_fixture_parent_exclusive(&moved_name)
        .expect("exclusively restore exact fixture parent after the refusal assertion");
    fixture
        .verify_roots()
        .expect("restored parent identity must match");
    fixture.cleanup().expect("remove verified fixed fixture");
}

#[test]
fn prepare_requires_explicit_matching_git_control_identity() {
    let fixture = Fixture::create_fixed_tree().expect("create controlled fixed fixture");
    let mut undeclared = fixture.request();
    undeclared.protected_git = None;
    assert!(matches!(
        prepare_attempt(&undeclared, Backend::ApfsFileClone),
        Err(MaterializationError::UnknownTargetEntry { ref path })
            if path.display == ".git"
    ));

    let mut wrong_identity = fixture.request();
    let declared = wrong_identity
        .protected_git
        .as_mut()
        .expect("fixture declares .git");
    declared.inode = declared.inode.wrapping_add(1);
    assert!(matches!(
        prepare_attempt(&wrong_identity, Backend::ApfsFileClone),
        Err(MaterializationError::TargetChanged { ref path })
            if path == &fixture.target.join(".git")
    ));
    assert_eq!(fs::read_dir(&fixture.target).unwrap().count(), 1);
    assert_eq!(
        fs::read(fixture.target.join(".git")).unwrap(),
        b"protected control sentinel\n"
    );
    fixture.cleanup().expect("remove verified fixed fixture");
}

#[test]
fn prepare_rejects_unknown_target_and_special_source_without_writing() {
    let mut unknown = Fixture::create_fixed_tree().expect("create controlled fixed fixture");
    unknown
        .tree
        .create_file(Path::new("target/unplanned"), b"do not overwrite\n", 0o600)
        .expect("create fixture-owned unknown target entry");
    let result = prepare_attempt(&unknown.request(), Backend::ApfsFileClone);
    assert!(matches!(
        result,
        Err(MaterializationError::UnknownTargetEntry { ref path })
            if path.display == "unplanned"
    ));
    assert_eq!(
        fs::read(unknown.target.join("unplanned")).unwrap(),
        b"do not overwrite\n"
    );
    assert_control_and_outside_sentinels_unchanged(&unknown);
    unknown.cleanup().expect("remove verified fixed fixture");

    let mut special = Fixture::create_fixed_tree_in(Path::new("/private/tmp"), "special-source")
        .expect("create controlled short-path fixture");
    let socket_path = special.source.join("special.sock");
    let listener = UnixListener::bind(&socket_path).expect("create fixture-owned Unix socket");
    special
        .tree
        .track_existing(Path::new("source/special.sock"))
        .expect("register fixture-created socket identity");
    let result = prepare_attempt(&special.request(), Backend::FullCopy);
    assert!(matches!(
        result,
        Err(MaterializationError::UnsupportedSourceEntry { ref path, ref kind })
            if path == &socket_path && kind == "special"
    ));
    assert_eq!(fs::read_dir(&special.target).unwrap().count(), 1);
    assert_control_and_outside_sentinels_unchanged(&special);
    drop(listener);
    special.cleanup().expect("remove verified fixed fixture");
}

#[test]
fn rollback_fault_does_not_add_another_object_after_preexisting_unknown_target() {
    let mut fixture = Fixture::create_fixed_tree().expect("create controlled fixed fixture");
    let prepared = prepare_attempt(&fixture.request(), Backend::ApfsFileClone)
        .expect("prepare before independently adding an unknown target");
    fixture
        .tree
        .create_file(
            Path::new("target/preexisting-unknown"),
            b"fixture-owned unknown\n",
            0o600,
        )
        .expect("create and independently register the preexisting unknown target");
    let preexisting = fixture
        .metadata_relative(Path::new("target/preexisting-unknown"))
        .expect("record preexisting unknown identity");

    let failure = execute_prepared(
        &prepared,
        &FaultInjection {
            rollback: thinws_p0_materialize::RollbackFault::AddUnknownEntry,
            ..FaultInjection::default()
        },
    )
    .expect_err("unknown target must reject execution");
    assert!(matches!(
        failure.error,
        MaterializationError::UnknownTargetEntry { ref path }
            if path.display == "preexisting-unknown"
    ));
    assert!(failure.evidence.created.is_empty());
    assert_eq!(failure.evidence.rollback.status, RollbackStatus::Incomplete);
    assert_eq!(
        fixture
            .metadata_relative(Path::new("target/preexisting-unknown"))
            .expect("preexisting unknown is preserved"),
        preexisting
    );
    if let Ok(injected) = fixture.metadata_relative(Path::new("target/p0-02-injected-unknown")) {
        eprintln!(
            "RED preserving rollback-fault fixture at {}: implementation created unowned p0-02-injected-unknown device={} inode={}",
            fixture.root.display(),
            injected.device,
            injected.inode
        );
    }
    assert!(
        !fixture.target.join("p0-02-injected-unknown").exists(),
        "rollback fault injection must not mutate an already-unknown target"
    );
    assert_control_and_outside_sentinels_unchanged(&fixture);
    fixture.cleanup().expect("remove verified fixed fixture");
}

#[test]
fn partial_full_copy_write_failure_registers_partial_file_then_rolls_back_reverse() {
    let fixture = Fixture::create_fixed_tree().expect("create controlled fixed fixture");
    let prepared = prepare_attempt(&fixture.request(), Backend::FullCopy)
        .expect("prepare fixed Full Copy attempt");
    let faults = FaultInjection {
        copy_write: Some(CopyWriteFault {
            ordinary_file_index: 1,
            after_bytes: 5,
            errno: libc::ENOSPC,
        }),
        ..FaultInjection::default()
    };

    let failure = execute_prepared(&prepared, &faults).expect_err("copy write must fail");
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
    assert_eq!(failure.evidence.clone_calls_succeeded, 0);
    assert_eq!(failure.evidence.ordinary_files_materialized, 1);
    assert_eq!(
        failure
            .evidence
            .created
            .iter()
            .map(|entry| entry.path.display.as_str())
            .collect::<Vec<_>>(),
        vec!["empty", "executable.sh"]
    );
    assert_eq!(
        failure
            .evidence
            .rollback
            .removed
            .iter()
            .map(|path| path.display.as_str())
            .collect::<Vec<_>>(),
        vec!["executable.sh", "empty"]
    );
    assert_eq!(
        failure.evidence.rollback.status,
        RollbackStatus::ConfirmedBaseline
    );
    assert_eq!(fs::read_dir(&fixture.target).unwrap().count(), 1);
    fixture.cleanup().expect("remove verified fixed fixture");
}

#[test]
fn registration_failure_retains_unconfirmed_created_object_for_recovery() {
    let fixture = Fixture::create_fixed_tree().expect("create controlled fixed fixture");
    let prepared = prepare_attempt(&fixture.request(), Backend::ApfsFileClone)
        .expect("prepare fixed clone attempt");
    let faults = FaultInjection {
        identity_registration_at: Some(0),
        ..FaultInjection::default()
    };

    let failure = execute_prepared(&prepared, &faults)
        .expect_err("post-create identity registration must fail");
    assert!(matches!(
        failure.error,
        MaterializationError::IdentityRegistrationFailed { .. }
    ));
    assert_eq!(failure.evidence.created.len(), 1);
    assert_eq!(
        failure.evidence.created[0].identity,
        CreatedIdentity::Unconfirmed
    );
    assert_eq!(failure.evidence.rollback.status, RollbackStatus::Incomplete);
    assert!(failure.evidence.rollback.removed.is_empty());
    assert_eq!(failure.evidence.rollback.remaining.len(), 1);
    drop(failure);
    assert!(
        fixture.target.join("empty").is_file(),
        "dropping evidence must not remove an unconfirmed object"
    );

    let observed = fixture
        .metadata_relative(Path::new("target/empty"))
        .expect("independently observe the retained object without claiming ownership");
    eprintln!(
        "preserving registration-failure fixture at {}: target/empty is unconfirmed; observed device={} inode={}",
        fixture.root.display(),
        observed.device,
        observed.inode
    );
}

#[test]
fn preflight_unsupported_policy_starts_copy_only_when_explicitly_allowed() {
    let mut allowed = Fixture::create_fixed_tree().expect("create controlled fixed fixture");
    let options = PolicyExperimentOptions {
        requested_mode: RequestedMode::CowClone,
        fallback_policy: FallbackPolicy::AllowFullCopyOnCowUnsupported,
        clone_preflight: ClonePreflightOverride::InjectUnsupported,
        ..PolicyExperimentOptions::default()
    };
    let receipt = run_policy_experiment(&allowed.request(), &options)
        .expect("explicit preflight policy should start Full Copy directly");
    allowed
        .adopt_materialized_fixed_tree(&receipt.attempts[0])
        .expect("receipt identities must match independent observations");
    assert_eq!(receipt.actual_backend, Backend::FullCopy);
    assert_eq!(receipt.cow_evidence, CowEvidence::NotUsed);
    assert_eq!(
        receipt.fallback_reason,
        Some(FallbackReason::PreflightCloneUnsupported { injected: true })
    );
    assert_eq!(receipt.attempts.len(), 1);
    assert_eq!(receipt.attempts[0].backend, Backend::FullCopy);
    assert_eq!(receipt.attempts[0].clone_calls_succeeded, 0);
    assert!(
        receipt.attempts[0]
            .ordinary_files
            .iter()
            .all(|file| !file.real_clone_call_succeeded)
    );
    assert_fixed_manifest(&receipt.attempts[0]);
    allowed.cleanup().expect("remove verified fixed fixture");

    let denied = Fixture::create_fixed_tree().expect("create controlled fixed fixture");
    let denied_options = PolicyExperimentOptions {
        fallback_policy: FallbackPolicy::Deny,
        ..options
    };
    let failure = run_policy_experiment(&denied.request(), &denied_options)
        .expect_err("deny policy must perform no attempt");
    assert!(matches!(
        failure.error,
        MaterializationError::PreflightUnsupported { .. }
    ));
    assert!(failure.attempts.is_empty());
    assert_eq!(fs::read_dir(&denied.target).unwrap().count(), 1);
    denied.cleanup().expect("remove verified fixed fixture");
}

#[test]
fn runtime_enotsup_after_zero_or_one_clone_reprobes_then_runs_fresh_copy() {
    for (clone_index, expected_clones) in [(0, 0), (1, 1)] {
        let mut fixture = Fixture::create_fixed_tree().expect("create controlled fixed fixture");
        let options = PolicyExperimentOptions {
            requested_mode: RequestedMode::CowClone,
            fallback_policy: FallbackPolicy::AllowFullCopyOnCowUnsupported,
            clone_preflight: ClonePreflightOverride::Observed,
            first_attempt_faults: FaultInjection {
                clone_errno_at: Some((clone_index, libc::ENOTSUP)),
                ..FaultInjection::default()
            },
            second_attempt_faults: FaultInjection::default(),
        };

        let receipt = run_policy_experiment(&fixture.request(), &options)
            .expect("confirmed baseline permits a fresh Full Copy attempt");
        fixture
            .adopt_materialized_fixed_tree(&receipt.attempts[1])
            .expect("receipt identities must match independent observations");
        assert_eq!(receipt.actual_backend, Backend::FullCopy);
        assert_eq!(receipt.cow_evidence, CowEvidence::NotUsed);
        assert_eq!(
            receipt.fallback_reason,
            Some(FallbackReason::RuntimeCloneUnsupported {
                errno: libc::ENOTSUP,
            })
        );
        assert_eq!(receipt.attempts.len(), 2);
        assert_eq!(receipt.attempts[0].backend, Backend::ApfsFileClone);
        assert_eq!(receipt.attempts[0].clone_calls_succeeded, expected_clones);
        assert_eq!(
            receipt.attempts[0].rollback.status,
            RollbackStatus::ConfirmedBaseline
        );
        assert_eq!(receipt.attempts[1].backend, Backend::FullCopy);
        assert_eq!(receipt.attempts[1].outcome, AttemptOutcome::Succeeded);
        assert_eq!(receipt.attempts[1].clone_calls_succeeded, 0);
        assert_fixed_manifest(&receipt.attempts[1]);
        fixture.cleanup().expect("remove verified fixed fixture");
    }
}

#[test]
fn runtime_clone_enotsup_then_full_copy_enospc_retains_both_attempts_and_rollbacks() {
    let fixture = Fixture::create_fixed_tree().expect("create controlled fixed fixture");
    let options = PolicyExperimentOptions {
        requested_mode: RequestedMode::CowClone,
        fallback_policy: FallbackPolicy::AllowFullCopyOnCowUnsupported,
        clone_preflight: ClonePreflightOverride::Observed,
        first_attempt_faults: FaultInjection {
            clone_errno_at: Some((1, libc::ENOTSUP)),
            ..FaultInjection::default()
        },
        second_attempt_faults: FaultInjection {
            copy_write: Some(CopyWriteFault {
                ordinary_file_index: 1,
                after_bytes: 5,
                errno: libc::ENOSPC,
            }),
            ..FaultInjection::default()
        },
    };

    let failure = run_policy_experiment(&fixture.request(), &options)
        .expect_err("the fresh Full Copy attempt must retain its own write failure");
    assert!(matches!(
        failure.error,
        MaterializationError::SystemCall {
            ref operation,
            errno: Some(libc::ENOSPC),
            injected: true,
            ..
        } if operation == &SystemOperation::Named("write target file".to_owned())
    ));
    assert_eq!(failure.attempts.len(), 2, "no third backend may be started");

    let clone = &failure.attempts[0];
    assert_eq!(clone.backend, Backend::ApfsFileClone);
    assert_eq!(clone.outcome, AttemptOutcome::Partial);
    assert_eq!(clone.clone_calls_succeeded, 1);
    assert_eq!(clone.ordinary_files_materialized, 1);
    assert!(matches!(
        clone.failure,
        Some(MaterializationError::SystemCall {
            operation: SystemOperation::CloneFileAt,
            errno: Some(libc::ENOTSUP),
            injected: true,
            ..
        })
    ));
    assert_eq!(
        clone
            .created
            .iter()
            .map(|entry| entry.path.display.as_str())
            .collect::<Vec<_>>(),
        vec!["empty"]
    );
    assert!(
        clone
            .created
            .iter()
            .all(|entry| matches!(entry.identity, CreatedIdentity::Confirmed(_)))
    );
    assert_eq!(
        clone
            .rollback
            .removed
            .iter()
            .map(|path| path.display.as_str())
            .collect::<Vec<_>>(),
        vec!["empty"]
    );
    assert!(clone.rollback.remaining.is_empty());
    assert!(clone.rollback.problems.is_empty());
    assert_eq!(clone.rollback.status, RollbackStatus::ConfirmedBaseline);

    let copy = &failure.attempts[1];
    assert_eq!(copy.backend, Backend::FullCopy);
    assert_eq!(copy.outcome, AttemptOutcome::Partial);
    assert_eq!(copy.clone_calls_succeeded, 0);
    assert_eq!(copy.ordinary_files_materialized, 1);
    assert!(matches!(
        copy.failure,
        Some(MaterializationError::SystemCall {
            operation: SystemOperation::Named(ref name),
            errno: Some(libc::ENOSPC),
            injected: true,
            ..
        }) if name == "write target file"
    ));
    assert_eq!(
        copy.created
            .iter()
            .map(|entry| entry.path.display.as_str())
            .collect::<Vec<_>>(),
        vec!["empty", "executable.sh"]
    );
    assert!(
        copy.created
            .iter()
            .all(|entry| matches!(entry.identity, CreatedIdentity::Confirmed(_)))
    );
    assert_eq!(
        copy.rollback
            .removed
            .iter()
            .map(|path| path.display.as_str())
            .collect::<Vec<_>>(),
        vec!["executable.sh", "empty"]
    );
    assert!(copy.rollback.remaining.is_empty());
    assert!(copy.rollback.problems.is_empty());
    assert_eq!(copy.rollback.status, RollbackStatus::ConfirmedBaseline);

    assert_eq!(fs::read_dir(&fixture.target).unwrap().count(), 1);
    assert_eq!(fs::read(fixture.source.join("empty")).unwrap(), b"");
    assert_eq!(
        fs::read(fixture.source.join("executable.sh")).unwrap(),
        b"#!/bin/sh\necho p0\n"
    );
    assert_eq!(
        fs::read(fixture.source.join("nested/payload.bin")).unwrap(),
        b"\x00\xffP0-02 fixed payload\n"
    );
    assert_eq!(
        fs::read_link(fixture.source.join("link-to-sentinel")).unwrap(),
        PathBuf::from("../outside-sentinel")
    );
    assert_control_and_outside_sentinels_unchanged(&fixture);
    fixture.cleanup().expect("remove verified fixed fixture");
}

#[test]
fn runtime_non_enotsup_never_starts_a_second_backend() {
    for errno in [
        libc::EXDEV,
        libc::ENOSPC,
        libc::EACCES,
        libc::EPERM,
        libc::EIO,
        libc::EINVAL,
    ] {
        let fixture = Fixture::create_fixed_tree().expect("create controlled fixed fixture");
        let options = PolicyExperimentOptions {
            requested_mode: RequestedMode::CowClone,
            fallback_policy: FallbackPolicy::AllowFullCopyOnCowUnsupported,
            clone_preflight: ClonePreflightOverride::Observed,
            first_attempt_faults: FaultInjection {
                clone_errno_at: Some((1, errno)),
                ..FaultInjection::default()
            },
            second_attempt_faults: FaultInjection::default(),
        };
        let failure = run_policy_experiment(&fixture.request(), &options)
            .expect_err("non-ENOTSUP clone failure must not trigger Full Copy");
        assert!(matches!(
            failure.error,
            MaterializationError::SystemCall {
                operation: SystemOperation::CloneFileAt,
                errno: Some(observed),
                injected: true,
                ..
            } if observed == errno
        ));
        assert_eq!(failure.attempts.len(), 1);
        assert_eq!(failure.attempts[0].backend, Backend::ApfsFileClone);
        assert_eq!(
            failure.attempts[0].rollback.status,
            RollbackStatus::ConfirmedBaseline
        );
        assert_eq!(fs::read_dir(&fixture.target).unwrap().count(), 1);
        fixture.cleanup().expect("remove verified fixed fixture");
    }
}

#[test]
fn runtime_enotsup_with_deny_policy_never_starts_full_copy() {
    let fixture = Fixture::create_fixed_tree().expect("create controlled fixed fixture");
    let options = PolicyExperimentOptions {
        requested_mode: RequestedMode::CowClone,
        fallback_policy: FallbackPolicy::Deny,
        clone_preflight: ClonePreflightOverride::Observed,
        first_attempt_faults: FaultInjection {
            clone_errno_at: Some((1, libc::ENOTSUP)),
            ..FaultInjection::default()
        },
        second_attempt_faults: FaultInjection::default(),
    };
    let failure = run_policy_experiment(&fixture.request(), &options)
        .expect_err("deny policy must not start Full Copy after runtime ENOTSUP");
    assert!(matches!(
        failure.error,
        MaterializationError::SystemCall {
            operation: SystemOperation::CloneFileAt,
            errno: Some(libc::ENOTSUP),
            injected: true,
            ..
        }
    ));
    assert_eq!(failure.attempts.len(), 1);
    assert_eq!(failure.attempts[0].backend, Backend::ApfsFileClone);
    assert_eq!(
        failure.attempts[0].rollback.status,
        RollbackStatus::ConfirmedBaseline
    );
    assert_eq!(fs::read_dir(&fixture.target).unwrap().count(), 1);
    fixture.cleanup().expect("remove verified fixed fixture");
}

#[test]
fn same_inode_source_content_change_rejects_clone_and_full_copy_success() {
    for backend in [Backend::ApfsFileClone, Backend::FullCopy] {
        let fixture = Fixture::create_fixed_tree().expect("create controlled fixed fixture");
        let prepared = prepare_attempt(&fixture.request(), backend).expect("prepare fixed attempt");
        let failure = execute_prepared(
            &prepared,
            &FaultInjection {
                mutate_source_after_file: Some(1),
                ..FaultInjection::default()
            },
        )
        .expect_err("same-inode source content mutation must reject success");
        assert!(matches!(
            failure.error,
            MaterializationError::SourceChanged { .. }
        ));
        assert_eq!(failure.evidence.backend, backend);
        assert_eq!(
            failure.evidence.rollback.status,
            RollbackStatus::ConfirmedBaseline
        );
        assert_eq!(fs::read_dir(&fixture.target).unwrap().count(), 1);
        assert_control_and_outside_sentinels_unchanged(&fixture);
        assert_eq!(
            &fs::read(fixture.source.join("executable.sh")).unwrap()[..1],
            b"!"
        );
        fixture.cleanup().expect("remove verified fixed fixture");
    }
}

#[test]
fn fresh_copy_prepare_reports_latest_source_change_and_retains_first_enotsup_attempt() {
    let fixture = Fixture::create_fixed_tree().expect("create controlled fixed fixture");
    let options = PolicyExperimentOptions {
        requested_mode: RequestedMode::CowClone,
        fallback_policy: FallbackPolicy::AllowFullCopyOnCowUnsupported,
        clone_preflight: ClonePreflightOverride::Observed,
        first_attempt_faults: FaultInjection {
            clone_errno_at: Some((2, libc::ENOTSUP)),
            mutate_source_after_file: Some(1),
            ..FaultInjection::default()
        },
        second_attempt_faults: FaultInjection::default(),
    };
    let failure = run_policy_experiment(&fixture.request(), &options)
        .expect_err("fresh fallback preparation must detect changed source contents");
    assert!(matches!(
        failure.error,
        MaterializationError::SourceChanged { .. }
    ));
    assert_eq!(failure.attempts.len(), 1);
    assert!(matches!(
        failure.attempts[0].failure,
        Some(MaterializationError::SystemCall {
            operation: SystemOperation::CloneFileAt,
            errno: Some(libc::ENOTSUP),
            injected: true,
            ..
        })
    ));
    assert_eq!(
        failure.attempts[0].rollback.status,
        RollbackStatus::ConfirmedBaseline
    );
    assert_eq!(fs::read_dir(&fixture.target).unwrap().count(), 1);
    assert_control_and_outside_sentinels_unchanged(&fixture);
    fixture.cleanup().expect("remove verified fixed fixture");
}

#[test]
fn incomplete_deletion_blocks_runtime_enotsup_fallback_and_retains_confirmed_objects() {
    for (reverse_index, expected_removed, expected_remaining) in [
        (
            0,
            Vec::<&str>::new(),
            vec!["empty", "executable.sh", "link-to-sentinel", "nested"],
        ),
        (
            2,
            vec!["nested", "link-to-sentinel"],
            vec!["empty", "executable.sh"],
        ),
    ] {
        let mut fixture = Fixture::create_fixed_tree().expect("create controlled fixed fixture");
        let options = PolicyExperimentOptions {
            requested_mode: RequestedMode::CowClone,
            fallback_policy: FallbackPolicy::AllowFullCopyOnCowUnsupported,
            clone_preflight: ClonePreflightOverride::Observed,
            first_attempt_faults: FaultInjection {
                clone_errno_at: Some((2, libc::ENOTSUP)),
                rollback: thinws_p0_materialize::RollbackFault::FailDeletion {
                    reverse_index,
                    errno: libc::EBUSY,
                },
                ..FaultInjection::default()
            },
            second_attempt_faults: FaultInjection::default(),
        };
        let failure = run_policy_experiment(&fixture.request(), &options)
            .expect_err("incomplete rollback must block Full Copy fallback");
        assert!(matches!(
            failure.error,
            MaterializationError::SystemCall {
                operation: SystemOperation::CloneFileAt,
                errno: Some(libc::ENOTSUP),
                ..
            }
        ));
        assert_eq!(failure.attempts.len(), 1);
        let attempt = &failure.attempts[0];
        assert_eq!(attempt.rollback.status, RollbackStatus::Incomplete);
        assert_eq!(
            attempt
                .rollback
                .removed
                .iter()
                .map(|path| path.display.as_str())
                .collect::<Vec<_>>(),
            expected_removed
        );
        assert_eq!(
            attempt
                .rollback
                .remaining
                .iter()
                .map(|path| path.display.as_str())
                .collect::<Vec<_>>(),
            expected_remaining
        );
        assert!(attempt.rollback.problems.iter().any(|problem| {
            problem.operation == "unlinkat injected rollback failure"
                && problem.errno == Some(libc::EBUSY)
        }));
        fixture
            .adopt_remaining_created(attempt)
            .expect("remaining objects have independently matching receipt identities");
        assert_control_and_outside_sentinels_unchanged(&fixture);
        fixture.cleanup().expect("remove verified fixed fixture");
    }
}

#[test]
fn unknown_entry_during_rollback_stops_without_fallback_and_is_preserved() {
    let fixture = Fixture::create_fixed_tree().expect("create controlled fixed fixture");
    let options = PolicyExperimentOptions {
        requested_mode: RequestedMode::CowClone,
        fallback_policy: FallbackPolicy::AllowFullCopyOnCowUnsupported,
        clone_preflight: ClonePreflightOverride::Observed,
        first_attempt_faults: FaultInjection {
            clone_errno_at: Some((2, libc::ENOTSUP)),
            rollback: thinws_p0_materialize::RollbackFault::AddUnknownEntry,
            ..FaultInjection::default()
        },
        second_attempt_faults: FaultInjection::default(),
    };
    let failure = run_policy_experiment(&fixture.request(), &options)
        .expect_err("unknown rollback entry must block Full Copy fallback");
    assert_eq!(failure.attempts.len(), 1);
    let attempt = &failure.attempts[0];
    assert_eq!(attempt.rollback.status, RollbackStatus::Incomplete);
    assert!(attempt.rollback.removed.is_empty());
    assert_eq!(attempt.rollback.remaining.len(), 4);
    let problem = attempt
        .rollback
        .problems
        .iter()
        .find(|problem| problem.operation == "compare rollback target set")
        .expect("rollback reports the unexpected target set");
    assert_eq!(
        problem.path.as_ref().map(|path| path.display.as_str()),
        Some("p0-02-injected-unknown")
    );
    assert!(problem.expected_identity.is_none());
    assert!(problem.observed_identity.is_some());
    let observed = fixture
        .metadata_relative(Path::new("target/p0-02-injected-unknown"))
        .expect("independently observe preserved unknown entry");
    assert_eq!(
        problem.observed_identity,
        Some(FileIdentity {
            device: observed.device,
            inode: observed.inode,
        })
    );
    assert!(fixture.target.join("p0-02-injected-unknown").is_file());
    assert_control_and_outside_sentinels_unchanged(&fixture);
    eprintln!(
        "preserving unknown-entry rollback fixture at {}: injected entry has no independent fixture ownership identity",
        fixture.root.display()
    );
}

#[test]
fn replaced_created_entry_during_rollback_stops_without_fallback_and_is_preserved() {
    let fixture = Fixture::create_fixed_tree().expect("create controlled fixed fixture");
    let options = PolicyExperimentOptions {
        requested_mode: RequestedMode::CowClone,
        fallback_policy: FallbackPolicy::AllowFullCopyOnCowUnsupported,
        clone_preflight: ClonePreflightOverride::Observed,
        first_attempt_faults: FaultInjection {
            clone_errno_at: Some((2, libc::ENOTSUP)),
            rollback: thinws_p0_materialize::RollbackFault::ReplaceCreated { created_index: 0 },
            ..FaultInjection::default()
        },
        second_attempt_faults: FaultInjection::default(),
    };
    let failure = run_policy_experiment(&fixture.request(), &options)
        .expect_err("replaced rollback object must block Full Copy fallback");
    assert_eq!(failure.attempts.len(), 1);
    let attempt = &failure.attempts[0];
    assert_eq!(attempt.rollback.status, RollbackStatus::Incomplete);
    assert!(attempt.rollback.removed.is_empty());
    assert_eq!(attempt.rollback.remaining.len(), 4);
    let problem = attempt
        .rollback
        .problems
        .iter()
        .find(|problem| problem.operation == "compare rollback target set")
        .expect("rollback reports the replaced target identity");
    assert_eq!(
        problem.path.as_ref().map(|path| path.display.as_str()),
        Some("empty")
    );
    assert_ne!(problem.expected_identity, problem.observed_identity);
    assert!(problem.expected_identity.is_some());
    assert!(problem.observed_identity.is_some());
    let observed = fixture
        .metadata_relative(Path::new("target/empty"))
        .expect("independently observe preserved replacement");
    assert_eq!(
        problem.observed_identity,
        Some(FileIdentity {
            device: observed.device,
            inode: observed.inode,
        })
    );
    assert!(fixture.target.join("empty").is_file());
    assert_control_and_outside_sentinels_unchanged(&fixture);
    eprintln!(
        "preserving replacement rollback fixture at {}: target/empty replacement has no independent fixture ownership identity",
        fixture.root.display()
    );
}

#[test]
fn execution_rejects_each_replaced_root_before_writing_and_fixture_restores_exact_identity() {
    for role in ["source", "target", "staging", "trash"] {
        let mut fixture = Fixture::create_fixed_tree().expect("create controlled fixed fixture");
        let prepared = prepare_attempt(&fixture.request(), Backend::ApfsFileClone)
            .expect("prepare fixed clone attempt");
        let baseline = fixture
            .tree
            .observed_tree()
            .expect("record controlled fixture before root replacement");
        let original = Path::new(role);
        let backup = PathBuf::from(format!("original-{role}"));
        fixture
            .tree
            .rename_tracked_exclusive(original, &backup)
            .expect("move the exact fixture-owned root without replacement");
        fixture
            .tree
            .create_directory(original, 0o700)
            .expect("create fixture-owned replacement root");

        let failure = execute_prepared(&prepared, &FaultInjection::default())
            .expect_err("changed root identity must reject execution before writing");
        let expected_path = match role {
            "source" => &fixture.source,
            "target" => &fixture.target,
            "staging" => &fixture.staging,
            "trash" => &fixture.trash,
            _ => unreachable!("fixed role list"),
        };
        assert!(matches!(
            failure.error,
            MaterializationError::PlanStale { ref path } if path == expected_path
        ));
        assert!(failure.evidence.created.is_empty());
        assert_eq!(failure.evidence.rollback.status, RollbackStatus::NotNeeded);

        fixture
            .tree
            .remove_tracked_exact(original)
            .expect("remove exact fixture-owned replacement root");
        fixture
            .tree
            .rename_tracked_exclusive(&backup, original)
            .expect("restore exact original root identity and descendants");
        assert_eq!(
            fixture
                .tree
                .observed_tree()
                .expect("observe restored controlled fixture"),
            baseline
        );
        assert_control_and_outside_sentinels_unchanged(&fixture);
        fixture.cleanup().expect("remove verified fixed fixture");
    }
}

#[test]
fn execution_rejects_moved_common_ancestor_before_writing() {
    let fixture = Fixture::create_fixed_tree().expect("create controlled fixed fixture");
    let prepared = prepare_attempt(&fixture.request(), Backend::ApfsFileClone)
        .expect("prepare fixed clone attempt");
    let baseline = fixture
        .tree
        .observed_tree()
        .expect("record controlled fixture before ancestor move");
    let moved_name = fixture
        .tree
        .move_fixture_parent_exclusive()
        .expect("exclusively move the fixture-owned common ancestor");

    let failure = execute_prepared(&prepared, &FaultInjection::default())
        .expect_err("moved common ancestor must reject execution before writing");
    assert!(matches!(
        failure.error,
        MaterializationError::PlanStale { .. } | MaterializationError::Probe { .. }
    ));
    assert!(failure.evidence.created.is_empty());
    assert_eq!(failure.evidence.rollback.status, RollbackStatus::NotNeeded);
    assert_eq!(
        fixture
            .tree
            .observed_tree()
            .expect("held fixture remains unchanged after refusal"),
        baseline
    );

    fixture
        .tree
        .restore_fixture_parent_exclusive(&moved_name)
        .expect("restore exact common ancestor after refusal assertion");
    assert_control_and_outside_sentinels_unchanged(&fixture);
    fixture.cleanup().expect("remove verified fixed fixture");
}

#[test]
#[ignore = "requires THINWS_P0_CROSS_VOLUME_ROOT on a distinct writable APFS volume"]
fn actual_cross_volume_clone_returns_exdev_without_target_then_explicit_copy_succeeds() {
    let cross_root = std::env::var_os("THINWS_P0_CROSS_VOLUME_ROOT")
        .expect("THINWS_P0_CROSS_VOLUME_ROOT must name the prepared APFS mount");
    let cross_root = PathBuf::from(cross_root)
        .canonicalize()
        .expect("canonicalize configured cross-volume root");
    assert!(cross_root.is_absolute());

    let local = Fixture::create_fixed_tree().expect("create local-volume source fixture");
    let mut remote = Fixture::create_fixed_tree_in(&cross_root, "cross-volume")
        .expect("create private target fixture on configured volume");
    assert_ne!(
        local.tree.root_identity().device,
        remote.tree.root_identity().device,
        "configured root must be a genuinely distinct volume"
    );
    let local_probe = inspect_path(&local.source).expect("probe local APFS source");
    let remote_probe = inspect_path(&remote.target).expect("probe remote APFS target");
    assert_eq!(local_probe.filesystem.type_name, "apfs");
    assert_eq!(remote_probe.filesystem.type_name, "apfs");
    let Evidence::Known {
        value: local_uuid, ..
    } = &local_probe.filesystem.volume_uuid
    else {
        panic!("local APFS Volume UUID must be known");
    };
    let Evidence::Known {
        value: remote_uuid, ..
    } = &remote_probe.filesystem.volume_uuid
    else {
        panic!("remote APFS Volume UUID must be known");
    };
    assert!(!local_uuid.is_empty());
    assert!(!remote_uuid.is_empty());
    assert_ne!(local_uuid, remote_uuid);
    let request = MaterializeRequest {
        source: &local.source,
        target: &remote.target,
        staging: &remote.staging,
        trash: &remote.trash,
        protected_git: remote.request().protected_git,
    };

    let injected_policy = PolicyExperimentOptions {
        requested_mode: RequestedMode::CowClone,
        fallback_policy: FallbackPolicy::AllowFullCopyOnCowUnsupported,
        clone_preflight: ClonePreflightOverride::InjectUnsupported,
        ..PolicyExperimentOptions::default()
    };
    let preflight_failure = run_policy_experiment(&request, &injected_policy)
        .expect_err("injection must not override real cross-volume evidence");
    assert!(matches!(
        preflight_failure.error,
        MaterializationError::PreflightUnsupported { .. }
    ));
    assert!(preflight_failure.attempts.is_empty());
    assert_eq!(fs::read_dir(&remote.target).unwrap().count(), 1);

    let clone_failure = match materialize_once(&request, Backend::ApfsFileClone) {
        Err(failure) => failure,
        Ok(receipt) => {
            remote
                .adopt_materialized_fixed_tree(&receipt)
                .expect("record unexpected clone result before safe cleanup");
            remote.cleanup().expect("remove verified remote fixture");
            local.cleanup().expect("remove verified local fixture");
            panic!("cross-volume fclonefileat unexpectedly succeeded");
        }
    };
    assert!(matches!(
        clone_failure.error,
        MaterializationError::SystemCall {
            operation: SystemOperation::CloneFileAt,
            errno: Some(libc::EXDEV),
            injected: false,
            ..
        }
    ));
    assert_eq!(clone_failure.evidence.clone_calls_succeeded, 0);
    assert!(clone_failure.evidence.created.is_empty());
    assert_eq!(
        clone_failure.evidence.rollback.status,
        RollbackStatus::ConfirmedBaseline
    );
    assert_eq!(fs::read_dir(&remote.target).unwrap().count(), 1);

    let copy = materialize_once(&request, Backend::FullCopy)
        .expect("explicit userspace Full Copy should work across volumes");
    assert_eq!(copy.backend, Backend::FullCopy);
    assert_eq!(copy.cow_evidence, CowEvidence::NotUsed);
    assert_eq!(copy.clone_calls_succeeded, 0);
    assert!(
        copy.ordinary_files
            .iter()
            .all(|file| !file.real_clone_call_succeeded)
    );
    assert_fixed_manifest(&copy);
    remote
        .adopt_materialized_fixed_tree(&copy)
        .expect("copy receipt identities must match independent observations");

    remote.cleanup().expect("remove verified remote fixture");
    local.cleanup().expect("remove verified local fixture");
}
