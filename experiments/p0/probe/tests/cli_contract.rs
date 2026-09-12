//! Experimental CLI commands stay read-only and handle output errors without panicking.

use std::fs;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn host_report_has_measured_identity_and_versioned_experiment_marker() {
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("UTC clock")
        .as_millis();
    let output = Command::new(env!("CARGO_BIN_EXE_thinws-p0-probe"))
        .arg("host")
        .output()
        .expect("run host experiment");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON report");
    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["experiment"], "p0-01-host-path-probe");
    assert_eq!(json["operating_system"], "macos");
    assert_eq!(json["cow_evidence"], "not_executed_by_probe");
    let ended = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("UTC clock")
        .as_millis();
    let reported = u128::from(json["observed_at_unix_ms"].as_u64().expect("UTC timestamp"));
    // Allow a small wall-clock adjustment without accepting a fabricated constant timestamp.
    assert!((started.saturating_sub(60_000)..=ended + 60_000).contains(&reported));
    for field in ["architecture", "kernel_release", "product_version"] {
        assert!(!json[field].as_str().expect("measured identity").is_empty());
    }
    let system_version = Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .expect("independent macOS version query");
    assert!(system_version.status.success());
    assert_eq!(
        json["product_version"],
        String::from_utf8(system_version.stdout)
            .expect("system version is UTF-8")
            .trim()
    );
}

#[test]
fn materialization_command_handles_spaces_without_creating_the_missing_target() {
    let root = tempfile::tempdir_in("/private/tmp").expect("controlled APFS root");
    let paths = ["source with spaces", "missing target", "staging", "trash"]
        .map(|name| root.path().join(name));
    for index in [0, 2, 3] {
        fs::create_dir(&paths[index]).expect("create controlled input directory");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_thinws-p0-probe"))
        .arg("materialization")
        .args(&paths)
        .output()
        .expect("run materialization probe");
    assert!(output.status.success(), "{:?}", output.stderr);
    assert!(output.stderr.is_empty());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON report");
    assert_eq!(json["schema_version"], 1);
    assert_eq!(
        json["source"]["requested_path"]["display"],
        paths[0].to_str().unwrap()
    );
    assert_eq!(json["target_root"]["resolution"], "missing_target");
    assert_eq!(json["pairs"].as_array().expect("all pairs").len(), 6);
    let candidates = json["candidates"].as_array().expect("both backend reports");
    assert_eq!(candidates.len(), 2);
    for candidate in candidates {
        assert_eq!(candidate["state"], "supported");
        assert_eq!(candidate["assurance"], "preflight_only");
        assert_eq!(candidate["execution"], "not_attempted_by_probe");
        assert_eq!(
            candidate["policy_decision"],
            "not_evaluated_by_platform_probe"
        );
    }
    assert!(!paths[1].exists());
}

#[test]
fn invalid_command_shapes_report_usage_only_on_stderr() {
    for arguments in [vec![], vec!["unknown"], vec!["host", "extra"], vec!["path"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_thinws-p0-probe"))
            .args(arguments)
            .output()
            .expect("run invalid command");
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        let json: serde_json::Value =
            serde_json::from_slice(&output.stderr).expect("JSON usage error");
        assert_eq!(json["experiment"], "p0-01-host-path-probe");
        assert_eq!(json["outcome"], "error");
        assert_eq!(json["error"]["code"], "usage");
        assert!(
            json["error"]["message"]
                .as_str()
                .expect("usage")
                .starts_with("usage:")
        );
    }
}

#[test]
fn closed_output_and_diagnostic_channels_return_failure_without_panic() {
    let (output, reader) = UnixStream::pair().expect("controlled output socket pair");
    let diagnostic = output.try_clone().expect("independent owned descriptor");
    drop(reader);
    let status = Command::new(env!("CARGO_BIN_EXE_thinws-p0-probe"))
        .arg("host")
        .stdout(Stdio::from(OwnedFd::from(output)))
        .stderr(Stdio::from(OwnedFd::from(diagnostic)))
        .status()
        .expect("run with closed output channels");
    assert_eq!(status.code(), Some(1), "an I/O error is not a panic");
}
