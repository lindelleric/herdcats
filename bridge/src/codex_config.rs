use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use toml_edit::{DocumentMut, Item, TableLike, Value};

#[derive(Debug, PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessStatus {
    Running,
    NotRunning,
    Unknown,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonStopStatus {
    Stopped,
    NotRunning,
    StopFailed,
    /// A process that may be the daemon/updater is alive but its identity could
    /// not be verified (e.g. pid-file start-time mismatch). It is never
    /// signalled or stopped, and the result must not claim "not_running".
    Unknown,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonStatusResult {
    pub daemon: ProcessStatus,
    pub updater: ProcessStatus,
    pub auto_start: Option<bool>,
}

impl DaemonStatusResult {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum CodexConfigResult {
    Changed {
        backup: Option<String>,
        daemon: Option<DaemonStopStatus>,
        updater: Option<DaemonStopStatus>,
    },
    AlreadyDisabled {
        daemon: Option<DaemonStopStatus>,
        updater: Option<DaemonStopStatus>,
    },
    CodexNotInstalled,
    Error(&'static str),
}

impl CodexConfigResult {
    pub fn to_json(&self) -> String {
        match self {
            CodexConfigResult::Changed {
                backup,
                daemon,
                updater,
            } => {
                let mut map = serde_json::Map::new();
                map.insert("status".into(), serde_json::Value::String("changed".into()));
                if let Some(b) = backup {
                    map.insert("backup".into(), serde_json::Value::String(b.clone()));
                }
                if let Some(d) = daemon {
                    map.insert("daemon".into(), serde_json::to_value(d).unwrap());
                }
                if let Some(u) = updater {
                    map.insert("updater".into(), serde_json::to_value(u).unwrap());
                }
                serde_json::Value::Object(map).to_string()
            }
            CodexConfigResult::AlreadyDisabled { daemon, updater } => {
                let mut map = serde_json::Map::new();
                map.insert(
                    "status".into(),
                    serde_json::Value::String("already_disabled".into()),
                );
                if let Some(d) = daemon {
                    map.insert("daemon".into(), serde_json::to_value(d).unwrap());
                }
                if let Some(u) = updater {
                    map.insert("updater".into(), serde_json::to_value(u).unwrap());
                }
                serde_json::Value::Object(map).to_string()
            }
            CodexConfigResult::CodexNotInstalled => {
                r#"{"status":"codex_not_installed"}"#.to_string()
            }
            CodexConfigResult::Error(code) => {
                format!(r#"{{"error":"{code}"}}"#)
            }
        }
    }
}

pub fn resolve_codex_dir() -> Option<PathBuf> {
    resolve_codex_dir_from(std::env::var_os("CODEX_HOME"), std::env::var_os("HOME"))
}

pub fn resolve_codex_dir_from(
    codex_home: Option<impl AsRef<Path>>,
    home: Option<impl AsRef<Path>>,
) -> Option<PathBuf> {
    if let Some(val) = codex_home {
        let path = val.as_ref();
        if !path.as_os_str().is_empty() && path.is_absolute() {
            return Some(path.to_path_buf());
        }
    }
    if let Some(val) = home {
        let path = val.as_ref();
        if !path.as_os_str().is_empty() && path.is_absolute() {
            return Some(path.join(".codex"));
        }
    }
    None
}

pub fn utc_timestamp_string() -> String {
    let now = std::time::SystemTime::now();
    let duration = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    let days = (secs / 86400) as i64;
    let day_secs = (secs % 86400) as u32;
    let hour = day_secs / 3600;
    let min = (day_secs % 3600) / 60;
    let sec = day_secs % 60;
    let (year, mon, day) = crate::codex::civil_from_days(days);
    format!("{year:04}{mon:02}{day:02}T{hour:02}{min:02}{sec:02}Z")
}

pub fn disable_daemon() -> CodexConfigResult {
    disable_daemon_with_options(false)
}

pub fn disable_daemon_with_options(stop_running: bool) -> CodexConfigResult {
    let codex_dir = match resolve_codex_dir() {
        Some(d) => d,
        None => return CodexConfigResult::CodexNotInstalled,
    };
    disable_daemon_in_dir_with_options(&codex_dir, stop_running)
}

pub fn disable_daemon_in_dir(codex_dir: &Path) -> CodexConfigResult {
    disable_daemon_in_dir_with_options(codex_dir, false)
}

pub fn disable_daemon_in_dir_with_options(
    codex_dir: &Path,
    stop_running: bool,
) -> CodexConfigResult {
    let config_res = disable_daemon_in_dir_inner(codex_dir);
    if !stop_running {
        return config_res;
    }
    match config_res {
        CodexConfigResult::Changed { backup, .. } => {
            let (daemon, updater) = stop_daemon_and_updater(codex_dir);
            CodexConfigResult::Changed {
                backup,
                daemon: Some(daemon),
                updater: Some(updater),
            }
        }
        CodexConfigResult::AlreadyDisabled { .. } => {
            let (daemon, updater) = stop_daemon_and_updater(codex_dir);
            CodexConfigResult::AlreadyDisabled {
                daemon: Some(daemon),
                updater: Some(updater),
            }
        }
        other => other,
    }
}

fn disable_daemon_in_dir_inner(codex_dir: &Path) -> CodexConfigResult {
    if !codex_dir.is_dir() {
        return CodexConfigResult::CodexNotInstalled;
    }
    let config_path = codex_dir.join("config.toml");
    match fs::symlink_metadata(&config_path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return CodexConfigResult::Error("config_symlink");
            }
            if !meta.is_file() {
                return CodexConfigResult::Error("write_failed");
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            // Missing config.toml: create with mode 0600
            let write_res: std::io::Result<()> = (|| {
                use std::io::Write;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    let mut file = fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(0o600)
                        .open(&config_path)?;
                    file.write_all(b"[features]\ndaemon_auto_start = false\n")?;
                    file.sync_all()?;
                }
                #[cfg(not(unix))]
                {
                    fs::write(&config_path, b"[features]\ndaemon_auto_start = false\n")?;
                }
                Ok(())
            })();
            if write_res.is_err() {
                return CodexConfigResult::Error("write_failed");
            }
            return CodexConfigResult::Changed {
                backup: None,
                daemon: None,
                updater: None,
            };
        }
        Err(_) => return CodexConfigResult::Error("write_failed"),
    }

    let orig_metadata = match fs::symlink_metadata(&config_path) {
        Ok(m) => m,
        Err(_) => return CodexConfigResult::Error("write_failed"),
    };
    #[cfg(unix)]
    let orig_mode = {
        use std::os::unix::fs::PermissionsExt;
        orig_metadata.permissions().mode()
    };

    let content = match fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(_) => return CodexConfigResult::Error("write_failed"),
    };

    let mut doc: DocumentMut = match content.parse() {
        Ok(d) => d,
        Err(_) => return CodexConfigResult::Error("config_invalid"),
    };

    if let Some(item) = doc.get("features") {
        if !item.is_table_like() {
            return CodexConfigResult::Error("features_not_table");
        }
        if let Some(tbl) = item.as_table_like() {
            if tbl.get("daemon_auto_start").and_then(|v| v.as_bool()) == Some(false) {
                return CodexConfigResult::AlreadyDisabled {
                    daemon: None,
                    updater: None,
                };
            }
        }
    }

    if doc.get("features").is_none() {
        let mut t = toml_edit::Table::new();
        t.insert("daemon_auto_start", toml_edit::value(false));
        doc.insert("features", Item::Table(t));
    } else {
        let feat = doc.get_mut("features").unwrap();
        let tbl = feat.as_table_like_mut().unwrap();
        if let Some(item) = tbl.get_mut("daemon_auto_start") {
            match item.as_value_mut() {
                Some(val) if !val.is_array() && !val.is_inline_table() => {
                    let decor = val.decor().clone();
                    *val = Value::from(false);
                    *val.decor_mut() = decor;
                }
                _ => return CodexConfigResult::Error("daemon_setting_unsupported"),
            }
        } else {
            tbl.insert("daemon_auto_start", toml_edit::value(false));
        }
    }

    let timestamp = utc_timestamp_string();
    let mut backup_name = format!("config.toml.bak.herdcats-{timestamp}");
    let mut backup_path = codex_dir.join(&backup_name);
    let mut counter = 1;
    while backup_path.exists() {
        backup_name = format!("config.toml.bak.herdcats-{timestamp}_{counter}");
        backup_path = codex_dir.join(&backup_name);
        counter += 1;
    }

    if fs::copy(&config_path, &backup_path).is_err() {
        return CodexConfigResult::Error("write_failed");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&backup_path, fs::Permissions::from_mode(orig_mode));
    }

    let temp_name = format!(".config.toml.tmp.herdcats-{}", std::process::id());
    let temp_path = codex_dir.join(&temp_name);
    let write_op: std::io::Result<()> = (|| {
        use std::io::Write;
        #[cfg(unix)]
        let mut file = {
            use std::os::unix::fs::OpenOptionsExt;
            fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(orig_mode)
                .open(&temp_path)?
        };
        #[cfg(not(unix))]
        let mut file = fs::File::create(&temp_path)?;

        file.write_all(doc.to_string().as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp_path, &config_path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&config_path, fs::Permissions::from_mode(orig_mode));
        }
        Ok(())
    })();

    if write_op.is_err() {
        let _ = fs::remove_file(&temp_path);
        return CodexConfigResult::Error("write_failed");
    }

    let reparsed_content = match fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(_) => {
            let _ = fs::copy(&backup_path, &config_path);
            return CodexConfigResult::Error("verification_failed");
        }
    };
    let reparsed_doc: DocumentMut = match reparsed_content.parse() {
        Ok(d) => d,
        Err(_) => {
            let _ = fs::copy(&backup_path, &config_path);
            return CodexConfigResult::Error("verification_failed");
        }
    };
    let orig_doc: DocumentMut = match content.parse() {
        Ok(d) => d,
        Err(_) => {
            let _ = fs::copy(&backup_path, &config_path);
            return CodexConfigResult::Error("verification_failed");
        }
    };

    if !verify_config(&orig_doc, &reparsed_doc) {
        let _ = fs::copy(&backup_path, &config_path);
        return CodexConfigResult::Error("verification_failed");
    }

    CodexConfigResult::Changed {
        backup: Some(backup_name),
        daemon: None,
        updater: None,
    }
}

fn verify_config(orig: &DocumentMut, reparsed: &DocumentMut) -> bool {
    let feat_rep = match reparsed.get("features").and_then(|it| it.as_table_like()) {
        Some(t) => t,
        None => return false,
    };
    if feat_rep.get("daemon_auto_start").and_then(|v| v.as_bool()) != Some(false) {
        return false;
    }

    let orig_keys: std::collections::BTreeSet<_> = orig.iter().map(|(k, _)| k).collect();
    let rep_keys: std::collections::BTreeSet<_> = reparsed.iter().map(|(k, _)| k).collect();
    let all_keys: std::collections::BTreeSet<_> = orig_keys.union(&rep_keys).copied().collect();

    for k in all_keys {
        if k == "features" {
            match orig.get("features") {
                None => {
                    let keys: Vec<_> = feat_rep.iter().map(|(k, _)| k).collect();
                    if keys != vec!["daemon_auto_start"] {
                        return false;
                    }
                }
                Some(orig_feat_item) => {
                    let orig_feat = match orig_feat_item.as_table_like() {
                        Some(t) => t,
                        None => return false,
                    };
                    let f_orig_keys: std::collections::BTreeSet<_> =
                        orig_feat.iter().map(|(k, _)| k).collect();
                    let f_rep_keys: std::collections::BTreeSet<_> =
                        feat_rep.iter().map(|(k, _)| k).collect();
                    let f_all_keys: std::collections::BTreeSet<_> =
                        f_orig_keys.union(&f_rep_keys).copied().collect();
                    for fk in f_all_keys {
                        if fk == "daemon_auto_start" {
                            continue;
                        }
                        match (orig_feat.get(fk), feat_rep.get(fk)) {
                            (Some(ia), Some(ib)) => {
                                if !items_equal(ia, ib) {
                                    return false;
                                }
                            }
                            _ => return false,
                        }
                    }
                }
            }
        } else {
            match (orig.get(k), reparsed.get(k)) {
                (Some(ia), Some(ib)) => {
                    if !items_equal(ia, ib) {
                        return false;
                    }
                }
                _ => return false,
            }
        }
    }
    true
}

fn items_equal(a: &Item, b: &Item) -> bool {
    match (a, b) {
        (Item::None, Item::None) => true,
        (Item::Value(va), Item::Value(vb)) => values_equal(va, vb),
        (Item::Table(ta), Item::Table(tb)) => tables_equal(ta, tb),
        (Item::ArrayOfTables(aa), Item::ArrayOfTables(ab)) => {
            if aa.len() != ab.len() {
                return false;
            }
            aa.iter()
                .zip(ab.iter())
                .all(|(ta, tb)| tables_equal(ta, tb))
        }
        _ => {
            if let (Some(ta), Some(tb)) = (a.as_table_like(), b.as_table_like()) {
                table_likes_equal(ta, tb)
            } else {
                false
            }
        }
    }
}

fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(sa), Value::String(sb)) => sa.value() == sb.value(),
        (Value::Integer(ia), Value::Integer(ib)) => ia.value() == ib.value(),
        (Value::Float(fa), Value::Float(fb)) => {
            let va = *fa.value();
            let vb = *fb.value();
            va == vb || (va.is_nan() && vb.is_nan())
        }
        (Value::Boolean(ba), Value::Boolean(bb)) => ba.value() == bb.value(),
        (Value::Datetime(da), Value::Datetime(db)) => da.value() == db.value(),
        (Value::Array(aa), Value::Array(ab)) => {
            if aa.len() != ab.len() {
                return false;
            }
            aa.iter()
                .zip(ab.iter())
                .all(|(va, vb)| values_equal(va, vb))
        }
        (Value::InlineTable(ia), Value::InlineTable(ib)) => table_likes_equal(ia, ib),
        _ => false,
    }
}

fn tables_equal(a: &toml_edit::Table, b: &toml_edit::Table) -> bool {
    table_likes_equal(a, b)
}

fn table_likes_equal(a: &dyn TableLike, b: &dyn TableLike) -> bool {
    let keys_a: std::collections::BTreeSet<_> = a.iter().map(|(k, _)| k).collect();
    let keys_b: std::collections::BTreeSet<_> = b.iter().map(|(k, _)| k).collect();
    if keys_a != keys_b {
        return false;
    }
    for k in keys_a {
        match (a.get(k), b.get(k)) {
            (Some(ia), Some(ib)) => {
                if !items_equal(ia, ib) {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

pub fn daemon_status() -> DaemonStatusResult {
    let codex_dir = match resolve_codex_dir() {
        Some(d) => d,
        None => {
            return DaemonStatusResult {
                daemon: ProcessStatus::NotRunning,
                updater: ProcessStatus::NotRunning,
                auto_start: None,
            };
        }
    };
    daemon_status_in_dir(&codex_dir)
}

pub fn daemon_status_in_dir(codex_dir: &Path) -> DaemonStatusResult {
    let daemon_pid = codex_dir.join("app-server-daemon").join("daemon.pid");
    let updater_pid = codex_dir.join("app-server-daemon").join("daemon-updater.pid");
    let daemon = check_process(&daemon_pid, codex_dir);
    let updater = check_process(&updater_pid, codex_dir);
    let auto_start = read_auto_start(codex_dir);
    DaemonStatusResult {
        daemon,
        updater,
        auto_start,
    }
}

pub fn read_auto_start(codex_dir: &Path) -> Option<bool> {
    let config_path = codex_dir.join("config.toml");
    let content = fs::read_to_string(&config_path).ok()?;
    let doc: DocumentMut = content.parse().ok()?;
    let features = doc.get("features")?.as_table_like()?;
    features.get("daemon_auto_start")?.as_bool()
}

/// Classify the process recorded in `pid_file`:
/// - `Running`: alive, owned by us, executable under
///   `$CODEX_HOME/packages/app-server-daemon/`, and the pid-file start time
///   matches.
/// - `NotRunning`: no pid file, no such process, another user's process, or an
///   executable outside the packages dir.
/// - `Unknown`: alive and plausibly ours, but identity is inconclusive (e.g. the
///   pid-file start time does not match). Codex's pid-file `startSeconds`
///   semantics are verified on macOS only; on Linux they are unverified, so a
///   start-time mismatch there must NOT be reported as `NotRunning` — a daemon
///   could really be running while the app tells the user to "send a message"
///   forever.
pub fn check_process(pid_file: &Path, codex_dir: &Path) -> ProcessStatus {
    if !pid_file.exists() {
        return ProcessStatus::NotRunning;
    }
    let metadata = match fs::symlink_metadata(pid_file) {
        Ok(m) => m,
        Err(_) => return ProcessStatus::Unknown,
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return ProcessStatus::Unknown;
    }
    let content = match fs::read_to_string(pid_file) {
        Ok(c) => c,
        Err(_) => return ProcessStatus::Unknown,
    };
    let val: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return ProcessStatus::Unknown,
    };
    let pid_i64 = match val.get("pid").and_then(|p| p.as_i64()) {
        Some(p) if p > 0 => p,
        _ => return ProcessStatus::Unknown,
    };
    let pid: libc::pid_t = match pid_i64.try_into() {
        Ok(p) => p,
        Err(_) => return ProcessStatus::Unknown,
    };

    let start_seconds = match val
        .get("processIdentity")
        .and_then(|i| i.get("startSeconds"))
        .and_then(|s| s.as_i64())
    {
        Some(s) => s,
        None => return ProcessStatus::Unknown,
    };
    let start_microseconds = val
        .get("processIdentity")
        .and_then(|i| i.get("startMicroseconds"))
        .and_then(|u| u.as_i64());
    let boot_id = val
        .get("processIdentity")
        .and_then(|i| i.get("bootId"))
        .and_then(|b| b.as_str());

    let alive = unsafe { libc::kill(pid, 0) };
    if alive != 0 {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ESRCH) || err.raw_os_error() == Some(libc::EPERM) {
            return ProcessStatus::NotRunning;
        }
        return ProcessStatus::Unknown;
    }

    match verify_process_identity(pid, codex_dir, boot_id, start_seconds, start_microseconds) {
        Ok(true) => ProcessStatus::Running,
        Ok(false) => ProcessStatus::NotRunning,
        Err(_) => ProcessStatus::Unknown,
    }
}

/// How long `codex app-server daemon stop` may run before it is killed.
const DAEMON_STOP_COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
/// How long to wait for a signalled process to disappear before declaring failure.
const PROCESS_EXIT_WAIT: Duration = Duration::from_secs(5);
const PROCESS_EXIT_POLL_STEP: Duration = Duration::from_millis(100);

fn stop_daemon_and_updater(codex_dir: &Path) -> (DaemonStopStatus, DaemonStopStatus) {
    stop_daemon_and_updater_with_timeout(codex_dir, DAEMON_STOP_COMMAND_TIMEOUT)
}

// Exposed so integration tests can shorten the `daemon stop` timeout.
#[doc(hidden)]
pub fn stop_daemon_and_updater_with_timeout(
    codex_dir: &Path,
    stop_command_timeout: Duration,
) -> (DaemonStopStatus, DaemonStopStatus) {
    let daemon_pid_file = codex_dir.join("app-server-daemon").join("daemon.pid");
    let updater_pid_file = codex_dir.join("app-server-daemon").join("daemon-updater.pid");

    let initial_daemon = check_process(&daemon_pid_file, codex_dir);
    let initial_updater = check_process(&updater_pid_file, codex_dir);

    // Stop the updater FIRST: SIGTERM it (after re-checking its identity) and wait
    // for it to disappear before running `daemon stop`, so it cannot restart the
    // daemon in between. A process whose identity is only Unknown is never signalled.
    if initial_updater == ProcessStatus::Running && term_updater(&updater_pid_file, codex_dir) {
        wait_for_process_exit(&updater_pid_file, codex_dir);
    }

    // Then ask the official binary to stop the daemon, bounded by a hard timeout.
    let mut daemon_stop_failed = false;
    if initial_daemon == ProcessStatus::Running {
        let stop_bin = codex_dir
            .join("packages")
            .join("app-server-daemon")
            .join("current")
            .join("bin")
            .join("codex");
        if stop_bin.is_file() {
            daemon_stop_failed = run_daemon_stop_command(&stop_bin, codex_dir, stop_command_timeout);
        }
    }

    // Re-check both. The updater may need a while to exit after SIGTERM, so poll
    // for up to 5 s total before reporting stop_failed.
    let mut final_daemon = check_process(&daemon_pid_file, codex_dir);
    let mut final_updater = check_process(&updater_pid_file, codex_dir);
    let deadline = Instant::now() + PROCESS_EXIT_WAIT;
    while Instant::now() < deadline {
        let daemon_done = initial_daemon != ProcessStatus::Running
            || daemon_stop_failed
            || final_daemon == ProcessStatus::NotRunning;
        let updater_done =
            initial_updater != ProcessStatus::Running || final_updater == ProcessStatus::NotRunning;
        if daemon_done && updater_done {
            break;
        }
        std::thread::sleep(PROCESS_EXIT_POLL_STEP);
        final_daemon = check_process(&daemon_pid_file, codex_dir);
        final_updater = check_process(&updater_pid_file, codex_dir);
    }

    (
        stop_status(initial_daemon, final_daemon, daemon_stop_failed),
        stop_status(initial_updater, final_updater, false),
    )
}

fn stop_status(
    initial: ProcessStatus,
    final_status: ProcessStatus,
    stop_failed: bool,
) -> DaemonStopStatus {
    match initial {
        ProcessStatus::Running => {
            if stop_failed || final_status != ProcessStatus::NotRunning {
                DaemonStopStatus::StopFailed
            } else {
                DaemonStopStatus::Stopped
            }
        }
        // Unknown (e.g. alive, owned by us, under the packages dir, but with a
        // mismatching pid-file start time) is never signalled or stopped, and
        // the result must not claim not_running — a daemon may really be
        // running. Report Unknown so the app does not tell the user that
        // nothing was running.
        ProcessStatus::Unknown => DaemonStopStatus::Unknown,
        ProcessStatus::NotRunning => DaemonStopStatus::NotRunning,
    }
}

// SIGTERM the updater recorded in `updater_pid_file` after re-verifying its
// identity right before signalling, so a reused pid is never signalled.
// Returns true iff the signal was sent.
fn term_updater(updater_pid_file: &Path, codex_dir: &Path) -> bool {
    if let Ok(content) = fs::read_to_string(updater_pid_file) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(pid_i64) = val.get("pid").and_then(|p| p.as_i64()) {
                if let Ok(pid) = libc::pid_t::try_from(pid_i64) {
                    if check_process(updater_pid_file, codex_dir) == ProcessStatus::Running {
                        unsafe {
                            libc::kill(pid, libc::SIGTERM);
                        }
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn wait_for_process_exit(pid_file: &Path, codex_dir: &Path) {
    let deadline = Instant::now() + PROCESS_EXIT_WAIT;
    while Instant::now() < deadline {
        if check_process(pid_file, codex_dir) != ProcessStatus::Running {
            return;
        }
        std::thread::sleep(PROCESS_EXIT_POLL_STEP);
    }
}

// Runs `<codex> app-server daemon stop` with stdin/stdout/stderr tied to /dev/null
// (the bridge speaks NDJSON on its own stdout and must not inherit stdio into the
// child, nor block on its output) and polls try_wait for up to `timeout`. On
// timeout the child is killed and the stop is reported as failed.
fn run_daemon_stop_command(stop_bin: &Path, codex_dir: &Path, timeout: Duration) -> bool {
    use std::process::Stdio;
    let child = std::process::Command::new(stop_bin)
        .args(["app-server", "daemon", "stop"])
        .env("CODEX_HOME", codex_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        // Could not even start the command; whether the daemon stopped is left
        // to the final re-check.
        Err(_) => return false,
    };
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return false,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return true;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return true;
            }
        }
    }
}

fn path_is_under(path: &Path, packages_dir: &Path, canon_packages_dir: &Path) -> bool {
    if (path.starts_with(packages_dir) && path != packages_dir)
        || (path.starts_with(canon_packages_dir) && path != canon_packages_dir)
    {
        return true;
    }
    if let Ok(canon) = path.canonicalize() {
        if (canon.starts_with(packages_dir) && canon != packages_dir)
            || (canon.starts_with(canon_packages_dir) && canon != canon_packages_dir)
        {
            return true;
        }
    }
    false
}

pub fn current_boot_id() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        macos_boot_id()
    }
    #[cfg(target_os = "linux")]
    {
        fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .ok()
            .map(|s| s.trim().to_string())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

pub fn process_start_time(pid: libc::pid_t) -> Option<(i64, Option<i64>)> {
    #[cfg(target_os = "macos")]
    {
        let mut bsd_info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        let ret = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                &mut bsd_info as *mut _ as *mut libc::c_void,
                size,
            )
        };
        if ret == size {
            Some((
                bsd_info.pbi_start_tvsec as i64,
                Some(bsd_info.pbi_start_tvusec as i64),
            ))
        } else {
            None
        }
    }
    #[cfg(target_os = "linux")]
    {
        let stat_content = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let last_paren = stat_content.rfind(')')?;
        let rest = stat_content[last_paren + 1..].trim();
        let fields: Vec<&str> = rest.split_whitespace().collect();
        let starttime_ticks: u64 = fields.get(19)?.parse().ok()?;
        let btime = linux_btime()?;
        let ticks_per_sec = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        let ticks_per_sec = if ticks_per_sec > 0 {
            ticks_per_sec as u64
        } else {
            100
        };
        let start_sec = btime + (starttime_ticks / ticks_per_sec);
        let start_usec = ((starttime_ticks % ticks_per_sec) * 1_000_000) / ticks_per_sec;
        Some((start_sec as i64, Some(start_usec as i64)))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = pid;
        None
    }
}

#[cfg(target_os = "macos")]
fn macos_boot_id() -> Option<String> {
    let name = std::ffi::CString::new("kern.bootsessionuuid").ok()?;
    let mut buf = [0u8; 128];
    let mut len = buf.len();
    let res = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            buf.as_mut_ptr() as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if res == 0 && len > 0 {
        let slice = &buf[..len];
        let end = slice.iter().position(|&b| b == 0).unwrap_or(len);
        std::str::from_utf8(&slice[..end])
            .ok()
            .map(|s| s.to_string())
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
fn macos_proc_args(pid: libc::pid_t) -> Option<(Option<String>, Vec<String>)> {
    let mut mib = [libc::CTL_KERN, 49, pid]; // 49 = KERN_PROCARGS2
    let mut size: libc::size_t = 0;
    unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        );
    }
    if size < 4 {
        return None;
    }
    let mut buf = vec![0u8; size];
    let res = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr() as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if res != 0 || size < 4 {
        return None;
    }
    let argc = u32::from_ne_bytes(buf[0..4].try_into().ok()?);
    let exec_end = buf[4..].iter().position(|&b| b == 0)?;
    let exec_path = std::str::from_utf8(&buf[4..4 + exec_end])
        .ok()
        .map(|s| s.to_string());
    let mut idx = 4 + exec_end;
    while idx < buf.len() && buf[idx] == 0 {
        idx += 1;
    }
    let mut args = Vec::new();
    for _ in 0..argc {
        if idx >= buf.len() {
            break;
        }
        let end = buf[idx..]
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(buf.len() - idx);
        if let Ok(arg) = std::str::from_utf8(&buf[idx..idx + end]) {
            args.push(arg.to_string());
        }
        idx += end + 1;
    }
    Some((exec_path, args))
}

#[cfg(target_os = "macos")]
fn verify_process_identity(
    pid: libc::pid_t,
    codex_dir: &Path,
    expected_boot_id: Option<&str>,
    expected_start_sec: i64,
    expected_start_usec: Option<i64>,
) -> std::result::Result<bool, &'static str> {
    if let Some(boot_id) = expected_boot_id {
        if let Some(cur) = macos_boot_id() {
            if cur != boot_id {
                return Ok(false);
            }
        }
    }

    let mut bsd_info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let ret = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            &mut bsd_info as *mut _ as *mut libc::c_void,
            size,
        )
    };
    if ret != size {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ESRCH) {
            return Ok(false);
        }
        return Err("proc_pidinfo_failed");
    }

    let current_uid = unsafe { libc::getuid() };
    if bsd_info.pbi_uid != current_uid {
        return Ok(false);
    }

    let packages_dir = codex_dir.join("packages").join("app-server-daemon");
    let canon_packages_dir = packages_dir
        .canonicalize()
        .unwrap_or_else(|_| packages_dir.clone());

    // Check the executable (and argv) BEFORE the start time: only a process
    // that is not ours (executable outside the packages dir) may be reported
    // as definitely NotRunning on the basis of a mismatch.
    let mut path_ok = false;
    let mut path_buf = [0u8; 4096];
    let path_len = unsafe {
        libc::proc_pidpath(
            pid,
            path_buf.as_mut_ptr() as *mut libc::c_void,
            path_buf.len() as u32,
        )
    };
    if path_len > 0 {
        if let Ok(path_str) = std::str::from_utf8(&path_buf[..path_len as usize]) {
            let p = Path::new(path_str);
            if path_is_under(p, &packages_dir, &canon_packages_dir) {
                path_ok = true;
            }
        }
    }

    if !path_ok {
        if let Some((exec_path, args)) = macos_proc_args(pid) {
            if let Some(ref ep) = exec_path {
                if path_is_under(Path::new(ep), &packages_dir, &canon_packages_dir) {
                    path_ok = true;
                }
            }
            if !path_ok {
                for arg in &args {
                    if path_is_under(Path::new(arg), &packages_dir, &canon_packages_dir) {
                        path_ok = true;
                        break;
                    }
                }
            }
        }
    }

    if !path_ok {
        return Ok(false);
    }

    // Alive, owned by us, and running from under the packages dir. On macOS a
    // full start-time match means Running; a mismatch is inconclusive (pid
    // reuse is the likely cause, but the pid file may also be wrong), so it is
    // reported as Unknown rather than NotRunning. See the note on
    // `check_process`: Codex's pid-file `startSeconds` semantics are only
    // verified on macOS and unverified on Linux, so a mismatch must never be
    // read as "no daemon is running".
    let proc_start_sec = bsd_info.pbi_start_tvsec as i64;
    let proc_start_usec = bsd_info.pbi_start_tvusec as i64;
    if proc_start_sec != expected_start_sec {
        return Err("start_time_mismatch");
    }
    if let Some(usec) = expected_start_usec {
        if proc_start_usec != usec {
            return Err("start_time_mismatch");
        }
    }

    Ok(true)
}

#[cfg(target_os = "linux")]
fn linux_btime() -> Option<u64> {
    let stat = fs::read_to_string("/proc/stat").ok()?;
    for line in stat.lines() {
        if line.starts_with("btime ") {
            return line.split_whitespace().nth(1).and_then(|s| s.parse().ok());
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn verify_process_identity(
    pid: libc::pid_t,
    codex_dir: &Path,
    expected_boot_id: Option<&str>,
    expected_start_sec: i64,
    _expected_start_usec: Option<i64>,
) -> std::result::Result<bool, &'static str> {
    use std::os::unix::fs::MetadataExt;

    if let Some(boot_id) = expected_boot_id {
        if let Ok(cur) = fs::read_to_string("/proc/sys/kernel/random/boot_id") {
            if cur.trim() != boot_id.trim() {
                return Ok(false);
            }
        }
    }

    let proc_path = PathBuf::from(format!("/proc/{pid}"));
    let meta = match fs::metadata(&proc_path) {
        Ok(m) => m,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err("proc_stat_failed"),
    };
    let current_uid = unsafe { libc::getuid() };
    if meta.uid() != current_uid {
        return Ok(false);
    }

    let stat_content = match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(c) => c,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err("proc_stat_failed"),
    };
    let last_paren = match stat_content.rfind(')') {
        Some(idx) => idx,
        None => return Err("stat_format_invalid"),
    };
    let rest = stat_content[last_paren + 1..].trim();
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let starttime_ticks: u64 = match fields.get(19).and_then(|f| f.parse().ok()) {
        Some(t) => t,
        None => return Err("stat_starttime_invalid"),
    };

    let btime = linux_btime().ok_or("btime_read_failed")?;
    let ticks_per_sec = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    let ticks_per_sec = if ticks_per_sec > 0 {
        ticks_per_sec as u64
    } else {
        100
    };

    let packages_dir = codex_dir.join("packages").join("app-server-daemon");
    let canon_packages_dir = packages_dir
        .canonicalize()
        .unwrap_or_else(|_| packages_dir.clone());

    // Check the executable (and argv) BEFORE the start time: only a process
    // that is not ours (executable outside the packages dir) may be reported
    // as definitely NotRunning on the basis of a mismatch.
    let mut path_ok = false;
    if let Ok(exe_link) = fs::read_link(format!("/proc/{pid}/exe")) {
        if path_is_under(&exe_link, &packages_dir, &canon_packages_dir) {
            path_ok = true;
        }
    }
    if !path_ok {
        if let Ok(cmdline_bytes) = fs::read(format!("/proc/{pid}/cmdline")) {
            for arg in cmdline_bytes.split(|&b| b == 0) {
                if let Ok(arg_str) = std::str::from_utf8(arg) {
                    if !arg_str.is_empty()
                        && path_is_under(Path::new(arg_str), &packages_dir, &canon_packages_dir)
                    {
                        path_ok = true;
                        break;
                    }
                }
            }
        }
    }
    if !path_ok {
        return Ok(false);
    }

    // Alive, owned by us, and running from under the packages dir. NOTE:
    // Codex's pid-file `startSeconds` semantics have NOT been verified on
    // Linux, so a start-time mismatch here is inconclusive (pid reuse is the
    // likely cause, but our reading of the pid file or of /proc could also be
    // wrong) and is reported as Unknown, never as NotRunning — reporting
    // NotRunning while a daemon is really running would make the app tell the
    // user to "send a message" forever.
    let start_sec = btime + (starttime_ticks / ticks_per_sec);
    if (start_sec as i64 - expected_start_sec).abs() > 2 {
        return Err("start_time_mismatch");
    }

    Ok(true)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn verify_process_identity(
    _pid: libc::pid_t,
    _codex_dir: &Path,
    _expected_boot_id: Option<&str>,
    _expected_start_sec: i64,
    _expected_start_usec: Option<i64>,
) -> std::result::Result<bool, &'static str> {
    Err("unsupported_platform")
}
