#![deny(unsafe_code)]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rustix::fs::AtFlags;
use thinws_p0_materialize::{
    AttemptEvidence, AttemptOutcome, Backend, CowEvidence, CreatedIdentity, ManifestEntryKind,
    MaterializeRequest, materialize_once,
};

mod support;

use support::{ControlledTree, TrackedKind};

const ORIGINAL_BYTES: &[u8] = b"shared source bytes\n";
const TARGET_MUTATION: &[u8] = b"target-only bytes\n";
const SOURCE_MUTATION: &[u8] = b"source-only bytes\n";

struct Fixture {
    tree: ControlledTree,
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
                &Path::new("target").join(&entry.path.display),
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

fn backend_label(backend: Backend) -> &'static str {
    match backend {
        Backend::ApfsFileClone => "apfs-clone",
        Backend::FullCopy => "full-copy",
    }
}

fn assert_successful_backend(receipt: &AttemptEvidence, backend: Backend, file_count: usize) {
    assert_eq!(receipt.backend, backend);
    assert_eq!(receipt.outcome, AttemptOutcome::Succeeded);
    assert_eq!(receipt.ordinary_files_materialized, file_count);
    assert_eq!(receipt.source_manifest, receipt.target_manifest);
    match backend {
        Backend::ApfsFileClone => {
            assert_eq!(receipt.cow_evidence, CowEvidence::Confirmed);
            assert_eq!(receipt.clone_calls_succeeded, file_count);
            assert!(
                receipt
                    .ordinary_files
                    .iter()
                    .all(|file| file.real_clone_call_succeeded)
            );
        }
        Backend::FullCopy => {
            assert_eq!(receipt.cow_evidence, CowEvidence::NotUsed);
            assert_eq!(receipt.clone_calls_succeeded, 0);
            assert!(
                receipt
                    .ordinary_files
                    .iter()
                    .all(|file| !file.real_clone_call_succeeded)
            );
        }
    }
}

#[test]
fn hard_link_names_become_identity_and_write_isolated_files_for_both_backends() {
    for backend in [Backend::ApfsFileClone, Backend::FullCopy] {
        let mut fixture = Fixture::create(&format!("hard-link-{}", backend_label(backend)))
            .expect("create controlled fixture");
        fixture
            .tree
            .create_file("source/original.bin", ORIGINAL_BYTES, 0o640)
            .expect("create source file");
        let source_identity = fixture
            .tree
            .identity_at(Path::new("source/original.bin"))
            .expect("record known source identity");
        rustix::fs::linkat(
            fixture.tree.root_fd(),
            Path::new("source/original.bin"),
            fixture.tree.root_fd(),
            Path::new("source/alias.bin"),
            AtFlags::empty(),
        )
        .expect("create fixture-scoped hard link with dirfd-relative linkat");
        fixture
            .tree
            .adopt_confirmed(
                Path::new("source/alias.bin"),
                source_identity.device,
                source_identity.inode,
                TrackedKind::RegularFile,
            )
            .expect("register hard link only after matching its known source identity");
        assert_eq!(
            fixture
                .tree
                .identity_at(Path::new("source/alias.bin"))
                .expect("observe registered hard link"),
            source_identity
        );

        let receipt = materialize_once(&fixture.request(), backend)
            .expect("both hard-link names must materialize as ordinary files");
        fixture
            .adopt_created(&receipt)
            .expect("adopt only receipt-confirmed target identities");
        assert_successful_backend(&receipt, backend, 2);

        let target_original = fixture
            .tree
            .identity_at(Path::new("target/original.bin"))
            .expect("observe materialized original name");
        let target_alias = fixture
            .tree
            .identity_at(Path::new("target/alias.bin"))
            .expect("observe materialized alias name");
        assert_ne!(target_original, source_identity);
        assert_ne!(target_alias, source_identity);
        assert_ne!(target_original, target_alias);
        for (path, target_identity) in [
            ("original.bin", target_original),
            ("alias.bin", target_alias),
        ] {
            let evidence = receipt
                .ordinary_files
                .iter()
                .find(|file| file.path.display == path)
                .expect("receipt records each hard-link name separately");
            assert_eq!(evidence.source_identity.device, source_identity.device);
            assert_eq!(evidence.source_identity.inode, source_identity.inode);
            assert_eq!(evidence.target_identity.device, target_identity.device);
            assert_eq!(evidence.target_identity.inode, target_identity.inode);
        }
        assert_eq!(
            fs::read(fixture.source.join("original.bin")).unwrap(),
            ORIGINAL_BYTES
        );
        assert_eq!(
            fs::read(fixture.source.join("alias.bin")).unwrap(),
            ORIGINAL_BYTES
        );
        assert_eq!(
            fs::read(fixture.target.join("original.bin")).unwrap(),
            ORIGINAL_BYTES
        );
        assert_eq!(
            fs::read(fixture.target.join("alias.bin")).unwrap(),
            ORIGINAL_BYTES
        );

        fs::write(fixture.target.join("original.bin"), TARGET_MUTATION)
            .expect("modify one materialized target");
        assert_eq!(
            fs::read(fixture.target.join("original.bin")).unwrap(),
            TARGET_MUTATION
        );
        assert_eq!(
            fs::read(fixture.target.join("alias.bin")).unwrap(),
            ORIGINAL_BYTES
        );
        assert_eq!(
            fs::read(fixture.source.join("original.bin")).unwrap(),
            ORIGINAL_BYTES
        );
        assert_eq!(
            fs::read(fixture.source.join("alias.bin")).unwrap(),
            ORIGINAL_BYTES
        );

        fs::write(fixture.source.join("alias.bin"), SOURCE_MUTATION)
            .expect("modify the shared source inode through its alias");
        assert_eq!(
            fs::read(fixture.source.join("original.bin")).unwrap(),
            SOURCE_MUTATION
        );
        assert_eq!(
            fs::read(fixture.source.join("alias.bin")).unwrap(),
            SOURCE_MUTATION
        );
        assert_eq!(
            fixture
                .tree
                .identity_at(Path::new("source/original.bin"))
                .expect("source original identity remains registered"),
            source_identity
        );
        assert_eq!(
            fixture
                .tree
                .identity_at(Path::new("source/alias.bin"))
                .expect("source alias identity remains registered"),
            source_identity
        );
        assert_eq!(
            fs::read(fixture.target.join("original.bin")).unwrap(),
            TARGET_MUTATION
        );
        assert_eq!(
            fs::read(fixture.target.join("alias.bin")).unwrap(),
            ORIGINAL_BYTES
        );
        assert_eq!(
            fixture
                .tree
                .identity_at(Path::new("target/original.bin"))
                .expect("target original identity remains registered"),
            target_original
        );
        assert_eq!(
            fixture
                .tree
                .identity_at(Path::new("target/alias.bin"))
                .expect("target alias identity remains registered"),
            target_alias
        );

        fixture.cleanup().expect("clean controlled fixture");
    }
}

#[test]
fn materialized_file_survives_exact_source_removal_for_both_backends() {
    for backend in [Backend::ApfsFileClone, Backend::FullCopy] {
        let mut fixture = Fixture::create(&format!("source-removal-{}", backend_label(backend)))
            .expect("create controlled fixture");
        fixture
            .tree
            .create_file("source/payload.bin", ORIGINAL_BYTES, 0o600)
            .expect("create registered source file");
        let source_identity = fixture
            .tree
            .identity_at(Path::new("source/payload.bin"))
            .expect("observe source identity before materialization");

        let receipt = materialize_once(&fixture.request(), backend)
            .expect("source file must materialize before source removal");
        fixture
            .adopt_created(&receipt)
            .expect("adopt only receipt-confirmed target identity");
        assert_successful_backend(&receipt, backend, 1);
        let target_identity = fixture
            .tree
            .identity_at(Path::new("target/payload.bin"))
            .expect("observe materialized target identity");
        assert_ne!(target_identity, source_identity);
        assert_eq!(
            fs::read(fixture.target.join("payload.bin")).unwrap(),
            ORIGINAL_BYTES
        );

        fixture
            .tree
            .remove_tracked_exact(Path::new("source/payload.bin"))
            .expect("remove only the exact registered source file");
        fixture
            .tree
            .remove_tracked_exact(Path::new("source"))
            .expect("remove only the now-empty registered source directory");
        assert_eq!(
            fs::symlink_metadata(&fixture.source)
                .expect_err("registered source directory is absent")
                .kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(
            fs::read(fixture.target.join("payload.bin")).unwrap(),
            ORIGINAL_BYTES
        );

        fs::write(fixture.target.join("payload.bin"), TARGET_MUTATION)
            .expect("modify materialized target after source removal");
        assert_eq!(
            fs::read(fixture.target.join("payload.bin")).unwrap(),
            TARGET_MUTATION
        );
        assert_eq!(
            fixture
                .tree
                .identity_at(Path::new("target/payload.bin"))
                .expect("target identity remains registered after source removal and write"),
            target_identity
        );

        fixture.cleanup().expect("clean controlled fixture");
    }
}
