//! Real directory-entry changes must invalidate previously observed paths.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;

use tempfile::TempDir;
use thinws_p0_probe::{
    PathResolution, ProbeError, RevalidationChange, RevalidationStatus, inspect_path,
    revalidate_path,
};

fn isolated_root() -> TempDir {
    let existing_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .canonicalize()
        .expect("existing test checkout");
    tempfile::tempdir_in(existing_root).expect("isolated temporary test root")
}

#[test]
fn unchanged_directory_and_missing_target_remain_unchanged() {
    let root = isolated_root();
    for path in [root.path().to_path_buf(), root.path().join("missing/leaf")] {
        let before = inspect_path(&path).expect("initial observation");
        let checked = revalidate_path(&before).expect("revalidation");
        assert_eq!(checked.status, RevalidationStatus::Unchanged);
        assert!(checked.changes.is_empty());
        assert_eq!(
            checked.current.expect("current facts").resolution,
            before.resolution
        );
    }
}

#[test]
fn replacement_leaf_directory_invalidates_its_identity() {
    let root = isolated_root();
    let target = root.path().join("target");
    fs::create_dir(&target).expect("test target");
    let before = inspect_path(&target).expect("initial observation");
    // Keep the old directory alive so the new entry cannot reuse its inode.
    fs::rename(&target, root.path().join("old-target")).expect("preserve old target");
    fs::create_dir(&target).expect("replacement target");

    let checked = revalidate_path(&before).expect("revalidation");
    assert_eq!(checked.status, RevalidationStatus::Stale);
    assert!(
        checked
            .changes
            .iter()
            .any(|change| matches!(change, RevalidationChange::AncestryIdentityChanged { .. }))
    );
}

#[test]
fn replacing_ancestor_is_detected_even_when_leaf_identity_is_preserved() {
    let root = isolated_root();
    let parent = root.path().join("parent");
    let leaf = parent.join("leaf");
    fs::create_dir_all(&leaf).expect("test ancestry");
    let before = inspect_path(&leaf).expect("initial observation");
    let previous_leaf = before.ancestry.last().expect("leaf evidence").identity;

    let old_parent = root.path().join("old-parent");
    fs::rename(&parent, &old_parent).expect("retain old parent");
    fs::create_dir(&parent).expect("replacement parent");
    fs::rename(old_parent.join("leaf"), &leaf).expect("preserve original leaf");

    let checked = revalidate_path(&before).expect("revalidation");
    assert_eq!(checked.status, RevalidationStatus::Stale);
    let current = checked.current.expect("replacement is a valid directory");
    assert_eq!(
        current.ancestry.last().expect("leaf").identity,
        previous_leaf
    );
    assert_eq!(current.filesystem, before.filesystem);
    assert!(
        checked
            .changes
            .iter()
            .any(|change| matches!(change, RevalidationChange::AncestryIdentityChanged { .. }))
    );
}

#[test]
fn missing_leaf_appearing_invalidates_previous_observation() {
    let root = isolated_root();
    let target = root.path().join("target");
    let before = inspect_path(&target).expect("missing target observation");
    assert_eq!(before.resolution, PathResolution::MissingTarget);
    fs::create_dir(&target).expect("materialize target");
    let checked = revalidate_path(&before).expect("revalidation");
    assert_eq!(checked.status, RevalidationStatus::Stale);
    assert!(checked.changes.iter().any(|change| matches!(
        change,
        RevalidationChange::ResolutionChanged {
            before: PathResolution::MissingTarget,
            after: PathResolution::ExistingDirectory,
        }
    )));
}

#[test]
fn missing_intermediate_appearing_invalidates_still_missing_leaf() {
    let root = isolated_root();
    let target = root.path().join("parent/leaf");
    let before = inspect_path(&target).expect("missing ancestry observation");
    fs::create_dir(root.path().join("parent")).expect("create intermediate only");
    let checked = revalidate_path(&before).expect("revalidation");
    assert_eq!(checked.status, RevalidationStatus::Stale);
    assert_eq!(
        checked.current.expect("new facts").resolution,
        PathResolution::MissingTarget
    );
    assert!(
        checked
            .changes
            .iter()
            .any(|change| matches!(change, RevalidationChange::MissingSuffixChanged { .. }))
    );
    assert!(!target.exists());
}

#[test]
fn symlinks_are_rejected_for_leaf_intermediate_and_dangling_targets() {
    let root = isolated_root();
    let actual = root.path().join("actual");
    fs::create_dir(&actual).expect("real directory");
    let link = root.path().join("link");
    symlink(&actual, &link).expect("test symlink");
    let dangling = root.path().join("dangling");
    symlink(root.path().join("never-created"), &dangling).expect("dangling symlink");
    for path in [link.clone(), link.join("missing"), dangling] {
        assert!(
            matches!(
                inspect_path(&path),
                Err(ProbeError::SymbolicLinkEncountered { .. })
            ),
            "symlink must not be followed or mistaken for a missing directory"
        );
    }
}

#[test]
fn ancestor_replaced_by_symlink_invalidates_previous_report() {
    let root = isolated_root();
    let parent = root.path().join("parent");
    fs::create_dir(&parent).expect("original parent");
    let before = inspect_path(&parent.join("missing")).expect("initial observation");
    let preserved = root.path().join("preserved");
    fs::rename(&parent, &preserved).expect("preserve original directory");
    symlink(&preserved, &parent).expect("replace ancestor with link");
    let checked = revalidate_path(&before).expect("revalidation");
    assert_eq!(checked.status, RevalidationStatus::Stale);
    assert!(checked.current.is_none());
    assert!(checked.changes.iter().any(|change| matches!(
        change,
        RevalidationChange::PathRejected {
            error: ProbeError::SymbolicLinkEncountered { .. },
        }
    )));
    assert!(!preserved.join("missing").exists());
}
