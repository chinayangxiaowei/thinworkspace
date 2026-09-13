#![no_main]

use libfuzzer_sys::fuzz_target;
use thinws_p0_cleanup::removal_log::{
    EventOutcome, RemovalLogEvent, RepositoryEvidence, RepositoryRelativePath, SafeId,
};
use thinws_p0_cleanup::{
    DiscoveryCompleteness, GitState, PathValidation, ProcessUse, RemovalDecision, RemovalMode,
    RemovalPreflight, RepositoryState, VolumeValidation, aggregate_git_state, decide_removal,
    parse_tracked_change_count,
};

const ORDINARY: &[u8] = b"1 .M N... 100644 100644 100644 0123456789012345678901234567890123456789 0123456789012345678901234567890123456789 ";

fuzz_target!(|data: &[u8]| {
    // Raw bytes exercise malformed records without filesystem or Git access.
    let _ = parse_tracked_change_count(data);
    #[cfg(fuzzing)]
    thinws_p0_cleanup::git_query::exercise_pure_parsers(data);

    // Exercise the event's bounded field types and JSON escaping in memory.
    // No fuzz input is passed to File, Git, a filesystem path, or deletion.
    if let Ok(text) = std::str::from_utf8(data) {
        let expected = !text.is_empty()
            && text.len() <= 128
            && text.chars().all(
                |character| matches!(character, 'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '-'),
            );
        assert_eq!(SafeId::new(text).is_ok(), expected);
    }
    if let Ok(relative_path) = RepositoryRelativePath::new(data) {
        let (process_use, expected_process_use) = match data.first().copied().unwrap_or(0) % 3 {
            0 => (ProcessUse::NoEvidence, "no_evidence"),
            1 => (ProcessUse::ScanIncomplete, "scan_incomplete"),
            _ => (ProcessUse::ConfirmedInUse, "confirmed_in_use"),
        };
        let repositories = [RepositoryEvidence {
            relative_path,
            tracked_changes: None,
        }];
        let event = RemovalLogEvent {
            utc_unix_ms: 0,
            operation_id: SafeId::new("op_fuzz").expect("static valid experiment ID"),
            workspace_id: SafeId::new("ws_fuzz").expect("static valid experiment ID"),
            force: true,
            check_completeness: DiscoveryCompleteness::Incomplete.into(),
            process_use: process_use.into(),
            repositories: &repositories,
            outcome: EventOutcome::Started,
        };
        let encoded = serde_json::to_vec(&event).expect("typed event is serializable");
        assert!(encoded.is_ascii());
        assert!(!encoded.contains(&b'\n'));
        assert!(!encoded.contains(&b'\r'));
        let decoded: serde_json::Value = serde_json::from_slice(&encoded).expect("valid JSON");
        let escaped = decoded["repositories"][0]["relative_path"]
            .as_str()
            .expect("path is an escaped string");
        assert!(!escaped.bytes().any(|byte| byte.is_ascii_control()));
        assert!(decoded["repositories"][0]["tracked_changes"].is_null());
        assert_eq!(decoded["event"], "started");
        assert_eq!(decoded["force"], true);
        assert_eq!(decoded["process_use"], expected_process_use);
    }
    for forbidden_prefix in [b"/".as_slice(), b"../", b"\0"] {
        let mut invalid = forbidden_prefix.to_vec();
        invalid.extend_from_slice(data);
        assert!(RepositoryRelativePath::new(&invalid).is_err());
    }

    // A generated valid path makes the success-side invariants reachable without
    // requiring libFuzzer to discover every fixed porcelain field first.
    let mut path = b"tracked/".to_vec();
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in data {
        path.push(HEX[usize::from(byte >> 4)]);
        path.push(HEX[usize::from(byte & 15)]);
    }
    path.push(b'x');
    let mut record = ORDINARY.to_vec();
    record.extend_from_slice(&path);
    record.push(0);
    assert_eq!(parse_tracked_change_count(&record), Ok(1));
    assert!(parse_tracked_change_count(&record[..record.len() - 1]).is_err());

    let mut duplicate = record.clone();
    duplicate.extend_from_slice(&record);
    assert_eq!(parse_tracked_change_count(&duplicate), Ok(1));
    for prefix in [b"? ", b"! "] {
        duplicate.extend_from_slice(prefix);
        duplicate.extend_from_slice(&path);
        duplicate.push(0);
    }
    assert_eq!(parse_tracked_change_count(&duplicate), Ok(1));

    let mut second = ORDINARY.to_vec();
    second.extend_from_slice(b"second-");
    second.extend_from_slice(&path);
    second.push(0);
    duplicate.extend_from_slice(&second);
    assert_eq!(parse_tracked_change_count(&duplicate), Ok(2));

    let kind = if data.first().is_some_and(|byte| byte & 1 != 0) {
        'C'
    } else {
        'R'
    };
    let score = data.get(1).copied().unwrap_or(0) % 101;
    let mut rename = format!(
        "2 {kind}. N... 100644 100644 100644 0123456789012345678901234567890123456789 0123456789012345678901234567890123456789 {kind}{score} "
    ).into_bytes();
    rename.extend_from_slice(&path);
    rename.push(0);
    assert!(parse_tracked_change_count(&rename).is_err());
    rename.extend_from_slice(b"original-");
    rename.extend_from_slice(&path);
    rename.push(0);
    assert_eq!(parse_tracked_change_count(&rename), Ok(1));
    rename.extend_from_slice(&record);
    assert_eq!(parse_tracked_change_count(&rename), Ok(1));
    rename.extend_from_slice(&second);
    assert_eq!(parse_tracked_change_count(&rename), Ok(2));

    let conflicts = ["DD", "AU", "UD", "UA", "DU", "AA", "UU"];
    let xy = conflicts[usize::from(data.get(2).copied().unwrap_or(0)) % conflicts.len()];
    let mut conflict = format!(
        "u {xy} N... 100644 100644 100644 100644 0123456789012345678901234567890123456789 0123456789012345678901234567890123456789 0123456789012345678901234567890123456789 "
    ).into_bytes();
    conflict.extend_from_slice(&path);
    conflict.push(0);
    assert_eq!(parse_tracked_change_count(&conflict), Ok(1));

    let repositories = [
        RepositoryState::from_tracked_change_count(data.len()),
        RepositoryState::Unknown,
    ];
    assert_eq!(
        aggregate_git_state(DiscoveryCompleteness::Complete, &repositories),
        GitState::Unknown
    );
    assert_eq!(RepositoryState::Unknown.tracked_change_count(), None);
    assert_eq!(
        aggregate_git_state(DiscoveryCompleteness::Incomplete, &repositories[..1]),
        GitState::Unknown
    );

    // Force changes content policy, never identity or confirmed-occupancy checks.
    for git in [
        GitState::Clean,
        GitState::Dirty,
        GitState::Unknown,
        GitState::NotApplicable,
    ] {
        let valid = RemovalPreflight {
            path: PathValidation::Valid,
            volume: VolumeValidation::Valid,
            process_use: ProcessUse::NoEvidence,
            git,
        };
        assert!(matches!(
            decide_removal(valid, RemovalMode::Force),
            RemovalDecision::Proceed { .. }
        ));
        for mode in [RemovalMode::Normal, RemovalMode::Force] {
            for protected in [
                RemovalPreflight {
                    path: PathValidation::Failed,
                    ..valid
                },
                RemovalPreflight {
                    volume: VolumeValidation::Failed,
                    ..valid
                },
                RemovalPreflight {
                    process_use: ProcessUse::ConfirmedInUse,
                    ..valid
                },
            ] {
                assert!(matches!(
                    decide_removal(protected, mode),
                    RemovalDecision::Refuse(_)
                ));
            }
        }
    }
});
