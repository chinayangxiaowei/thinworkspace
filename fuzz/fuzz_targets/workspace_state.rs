#![no_main]

use libfuzzer_sys::fuzz_target;
use thinws_core::{RemovalMode, WorkspaceEvent, WorkspaceState};

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }
    let mut state = state_from_byte(data[0]);
    for byte in &data[1..] {
        let event = event_from_byte(*byte);
        let previous = state;
        match state.transition(event) {
            Ok(next) => {
                assert_transition(previous, event, next);
                state = next;
            }
            Err(error) => {
                assert_eq!(error.current, previous);
                assert_eq!(error.event, event);
            }
        }
        assert_eq!(state.as_str().parse::<WorkspaceState>().unwrap(), state);
    }
});

fn state_from_byte(byte: u8) -> WorkspaceState {
    match byte % 4 {
        0 => WorkspaceState::Creating,
        1 => WorkspaceState::Ready,
        2 => WorkspaceState::Deleting,
        _ => WorkspaceState::Error,
    }
}

fn event_from_byte(byte: u8) -> WorkspaceEvent {
    match byte % 4 {
        0 => WorkspaceEvent::Materialized,
        1 => WorkspaceEvent::Failed,
        2 => WorkspaceEvent::BeginRemoval(RemovalMode::Normal),
        _ => WorkspaceEvent::BeginRemoval(RemovalMode::Force),
    }
}

fn assert_transition(previous: WorkspaceState, event: WorkspaceEvent, next: WorkspaceState) {
    match next {
        WorkspaceState::Creating => panic!("no event returns to Creating"),
        WorkspaceState::Ready => {
            assert_eq!(previous, WorkspaceState::Creating);
            assert_eq!(event, WorkspaceEvent::Materialized);
        }
        WorkspaceState::Error => {
            assert_ne!(previous, WorkspaceState::Error);
            assert_eq!(event, WorkspaceEvent::Failed);
        }
        WorkspaceState::Deleting => match event {
            WorkspaceEvent::BeginRemoval(RemovalMode::Normal) => {
                assert_eq!(previous, WorkspaceState::Ready);
            }
            WorkspaceEvent::BeginRemoval(RemovalMode::Force) => {}
            _ => panic!("only removal events enter Deleting"),
        },
    }
}
