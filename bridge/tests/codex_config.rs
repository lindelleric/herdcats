use herdcats_bridge::codex_config::{
    CodexConfigResult, DaemonStatusResult, DaemonStopStatus, ProcessStatus, check_process,
    check_process_snapshot, current_boot_id, daemon_status_in_dir, disable_daemon_in_dir,
    disable_daemon_in_dir_with_options, process_start_time,
    stop_daemon_and_updater_with_timeouts, term_updater,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::process::Stdio;

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
    assert_eq!(
        res,
        CodexConfigResult::Changed {
            backup: None,
            daemon: None,
            updater: None
        }
    );
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
        CodexConfigResult::Changed {
            backup: Some(b), ..
        } => {
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
            ..
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
    assert_eq!(
        res2,
        CodexConfigResult::AlreadyDisabled {
            daemon: None,
            updater: None
        }
    );
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
fn test_daemon_setting_with_unsupported_shape_is_left_alone() {
    for config in [
        "[features.daemon_auto_start]\ncustom = true\n",
        "[features]\ndaemon_auto_start = [true]\n",
        "[features]\ndaemon_auto_start = { custom = true }\n",
    ] {
        let temp = TempCodex::new("daemon-unsupported-shape");
        let config_path = temp.path.join("config.toml");
        fs::write(&config_path, config).unwrap();
        let res = disable_daemon_in_dir(&temp.path);
        assert_eq!(
            res,
            CodexConfigResult::Error("daemon_setting_unsupported"),
            "{config}"
        );
        assert_eq!(fs::read_to_string(&config_path).unwrap(), config);
    }
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
            ..
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

struct ChildGuard(std::process::Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        kill_tree(&mut self.0);
    }
}

// A detached dummy (spawned via a shell double-fork) plus the outer shell
// child, so the whole tree can be killed and the outer child reaped even if
// an assertion fails.
struct OrphanGuard {
    outer: std::process::Child,
    pid: libc::pid_t,
    pgid: libc::pid_t,
}

impl Drop for OrphanGuard {
    fn drop(&mut self) {
        unsafe {
            libc::kill(-self.pgid, libc::SIGKILL);
            libc::kill(self.pid, libc::SIGKILL);
        }
        let _ = self.outer.wait();
    }
}

// Spawns a dummy script detached from the test runner's stdio (so it can
// never hold cargo's output pipe open) and in its own process group (so it
// and anything it forks can be killed wholesale).
fn spawn_script(exe: &Path, script: &Path) -> std::process::Child {
    std::process::Command::new(exe)
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .unwrap()
}

// SIGKILL the child's whole process group (dummies are spawned with
// process_group(0), covering e.g. forked `sleep`s), then the child itself as
// a fallback, then reap it.
fn kill_tree(child: &mut std::process::Child) {
    let pgid = child.id() as libc::pid_t;
    unsafe {
        libc::kill(-pgid, libc::SIGKILL);
        libc::kill(pgid, libc::SIGKILL);
    }
    let _ = child.wait();
}

fn write_pid_file(
    path: &Path,
    pid: u32,
    boot_id: Option<&str>,
    start_sec: i64,
    start_usec: Option<i64>,
) {
    let mut identity = serde_json::Map::new();
    if let Some(b) = boot_id {
        identity.insert("bootId".into(), serde_json::Value::String(b.to_string()));
    }
    identity.insert("startSeconds".into(), serde_json::json!(start_sec));
    if let Some(u) = start_usec {
        identity.insert("startMicroseconds".into(), serde_json::json!(u));
    }
    let data = serde_json::json!({
        "pid": pid,
        "processStartTime": "test",
        "processIdentity": identity
    });
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_string(&data).unwrap()).unwrap();
}

// Copies nothing: dummies run under /bin/sh and are identified via argv
// (KERN_PROCARGS2), which can be briefly unavailable right after spawn, so
// tests call wait_for_check_status to let the classification settle instead
// of racing process startup.
fn wait_for_check_status(
    pid_file: &Path,
    codex_dir: &Path,
    expected: ProcessStatus,
    what: &str,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if check_process(pid_file, codex_dir) == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "dummy never became {what} ({expected:?})"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

// Spawns a dummy whose TERM trap removes its pid file, so its death is
// observed through the missing pid file instead of introspecting a possibly
// unreaped zombie (whose cached argv can make a dead process look alive).
// The dummy writes a ready marker AFTER installing its trap, and this helper
// waits for it: a SIGTERM that arrives between exec and trap installation
// would kill the dummy via the default action without running the trap.
fn spawn_dummy_in_packages(
    codex_dir: &Path,
    name: &str,
    pid_file: &Path,
) -> (ChildGuard, i64, Option<i64>) {
    let packages_dir = codex_dir
        .join("packages")
        .join("app-server-daemon")
        .join("bin");
    fs::create_dir_all(&packages_dir).unwrap();
    let script_path = packages_dir.join(name);
    let ready_marker = packages_dir.join(format!("{name}.ready"));
    fs::write(
        &script_path,
        format!(
            "#!/bin/sh\ntrap \"rm -f '{}'; exit 0\" TERM\necho $$ > '{}'\nwhile true; do sleep 0.05; done\n",
            pid_file.display(),
            ready_marker.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let child = spawn_script(Path::new("/bin/sh"), &script_path);
    let pid = child.id() as libc::pid_t;
    wait_for_ready_marker(&ready_marker);
    let (start_sec, start_usec) = process_start_time(pid).expect("read start time");
    (ChildGuard(child), start_sec, start_usec)
}

// Blocks until a dummy has installed its TERM trap and published its ready
// marker (the marker is written after the trap statement).
fn wait_for_ready_marker(ready_marker: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready_marker.is_file() {
        assert!(
            Instant::now() < deadline,
            "dummy did not become ready (trap not installed)"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn spawn_dummy_outside_packages(codex_dir: &Path, name: &str) -> (ChildGuard, i64, Option<i64>) {
    let outside_dir = codex_dir.join("outside");
    fs::create_dir_all(&outside_dir).unwrap();
    let script_path = outside_dir.join(name);
    fs::write(
        &script_path,
        "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do sleep 0.05; done\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let child = spawn_script(Path::new("/bin/sh"), &script_path);
    let pid = child.id() as libc::pid_t;
    std::thread::sleep(Duration::from_millis(10));
    let (start_sec, start_usec) = process_start_time(pid).expect("read start time");
    (ChildGuard(child), start_sec, start_usec)
}

// Spawns a dummy under the packages dir that is NOT a direct child of this test
// process (shell double-fork), so the system reaps it once it exits and
// `kill(pid, 0)` becomes a reliable "process is gone" signal. The outer shell
// is spawned detached from the test runner's stdio and in its own process
// group (inherited by the dummy), so the guard can kill the whole tree.
fn spawn_orphan_in_packages(
    codex_dir: &Path,
    name: &str,
    pid_record: &Path,
    term_handler: &str,
) -> OrphanGuard {
    let packages_dir = codex_dir
        .join("packages")
        .join("app-server-daemon")
        .join("bin");
    fs::create_dir_all(&packages_dir).unwrap();
    let script_path = packages_dir.join(name);
    let script = format!(
        "#!/bin/sh\ntrap '{term_handler}' TERM\necho $$ > '{record}'\nwhile true; do sleep 0.05; done\n",
        term_handler = term_handler,
        record = pid_record.display()
    );
    fs::write(&script_path, script).unwrap();
    #[cfg(unix)]
    {
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut outer = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(format!("'{}' &", script_path.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .unwrap();
    let pgid = outer.id() as libc::pid_t;
    let _ = outer.wait();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !pid_record.is_file() {
        assert!(
            Instant::now() < deadline,
            "orphan dummy did not record its pid"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
    let content = fs::read_to_string(pid_record).unwrap();
    let pid: libc::pid_t = content.trim().parse().unwrap();
    OrphanGuard { outer, pid, pgid }
}

#[test]
fn test_daemon_status_missing_files() {
    let temp = TempCodex::new("daemon-status-missing");
    let status = daemon_status_in_dir(&temp.path);
    assert_eq!(
        status,
        DaemonStatusResult {
            daemon: ProcessStatus::NotRunning,
            updater: ProcessStatus::NotRunning,
            auto_start: None,
        }
    );
    assert_eq!(
        status.to_json(),
        r#"{"daemon":"not_running","updater":"not_running","auto_start":null}"#
    );
}

#[test]
fn test_daemon_status_running() {
    let temp = TempCodex::new("daemon-status-running");
    let config_path = temp.path.join("config.toml");
    fs::write(&config_path, "[features]\ndaemon_auto_start = true\n").unwrap();

    let boot_id = current_boot_id();
    let daemon_pid_file = temp.path.join("app-server-daemon").join("daemon.pid");
    let updater_pid_file = temp.path.join("app-server-daemon").join("daemon-updater.pid");
    let (daemon_guard, d_sec, d_usec) = spawn_dummy_in_packages(&temp.path, "daemon.sh", &daemon_pid_file);
    let (updater_guard, u_sec, u_usec) =
        spawn_dummy_in_packages(&temp.path, "updater.sh", &updater_pid_file);

    write_pid_file(
        &daemon_pid_file,
        daemon_guard.0.id(),
        boot_id.as_deref(),
        d_sec,
        d_usec,
    );
    write_pid_file(
        &updater_pid_file,
        updater_guard.0.id(),
        boot_id.as_deref(),
        u_sec,
        u_usec,
    );
    wait_for_check_status(&daemon_pid_file, &temp.path, ProcessStatus::Running, "running");
    wait_for_check_status(&updater_pid_file, &temp.path, ProcessStatus::Running, "running");

    let status = daemon_status_in_dir(&temp.path);
    assert_eq!(
        status,
        DaemonStatusResult {
            daemon: ProcessStatus::Running,
            updater: ProcessStatus::Running,
            auto_start: Some(true),
        }
    );
    assert_eq!(
        status.to_json(),
        r#"{"daemon":"running","updater":"running","auto_start":true}"#
    );
}

#[test]
fn test_daemon_status_stale_pid_dead() {
    let temp = TempCodex::new("daemon-status-dead");
    let pid_file = temp.path.join("app-server-daemon").join("daemon.pid");
    write_pid_file(
        &pid_file,
        9999999,
        current_boot_id().as_deref(),
        1000000,
        Some(0),
    );

    let status = check_process(&pid_file, &temp.path);
    assert_eq!(status, ProcessStatus::NotRunning);
}

#[test]
fn test_daemon_status_pid_reused_start_time_mismatch() {
    let temp = TempCodex::new("daemon-status-reuse");
    let pid_file = temp.path.join("app-server-daemon").join("daemon.pid");
    let (guard, real_sec, _) = spawn_dummy_in_packages(&temp.path, "dummy.sh", &pid_file);

    write_pid_file(
        &pid_file,
        guard.0.id(),
        current_boot_id().as_deref(),
        real_sec + 1000,
        Some(0),
    );

    // Alive, owned by us, executable under the packages dir, but the pid-file
    // start time does not match: identity is inconclusive => Unknown. It must
    // not be reported as not_running (Linux pid-file startSeconds semantics
    // are unverified), otherwise the app would claim no daemon is running.
    let status = check_process(&pid_file, &temp.path);
    assert_eq!(status, ProcessStatus::Unknown);
    let full = daemon_status_in_dir(&temp.path);
    assert_eq!(full.daemon, ProcessStatus::Unknown);
    assert_eq!(
        full.to_json(),
        r#"{"daemon":"unknown","updater":"not_running","auto_start":null}"#
    );
}

#[test]
fn test_daemon_status_wrong_owner_or_path() {
    let temp = TempCodex::new("daemon-status-wrong-path");
    let (guard, real_sec, real_usec) = spawn_dummy_outside_packages(&temp.path, "outside.sh");
    let pid_file = temp.path.join("app-server-daemon").join("daemon.pid");

    write_pid_file(
        &pid_file,
        guard.0.id(),
        current_boot_id().as_deref(),
        real_sec,
        real_usec,
    );

    let status = check_process(&pid_file, &temp.path);
    assert_eq!(status, ProcessStatus::NotRunning);
}

#[test]
fn test_daemon_status_malformed_json() {
    let temp = TempCodex::new("daemon-status-malformed");
    let pid_file = temp.path.join("app-server-daemon").join("daemon.pid");
    fs::create_dir_all(pid_file.parent().unwrap()).unwrap();

    fs::write(&pid_file, "not json content").unwrap();
    assert_eq!(check_process(&pid_file, &temp.path), ProcessStatus::Unknown);

    fs::write(&pid_file, r#"{"processStartTime":"now"}"#).unwrap();
    assert_eq!(check_process(&pid_file, &temp.path), ProcessStatus::Unknown);

    fs::write(&pid_file, r#"{"pid":"123"}"#).unwrap();
    assert_eq!(check_process(&pid_file, &temp.path), ProcessStatus::Unknown);

    fs::write(&pid_file, r#"{"pid":123}"#).unwrap();
    assert_eq!(check_process(&pid_file, &temp.path), ProcessStatus::Unknown);
}

#[test]
fn test_stop_running_successful_stop() {
    let temp = TempCodex::new("stop-running-success");
    let config_path = temp.path.join("config.toml");
    fs::write(&config_path, "[features]\ndaemon_auto_start = true\n").unwrap();

    let stop_bin_dir = temp
        .path
        .join("packages")
        .join("app-server-daemon")
        .join("current")
        .join("bin");
    fs::create_dir_all(&stop_bin_dir).unwrap();
    let stop_bin = stop_bin_dir.join("codex");
    let stop_script = r#"#!/bin/sh
if [ "$1" = "app-server" ] && [ "$2" = "daemon" ] && [ "$3" = "stop" ]; then
    echo "called" > "$CODEX_HOME/codex_stop_called"
    if [ -f "$CODEX_HOME/app-server-daemon/daemon.pid" ]; then
        rm -f "$CODEX_HOME/app-server-daemon/daemon.pid"
    fi
    exit 0
fi
exit 1
"#;
    fs::write(&stop_bin, stop_script).unwrap();
    #[cfg(unix)]
    {
        fs::set_permissions(&stop_bin, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let updater_pid_file = temp.path.join("app-server-daemon").join("daemon-updater.pid");
    let daemon_pid_file = temp.path.join("app-server-daemon").join("daemon.pid");
    let (mut updater_guard, u_sec, u_usec) =
        spawn_dummy_in_packages(&temp.path, "updater.sh", &updater_pid_file);
    let boot_id = current_boot_id();
    write_pid_file(
        &updater_pid_file,
        updater_guard.0.id(),
        boot_id.as_deref(),
        u_sec,
        u_usec,
    );

    let (daemon_guard, d_sec, d_usec) =
        spawn_dummy_in_packages(&temp.path, "daemon.sh", &daemon_pid_file);
    write_pid_file(
        &daemon_pid_file,
        daemon_guard.0.id(),
        boot_id.as_deref(),
        d_sec,
        d_usec,
    );
    wait_for_check_status(&updater_pid_file, &temp.path, ProcessStatus::Running, "running");
    wait_for_check_status(&daemon_pid_file, &temp.path, ProcessStatus::Running, "running");

    let res = disable_daemon_in_dir_with_options(&temp.path, true);
    match &res {
        CodexConfigResult::Changed {
            backup: Some(_),
            daemon: Some(DaemonStopStatus::Stopped),
            updater: Some(DaemonStopStatus::Stopped),
        } => {}
        _ => panic!(
            "Expected Changed with Stopped daemon and updater, got: {:?}",
            res
        ),
    }

    assert!(temp.path.join("codex_stop_called").is_file());

    let mut dead = false;
    for _ in 0..20 {
        if let Ok(Some(_)) = updater_guard.0.try_wait() {
            dead = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(dead, "Updater process should have been terminated by SIGTERM");
}

#[test]
fn test_stop_running_mismatched_identity_left_alone() {
    let temp = TempCodex::new("stop-running-mismatched");
    let config_path = temp.path.join("config.toml");
    fs::write(&config_path, "[features]\ndaemon_auto_start = true\n").unwrap();

    let updater_pid_file = temp.path.join("app-server-daemon").join("daemon-updater.pid");
    let (updater_guard, u_sec, _) =
        spawn_dummy_in_packages(&temp.path, "updater.sh", &updater_pid_file);
    let updater_pid = updater_guard.0.id() as libc::pid_t;

    write_pid_file(
        &updater_pid_file,
        updater_guard.0.id(),
        current_boot_id().as_deref(),
        u_sec + 5000,
        Some(0),
    );

    // Path is under the packages dir but the start time does not match:
    // Unknown, and it must never be signalled.
    wait_for_check_status(
        &updater_pid_file,
        &temp.path,
        ProcessStatus::Unknown,
        "identity-unknown",
    );
    assert_eq!(
        check_process(&updater_pid_file, &temp.path),
        ProcessStatus::Unknown
    );

    let res = disable_daemon_in_dir_with_options(&temp.path, true);
    match &res {
        CodexConfigResult::Changed {
            backup: Some(_),
            daemon: Some(DaemonStopStatus::NotRunning),
            updater: Some(DaemonStopStatus::Unknown),
        } => {}
        _ => panic!("Expected daemon NotRunning and updater Unknown, got: {:?}", res),
    }

    assert_eq!(unsafe { libc::kill(updater_pid, 0) }, 0);
}

#[test]
fn test_stop_running_config_error_no_stop_attempted() {
    let temp = TempCodex::new("stop-running-config-error");
    let config_path = temp.path.join("config.toml");
    let target_path = temp.path.join("target.toml");
    fs::write(&target_path, "[features]\ndaemon_auto_start = true\n").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target_path, &config_path).unwrap();

    let stop_bin_dir = temp
        .path
        .join("packages")
        .join("app-server-daemon")
        .join("current")
        .join("bin");
    fs::create_dir_all(&stop_bin_dir).unwrap();
    let stop_bin = stop_bin_dir.join("codex");
    fs::write(
        &stop_bin,
        "#!/bin/sh\necho called > \"$CODEX_HOME/codex_stop_called\"\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        fs::set_permissions(&stop_bin, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let updater_pid_file = temp.path.join("app-server-daemon").join("daemon-updater.pid");
    let (updater_guard, u_sec, u_usec) =
        spawn_dummy_in_packages(&temp.path, "updater.sh", &updater_pid_file);
    let updater_pid = updater_guard.0.id() as libc::pid_t;
    write_pid_file(
        &updater_pid_file,
        updater_guard.0.id(),
        current_boot_id().as_deref(),
        u_sec,
        u_usec,
    );
    wait_for_check_status(
        &updater_pid_file,
        &temp.path,
        ProcessStatus::Running,
        "running",
    );

    let res = disable_daemon_in_dir_with_options(&temp.path, true);
    assert_eq!(res, CodexConfigResult::Error("config_symlink"));

    assert!(!temp.path.join("codex_stop_called").exists());
    assert_eq!(unsafe { libc::kill(updater_pid, 0) }, 0);
}

#[test]
fn test_cli_daemon_status_and_stop_running() {
    let temp = TempCodex::new("cli-daemon-status");
    let codex_home = temp.path.join(".codex");
    fs::create_dir_all(&codex_home).unwrap();
    let config_path = codex_home.join("config.toml");
    fs::write(&config_path, "[features]\ndaemon_auto_start = true\n").unwrap();

    let binary = env!("CARGO_BIN_EXE_herdcats-bridge");

    let output_status = std::process::Command::new(binary)
        .args(["codex-config", "daemon-status"])
        .env("CODEX_HOME", &codex_home)
        .output()
        .unwrap();
    assert_eq!(output_status.status.code(), Some(0));
    let stdout = String::from_utf8(output_status.stdout).unwrap();
    let val: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(val["daemon"], "not_running");
    assert_eq!(val["updater"], "not_running");
    assert_eq!(val["auto_start"], true);

    let output_stop = std::process::Command::new(binary)
        .args(["codex-config", "disable-daemon", "--stop-running"])
        .env("CODEX_HOME", &codex_home)
        .output()
        .unwrap();
    assert_eq!(output_stop.status.code(), Some(0));
    let stdout = String::from_utf8(output_stop.stdout).unwrap();
    let val: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(val["status"], "changed");
    assert_eq!(val["daemon"], "not_running");
    assert_eq!(val["updater"], "not_running");

    let output_status2 = std::process::Command::new(binary)
        .args(["codex-config", "daemon-status"])
        .env("CODEX_HOME", &codex_home)
        .output()
        .unwrap();
    assert_eq!(output_status2.status.code(), Some(0));
    let stdout2 = String::from_utf8(output_status2.stdout).unwrap();
    let val2: serde_json::Value = serde_json::from_str(stdout2.trim()).unwrap();
    assert_eq!(val2["auto_start"], false);
}

#[test]
fn test_stop_running_mismatched_daemon_reports_unknown_and_no_stop() {
    let temp = TempCodex::new("cli-stop-mismatched-daemon");
    let codex_home = temp.path.join(".codex");
    fs::create_dir_all(&codex_home).unwrap();
    let config_path = codex_home.join("config.toml");
    fs::write(&config_path, "[features]\ndaemon_auto_start = true\n").unwrap();

    // Fake `codex app-server daemon stop` that records being invoked. It must
    // NOT run: the daemon pid file's start time does not match the live
    // process, so the daemon's identity is Unknown and it is never stopped.
    let stop_bin_dir = codex_home
        .join("packages")
        .join("app-server-daemon")
        .join("current")
        .join("bin");
    fs::create_dir_all(&stop_bin_dir).unwrap();
    let stop_bin = stop_bin_dir.join("codex");
    fs::write(
        &stop_bin,
        "#!/bin/sh\necho called > \"$CODEX_HOME/codex_stop_called\"\nexit 0\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        fs::set_permissions(&stop_bin, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let daemon_pid_file = codex_home.join("app-server-daemon").join("daemon.pid");
    let (daemon_guard, d_sec, _) =
        spawn_dummy_in_packages(&codex_home, "daemon.sh", &daemon_pid_file);
    let daemon_pid = daemon_guard.0.id() as libc::pid_t;
    write_pid_file(
        &daemon_pid_file,
        daemon_guard.0.id(),
        current_boot_id().as_deref(),
        d_sec + 1000,
        Some(0),
    );
    wait_for_check_status(
        &daemon_pid_file,
        &codex_home,
        ProcessStatus::Unknown,
        "identity-unknown",
    );
    assert_eq!(
        check_process(&daemon_pid_file, &codex_home),
        ProcessStatus::Unknown
    );

    let binary = env!("CARGO_BIN_EXE_herdcats-bridge");
    let output = std::process::Command::new(binary)
        .args(["codex-config", "disable-daemon", "--stop-running"])
        .env("CODEX_HOME", &codex_home)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let val: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(val["status"], "changed");
    assert_eq!(val["daemon"], "unknown");
    assert_eq!(val["updater"], "not_running");

    // The fake `daemon stop` was never invoked and the process was not signalled.
    assert!(!codex_home.join("codex_stop_called").exists());
    assert_eq!(unsafe { libc::kill(daemon_pid, 0) }, 0);
}

#[test]
fn test_stop_running_live_updater_ignoring_term_blocks_daemon_stop() {
    let temp = TempCodex::new("stop-updater-ignores-term");
    let stop_bin = temp
        .path
        .join("packages/app-server-daemon/current/bin/codex");
    fs::create_dir_all(stop_bin.parent().unwrap()).unwrap();
    fs::write(
        &stop_bin,
        r#"#!/bin/sh
[ "$1 $2 $3" = "app-server daemon stop" ] || exit 2
echo called >> "$CODEX_HOME/stop-calls"
rm -f "$CODEX_HOME/app-server-daemon/daemon.pid"
"#,
    )
    .unwrap();
    fs::set_permissions(&stop_bin, fs::Permissions::from_mode(0o755)).unwrap();

    let daemon = spawn_orphan_in_packages(
        &temp.path,
        "daemon.sh",
        &temp.path.join("daemon-record"),
        "exit 0",
    );
    let daemon_file = temp.path.join("app-server-daemon/daemon.pid");
    let (sec, usec) = process_start_time(daemon.pid).unwrap();
    write_pid_file(
        &daemon_file,
        daemon.pid as u32,
        current_boot_id().as_deref(),
        sec,
        usec,
    );

    let updater_file = temp.path.join("app-server-daemon/daemon-updater.pid");
    let term_marker = temp.path.join("updater-got-term");
    // The updater handles TERM but stays alive with its pid file untouched,
    // which must block `daemon stop`. A transiently unreadable pid file is
    // not simulated here: any such window is at the scheduler's mercy on a
    // loaded runner, so the rule that a lone non-Running observation is not
    // believed is pinned by the StatusPoll unit tests in src/codex_config.rs.
    let handler = format!(
        r#"trap "" TERM; touch "{}""#,
        term_marker.display()
    );
    let updater = spawn_orphan_in_packages(
        &temp.path,
        "updater.sh",
        &temp.path.join("updater-record"),
        &handler,
    );
    let (sec, usec) = process_start_time(updater.pid).unwrap();
    write_pid_file(
        &updater_file,
        updater.pid as u32,
        current_boot_id().as_deref(),
        sec,
        usec,
    );
    wait_for_check_status(&daemon_file, &temp.path, ProcessStatus::Running, "running");
    wait_for_check_status(&updater_file, &temp.path, ProcessStatus::Running, "running");

    let (daemon_status, updater_status) = stop_daemon_and_updater_with_timeouts(
        &temp.path,
        Duration::from_secs(1),
        Duration::from_millis(400),
    );
    assert!(term_marker.exists(), "updater must have handled TERM");
    assert!(
        !temp.path.join("stop-calls").exists(),
        "daemon stop ran while updater was alive"
    );
    assert_eq!(updater_status, DaemonStopStatus::StopFailed);
    assert_eq!(daemon_status, DaemonStopStatus::StopFailed);
    assert_eq!(
        check_process(&updater_file, &temp.path),
        ProcessStatus::Running
    );
}

#[test]
fn test_stop_running_updater_stopped_before_daemon_stop() {
    let temp = TempCodex::new("stop-order");
    let config_path = temp.path.join("config.toml");
    fs::write(&config_path, "[features]\ndaemon_auto_start = true\n").unwrap();

    // Fake `codex app-server daemon stop`: records whether the updater process
    // was already gone when it runs, then "stops" the daemon.
    let stop_bin_dir = temp
        .path
        .join("packages")
        .join("app-server-daemon")
        .join("current")
        .join("bin");
    fs::create_dir_all(&stop_bin_dir).unwrap();
    let stop_bin = stop_bin_dir.join("codex");
    let stop_script = r#"#!/bin/sh
if [ "$1" = "app-server" ] && [ "$2" = "daemon" ] && [ "$3" = "stop" ]; then
    updater_pid=$(cat "$CODEX_HOME/updater_pid_record" 2>/dev/null)
    if [ -n "$updater_pid" ] && kill -0 "$updater_pid" 2>/dev/null; then
        echo "updater-alive" > "$CODEX_HOME/stop_order"
    else
        echo "updater-gone" > "$CODEX_HOME/stop_order"
    fi
    rm -f "$CODEX_HOME/app-server-daemon/daemon.pid"
    exit 0
fi
exit 1
"#;
    fs::write(&stop_bin, stop_script).unwrap();
    #[cfg(unix)]
    {
        fs::set_permissions(&stop_bin, fs::Permissions::from_mode(0o755)).unwrap();
    }

    // The updater exits promptly on SIGTERM. It is spawned detached so the
    // system reaps it and the fake `daemon stop` can observe it as gone.
    let pid_record = temp.path.join("updater_pid_record");
    let updater = spawn_orphan_in_packages(&temp.path, "updater.sh", &pid_record, "exit 0");
    let (u_sec, u_usec) = process_start_time(updater.pid).expect("read start time");
    let updater_pid_file = temp.path.join("app-server-daemon").join("daemon-updater.pid");
    write_pid_file(
        &updater_pid_file,
        updater.pid as u32,
        current_boot_id().as_deref(),
        u_sec,
        u_usec,
    );
    wait_for_check_status(
        &updater_pid_file,
        &temp.path,
        ProcessStatus::Running,
        "running",
    );

    let daemon_pid_file = temp.path.join("app-server-daemon").join("daemon.pid");
    let (daemon_guard, d_sec, d_usec) =
        spawn_dummy_in_packages(&temp.path, "daemon.sh", &daemon_pid_file);
    write_pid_file(
        &daemon_pid_file,
        daemon_guard.0.id(),
        current_boot_id().as_deref(),
        d_sec,
        d_usec,
    );
    wait_for_check_status(
        &daemon_pid_file,
        &temp.path,
        ProcessStatus::Running,
        "running",
    );

    let res = disable_daemon_in_dir_with_options(&temp.path, true);
    match &res {
        CodexConfigResult::Changed {
            backup: Some(_),
            daemon: Some(DaemonStopStatus::Stopped),
            updater: Some(DaemonStopStatus::Stopped),
        } => {}
        _ => panic!("Expected Changed with both stopped, got: {:?}", res),
    }

    // The fake `daemon stop` must have observed that the updater was already gone.
    let order = fs::read_to_string(temp.path.join("stop_order")).unwrap();
    assert_eq!(order.trim(), "updater-gone");
    // And it still "stopped" the daemon afterwards.
    assert!(!daemon_pid_file.exists());
}

#[test]
fn test_stop_running_nonzero_exit_after_pid_removal_reports_failure() {
    assert_failed_stop_after_pid_removal("exit 1");
}

#[test]
fn test_stop_running_signal_exit_after_pid_removal_reports_failure() {
    assert_failed_stop_after_pid_removal("kill -TERM $$");
}

#[test]
fn test_stop_running_failure_is_preserved_after_successful_second_pass() {
    assert_failed_stop_after_pid_removal("exit 0");
}

fn assert_failed_stop_after_pid_removal(final_command: &str) {
    let temp = TempCodex::new("stop-failure");
    let stop_bin = temp
        .path
        .join("packages/app-server-daemon/current/bin/codex");
    fs::create_dir_all(stop_bin.parent().unwrap()).unwrap();
    // Fail once with the daemon still running, then succeed on the second
    // pass. The earlier failure must survive the successful stop.
    let first_pass = if final_command == "exit 0" {
        r#"if [ ! -f "$CODEX_HOME/first-pass" ]; then
    touch "$CODEX_HOME/first-pass"
    exit 1
fi"#
    } else {
        ""
    };
    fs::write(
        &stop_bin,
        format!(
            r#"#!/bin/sh
[ "$1 $2 $3" = "app-server daemon stop" ] || exit 2
{first_pass}
rm -f "$CODEX_HOME/app-server-daemon/daemon.pid"
{final_command}
"#
        ),
    )
    .unwrap();
    fs::set_permissions(&stop_bin, fs::Permissions::from_mode(0o755)).unwrap();
    let daemon = spawn_orphan_in_packages(
        &temp.path,
        "daemon.sh",
        &temp.path.join("daemon-record"),
        "exit 0",
    );
    let (sec, usec) = process_start_time(daemon.pid).unwrap();
    let pid_file = temp.path.join("app-server-daemon/daemon.pid");
    write_pid_file(
        &pid_file,
        daemon.pid as u32,
        current_boot_id().as_deref(),
        sec,
        usec,
    );
    wait_for_check_status(&pid_file, &temp.path, ProcessStatus::Running, "running");
    let (status, _) = stop_daemon_and_updater_with_timeouts(
        &temp.path,
        Duration::from_secs(1),
        Duration::from_millis(200),
    );
    assert!(!pid_file.exists());
    assert_eq!(status, DaemonStopStatus::StopFailed);
}

#[test]
fn test_stop_running_hanging_daemon_stop_is_killed_and_fails() {
    let temp = TempCodex::new("stop-hanging");

    // Fake `codex` that records its pid and then hangs for a long time.
    let stop_bin_dir = temp
        .path
        .join("packages")
        .join("app-server-daemon")
        .join("current")
        .join("bin");
    fs::create_dir_all(&stop_bin_dir).unwrap();
    let stop_bin = stop_bin_dir.join("codex");
    let stop_script = r#"#!/bin/sh
if [ "$1" = "app-server" ] && [ "$2" = "daemon" ] && [ "$3" = "stop" ]; then
    echo $$ > "$CODEX_HOME/hanging_stop_pid"
    exec sleep 60
fi
exit 1
"#;
    fs::write(&stop_bin, stop_script).unwrap();
    #[cfg(unix)]
    {
        fs::set_permissions(&stop_bin, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let daemon_pid_file = temp.path.join("app-server-daemon").join("daemon.pid");
    let (daemon_guard, d_sec, d_usec) =
        spawn_dummy_in_packages(&temp.path, "daemon.sh", &daemon_pid_file);
    write_pid_file(
        &daemon_pid_file,
        daemon_guard.0.id(),
        current_boot_id().as_deref(),
        d_sec,
        d_usec,
    );
    wait_for_check_status(
        &daemon_pid_file,
        &temp.path,
        ProcessStatus::Running,
        "running",
    );

    // Short timeout so the test runs fast; production uses 15 s.
    let timeout = Duration::from_millis(400);
    let start = Instant::now();
    let (daemon, updater) =
        stop_daemon_and_updater_with_timeouts(&temp.path, timeout, Duration::from_secs(1));
    let elapsed = start.elapsed();

    assert_eq!(daemon, DaemonStopStatus::StopFailed);
    assert_eq!(updater, DaemonStopStatus::NotRunning);
    assert!(
        elapsed >= timeout,
        "returned before the timeout expired: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "daemon stop was not bounded by the timeout: {elapsed:?}"
    );

    // The hung child must have been killed and reaped by the bridge.
    let hang_pid: libc::pid_t = fs::read_to_string(temp.path.join("hanging_stop_pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let mut gone = false;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let res = unsafe { libc::kill(hang_pid, 0) };
        if res != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            gone = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(gone, "hung daemon stop child was not killed");
}

#[test]
fn test_stop_running_slow_exiting_updater_still_stopped() {
    let temp = TempCodex::new("stop-slow-updater");
    let config_path = temp.path.join("config.toml");
    fs::write(&config_path, "[features]\ndaemon_auto_start = true\n").unwrap();

    // Updater traps TERM but only exits after a delay: 0.8 s is always slower
    // than the old 20 x 25 ms = 500 ms re-check window (so a regression to
    // that window deterministically fails this test) while leaving a wide
    // margin to the bridge's 5 s + 5 s exit budget under load. The trap
    // removes the pid file when it finally exits, so death is detected via
    // the missing pid file instead of introspecting an unreaped zombie.
    let packages_bin = temp
        .path
        .join("packages")
        .join("app-server-daemon")
        .join("bin");
    fs::create_dir_all(&packages_bin).unwrap();
    let updater_pid_file = temp
        .path
        .join("app-server-daemon")
        .join("daemon-updater.pid");
    let script_path = packages_bin.join("slow_updater.sh");
    let ready_marker = packages_bin.join("slow_updater.ready");
    fs::write(
        &script_path,
        format!(
            "#!/bin/sh\ntrap \"sleep 0.8; rm -f '{}'; exit 0\" TERM\necho $$ > '{}'\nwhile true; do sleep 0.05; done\n",
            updater_pid_file.display(),
            ready_marker.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut updater = ChildGuard(spawn_script(Path::new("/bin/sh"), &script_path));
    let updater_pid = updater.0.id() as libc::pid_t;
    wait_for_ready_marker(&ready_marker);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some((u_sec, u_usec)) = process_start_time(updater_pid) {
            write_pid_file(
                &updater_pid_file,
                updater.0.id(),
                current_boot_id().as_deref(),
                u_sec,
                u_usec,
            );
            break;
        }
        assert!(Instant::now() < deadline, "could not read updater start time");
        std::thread::sleep(Duration::from_millis(25));
    }
    wait_for_check_status(
        &updater_pid_file,
        &temp.path,
        ProcessStatus::Running,
        "running",
    );

    let res = disable_daemon_in_dir_with_options(&temp.path, true);
    match &res {
        CodexConfigResult::Changed {
            backup: Some(_),
            daemon: Some(DaemonStopStatus::NotRunning),
            updater: Some(DaemonStopStatus::Stopped),
        } => {}
        _ => panic!("Expected slow updater to be Stopped, got: {:?}", res),
    }

    // Reap the exited updater within its own generous deadline instead of a
    // fixed iteration count, so heavy load cannot fail this assertion.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(Some(_)) = updater.0.try_wait() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "slow updater should have exited after SIGTERM"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn test_check_process_snapshot_returns_the_pid_it_verified() {
    let temp = TempCodex::new("snapshot-pid");
    let pid_file = temp.path.join("app-server-daemon").join("daemon.pid");

    // No pid file: not running, nothing to signal.
    assert_eq!(
        check_process_snapshot(&pid_file, &temp.path),
        (ProcessStatus::NotRunning, None)
    );

    // Live dummy with matching identity: the returned pid is exactly the one
    // from the single snapshot that was validated.
    let (guard, sec, usec) = spawn_dummy_in_packages(&temp.path, "daemon.sh", &pid_file);
    write_pid_file(
        &pid_file,
        guard.0.id(),
        current_boot_id().as_deref(),
        sec,
        usec,
    );
    wait_for_check_status(&pid_file, &temp.path, ProcessStatus::Running, "running");
    assert_eq!(
        check_process_snapshot(&pid_file, &temp.path),
        (ProcessStatus::Running, Some(guard.0.id() as libc::pid_t))
    );

    // Malformed pid file: unknown, and no pid may be acted on.
    fs::write(&pid_file, "not json").unwrap();
    assert_eq!(
        check_process_snapshot(&pid_file, &temp.path),
        (ProcessStatus::Unknown, None)
    );
}

#[test]
fn test_term_updater_signals_only_the_verified_pid() {
    let temp = TempCodex::new("term-snapshot");
    let pid_file = temp.path.join("app-server-daemon").join("daemon-updater.pid");
    let (mut guard, sec, usec) = spawn_dummy_in_packages(&temp.path, "updater.sh", &pid_file);
    let pid = guard.0.id() as libc::pid_t;

    // Identity does NOT verify (start-time mismatch): term_updater must not
    // signal the live process even though the pid file names it.
    write_pid_file(
        &pid_file,
        guard.0.id(),
        current_boot_id().as_deref(),
        sec + 5000,
        Some(0),
    );
    wait_for_check_status(
        &pid_file,
        &temp.path,
        ProcessStatus::Unknown,
        "identity-unknown",
    );
    assert!(!term_updater(&pid_file, &temp.path));
    assert_eq!(unsafe { libc::kill(pid, 0) }, 0);

    // With a verifiable identity, term_updater signals exactly the pid from
    // the snapshot it validated: the dummy exits and removes its pid file.
    write_pid_file(
        &pid_file,
        guard.0.id(),
        current_boot_id().as_deref(),
        sec,
        usec,
    );
    wait_for_check_status(&pid_file, &temp.path, ProcessStatus::Running, "running");
    assert!(term_updater(&pid_file, &temp.path));

    let deadline = Instant::now() + Duration::from_secs(10);
    while pid_file.exists() {
        assert!(Instant::now() < deadline, "signalled updater did not exit");
        std::thread::sleep(Duration::from_millis(25));
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(Some(_)) = guard.0.try_wait() {
            break;
        }
        assert!(Instant::now() < deadline, "signalled updater was not reaped");
        std::thread::sleep(Duration::from_millis(25));
    }
}

// Spawns an updater dummy whose TERM trap installs `daemon_pid_record` as the
// daemon pid file before removing its own pid file and exiting — simulating an
// updater that restarts the daemon while it is itself being stopped.
fn spawn_restarting_updater(
    codex_dir: &Path,
    updater_pid_file: &Path,
    daemon_pid_record: &Path,
    daemon_pid_file: &Path,
) -> (ChildGuard, i64, Option<i64>) {
    let packages_bin = codex_dir
        .join("packages")
        .join("app-server-daemon")
        .join("bin");
    fs::create_dir_all(&packages_bin).unwrap();
    let script_path = packages_bin.join("restarting_updater.sh");
    let ready_marker = packages_bin.join("restarting_updater.ready");
    fs::write(
        &script_path,
        format!(
            "#!/bin/sh\ntrap \"cp '{}' '{}'; rm -f '{}'; exit 0\" TERM\necho $$ > '{}'\nwhile true; do sleep 0.05; done\n",
            daemon_pid_record.display(),
            daemon_pid_file.display(),
            updater_pid_file.display(),
            ready_marker.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let child = spawn_script(Path::new("/bin/sh"), &script_path);
    let pid = child.id() as libc::pid_t;
    wait_for_ready_marker(&ready_marker);
    let (start_sec, start_usec) = process_start_time(pid).expect("read start time");
    (ChildGuard(child), start_sec, start_usec)
}

#[test]
fn test_stop_running_daemon_appearing_mid_stop_reports_stop_failed() {
    let temp = TempCodex::new("stop-appearing-fails");
    let daemon_pid_file = temp.path.join("app-server-daemon").join("daemon.pid");
    let updater_pid_file = temp.path.join("app-server-daemon").join("daemon-updater.pid");
    let daemon_pid_record = temp.path.join("daemon_pid_record.json");

    // A live managed daemon that nothing will stop: no `codex` stop binary
    // exists in this CODEX_HOME.
    let (daemon_guard, d_sec, d_usec) =
        spawn_dummy_in_packages(&temp.path, "daemon.sh", &daemon_pid_file);
    write_pid_file(
        &daemon_pid_record,
        daemon_guard.0.id(),
        current_boot_id().as_deref(),
        d_sec,
        d_usec,
    );

    // The daemon pid file is ABSENT at the start; the updater's TERM trap
    // installs it (the "restart") while being stopped.
    let (updater_guard, u_sec, u_usec) = spawn_restarting_updater(
        &temp.path,
        &updater_pid_file,
        &daemon_pid_record,
        &daemon_pid_file,
    );
    write_pid_file(
        &updater_pid_file,
        updater_guard.0.id(),
        current_boot_id().as_deref(),
        u_sec,
        u_usec,
    );
    wait_for_check_status(
        &updater_pid_file,
        &temp.path,
        ProcessStatus::Running,
        "running",
    );
    assert!(!daemon_pid_file.exists());

    let (daemon, updater) = stop_daemon_and_updater_with_timeouts(
        &temp.path,
        Duration::from_secs(15),
        Duration::from_millis(300),
    );

    // The daemon appeared mid-operation and is running at the end: stop_failed,
    // never not_running. The updater was signalled and stopped.
    assert_eq!(daemon, DaemonStopStatus::StopFailed);
    assert_eq!(updater, DaemonStopStatus::Stopped);
    assert!(daemon_pid_file.exists());
    // The appearing daemon was never signalled and is still alive.
    let daemon_pid = daemon_guard.0.id() as libc::pid_t;
    assert_eq!(unsafe { libc::kill(daemon_pid, 0) }, 0);
}

#[test]
fn test_stop_running_daemon_appearing_mid_stop_is_stopped_by_second_pass() {
    let temp = TempCodex::new("stop-appearing-second-pass");
    let daemon_pid_file = temp.path.join("app-server-daemon").join("daemon.pid");
    let updater_pid_file = temp.path.join("app-server-daemon").join("daemon-updater.pid");
    let daemon_pid_record = temp.path.join("daemon_pid_record.json");

    // A `codex` binary whose `daemon stop` records invocations and "stops" the
    // daemon by removing its pid file.
    let stop_bin_dir = temp
        .path
        .join("packages")
        .join("app-server-daemon")
        .join("current")
        .join("bin");
    fs::create_dir_all(&stop_bin_dir).unwrap();
    let stop_bin = stop_bin_dir.join("codex");
    let stop_script = r#"#!/bin/sh
if [ "$1" = "app-server" ] && [ "$2" = "daemon" ] && [ "$3" = "stop" ]; then
    echo called >> "$CODEX_HOME/stop_calls"
    rm -f "$CODEX_HOME/app-server-daemon/daemon.pid"
    exit 0
fi
exit 1
"#;
    fs::write(&stop_bin, stop_script).unwrap();
    #[cfg(unix)]
    {
        fs::set_permissions(&stop_bin, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let (daemon_guard, d_sec, d_usec) =
        spawn_dummy_in_packages(&temp.path, "daemon.sh", &daemon_pid_file);
    write_pid_file(
        &daemon_pid_record,
        daemon_guard.0.id(),
        current_boot_id().as_deref(),
        d_sec,
        d_usec,
    );
    let (updater_guard, u_sec, u_usec) = spawn_restarting_updater(
        &temp.path,
        &updater_pid_file,
        &daemon_pid_record,
        &daemon_pid_file,
    );
    write_pid_file(
        &updater_pid_file,
        updater_guard.0.id(),
        current_boot_id().as_deref(),
        u_sec,
        u_usec,
    );
    wait_for_check_status(
        &updater_pid_file,
        &temp.path,
        ProcessStatus::Running,
        "running",
    );
    assert!(!daemon_pid_file.exists());

    let (daemon, updater) = stop_daemon_and_updater_with_timeouts(
        &temp.path,
        Duration::from_secs(15),
        Duration::from_millis(300),
    );

    // The daemon appeared during the first pass and the second pass stopped it.
    assert_eq!(daemon, DaemonStopStatus::Stopped);
    assert_eq!(updater, DaemonStopStatus::Stopped);
    assert!(!daemon_pid_file.exists());
    let calls = fs::read_to_string(temp.path.join("stop_calls")).unwrap();
    assert_eq!(calls.lines().count(), 1, "daemon stop calls: {calls:?}");
}
