use thinws_core::{
    DiscoveryCompleteness, GitState, RepositoryState, aggregate_git_state,
    parse_tracked_change_count,
};

const OBJECT_ID: &str = "eca27da8231da8857fcc4d985d8eefd8d68b9703";

#[test]
fn porcelain_v2_counts_only_current_tracked_paths() {
    let output = format!(
        "1 .M N... 100644 100644 100644 {OBJECT_ID} {OBJECT_ID} tracked.txt\0? untracked.txt\0! ignored.txt\0"
    );
    assert_eq!(parse_tracked_change_count(output.as_bytes()), Ok(1));
}

#[test]
fn no_repository_and_incomplete_discovery_are_distinct() {
    assert_eq!(
        aggregate_git_state(DiscoveryCompleteness::Complete, &[]),
        GitState::NotApplicable
    );
    assert_eq!(
        aggregate_git_state(DiscoveryCompleteness::Incomplete, &[]),
        GitState::Unknown
    );
    assert_eq!(
        aggregate_git_state(DiscoveryCompleteness::Complete, &[RepositoryState::Unknown]),
        GitState::Unknown
    );
}
