use serde_json::Value;
use std::process::Command;

#[test]
fn version_probe_needs_no_socket_or_home() {
    let output = Command::new(env!("CARGO_BIN_EXE_herdcats-bridge"))
        .arg("--version-json")
        .env_remove("HOME")
        .env_remove("HERDR_SOCKET_PATH")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["name"], "herdcats-bridge");
    assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(value["protocol"], 1);
    assert_eq!(value["herdr_protocol"], 22);
    assert_eq!(value["agents"], serde_json::json!(["claude", "codex"]));

    let output_v = Command::new(env!("CARGO_BIN_EXE_herdcats-bridge"))
        .arg("--version")
        .env_remove("HOME")
        .env_remove("HERDR_SOCKET_PATH")
        .output()
        .unwrap();
    assert!(output_v.status.success());
    let value_v: Value = serde_json::from_slice(&output_v.stdout).unwrap();
    assert_eq!(value_v["agents"], serde_json::json!(["claude", "codex"]));
}

#[test]
fn version_flag_rejects_extra_arguments() {
    let output = Command::new(env!("CARGO_BIN_EXE_herdcats-bridge"))
        .args(["--version-json", "serve"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}

#[test]
fn version_json_bytes_match_released_installer_contract() {
    let expected = format!(
        "{{\"agents\":[\"claude\",\"codex\"],\"herdr_protocol\":22,\"name\":\"herdcats-bridge\",\"protocol\":1,\"version\":\"{}\"}}\n",
        env!("CARGO_PKG_VERSION")
    );
    for flag in ["--version-json", "--version"] {
        let output = Command::new(env!("CARGO_BIN_EXE_herdcats-bridge"))
            .arg(flag)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
    }
}
