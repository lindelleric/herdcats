use herdcats_bridge::codex_config::{CodexConfigResult, disable_daemon_in_dir};
use std::fs;
use std::path::PathBuf;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

struct TempCodex {
    path: PathBuf,
}

impl TempCodex {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "herdcats-codex-test-{}-{}-{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self { path: dir }
    }
}

impl Drop for TempCodex {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn test_missing_dir() {
    let temp = TempCodex::new("missing-dir");
    let missing_path = temp.path.join("nonexistent_sub");
    let res = disable_daemon_in_dir(&missing_path);
    assert_eq!(res, CodexConfigResult::CodexNotInstalled);
    assert_eq!(res.to_json(), r#"{"status":"codex_not_installed"}"#);
    assert!(!missing_path.exists());
}

#[test]
fn test_missing_file() {
    let temp = TempCodex::new("missing-file");
    let config_path = temp.path.join("config.toml");
    let res = disable_daemon_in_dir(&temp.path);
    assert_eq!(res, CodexConfigResult::Changed { backup: None });
    assert_eq!(res.to_json(), r#"{"status":"changed"}"#);
    assert!(config_path.is_file());
    let content = fs::read_to_string(&config_path).unwrap();
    assert_eq!(content, "[features]\ndaemon_auto_start = false\n");
    #[cfg(unix)]
    {
        let mode = fs::metadata(&config_path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    // No backup created
    let backups: Vec<_> = fs::read_dir(&temp.path)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains(".bak."))
        .collect();
    assert!(backups.is_empty());
}

#[test]
fn test_features_with_other_keys_and_comments_byte_identical_except_one_setting() {
    let temp = TempCodex::new("comments-preservation");
    let config_path = temp.path.join("config.toml");
    let orig = "# Leading comment\n\
[user]\n\
name = \"Alice\"\n\
\n\
# Features comment\n\
[features]\n\
# Comment for daemon setting\n\
daemon_auto_start = true # inline comment\n\
# Another comment\n\
fast_mode = true\n";

    fs::write(&config_path, orig).unwrap();
    let res = disable_daemon_in_dir(&temp.path);
    match &res {
        CodexConfigResult::Changed { backup: Some(b) } => {
            assert!(b.starts_with("config.toml.bak.herdcats-"));
            let backup_file = temp.path.join(b);
            assert!(backup_file.is_file());
            assert_eq!(fs::read_to_string(backup_file).unwrap(), orig);
        }
        _ => panic!("Expected Changed with backup, got: {:?}", res),
    }

    let updated = fs::read_to_string(&config_path).unwrap();
    let expected = "# Leading comment\n\
[user]\n\
name = \"Alice\"\n\
\n\
# Features comment\n\
[features]\n\
# Comment for daemon setting\n\
daemon_auto_start = false # inline comment\n\
# Another comment\n\
fast_mode = true\n";
    assert_eq!(updated, expected);
}

#[test]
fn test_inline_table_with_other_keys() {
    let temp = TempCodex::new("inline-table");
    let config_path = temp.path.join("config.toml");
    let orig = "features = { not_daemon_auto_start = true }\n";
    fs::write(&config_path, orig).unwrap();

    let res = disable_daemon_in_dir(&temp.path);
    assert!(matches!(res, CodexConfigResult::Changed { .. }));

    let updated = fs::read_to_string(&config_path).unwrap();
    assert!(updated.contains("not_daemon_auto_start = true"));
    assert!(updated.contains("daemon_auto_start = false"));

    // Also test inline table that already has daemon_auto_start = true
    let orig2 = "features = { daemon_auto_start = true, fast_mode = true }\n";
    fs::write(&config_path, orig2).unwrap();
    let res2 = disable_daemon_in_dir(&temp.path);
    assert!(matches!(res2, CodexConfigResult::Changed { .. }));
    let updated2 = fs::read_to_string(&config_path).unwrap();
    assert_eq!(
        updated2,
        "features = { daemon_auto_start = false, fast_mode = true }\n"
    );
}

#[test]
fn test_dotted_keys() {
    let temp = TempCodex::new("dotted-keys");
    let config_path = temp.path.join("config.toml");

    // Dotted key directly: features.daemon_auto_start = true
    let orig = "features.daemon_auto_start = true\n";
    fs::write(&config_path, orig).unwrap();
    let res = disable_daemon_in_dir(&temp.path);
    assert!(matches!(res, CodexConfigResult::Changed { .. }));
    assert_eq!(
        fs::read_to_string(&config_path).unwrap(),
        "features.daemon_auto_start = false\n"
    );

    // Dotted key with another setting: features.other = true
    let orig2 = "features.other = true\n";
    fs::write(&config_path, orig2).unwrap();
    let res2 = disable_daemon_in_dir(&temp.path);
    assert!(matches!(res2, CodexConfigResult::Changed { .. }));
    let updated2 = fs::read_to_string(&config_path).unwrap();
    assert!(updated2.contains("features.other = true"));
    assert!(updated2.contains("features.daemon_auto_start = false"));
}

#[test]
fn test_quoted_keys() {
    let temp = TempCodex::new("quoted-keys");
    let config_path = temp.path.join("config.toml");

    let orig = "\"features\".\"daemon_auto_start\" = true\n";
    fs::write(&config_path, orig).unwrap();
    let res = disable_daemon_in_dir(&temp.path);
    assert!(matches!(res, CodexConfigResult::Changed { .. }));
    let updated = fs::read_to_string(&config_path).unwrap();
    assert_eq!(updated, "\"features\".\"daemon_auto_start\" = false\n");
}

#[test]
fn test_multiline_string_left_alone() {
    let temp = TempCodex::new("multiline-string");
    let config_path = temp.path.join("config.toml");

    let orig = "prompt = \"\"\"\n\
[features]\n\
daemon_auto_start = true\n\
\"\"\"\n";
    fs::write(&config_path, orig).unwrap();
    let res = disable_daemon_in_dir(&temp.path);
    assert!(matches!(res, CodexConfigResult::Changed { .. }));

    let updated = fs::read_to_string(&config_path).unwrap();
    // The multiline string prompt must be untouched
    assert!(updated.starts_with(orig));
    // A separate features table must have been appended
    assert!(updated.contains("\n[features]\ndaemon_auto_start = false\n"));
}

#[test]
fn test_true_to_false_and_already_false() {
    let temp = TempCodex::new("true-to-false");
    let config_path = temp.path.join("config.toml");

    fs::write(&config_path, "[features]\ndaemon_auto_start = true\n").unwrap();
    let res = disable_daemon_in_dir(&temp.path);
    match res {
        CodexConfigResult::Changed {
            backup: Some(ref b),
        } => {
            assert!(b.starts_with("config.toml.bak.herdcats-"));
        }
        _ => panic!("Expected changed with backup, got {:?}", res),
    }

    // Now it is false. Calling it again must return already_disabled with no backup
    let entries_before: Vec<_> = fs::read_dir(&temp.path)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    let res2 = disable_daemon_in_dir(&temp.path);
    assert_eq!(res2, CodexConfigResult::AlreadyDisabled);
    assert_eq!(res2.to_json(), r#"{"status":"already_disabled"}"#);
    let entries_after: Vec<_> = fs::read_dir(&temp.path)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(entries_before, entries_after);
}

#[test]
fn test_invalid_toml() {
    let temp = TempCodex::new("invalid-toml");
    let config_path = temp.path.join("config.toml");

    let bad = "this is not valid toml = = = 123";
    fs::write(&config_path, bad).unwrap();
    let res = disable_daemon_in_dir(&temp.path);
    assert_eq!(res, CodexConfigResult::Error("config_invalid"));
    assert_eq!(res.to_json(), r#"{"error":"config_invalid"}"#);
    assert_eq!(fs::read_to_string(&config_path).unwrap(), bad);
}

#[test]
fn test_features_not_a_table() {
    let temp = TempCodex::new("features-not-table");
    let config_path = temp.path.join("config.toml");

    let not_table = "features = \"string_not_table\"\n";
    fs::write(&config_path, not_table).unwrap();
    let res = disable_daemon_in_dir(&temp.path);
    assert_eq!(res, CodexConfigResult::Error("features_not_table"));
    assert_eq!(res.to_json(), r#"{"error":"features_not_table"}"#);
    assert_eq!(fs::read_to_string(&config_path).unwrap(), not_table);
}

#[test]
fn test_symlink_rejected() {
    let temp = TempCodex::new("symlink");
    let target = temp.path.join("target.toml");
    fs::write(&target, "[features]\ndaemon_auto_start = true\n").unwrap();
    let config_path = temp.path.join("config.toml");

    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &config_path).unwrap();

    let res = disable_daemon_in_dir(&temp.path);
    assert_eq!(res, CodexConfigResult::Error("config_symlink"));
    assert_eq!(res.to_json(), r#"{"error":"config_symlink"}"#);
    assert!(
        fs::symlink_metadata(&config_path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        fs::read_to_string(&target).unwrap(),
        "[features]\ndaemon_auto_start = true\n"
    );
}

#[test]
fn test_backup_created_and_mode_preserved() {
    let temp = TempCodex::new("backup-mode");
    let config_path = temp.path.join("config.toml");
    fs::write(&config_path, "[features]\ndaemon_auto_start = true\n").unwrap();

    #[cfg(unix)]
    {
        fs::set_permissions(&config_path, fs::Permissions::from_mode(0o640)).unwrap();
    }

    let res = disable_daemon_in_dir(&temp.path);
    match res {
        CodexConfigResult::Changed {
            backup: Some(ref b),
        } => {
            let backup_path = temp.path.join(b);
            assert!(backup_path.is_file());
            assert_eq!(
                fs::read_to_string(&backup_path).unwrap(),
                "[features]\ndaemon_auto_start = true\n"
            );
            #[cfg(unix)]
            {
                let backup_mode = fs::metadata(&backup_path).unwrap().permissions().mode();
                let config_mode = fs::metadata(&config_path).unwrap().permissions().mode();
                assert_eq!(backup_mode & 0o777, 0o640);
                assert_eq!(config_mode & 0o777, 0o640);
            }
        }
        _ => panic!("Expected changed with backup, got {:?}", res),
    }
}

#[test]
fn test_cli_subcommand_exits_0_and_prints_json() {
    let temp = TempCodex::new("cli-test");
    let config_path = temp.path.join("config.toml");
    fs::write(&config_path, "[features]\ndaemon_auto_start = true\n").unwrap();

    let binary = env!("CARGO_BIN_EXE_herdcats-bridge");
    let output = std::process::Command::new(binary)
        .args(["codex-config", "disable-daemon"])
        .env("CODEX_HOME", &temp.path)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1);
    let val: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(val["status"], "changed");
    let backup = val["backup"].as_str().unwrap();
    assert!(backup.starts_with("config.toml.bak.herdcats-"));

    // Second run: already disabled, exits 0
    let output2 = std::process::Command::new(binary)
        .args(["codex-config", "disable-daemon"])
        .env("CODEX_HOME", &temp.path)
        .output()
        .unwrap();
    assert_eq!(output2.status.code(), Some(0));
    let stdout2 = String::from_utf8(output2.stdout).unwrap();
    assert_eq!(stdout2.trim(), r#"{"status":"already_disabled"}"#);

    // Missing dir: codex_not_installed, exits 0
    let output3 = std::process::Command::new(binary)
        .args(["codex-config", "disable-daemon"])
        .env("CODEX_HOME", temp.path.join("does_not_exist"))
        .output()
        .unwrap();
    assert_eq!(output3.status.code(), Some(0));
    let stdout3 = String::from_utf8(output3.stdout).unwrap();
    assert_eq!(stdout3.trim(), r#"{"status":"codex_not_installed"}"#);

    // Invalid toml: error config_invalid, exits 0
    fs::write(&config_path, "invalid toml ====").unwrap();
    let output4 = std::process::Command::new(binary)
        .args(["codex-config", "disable-daemon"])
        .env("CODEX_HOME", &temp.path)
        .output()
        .unwrap();
    assert_eq!(output4.status.code(), Some(0));
    let stdout4 = String::from_utf8(output4.stdout).unwrap();
    assert_eq!(stdout4.trim(), r#"{"error":"config_invalid"}"#);
}

#[test]
fn test_codex_home_resolution_and_defaults() {
    use herdcats_bridge::codex_config::resolve_codex_dir_from;
    use herdcats_bridge::service::Service;

    // Absolute CODEX_HOME takes precedence
    assert_eq!(
        resolve_codex_dir_from(Some("/custom/codex"), Some("/home/user")),
        Some(PathBuf::from("/custom/codex"))
    );
    assert_eq!(
        Service::codex_root_default_from(Some("/custom/codex"), Some("/home/user")),
        Some(PathBuf::from("/custom/codex/sessions"))
    );

    // Empty CODEX_HOME falls back to HOME/.codex/sessions
    assert_eq!(
        resolve_codex_dir_from(Some(""), Some("/home/user")),
        Some(PathBuf::from("/home/user/.codex"))
    );
    assert_eq!(
        Service::codex_root_default_from(Some(""), Some("/home/user")),
        Some(PathBuf::from("/home/user/.codex/sessions"))
    );

    // Relative CODEX_HOME is ignored and falls back to HOME/.codex/sessions
    assert_eq!(
        resolve_codex_dir_from(Some("relative/codex"), Some("/home/user")),
        Some(PathBuf::from("/home/user/.codex"))
    );
    assert_eq!(
        Service::codex_root_default_from(Some("relative/codex"), Some("/home/user")),
        Some(PathBuf::from("/home/user/.codex/sessions"))
    );

    // Relative HOME is ignored and yields None
    assert_eq!(
        resolve_codex_dir_from(Some("relative/codex"), Some("relative/home")),
        None
    );
    assert_eq!(
        Service::codex_root_default_from(Some("relative/codex"), Some("relative/home")),
        None
    );
    assert_eq!(
        resolve_codex_dir_from(None::<&str>, Some("relative/home")),
        None
    );
    assert_eq!(
        Service::codex_root_default_from(None::<&str>, Some("relative/home")),
        None
    );

    // Empty HOME is ignored and yields None
    assert_eq!(resolve_codex_dir_from(None::<&str>, Some("")), None);
    assert_eq!(
        Service::codex_root_default_from(None::<&str>, Some("")),
        None
    );
}

#[test]
fn test_cli_empty_and_relative_codex_home_falls_back_to_home() {
    let temp = TempCodex::new("cli-fallback");
    let codex_home = temp.path.join(".codex");
    fs::create_dir_all(&codex_home).unwrap();
    let config_path = codex_home.join("config.toml");
    fs::write(&config_path, "[features]\ndaemon_auto_start = true\n").unwrap();

    let binary = env!("CARGO_BIN_EXE_herdcats-bridge");

    // Empty CODEX_HOME falls back to HOME/.codex
    let output_empty = std::process::Command::new(binary)
        .args(["codex-config", "disable-daemon"])
        .env("CODEX_HOME", "")
        .env("HOME", &temp.path)
        .output()
        .unwrap();
    assert_eq!(output_empty.status.code(), Some(0));
    let stdout = String::from_utf8(output_empty.stdout).unwrap();
    let val: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(val["status"], "changed");

    // Relative CODEX_HOME is ignored and falls back to HOME/.codex (now already_disabled)
    let output_rel = std::process::Command::new(binary)
        .args(["codex-config", "disable-daemon"])
        .env("CODEX_HOME", "relative/codex")
        .env("HOME", &temp.path)
        .output()
        .unwrap();
    assert_eq!(output_rel.status.code(), Some(0));
    let stdout = String::from_utf8(output_rel.stdout).unwrap();
    let val: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(val["status"], "already_disabled");

    // Relative HOME with relative CODEX_HOME means codex_not_installed
    let output_both_rel = std::process::Command::new(binary)
        .args(["codex-config", "disable-daemon"])
        .env("CODEX_HOME", "relative/codex")
        .env("HOME", "relative/home")
        .output()
        .unwrap();
    assert_eq!(output_both_rel.status.code(), Some(0));
    let stdout = String::from_utf8(output_both_rel.stdout).unwrap();
    let val: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(val["status"], "codex_not_installed");
}
