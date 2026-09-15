use std::{fs, process::Command};

#[test]
fn validation_cli_uses_shared_catalogue_rules_without_runtime_assets() {
    if !wind_common::os::eligible() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("installer chosen КИЛ.json");
    fs::write(
        &file,
        include_bytes!("../../../tests/fixtures/localized.json"),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_divvun-wind"))
        .arg("check-file")
        .arg(&file)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["valid"], true);
    assert_eq!(
        value["catalogue"]["entries"]["sjd-cyrl"]["abbreviation"],
        "КИЛ"
    );
    fs::write(
        dir.path().join("conflict.json"),
        r#"{"sjd-cyrl":{"abbreviation":"different"}}"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_divvun-wind"))
        .args(["check", "--directory"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["valid"], false);
    assert!(value["catalogue"]["entries"].get("sjd-cyrl").is_none());
    assert!(value["catalogue"]["entries"].get("gem-runr").is_some());
}

#[test]
fn ineligible_os_launch_is_a_silent_success_before_argument_validation() {
    if wind_common::os::eligible() {
        return;
    }
    let output = Command::new(env!("CARGO_BIN_EXE_divvun-wind"))
        .arg("deliberately-invalid-arguments")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}
