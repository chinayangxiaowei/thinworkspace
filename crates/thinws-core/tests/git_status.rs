use thinws_core::{
    DiscoveryCompleteness, GitState, RepositoryState, StatusField, StatusParseError,
    aggregate_git_state, parse_tracked_change_count,
};

const OBJECT_ID: &str = "eca27da8231da8857fcc4d985d8eefd8d68b9703";
const OTHER_OBJECT_ID: &str = "dca27da8231da8857fcc4d985d8eefd8d68b9703";

fn ordinary(xy: &str, submodule: &str, modes: [&str; 3], objects: [&str; 2]) -> Vec<u8> {
    format!(
        "1 {xy} {submodule} {} {} {} {} {} tracked.txt\0",
        modes[0], modes[1], modes[2], objects[0], objects[1]
    )
    .into_bytes()
}

fn renamed(xy: &str, score: &str) -> Vec<u8> {
    format!(
        "2 {xy} N... 100644 100644 100644 {OBJECT_ID} {OBJECT_ID} {score} target.txt\0source.txt\0"
    )
    .into_bytes()
}

fn invalid_field(record_type: u8, field: StatusField) -> StatusParseError {
    StatusParseError::InvalidField {
        offset: 0,
        record_type,
        field,
    }
}

#[test]
fn porcelain_v2_counts_only_current_tracked_paths() {
    let output = format!(
        "1 .M N... 100644 100644 100644 {OBJECT_ID} {OBJECT_ID} tracked.txt\0? untracked.txt\0! ignored.txt\0"
    );
    assert_eq!(parse_tracked_change_count(output.as_bytes()), Ok(1));
}

#[test]
fn porcelain_v2_validates_headers_and_other_item_paths() {
    assert_eq!(
        parse_tracked_change_count(b"# branch.oid abc123\0? new.txt\0! ignored.txt\0"),
        Ok(0)
    );
    assert_eq!(parse_tracked_change_count(b"# a\0? b\0! c\0"), Ok(0));
    for record in [
        b"#\0".as_slice(),
        b"# \0",
        b"#  branch.oid abc123\0",
        b"#branch.oid abc123\0",
        b"# branch.oid\nabc123\0",
        b"# branch.oid\rabc123\0",
    ] {
        assert_eq!(
            parse_tracked_change_count(record),
            Err(invalid_field(b'#', StatusField::Header)),
            "record: {record:?}"
        );
    }
    for record in [
        b"?\0".as_slice(),
        b"? \0",
        b"?x\0",
        b"!x\0",
        b"?x path.txt\0",
        b"!x path.txt\0",
    ] {
        assert_eq!(
            parse_tracked_change_count(record),
            Err(invalid_field(record[0], StatusField::Path)),
            "record: {record:?}"
        );
    }
    assert_eq!(
        parse_tracked_change_count(b"? new.txt\0# branch.oid abc123\0"),
        Ok(0)
    );
}

#[test]
fn porcelain_v2_reports_precise_truncated_and_unknown_records() {
    assert_eq!(
        parse_tracked_change_count(b"? new.txt\0# missing-nul"),
        Err(StatusParseError::TruncatedRecord {
            offset: 10,
            record_type: Some(b'#'),
        })
    );
    assert_eq!(
        parse_tracked_change_count(b"x unknown\0"),
        Err(StatusParseError::UnknownRecord {
            offset: 0,
            record_type: b'x',
        })
    );
    assert_eq!(
        StatusParseError::MissingRenameSource { offset: 7 }.to_string(),
        "invalid Git porcelain-v2 output: MissingRenameSource { offset: 7 }"
    );
}

#[test]
fn ordinary_untracked_only_submodule_requires_all_unchanged_gitlink_facts() {
    let modes = ["160000", "160000", "160000"];
    let objects = [OBJECT_ID, OBJECT_ID];
    assert_eq!(
        parse_tracked_change_count(&ordinary(".M", "S..U", modes, objects)),
        Ok(0)
    );
    for record in [
        ordinary("M.", "S..U", modes, objects),
        ordinary(".M", "S.MU", modes, objects),
        ordinary(".M", "S..U", ["100644", "100644", "100644"], objects),
        ordinary(".M", "S..U", ["160000", "100644", "100644"], objects),
        ordinary(".M", "S..U", ["160000", "160000", "100644"], objects),
        ordinary(".M", "S..U", modes, [OBJECT_ID, OTHER_OBJECT_ID]),
    ] {
        assert_eq!(parse_tracked_change_count(&record), Ok(1), "{record:?}");
    }
}

#[test]
fn ordinary_status_rejects_nonordinary_xy_and_invalid_fields() {
    let modes = ["100644", "100644", "100644"];
    let objects = [OBJECT_ID, OBJECT_ID];
    for xy in ["..", "R.", ".C", "Z.", "M"] {
        assert_eq!(
            parse_tracked_change_count(&ordinary(xy, "N...", modes, objects)),
            Err(invalid_field(b'1', StatusField::Xy)),
            "xy: {xy}"
        );
    }
    for submodule in ["X...", "Sx..", "S.x.", "S..x", "S...."] {
        assert_eq!(
            parse_tracked_change_count(&ordinary(".M", submodule, modes, objects)),
            Err(invalid_field(b'1', StatusField::Submodule)),
            "submodule: {submodule}"
        );
    }
    assert_eq!(
        parse_tracked_change_count(&ordinary(
            ".M",
            "N...",
            ["10064", "100644", "100644"],
            objects,
        )),
        Err(invalid_field(b'1', StatusField::HeadMode))
    );
    assert_eq!(
        parse_tracked_change_count(&ordinary(
            ".M",
            "N...",
            ["100644", "100644", "100644"],
            ["zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz", OBJECT_ID],
        )),
        Err(invalid_field(b'1', StatusField::HeadObject))
    );
}

#[test]
fn rename_scores_require_matching_kind_and_decimal_range() {
    assert_eq!(parse_tracked_change_count(&renamed("R.", "R100")), Ok(1));
    assert_eq!(parse_tracked_change_count(&renamed(".C", "C0")), Ok(1));
    for score in ["R101", "R1000", "R", "Rx", "C100", "X100"] {
        assert_eq!(
            parse_tracked_change_count(&renamed("R.", score)),
            Err(invalid_field(b'2', StatusField::Score)),
            "score: {score}"
        );
    }
    let mut missing_source = renamed("R.", "R100");
    missing_source.truncate(missing_source.iter().position(|byte| *byte == 0).unwrap() + 1);
    assert_eq!(
        parse_tracked_change_count(&missing_source),
        Err(StatusParseError::MissingRenameSource { offset: 0 })
    );

    let mut followed_by_ordinary = renamed("R.", "R100");
    followed_by_ordinary.extend(ordinary(
        ".M",
        "N...",
        ["100644", "100644", "100644"],
        [OBJECT_ID, OBJECT_ID],
    ));
    assert_eq!(parse_tracked_change_count(&followed_by_ordinary), Ok(2));
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
