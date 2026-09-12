use std::path::Path;
use std::process::Command;

#[test]
fn experiment_path_command_emits_json_without_cow_confirmation() {
    let output = Command::new(env!("CARGO_BIN_EXE_thinws-p0-probe"))
        .args(["path"])
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")))
        .output()
        .expect("run experimental probe binary");

    assert!(
        output.status.success(),
        "probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("probe stdout is JSON");
    assert_eq!(json["experiment"], "p0-01-host-path-probe");
    assert_eq!(json["cow_evidence"], "not_executed_by_probe");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("confirmed"));
}

#[test]
fn experiment_path_command_emits_structured_error_json() {
    let output = Command::new(env!("CARGO_BIN_EXE_thinws-p0-probe"))
        .args(["path", "relative"])
        .output()
        .expect("run experimental probe binary");

    assert_eq!(output.status.code(), Some(2));
    let json: serde_json::Value =
        serde_json::from_slice(&output.stderr).expect("probe stderr is JSON");
    assert_eq!(json["experiment"], "p0-01-host-path-probe");
    assert_eq!(json["outcome"], "error");
    assert_eq!(json["error"]["code"], "probe");
    assert_eq!(json["error"]["error"]["code"], "path_must_be_absolute");
}
