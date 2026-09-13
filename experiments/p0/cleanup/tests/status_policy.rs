use thinws_p0_cleanup::{
    DiscoveryCompleteness, GitState, PathValidation, ProcessUse, RemovalDecision, RemovalMode,
    RemovalPreflight, RemovalRefusal, RemovalWarning, RepositoryState, StatusField,
    StatusParseError, VolumeValidation, aggregate_git_state, decide_removal,
    parse_tracked_change_count,
};

const OID_A: &str = "eca27da8231da8857fcc4d985d8eefd8d68b9703";
const OID_B: &str = "7ad1242e1c389bad60b066800f3b0517ac8be466";

fn record(fields: &str, path: &[u8]) -> Vec<u8> {
    let mut output = fields.as_bytes().to_vec();
    output.extend_from_slice(path);
    output.push(0);
    output
}

fn ordinary(xy: &str, submodule: &str, path: &[u8]) -> Vec<u8> {
    record(
        &format!("1 {xy} {submodule} 100644 100644 100644 {OID_A} {OID_B} "),
        path,
    )
}

fn preflight(
    path: PathValidation,
    volume: VolumeValidation,
    process_use: ProcessUse,
    git: GitState,
) -> RemovalPreflight {
    RemovalPreflight {
        path,
        volume,
        process_use,
        git,
    }
}

#[test]
fn parses_real_parent_and_child_outputs() {
    let parent = concat!(
        "1 .M S.M. 160000 160000 160000 ",
        "eca27da8231da8857fcc4d985d8eefd8d68b9703 ",
        "eca27da8231da8857fcc4d985d8eefd8d68b9703 child\0",
        "2 R. N... 100644 100644 100644 ",
        "7ad1242e1c389bad60b066800f3b0517ac8be466 ",
        "7ad1242e1c389bad60b066800f3b0517ac8be466 ",
        "R100 renamed with space.txt\0tracked.txt\0",
    );
    let child = concat!(
        "1 .M N... 100644 100644 100644 ",
        "969f86ef5ffdaa93107e120fdb867fc9e72b7eb0 ",
        "969f86ef5ffdaa93107e120fdb867fc9e72b7eb0 tracked.txt\0",
    );

    assert_eq!(parse_tracked_change_count(parent.as_bytes()), Ok(2));
    assert_eq!(parse_tracked_change_count(child.as_bytes()), Ok(1));
}

#[test]
fn deduplicates_lossless_paths_and_ignores_untracked_and_ignored() {
    let mut output = b"# future.header arbitrary value\0".to_vec();
    output.extend(ordinary("M.", "N...", b"space and\nnewline.txt"));
    output.extend(ordinary(".M", "N...", b"space and\nnewline.txt"));
    output.extend(ordinary("A.", "N...", b"non-utf8-\xff"));
    output.extend_from_slice(b"? untracked\xff\0! ignored file\0");

    assert_eq!(parse_tracked_change_count(&output), Ok(2));
}

#[test]
fn rename_counts_only_the_target_and_requires_the_source() {
    let mut output =
        format!("2 R. N... 100644 100644 100644 {OID_A} {OID_B} R087 target name").into_bytes();
    output.push(0);
    output.extend_from_slice(b"original\nname");
    output.push(0);
    output.extend(ordinary(".M", "N...", b"target name"));
    output.extend(ordinary("M.", "N...", b"later target"));

    assert_eq!(parse_tracked_change_count(&output), Ok(2));

    let mut missing_source =
        format!("2 R. N... 100644 100644 100644 {OID_A} {OID_B} R100 target").into_bytes();
    missing_source.push(0);
    assert!(matches!(
        parse_tracked_change_count(&missing_source),
        Err(StatusParseError::MissingRenameSource { .. })
    ));
}

#[test]
fn parses_copy_records() {
    let output = format!("2 C. N... 100644 100644 100644 {OID_A} {OID_B} C075 copied\0source\0");
    assert_eq!(parse_tracked_change_count(output.as_bytes()), Ok(1));
}

#[test]
fn parses_unmerged_records() {
    let output =
        format!("u UU N... 100644 100644 100644 100644 {OID_A} {OID_A} {OID_B} conflict path\0");
    assert_eq!(parse_tracked_change_count(output.as_bytes()), Ok(1));
}

#[test]
fn pure_untracked_submodule_state_is_not_a_tracked_change() {
    let only_untracked = concat!(
        "1 .M S..U 160000 160000 160000 ",
        "eca27da8231da8857fcc4d985d8eefd8d68b9703 ",
        "eca27da8231da8857fcc4d985d8eefd8d68b9703 child\0",
    );
    assert_eq!(parse_tracked_change_count(only_untracked.as_bytes()), Ok(0));

    for must_count in [
        format!("1 M. S..U 160000 160000 160000 {OID_A} {OID_A} staged-gitlink\0"),
        format!("1 .M S.MU 160000 160000 160000 {OID_A} {OID_A} tracked-child\0"),
        format!("1 .T S..U 160000 160000 100644 {OID_A} {OID_A} mode-change\0"),
        format!("1 .M S..U 160000 160000 160000 {OID_A} {OID_B} unexplained-difference\0"),
    ] {
        assert_eq!(parse_tracked_change_count(must_count.as_bytes()), Ok(1));
    }
}

#[test]
fn rejects_unknown_invalid_and_truncated_structures() {
    assert!(matches!(
        parse_tracked_change_count(b"x future\0"),
        Err(StatusParseError::UnknownRecord {
            record_type: b'x',
            ..
        })
    ));
    assert!(matches!(
        parse_tracked_change_count(
            format!("1 Z. N... 100644 100644 100644 {OID_A} {OID_B} path\0").as_bytes()
        ),
        Err(StatusParseError::InvalidField {
            field: StatusField::Xy,
            ..
        })
    ));
    assert!(matches!(
        parse_tracked_change_count(b"1 M. N..."),
        Err(StatusParseError::TruncatedRecord { .. })
    ));
    assert!(matches!(
        parse_tracked_change_count(b"1 M. N...\0"),
        Err(StatusParseError::TruncatedRecord { .. })
    ));
    assert!(matches!(
        parse_tracked_change_count(
            format!("2 R. N... 100644 100644 100644 {OID_A} {OID_B} Z100 target\0source\0")
                .as_bytes()
        ),
        Err(StatusParseError::InvalidField {
            field: StatusField::Score,
            ..
        })
    ));
    assert!(matches!(
        parse_tracked_change_count(b"#malformed\0"),
        Err(StatusParseError::InvalidField {
            field: StatusField::Header,
            ..
        })
    ));
    let header_hiding_record =
        format!("# future.header value\n1 M. N... 100644 100644 100644 {OID_A} {OID_B} path\0");
    assert!(matches!(
        parse_tracked_change_count(header_hiding_record.as_bytes()),
        Err(StatusParseError::InvalidField {
            field: StatusField::Header,
            ..
        })
    ));
    assert!(matches!(
        parse_tracked_change_count(
            format!("1x M. N... 100644 100644 100644 {OID_A} {OID_B} path\0").as_bytes()
        ),
        Err(StatusParseError::InvalidField {
            field: StatusField::RecordType,
            ..
        })
    ));
    assert!(matches!(
        parse_tracked_change_count(
            format!("1 M. S.X. 160000 160000 160000 {OID_A} {OID_A} child\0").as_bytes()
        ),
        Err(StatusParseError::InvalidField {
            field: StatusField::Submodule,
            ..
        })
    ));
    assert!(matches!(
        parse_tracked_change_count(
            format!("1 M. N... 10064x 100644 100644 {OID_A} {OID_B} path\0").as_bytes()
        ),
        Err(StatusParseError::InvalidField {
            field: StatusField::HeadMode,
            ..
        })
    ));
    assert!(matches!(
        parse_tracked_change_count(
            format!("1 M. N... 100644 100644 100644 not-an-object {OID_B} path\0").as_bytes()
        ),
        Err(StatusParseError::InvalidField {
            field: StatusField::HeadObject,
            ..
        })
    ));
    assert!(matches!(
        parse_tracked_change_count(
            format!("1 .. N... 100644 100644 100644 {OID_A} {OID_B} path\0").as_bytes()
        ),
        Err(StatusParseError::InvalidField {
            field: StatusField::Xy,
            ..
        })
    ));
    assert!(matches!(
        parse_tracked_change_count(
            format!("1 R. N... 100644 100644 100644 {OID_A} {OID_B} path\0").as_bytes()
        ),
        Err(StatusParseError::InvalidField {
            field: StatusField::Xy,
            ..
        })
    ));
    assert!(matches!(
        parse_tracked_change_count(b"?\0"),
        Err(StatusParseError::InvalidField {
            field: StatusField::Path,
            ..
        })
    ));
}

#[test]
fn validates_other_item_and_header_boundaries() {
    assert_eq!(parse_tracked_change_count(b"? a\0! b\0"), Ok(0));

    for invalid_item in [b"?xa\0".as_slice(), b"!xb\0".as_slice()] {
        assert!(matches!(
            parse_tracked_change_count(invalid_item),
            Err(StatusParseError::InvalidField {
                field: StatusField::Path,
                ..
            })
        ));
    }

    assert_eq!(parse_tracked_change_count(b"# a\0"), Ok(0));
    for invalid_header in [
        b"#\0".as_slice(),
        b"# \0".as_slice(),
        b"#  a\0".as_slice(),
        b"# a\r\0".as_slice(),
        b"# a\n\0".as_slice(),
    ] {
        assert!(matches!(
            parse_tracked_change_count(invalid_header),
            Err(StatusParseError::InvalidField {
                field: StatusField::Header,
                ..
            })
        ));
    }
}

#[test]
fn validates_object_lengths_and_hex_content_independently() {
    for invalid_object in [
        format!("{}g", "a".repeat(39)),
        format!("{}g", "a".repeat(63)),
        "a".repeat(39),
        "a".repeat(65),
    ] {
        let output = format!("1 M. N... 100644 100644 100644 {invalid_object} {OID_B} path\0");
        assert!(matches!(
            parse_tracked_change_count(output.as_bytes()),
            Err(StatusParseError::InvalidField {
                field: StatusField::HeadObject,
                ..
            })
        ));
    }

    let sha256_object = "a".repeat(64);
    let output = format!("1 M. N... 100644 100644 100644 {sha256_object} {sha256_object} path\0");
    assert_eq!(parse_tracked_change_count(output.as_bytes()), Ok(1));
}

#[test]
fn rejects_out_of_range_and_mismatched_rename_scores() {
    for (xy, score) in [
        ("R.", "R101"),
        ("C.", "C999"),
        ("C.", "R100"),
        ("R.", "C100"),
    ] {
        let output =
            format!("2 {xy} N... 100644 100644 100644 {OID_A} {OID_B} {score} target\0source\0");
        assert!(matches!(
            parse_tracked_change_count(output.as_bytes()),
            Err(StatusParseError::InvalidField {
                field: StatusField::Score,
                ..
            })
        ));
    }
}

#[test]
fn empty_tracked_paths_are_rejected_by_fixed_field_parsing() {
    let empty_ordinary = format!("1 M. N... 100644 100644 100644 {OID_A} {OID_B} \0");
    let empty_rename = format!("2 R. N... 100644 100644 100644 {OID_A} {OID_B} R100 \0source\0");
    let empty_unmerged =
        format!("u UU N... 100644 100644 100644 100644 {OID_A} {OID_A} {OID_B} \0");

    for output in [empty_ordinary, empty_rename, empty_unmerged] {
        assert!(matches!(
            parse_tracked_change_count(output.as_bytes()),
            Err(StatusParseError::TruncatedRecord { .. })
        ));
    }
}

#[test]
fn parse_errors_have_nonempty_diagnostics() {
    let rendered = StatusParseError::UnknownRecord {
        offset: 7,
        record_type: b'x',
    }
    .to_string();

    assert!(!rendered.is_empty());
    assert!(rendered.contains("porcelain-v2"));
    assert!(rendered.contains("UnknownRecord"));
}

#[test]
fn aggregate_precedence_is_unknown_then_dirty_then_clean_or_not_applicable() {
    let clean = RepositoryState::from_tracked_change_count(0);
    let dirty = RepositoryState::from_tracked_change_count(3);

    assert_eq!(
        aggregate_git_state(DiscoveryCompleteness::Incomplete, &[dirty]),
        GitState::Unknown
    );
    assert_eq!(
        aggregate_git_state(
            DiscoveryCompleteness::Complete,
            &[dirty, RepositoryState::Unknown]
        ),
        GitState::Unknown
    );
    assert_eq!(
        aggregate_git_state(DiscoveryCompleteness::Complete, &[clean, dirty]),
        GitState::Dirty
    );
    assert_eq!(
        aggregate_git_state(DiscoveryCompleteness::Complete, &[clean]),
        GitState::Clean
    );
    assert_eq!(
        aggregate_git_state(DiscoveryCompleteness::Complete, &[]),
        GitState::NotApplicable
    );
    assert_eq!(
        aggregate_git_state(DiscoveryCompleteness::Incomplete, &[]),
        GitState::Unknown
    );
    assert_eq!(RepositoryState::Unknown.tracked_change_count(), None);
    assert_eq!(clean.tracked_change_count(), Some(0));
    assert_eq!(dirty.tracked_change_count(), Some(3));
}

#[test]
fn non_bypassable_refusals_have_stable_priority_for_both_modes() {
    for mode in [RemovalMode::Normal, RemovalMode::Force] {
        assert_eq!(
            decide_removal(
                preflight(
                    PathValidation::Failed,
                    VolumeValidation::Failed,
                    ProcessUse::ConfirmedInUse,
                    GitState::Unknown,
                ),
                mode,
            ),
            RemovalDecision::Refuse(RemovalRefusal::UnsafePath)
        );
        assert_eq!(
            decide_removal(
                preflight(
                    PathValidation::Valid,
                    VolumeValidation::Failed,
                    ProcessUse::ConfirmedInUse,
                    GitState::Dirty,
                ),
                mode,
            ),
            RemovalDecision::Refuse(RemovalRefusal::VolumeMismatch)
        );
        assert_eq!(
            decide_removal(
                preflight(
                    PathValidation::Valid,
                    VolumeValidation::Valid,
                    ProcessUse::ConfirmedInUse,
                    GitState::Unknown,
                ),
                mode,
            ),
            RemovalDecision::Refuse(RemovalRefusal::ConfirmedInUse)
        );
    }
}

#[test]
fn force_bypasses_only_git_dirty_and_unknown() {
    let safe = |git| {
        preflight(
            PathValidation::Valid,
            VolumeValidation::Valid,
            ProcessUse::NoEvidence,
            git,
        )
    };

    assert_eq!(
        decide_removal(safe(GitState::Unknown), RemovalMode::Normal),
        RemovalDecision::Refuse(RemovalRefusal::GitCheckIncomplete)
    );
    assert_eq!(
        decide_removal(safe(GitState::Dirty), RemovalMode::Normal),
        RemovalDecision::Refuse(RemovalRefusal::TrackedChanges)
    );
    assert_eq!(
        decide_removal(safe(GitState::Unknown), RemovalMode::Force),
        RemovalDecision::Proceed { warning: None }
    );
    assert_eq!(
        decide_removal(safe(GitState::Dirty), RemovalMode::Force),
        RemovalDecision::Proceed { warning: None }
    );
    assert_eq!(
        decide_removal(safe(GitState::Clean), RemovalMode::Normal),
        RemovalDecision::Proceed { warning: None }
    );
    assert_eq!(
        decide_removal(safe(GitState::NotApplicable), RemovalMode::Normal),
        RemovalDecision::Proceed { warning: None }
    );
}

#[test]
fn incomplete_process_scan_warns_but_does_not_refuse() {
    for mode in [RemovalMode::Normal, RemovalMode::Force] {
        assert_eq!(
            decide_removal(
                preflight(
                    PathValidation::Valid,
                    VolumeValidation::Valid,
                    ProcessUse::ScanIncomplete,
                    GitState::Clean,
                ),
                mode,
            ),
            RemovalDecision::Proceed {
                warning: Some(RemovalWarning::ProcessScanIncomplete)
            }
        );
    }
}
