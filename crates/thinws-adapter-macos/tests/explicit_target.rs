#![cfg(target_os = "macos")]

use std::fs;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use tempfile::{Builder, TempDir};
use thinws_adapter_macos::{MacOsDataRootLayout, MacOsHostAdapter, MacOsLockGuard};
use thinws_core::{
    AbsolutePath, InstallationIdentity, InstanceId, UnixMillis, WorkspaceId, WorkspaceName,
    WorkspaceReservation,
};
use thinws_ports::{
    BootstrapStore, LifecycleLock, PortErrorKind, PreparedDataRootEvidence,
    PreparedWorkspaceEvidence, WorkspaceRemoval,
};

fn absolute(path: &Path) -> AbsolutePath {
    AbsolutePath::try_from_bytes(path.as_os_str().as_bytes().to_vec()).unwrap()
}

fn fixture() -> (
    TempDir,
    MacOsHostAdapter,
    MacOsDataRootLayout,
    MacOsLockGuard,
    InstallationIdentity,
) {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-layout-v2-tests");
    fs::create_dir_all(&base).unwrap();
    let temp = Builder::new()
        .prefix("explicit-target-")
        .tempdir_in(fs::canonicalize(base).unwrap())
        .unwrap();
    let control = temp.path().join(".thinws");
    let adapter = MacOsHostAdapter::new(&control).unwrap();
    adapter.prepare_bootstrap().unwrap();
    let bootstrap_lock = adapter.acquire_bootstrap(Duration::from_secs(1)).unwrap();
    let prepared = adapter.prepare_data_root(&absolute(&control)).unwrap();
    let identity = InstallationIdentity::new(
        InstanceId::from_str("01890a5d-ac96-774b-bd5b-55c7b8d09f33").unwrap(),
        absolute(&control),
        prepared.volume_id(),
    );
    let proof = adapter
        .create_initializing(&bootstrap_lock, prepared, &identity)
        .unwrap();
    let layout = adapter.initialize_layout(&bootstrap_lock, &proof).unwrap();
    adapter.publish_ready(&bootstrap_lock, proof).unwrap();
    adapter.publish_config(&bootstrap_lock, &identity).unwrap();
    drop(bootstrap_lock);
    let lock = adapter
        .acquire_data_root(identity.data_root(), Duration::from_secs(1))
        .unwrap();
    (temp, adapter, layout, lock, identity)
}

fn reservation_for(
    identity: &InstallationIdentity,
    id: WorkspaceId,
    source: &Path,
    target: &Path,
) -> WorkspaceReservation {
    WorkspaceReservation::new(
        id,
        identity.instance_id(),
        WorkspaceName::from_str("clone").unwrap(),
        absolute(source),
        absolute(target),
        identity.volume_id(),
        identity.volume_id(),
        false,
        UnixMillis::new(1_700_000_000_000).unwrap(),
    )
}

#[test]
fn workspace_mutation_rejects_bootstrap_scoped_lock() {
    let (temp, adapter, layout, data_root_lock, _identity) = fixture();
    drop(data_root_lock);
    let bootstrap_lock = adapter.acquire_bootstrap(Duration::from_secs(1)).unwrap();
    let target = temp.path().join("clone");
    let id = WorkspaceId::new();

    assert_eq!(
        adapter
            .prepare_workspace(&bootstrap_lock, &layout, id, &absolute(&target))
            .err()
            .expect("bootstrap-scoped guard must not authorize Workspace mutation")
            .kind(),
        PortErrorKind::InvalidData
    );
    assert!(!target.exists());
    assert!(
        !temp
            .path()
            .join(format!(".thinws/metadata/ownership-{id}.toml"))
            .exists()
    );
}

#[test]
fn one_adapter_cannot_initialize_a_second_control_root() {
    let (temp, adapter, _layout, _lock, identity) = fixture();
    let alternate = temp.path().join("alternate-control");
    assert!(adapter.prepare_data_root(&absolute(&alternate)).is_err());
    assert!(!alternate.exists());
    let foreign = InstallationIdentity::new(
        identity.instance_id(),
        absolute(&alternate),
        identity.volume_id(),
    );
    assert!(adapter.validate_layout(&foreign).is_err());
}

#[test]
fn target_parent_cannot_be_a_case_alias_of_the_control_root() {
    let (temp, adapter, layout, lock, _identity) = fixture();
    let control = temp.path().join(".thinws");
    let alias = temp.path().join(".THINWS");
    let Ok(alias_metadata) = fs::metadata(&alias) else {
        eprintln!("skipping APFS case alias on a case-sensitive volume");
        return;
    };
    use std::os::unix::fs::MetadataExt as _;
    let control_metadata = fs::metadata(&control).unwrap();
    assert_eq!(alias_metadata.dev(), control_metadata.dev());
    assert_eq!(alias_metadata.ino(), control_metadata.ino());

    let target = alias.join("copy");
    let id = WorkspaceId::from_str("ws_01890a5d-ac96-774b-bd5b-55c7b8d09f47").unwrap();
    assert_eq!(
        adapter
            .prepare_workspace(&lock, &layout, id, &absolute(&target))
            .err()
            .expect("control-root alias must be rejected")
            .kind(),
        PortErrorKind::InvalidLayout
    );
    assert!(!target.exists());
    assert!(
        !control
            .join(format!("metadata/ownership-{id}.toml"))
            .exists()
    );
}

#[test]
fn explicit_target_is_ordinary_and_only_control_evidence_is_persistent() {
    let (temp, adapter, layout, lock, identity) = fixture();
    let parent = temp.path().join("copies");
    fs::create_dir(&parent).unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let target = parent.join("user-selected-clone");
    let id = WorkspaceId::from_str("ws_01890a5d-ac96-774b-bd5b-55c7b8d09f40").unwrap();
    let reservation = reservation_for(&identity, id, &source, &target);
    let prepared = adapter
        .prepare_workspace(&lock, &layout, id, reservation.target_path())
        .unwrap();
    assert_eq!(prepared.target_root(), reservation.target_path());
    assert!(target.is_dir());
    assert!(
        prepared
            .staging_root()
            .as_bytes()
            .starts_with(parent.as_os_str().as_bytes())
    );
    assert!(
        prepared
            .trash_root()
            .as_bytes()
            .starts_with(parent.as_os_str().as_bytes())
    );
    assert!(identity.data_root().as_bytes() != parent.as_os_str().as_bytes());
    assert!(!temp.path().join(".thinws/workspaces").exists());
    assert!(!temp.path().join(".thinws/staging").exists());
    assert!(!temp.path().join(".thinws/trash").exists());
    assert!(
        temp.path()
            .join(format!(".thinws/metadata/ownership-{id}.toml"))
            .is_file()
    );
    adapter
        .clear_workspace_incomplete(&lock, &layout, prepared)
        .unwrap();
    assert_eq!(
        adapter
            .validate_ready_workspace(&layout, &reservation)
            .unwrap(),
        absolute(&target)
    );
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), b"keep").unwrap();
    fs::write(target.join("file"), b"clone").unwrap();
    symlink(&outside, target.join("external-link")).unwrap();
    assert_eq!(
        adapter
            .remove_workspace(&lock, &layout, &reservation)
            .unwrap(),
        WorkspaceRemoval::Removed { root_entries: 2 }
    );
    assert!(!target.exists());
    assert_eq!(fs::read(outside.join("keep")).unwrap(), b"keep");
}

#[test]
fn missing_or_replaced_target_cannot_be_removed() {
    for replaced in [false, true] {
        let (temp, adapter, layout, lock, identity) = fixture();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        let target = temp.path().join("clone");
        let id = WorkspaceId::from_str("ws_01890a5d-ac96-774b-bd5b-55c7b8d09f41").unwrap();
        let reservation = reservation_for(&identity, id, &source, &target);
        let prepared = adapter
            .prepare_workspace(&lock, &layout, id, reservation.target_path())
            .unwrap();
        adapter
            .clear_workspace_incomplete(&lock, &layout, prepared)
            .unwrap();
        if replaced {
            fs::rename(&target, temp.path().join("original-clone")).unwrap();
            fs::create_dir(&target).unwrap();
            fs::write(target.join("foreign"), b"preserve").unwrap();
        } else {
            fs::remove_dir(&target).unwrap();
        }
        let expected = if replaced {
            PortErrorKind::InvalidLayout
        } else {
            PortErrorKind::NotFound
        };
        let inspection = adapter.inspect_removal_container(&lock, &layout, &reservation);
        if replaced {
            assert_eq!(inspection.unwrap_err().kind(), expected);
        } else {
            assert_eq!(inspection.unwrap(), None);
        }
        assert_eq!(
            adapter
                .remove_workspace(&lock, &layout, &reservation)
                .unwrap_err()
                .kind(),
            expected
        );
        assert!(
            temp.path()
                .join(format!(".thinws/metadata/ownership-{id}.toml"))
                .is_file()
        );
        if replaced {
            assert_eq!(fs::read(target.join("foreign")).unwrap(), b"preserve");
            assert!(temp.path().join("original-clone").exists());
        }
    }
}

#[test]
fn a_partial_delete_can_resume_only_from_its_durable_isolation() {
    let (temp, adapter, layout, lock, identity) = fixture();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let target = temp.path().join("clone");
    let id = WorkspaceId::from_str("ws_01890a5d-ac96-774b-bd5b-55c7b8d09f43").unwrap();
    let reservation = reservation_for(&identity, id, &source, &target);
    let prepared = adapter
        .prepare_workspace(&lock, &layout, id, reservation.target_path())
        .unwrap();
    adapter
        .clear_workspace_incomplete(&lock, &layout, prepared)
        .unwrap();
    let nested = target.join("nested");
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join("file"), b"keep until explicit retry").unwrap();
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(&nested, fs::Permissions::from_mode(0o500)).unwrap();
    assert_eq!(
        adapter
            .remove_workspace(&lock, &layout, &reservation)
            .unwrap_err()
            .kind(),
        PortErrorKind::Io
    );
    let isolated = temp.path().join(format!(".thinws-remove-{id}"));
    assert!(!target.exists());
    assert_eq!(
        fs::read(isolated.join("nested/file")).unwrap(),
        b"keep until explicit retry"
    );
    assert_eq!(
        adapter
            .inspect_removal_container(&lock, &layout, &reservation)
            .unwrap(),
        Some(absolute(&isolated)),
    );
    let ownership = fs::read_to_string(
        temp.path()
            .join(format!(".thinws/metadata/ownership-{id}.toml")),
    )
    .unwrap();
    assert!(ownership.contains("isolated_path_hex = "));
    fs::set_permissions(isolated.join("nested"), fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        adapter
            .remove_workspace(&lock, &layout, &reservation)
            .unwrap(),
        WorkspaceRemoval::Removed { root_entries: 2 }
    );
    assert!(!isolated.exists());
}

#[test]
fn a_conflicting_active_path_cannot_be_chosen_over_durable_isolation() {
    let (temp, adapter, layout, lock, identity) = fixture();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let target = temp.path().join("clone");
    let id = WorkspaceId::from_str("ws_01890a5d-ac96-774b-bd5b-55c7b8d09f44").unwrap();
    let reservation = reservation_for(&identity, id, &source, &target);
    let prepared = adapter
        .prepare_workspace(&lock, &layout, id, reservation.target_path())
        .unwrap();
    adapter
        .clear_workspace_incomplete(&lock, &layout, prepared)
        .unwrap();
    let nested = target.join("nested");
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join("file"), b"original").unwrap();
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(&nested, fs::Permissions::from_mode(0o500)).unwrap();
    assert!(
        adapter
            .remove_workspace(&lock, &layout, &reservation)
            .is_err()
    );
    let isolated = temp.path().join(format!(".thinws-remove-{id}"));
    fs::create_dir(&target).unwrap();
    fs::write(target.join("foreign"), b"preserve").unwrap();
    assert_eq!(
        adapter
            .inspect_removal_container(&lock, &layout, &reservation)
            .unwrap_err()
            .kind(),
        PortErrorKind::InvalidLayout
    );
    assert_eq!(
        adapter
            .remove_workspace(&lock, &layout, &reservation)
            .unwrap_err()
            .kind(),
        PortErrorKind::InvalidLayout
    );
    assert_eq!(fs::read(target.join("foreign")).unwrap(), b"preserve");
    assert_eq!(fs::read(isolated.join("nested/file")).unwrap(), b"original");
}

#[test]
fn registered_operation_directories_and_database_target_are_identity_bound() {
    let (temp, adapter, layout, lock, identity) = fixture();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let target = temp.path().join("clone");
    let id = WorkspaceId::from_str("ws_01890a5d-ac96-774b-bd5b-55c7b8d09f45").unwrap();
    let reservation = reservation_for(&identity, id, &source, &target);
    let prepared = adapter
        .prepare_workspace(&lock, &layout, id, reservation.target_path())
        .unwrap();
    let staging = PathBuf::from(std::ffi::OsString::from_vec(
        prepared.staging_root().as_bytes().to_vec(),
    ));
    fs::rename(&staging, temp.path().join("original-staging")).unwrap();
    fs::create_dir(&staging).unwrap();
    assert_eq!(
        prepared.revalidate().unwrap_err().kind(),
        PortErrorKind::InvalidData
    );
    assert!(
        adapter
            .clear_workspace_incomplete(&lock, &layout, prepared)
            .is_err()
    );
    assert!(target.exists());
    let wrong = reservation_for(
        &identity,
        id,
        &source,
        &temp.path().join("different-target"),
    );
    assert_eq!(
        adapter
            .inspect_removal_container(&lock, &layout, &wrong)
            .unwrap_err()
            .kind(),
        PortErrorKind::InvalidLayout
    );
    assert_eq!(
        adapter
            .remove_workspace(&lock, &layout, &wrong)
            .unwrap_err()
            .kind(),
        PortErrorKind::InvalidLayout
    );
    assert!(target.exists());
}

#[test]
fn ready_target_rejects_reappearing_operation_directories() {
    for kind in ["staging", "trash"] {
        let (temp, adapter, layout, lock, identity) = fixture();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        let target = temp.path().join("clone");
        let id = WorkspaceId::new();
        let reservation = reservation_for(&identity, id, &source, &target);
        let prepared = adapter
            .prepare_workspace(&lock, &layout, id, reservation.target_path())
            .unwrap();
        adapter
            .clear_workspace_incomplete(&lock, &layout, prepared)
            .unwrap();
        assert_eq!(
            adapter
                .validate_ready_workspace(&layout, &reservation)
                .unwrap(),
            absolute(&target)
        );

        let foreign = temp.path().join(format!(".thinws-{kind}-{id}"));
        fs::create_dir(&foreign).unwrap();
        assert_eq!(
            adapter
                .validate_ready_workspace(&layout, &reservation)
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );
        assert!(foreign.is_dir());
        assert!(target.is_dir());
    }
}

#[test]
fn removal_cleans_both_registered_operation_directories() {
    let (temp, adapter, layout, lock, identity) = fixture();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let target = temp.path().join("clone");
    let id = WorkspaceId::new();
    let reservation = reservation_for(&identity, id, &source, &target);
    let prepared = adapter
        .prepare_workspace(&lock, &layout, id, reservation.target_path())
        .unwrap();
    let staging = PathBuf::from(std::ffi::OsString::from_vec(
        prepared.staging_root().as_bytes().to_vec(),
    ));
    let trash = PathBuf::from(std::ffi::OsString::from_vec(
        prepared.trash_root().as_bytes().to_vec(),
    ));
    fs::write(staging.join("leftover"), b"staging").unwrap();
    fs::write(trash.join("leftover"), b"trash").unwrap();

    assert_eq!(
        adapter
            .remove_workspace(&lock, &layout, &reservation)
            .unwrap(),
        WorkspaceRemoval::Removed { root_entries: 0 }
    );
    assert!(!target.exists());
    assert!(!staging.exists());
    assert!(!trash.exists());
}

#[test]
fn removal_never_adopts_a_replaced_operation_directory() {
    for kind in ["staging", "trash"] {
        let (temp, adapter, layout, lock, identity) = fixture();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        let target = temp.path().join("clone");
        let id = WorkspaceId::new();
        let reservation = reservation_for(&identity, id, &source, &target);
        let prepared = adapter
            .prepare_workspace(&lock, &layout, id, reservation.target_path())
            .unwrap();
        let registered = match kind {
            "staging" => prepared.staging_root(),
            _ => prepared.trash_root(),
        };
        let operation = PathBuf::from(std::ffi::OsString::from_vec(registered.as_bytes().to_vec()));
        let original = temp.path().join(format!("original-{kind}"));
        fs::rename(&operation, &original).unwrap();
        fs::create_dir(&operation).unwrap();
        fs::write(operation.join("foreign"), b"preserve").unwrap();

        assert_eq!(
            adapter
                .remove_workspace(&lock, &layout, &reservation)
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );
        assert_eq!(fs::read(operation.join("foreign")).unwrap(), b"preserve");
        assert!(original.is_dir());
        assert!(target.is_dir());
    }
}

#[test]
fn registered_target_rejects_each_independent_identity_mismatch() {
    let (temp, adapter, layout, lock, identity) = fixture();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let target = temp.path().join("clone");
    let id = WorkspaceId::from_str("ws_01890a5d-ac96-774b-bd5b-55c7b8d09f49").unwrap();
    let reservation = reservation_for(&identity, id, &source, &target);
    let prepared = adapter
        .prepare_workspace(&lock, &layout, id, reservation.target_path())
        .unwrap();
    adapter
        .clear_workspace_incomplete(&lock, &layout, prepared)
        .unwrap();
    let other_instance = InstanceId::from_str("01890a5d-ac96-774b-bd5b-55c7b8d09f34").unwrap();
    let other_volume =
        thinws_core::VolumeId::from_str("550e8400-e29b-41d4-a716-446655440001").unwrap();
    let wrong_instance = WorkspaceReservation::new(
        id,
        other_instance,
        reservation.name().clone(),
        reservation.source_path().clone(),
        reservation.target_path().clone(),
        reservation.source_volume_id(),
        reservation.target_volume_id(),
        false,
        UnixMillis::new(1_700_000_000_000).unwrap(),
    );
    let wrong_source_volume = WorkspaceReservation::new(
        id,
        identity.instance_id(),
        reservation.name().clone(),
        reservation.source_path().clone(),
        reservation.target_path().clone(),
        other_volume,
        reservation.target_volume_id(),
        false,
        UnixMillis::new(1_700_000_000_000).unwrap(),
    );
    for invalid in [&wrong_instance, &wrong_source_volume] {
        assert_eq!(
            adapter
                .inspect_removal_container(&lock, &layout, invalid)
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout
        );
    }

    let document = temp
        .path()
        .join(format!(".thinws/metadata/ownership-{id}.toml"));
    let original = fs::read_to_string(&document).unwrap();
    for (field, old, new) in [
        (
            "instance_id",
            identity.instance_id().to_string(),
            other_instance.to_string(),
        ),
        (
            "volume_id",
            identity.volume_id().to_string(),
            other_volume.to_string(),
        ),
    ] {
        let changed = original.replace(
            &format!("{field} = \"{old}\""),
            &format!("{field} = \"{new}\""),
        );
        assert_ne!(changed, original);
        fs::write(&document, changed).unwrap();
        assert_eq!(
            adapter
                .inspect_removal_container(&lock, &layout, &reservation)
                .unwrap_err()
                .kind(),
            PortErrorKind::InvalidLayout,
            "{field}"
        );
        fs::write(&document, &original).unwrap();
    }
    assert_eq!(
        adapter
            .inspect_removal_container(&lock, &layout, &reservation)
            .unwrap(),
        Some(absolute(&target))
    );
}

#[test]
fn registered_target_rejects_replaced_parent_even_when_target_is_original() {
    let (temp, adapter, layout, lock, identity) = fixture();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let parent = temp.path().join("copies");
    fs::create_dir(&parent).unwrap();
    let target = parent.join("clone");
    let id = WorkspaceId::new();
    let reservation = reservation_for(&identity, id, &source, &target);
    let prepared = adapter
        .prepare_workspace(&lock, &layout, id, reservation.target_path())
        .unwrap();
    adapter
        .clear_workspace_incomplete(&lock, &layout, prepared)
        .unwrap();

    let old_parent = temp.path().join("old-copies");
    fs::rename(&parent, &old_parent).unwrap();
    fs::create_dir(&parent).unwrap();
    fs::rename(old_parent.join("clone"), &target).unwrap();

    assert_eq!(
        adapter
            .inspect_removal_container(&lock, &layout, &reservation)
            .unwrap_err()
            .kind(),
        PortErrorKind::InvalidLayout
    );
    assert!(target.is_dir());
}

#[test]
fn ready_target_rejects_missing_or_corrupted_control_proof() {
    let (temp, adapter, layout, lock, identity) = fixture();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let target = temp.path().join("clone");
    let id = WorkspaceId::from_str("ws_01890a5d-ac96-774b-bd5b-55c7b8d09f46").unwrap();
    let reservation = reservation_for(&identity, id, &source, &target);
    let prepared = adapter
        .prepare_workspace(&lock, &layout, id, reservation.target_path())
        .unwrap();
    adapter
        .clear_workspace_incomplete(&lock, &layout, prepared)
        .unwrap();
    let document = temp
        .path()
        .join(format!(".thinws/metadata/ownership-{id}.toml"));
    let original = fs::read(&document).unwrap();
    fs::write(&document, b"not a valid ownership document").unwrap();
    assert!(
        adapter
            .validate_ready_workspace(&layout, &reservation)
            .is_err()
    );
    assert!(
        adapter
            .remove_workspace(&lock, &layout, &reservation)
            .is_err()
    );
    assert!(target.exists());
    fs::write(&document, &original).unwrap();
    let displaced = document.with_file_name("displaced-proof");
    fs::rename(&document, &displaced).unwrap();
    assert!(
        adapter
            .validate_ready_workspace(&layout, &reservation)
            .is_err()
    );
    fs::rename(&displaced, &document).unwrap();
    assert_eq!(
        adapter
            .validate_ready_workspace(&layout, &reservation)
            .unwrap(),
        absolute(&target)
    );
}
