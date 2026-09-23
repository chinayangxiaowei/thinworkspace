use std::str::FromStr;

use thinws_core::{
    AbsolutePath, AbsolutePathError, ErrorCode, RemovalMode, UnixMillis, VolumeId, WorkspaceEvent,
    WorkspaceState,
};

#[test]
fn persisted_path_volume_time_and_error_code_values_reject_ambiguous_encodings() {
    let path = AbsolutePath::try_from_bytes(b"/Users/example/project\xff".to_vec())
        .expect("non-UTF-8 path bytes remain lossless");
    assert_eq!(path.as_bytes(), b"/Users/example/project\xff");
    assert!(AbsolutePath::try_from_bytes(b"/".to_vec()).is_ok());
    assert_eq!(
        AbsolutePath::try_from_bytes(b"relative".to_vec()).unwrap_err(),
        AbsolutePathError::NotAbsolute
    );
    for invalid in [
        b"/tmp//item".as_slice(),
        b"/tmp/./item".as_slice(),
        b"/tmp/../item".as_slice(),
        b"/tmp/item/".as_slice(),
    ] {
        assert_eq!(
            AbsolutePath::try_from_bytes(invalid.to_vec()).unwrap_err(),
            AbsolutePathError::NonCanonicalComponent
        );
    }
    assert_eq!(
        AbsolutePath::try_from_bytes(b"/tmp/a\0b".to_vec()).unwrap_err(),
        AbsolutePathError::ContainsNul
    );

    let volume = VolumeId::from_str("550e8400-e29b-41d4-a716-446655440000")
        .expect("volume IDs accept canonical UUIDs of any version");
    assert_eq!(volume.to_string(), "550e8400-e29b-41d4-a716-446655440000");
    assert!(VolumeId::from_str("550E8400-E29B-41D4-A716-446655440000").is_err());
    assert!(VolumeId::from_str("550e8400e29b41d4a716446655440000").is_err());

    assert_eq!(UnixMillis::new(0).unwrap().get(), 0);
    assert_eq!(UnixMillis::new(i64::MAX).unwrap().get(), i64::MAX);
    assert!(UnixMillis::new(-1).is_err());
    assert_eq!(
        ErrorCode::from_str("E_WORKSPACE_INCOMPLETE").unwrap(),
        ErrorCode::WorkspaceIncomplete
    );
    assert!(ErrorCode::from_str("E_UNKNOWN").is_err());
}

#[test]
fn workspace_state_machine_accepts_only_the_detailed_design_edges() {
    use RemovalMode::{Force, Normal};
    use WorkspaceEvent::{BeginRemoval, Failed, Materialized};
    use WorkspaceState::{Creating, Deleting, Error, Ready};

    let allowed = [
        (Creating, Materialized, Ready),
        (Creating, Failed, Error),
        (Creating, BeginRemoval(Force), Deleting),
        (Ready, Failed, Error),
        (Ready, BeginRemoval(Normal), Deleting),
        (Ready, BeginRemoval(Force), Deleting),
        (Deleting, Failed, Error),
        (Deleting, BeginRemoval(Force), Deleting),
        (Error, BeginRemoval(Force), Deleting),
    ];
    for (state, event, expected) in allowed {
        assert_eq!(
            state.transition(event),
            Ok(expected),
            "{state:?} + {event:?}"
        );
    }

    let all_states = [Creating, Ready, Deleting, Error];
    let all_events = [
        Materialized,
        Failed,
        BeginRemoval(Normal),
        BeginRemoval(Force),
    ];
    for state in all_states {
        for event in all_events {
            if allowed
                .iter()
                .any(|edge| edge.0 == state && edge.1 == event)
            {
                continue;
            }
            let error = state.transition(event).unwrap_err();
            assert_eq!(error.current, state);
            assert_eq!(error.event, event);
        }
    }
}
