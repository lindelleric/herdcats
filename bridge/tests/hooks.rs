use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "herdcats-hooks-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn run(&self, action: &str, agent: &str) -> Value {
        let o = Command::new(env!("CARGO_BIN_EXE_herdcats-bridge"))
            .args(["hooks", action, "--agent", agent])
            .env("CLAUDE_CONFIG_DIR", self.0.join("claude"))
            .env("CODEX_HOME", self.0.join("codex"))
            .env("XDG_STATE_HOME", self.0.join("state"))
            .output()
            .unwrap();
        assert!(o.status.success(), "{:?}", o);
        assert!(o.stderr.is_empty());
        serde_json::from_slice(&o.stdout).unwrap()
    }
    fn file(&self, agent: &str) -> PathBuf {
        self.0.join(agent).join(if agent == "claude" {
            "settings.json"
        } else {
            "hooks.json"
        })
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn missing_install_and_idempotence() {
    for agent in ["claude", "codex"] {
        let t = Temp::new();
        assert_eq!(t.run("status", agent)["status"], "missing");
        assert_eq!(t.run("install", agent)["status"], "installed");
        let p = t.file(agent);
        let before = fs::read(&p).unwrap();
        assert_eq!(t.run("install", agent)["status"], "installed");
        assert_eq!(before, fs::read(&p).unwrap());
        let v: Value = serde_json::from_slice(&before).unwrap();
        let group = &v["hooks"]["PermissionRequest"][0];
        assert_eq!(group["hooks"][0]["timeout"], 10);
        assert_eq!(group.get("matcher").is_some(), agent == "claude");
        assert_eq!(
            t.run("status", agent)["status"],
            if agent == "claude" {
                "installed"
            } else {
                "needs_trust"
            }
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(p).unwrap().permissions().mode() & 0o777,
                if agent == "claude" { 0o600 } else { 0o644 }
            );
        }
    }
}
#[test]
fn preserves_order_backup_and_replaces_stale_in_place() {
    let t = Temp::new();
    let p = t.file("claude");
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    let orig = r#"{"z":1,"hooks":{"PermissionRequest":[{"hooks":[{"command":"echo before"}]},{"matcher":"*","hooks":[{"type":"command","command":"/old/bridge hook record --agent claude","timeout":10},{"command":"echo keep"}]}]},"a":2}"#;
    fs::write(&p, orig).unwrap();
    assert_eq!(t.run("status", "claude")["status"], "stale");
    let result = t.run("install", "claude");
    assert!(result["backup"].is_string());
    assert_eq!(
        fs::read_to_string(p.parent().unwrap().join(result["backup"].as_str().unwrap())).unwrap(),
        orig
    );
    let s = fs::read_to_string(&p).unwrap();
    assert!(s.find("\"z\"").unwrap() < s.find("\"a\"").unwrap());
    let v: Value = serde_json::from_str(&s).unwrap();
    assert_eq!(v["hooks"]["PermissionRequest"].as_array().unwrap().len(), 2);
    assert_eq!(
        v["hooks"]["PermissionRequest"][1]["hooks"][1]["command"],
        "echo keep"
    );
    t.run("uninstall", "claude");
    let v: Value = serde_json::from_slice(&fs::read(p).unwrap()).unwrap();
    assert_eq!(
        v["hooks"]["PermissionRequest"][1]["hooks"],
        json!([{"command":"echo keep"}])
    );
}
#[test]
fn appends_last_and_trust_uses_position() {
    let t = Temp::new();
    let p = t.file("codex");
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(
        &p,
        r#"{"hooks":{"PermissionRequest":[{"hooks":[{"command":"echo keep"}]}]}}"#,
    )
    .unwrap();
    t.run("install", "codex");
    let key = format!("{}:permission_request:1:0", p.display());
    for (fields, status) in [
        ("enabled = false\ntrusted_hash = 'x'", "disabled"),
        ("trusted_hash = 'anything'", "installed"),
        ("enabled = true", "needs_trust"),
    ] {
        fs::write(
            p.parent().unwrap().join("config.toml"),
            format!(
                "[hooks.state.{}]\n{fields}\n",
                serde_json::to_string(&key).unwrap()
            ),
        )
        .unwrap();
        assert_eq!(t.run("status", "codex")["status"], status);
    }
}
#[test]
fn refusal_does_not_write() {
    let t = Temp::new();
    let p = t.file("claude");
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    for bad in ["", "{", "[]", "{\"hooks\":42}"] {
        fs::write(&p, bad).unwrap();
        assert!(t.run("install", "claude")["error"].is_string());
        assert_eq!(fs::read_to_string(&p).unwrap(), bad);
    }
    fs::remove_file(&p).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(t.0.join("absent"), &p).unwrap();
        assert_eq!(t.run("install", "claude")["error"], "hooks_symlink");
        assert!(!t.0.join("absent").exists());
    }
}
#[test]
fn uninstall_cleans_only_agent_spool_and_is_noop_when_absent() {
    let t = Temp::new();
    let dir = t.0.join("state/herdcats-bridge/requests");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("claude-one.jsonl"), "{\"agent\":\"claude\"}\n").unwrap();
    fs::write(dir.join("codex-one.jsonl"), "{}\n").unwrap();
    assert_eq!(t.run("status", "claude")["spool_confirmed"], true);
    t.run("uninstall", "claude");
    assert!(!dir.join("claude-one.jsonl").exists());
    assert!(dir.join("codex-one.jsonl").exists());
    assert!(!t.file("claude").exists());
    t.run("uninstall", "claude");
}

#[test]
fn directory_api_preserves_modes_and_empty_groups() {
    use herdcats_bridge::hooks::{Agent, install_in_dir, status_in_dir, uninstall_in_dir};
    let t = Temp::new();
    let dir = t.0.join("direct");
    let spool = t.0.join("spool");
    fs::create_dir_all(&dir).unwrap();
    let p = dir.join("settings.json");
    fs::write(&p, "{}").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&p, fs::Permissions::from_mode(0o640)).unwrap();
    }
    assert!(
        install_in_dir(Agent::Claude, &dir, &spool)
            .to_json()
            .contains("installed")
    );
    assert!(
        status_in_dir(Agent::Claude, &dir, &spool)
            .to_json()
            .contains("installed")
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
    uninstall_in_dir(Agent::Claude, &dir, &spool);
    let v: Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
    assert!(v["hooks"].get("PermissionRequest").is_none());
}

#[cfg(unix)]
fn output_retrying_text_file_busy(command: &mut Command) -> std::io::Result<std::process::Output> {
    // Parallel fork/execs can briefly inherit fs::copy's writable descriptor on
    // Linux. Retry only ETXTBSY, including the shell's equivalent diagnostic.
    let mut attempts = 0;
    loop {
        attempts += 1;
        let result = command.output();
        let busy = match &result {
            Err(error) => error.kind() == std::io::ErrorKind::ExecutableFileBusy,
            Ok(output) => {
                output.status.code() == Some(126)
                    && String::from_utf8_lossy(&output.stderr).contains("Text file busy")
            }
        };
        if !busy || attempts == 50 {
            return result;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[cfg(unix)]
#[test]
fn executable_with_spaces_is_shell_quoted_and_backups_do_not_collide() {
    let t = Temp::new();
    let bin = t.0.join("bridge's executable");
    fs::copy(env!("CARGO_BIN_EXE_herdcats-bridge"), &bin).unwrap();
    let dir = t.0.join("claude");
    let output = output_retrying_text_file_busy(
        Command::new(&bin)
            .args(["hooks", "install", "--agent", "claude"])
            .env("CLAUDE_CONFIG_DIR", &dir)
            .env("XDG_STATE_HOME", t.0.join("state")),
    )
    .unwrap();
    assert!(output.status.success());
    let p = dir.join("settings.json");
    let v: Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
    let command = v["hooks"]["PermissionRequest"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    let executable = command.strip_suffix(" hook record --agent claude").unwrap();
    let output = output_retrying_text_file_busy(
        Command::new("sh").args(["-c", &format!("{executable} --version-json")]),
    )
    .unwrap();
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["name"],
        "herdcats-bridge"
    );
    let first = t.run("install", "claude"); // Replaces the copied binary's stale path.
    let second = t.run("uninstall", "claude");
    assert_ne!(first["backup"], second["backup"]);
    for result in [first, second] {
        assert!(dir.join(result["backup"].as_str().unwrap()).exists());
    }
}

#[test]
fn install_uninstall_round_trips_populated_file_and_leaves_absent_entries_alone() {
    for agent in ["claude", "codex"] {
        let t = Temp::new();
        let p = t.file(agent);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        let original = format!("{}\n", serde_json::to_string_pretty(&json!({
            "z": "keep", "hooks": {"SessionStart": [{"hooks": [{"command": "echo keep"}]}]}, "a": true
        })).unwrap());
        fs::write(&p, &original).unwrap();
        t.run("install", agent);
        t.run("uninstall", agent);
        assert_eq!(fs::read(&p).unwrap(), original.as_bytes());
        // An empty PermissionRequest array belonging to the user is a no-op.
        let empty = "{\"hooks\":{\"PermissionRequest\":[]}}";
        fs::write(&p, empty).unwrap();
        t.run("uninstall", agent);
        assert_eq!(fs::read_to_string(&p).unwrap(), empty);
    }
}

#[test]
fn stale_duplicates_keep_first_position_and_remove_only_empty_owned_groups() {
    for agent in ["claude", "codex"] {
        let t = Temp::new();
        let p = t.file(agent);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        let stale = |path: &str| json!({"type":"command", "command":format!("{path} hook record --agent {agent}"), "timeout":10});
        let other_agent = if agent == "claude" { "codex" } else { "claude" };
        let foreign =
            json!({"command":format!("/foreign/bridge hook record --agent {other_agent}")});
        let mut doc = json!({"hooks":{"PermissionRequest":[
            {"hooks":[{"command":"echo before"}]},
            {"matcher":"*", "hooks":[{"command":"echo first"},stale("/old/one"),stale("/old/two")]},
            {"hooks":[stale("/old/three"),{"command":"echo keep"}]},
            {"hooks":[stale("/old/four")]},
            {"hooks":[]},
            {"hooks":[foreign.clone()]}
        ]}});
        if agent == "codex" {
            doc["hooks"]["PermissionRequest"][1]
                .as_object_mut()
                .unwrap()
                .shift_remove("matcher");
        }
        fs::write(&p, serde_json::to_vec(&doc).unwrap()).unwrap();
        assert_eq!(t.run("install", agent)["status"], "installed");
        let v: Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
        let groups = v["hooks"]["PermissionRequest"].as_array().unwrap();
        assert_eq!(groups.len(), 5);
        assert_eq!(groups[0], json!({"hooks":[{"command":"echo before"}]}));
        assert_eq!(groups[1]["hooks"].as_array().unwrap().len(), 2);
        assert_eq!(groups[1]["hooks"][0]["command"], "echo first");
        assert!(
            groups[1]["hooks"][1]["command"]
                .as_str()
                .unwrap()
                .ends_with(&format!(" hook record --agent {agent}"))
        );
        assert!(
            !groups[1]["hooks"][1]["command"]
                .as_str()
                .unwrap()
                .contains("/old/")
        );
        assert_eq!(groups[2], json!({"hooks":[{"command":"echo keep"}]}));
        assert_eq!(groups[3], json!({"hooks":[]}));
        assert_eq!(groups[4], json!({"hooks":[foreign]}));
        let before = fs::read(&p).unwrap();
        t.run("install", agent);
        assert_eq!(fs::read(&p).unwrap(), before);
    }
}

impl Temp {
    fn record_command(&self, args: &[&str], enabled: bool) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_herdcats-bridge"));
        command
            .args(args)
            .env("HOME", &self.0)
            .env("CODEX_HOME", self.0.join("codex"))
            .env("XDG_STATE_HOME", self.0.join("state"))
            .env("HERDR_PANE_ID", "fixture:pane")
            .env_remove("HERDR_ENV");
        if enabled {
            command.env("HERDR_ENV", "1");
        }
        command
    }
    fn record(&self, args: &[&str], payload: &[u8], enabled: bool) {
        use std::{io::Write, process::Stdio};
        let mut child = self
            .record_command(args, enabled)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // A failed recording may close stdin early; it must still exit silently.
        let _ = child.stdin.take().unwrap().write_all(payload);
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
    }
    fn spool(&self) -> PathBuf {
        self.0.join("state/herdcats-bridge/requests")
    }
    fn records(&self, agent: &str, session: &str) -> Vec<Value> {
        fs::read_to_string(self.spool().join(format!("{agent}-{session}.jsonl")))
            .unwrap()
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect()
    }
}

#[test]
fn recorder_captures_real_fixture_payloads_and_confirms_status() {
    for (agent, fixture) in [
        (
            "claude",
            include_str!("../../compatibility/phase1/fixtures/claude-hooks.jsonl"),
        ),
        (
            "codex",
            include_str!("../../compatibility/phase1/fixtures/codex-hooks.jsonl"),
        ),
    ] {
        let t = Temp::new();
        if agent == "codex" {
            fs::create_dir_all(t.0.join("codex")).unwrap();
            fs::write(
                t.0.join("codex/config.toml"),
                "[features]\ndaemon_auto_start = false\n",
            )
            .unwrap();
        }
        for line in fixture.lines() {
            let fixture: Value = serde_json::from_str(line).unwrap();
            let payload = &fixture["payload"];
            let session = payload["session_id"].as_str().unwrap();
            t.record(
                &["hook", "record", "--agent", agent],
                &serde_json::to_vec(payload).unwrap(),
                true,
            );
            let records = t.records(agent, session);
            let record = records.last().unwrap();
            assert_eq!(record["v"], 1);
            assert!(record["t"].as_f64().unwrap() > 0.0);
            assert_eq!(record["agent"], agent);
            assert_eq!(record["event"], payload["hook_event_name"]);
            assert_eq!(record["session_id"], session);
            assert_eq!(record["pane"], "fixture:pane");
            assert_eq!(record["payload"], *payload);
        }
        assert_eq!(t.run("status", agent)["spool_confirmed"], true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(t.spool()).unwrap().permissions().mode() & 0o777,
                0o700
            );
            for file in fs::read_dir(t.spool()).unwrap() {
                assert_eq!(
                    file.unwrap().metadata().unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
        }
    }
}

#[test]
fn recorder_is_silent_on_invalid_input_arguments_and_outside_herdr() {
    let t = Temp::new();
    for payload in [
        b"garbage".as_slice(),
        b"{}",
        b"{\"session_id\":4}",
        b"{\"session_id\":\"../escape\"}",
        b"{\"session_id\":\"\"}",
        b"{\"session_id\":\"bad.name\"}",
    ] {
        t.record(&["hook", "record", "--agent", "claude"], payload, true);
    }
    for args in [
        vec!["hook"],
        vec!["hook", "bad"],
        vec!["hook", "record", "--agent", "unknown"],
        vec!["hook", "record", "--agent", "claude", "extra"],
    ] {
        t.record(&args, b"{}", true);
    }
    t.record(
        &["hook", "record", "--agent", "claude"],
        br#"{"session_id":"valid"}"#,
        false,
    );
    assert!(!t.spool().exists());
}

#[test]
fn recorder_trims_to_newest_records_by_count_and_bytes() {
    let t = Temp::new();
    fs::create_dir_all(t.0.join("codex")).unwrap();
    fs::write(
        t.0.join("codex/config.toml"),
        "[features]\ndaemon_auto_start = false\n",
    )
    .unwrap();
    for i in 0..70 {
        let payload = json!({"session_id":"count","hook_event_name":"PermissionRequest","i":i});
        t.record(
            &["hook", "record", "--agent", "claude"],
            &serde_json::to_vec(&payload).unwrap(),
            true,
        );
    }
    let records = t.records("claude", "count");
    assert_eq!(records.len(), 64);
    assert_eq!(records[0]["payload"]["i"], 6);
    assert_eq!(records[63]["payload"]["i"], 69);
    for i in 0..8 {
        let payload = json!({"session_id":"bytes","i":i,"data":"x".repeat(60*1024)});
        t.record(
            &["hook", "record", "--agent", "codex"],
            &serde_json::to_vec(&payload).unwrap(),
            true,
        );
    }
    let records = t.records("codex", "bytes");
    assert_eq!(records.len(), 4);
    assert_eq!(records[0]["payload"]["i"], 4);
    assert_eq!(records[3]["payload"]["i"], 7);
    assert!(
        fs::metadata(t.spool().join("codex-bytes.jsonl"))
            .unwrap()
            .len()
            <= 256 * 1024
    );
}

#[test]
fn recorder_marks_oversized_input_instead_of_storing_payload() {
    let t = Temp::new();
    let payload = format!(
        "{{\"session_id\":\"large\",\"hook_event_name\":\"PermissionRequest\",\"data\":\"{}\"}}",
        "x".repeat(1024 * 1024 + 1)
    );
    t.record(
        &["hook", "record", "--agent", "claude"],
        payload.as_bytes(),
        true,
    );
    let records = t.records("claude", "large");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["payload"]["truncated"], true);
    assert_eq!(records[0]["event"], "PermissionRequest");
    assert!(
        fs::metadata(t.spool().join("claude-large.jsonl"))
            .unwrap()
            .len()
            < 1024
    );
}

#[cfg(unix)]
#[test]
fn recorder_refuses_symlinks_and_filesystem_errors_silently() {
    use std::os::unix::fs::symlink;
    let t = Temp::new();
    let target = t.0.join("target");
    fs::create_dir(&target).unwrap();
    fs::create_dir_all(t.spool().parent().unwrap()).unwrap();
    symlink(&target, t.spool()).unwrap();
    let args = ["hook", "record", "--agent", "claude"];
    let input = br#"{"session_id":"safe"}"#;
    t.record(&args, input, true);
    assert_eq!(fs::read_dir(&target).unwrap().count(), 0);
    fs::remove_file(t.spool()).unwrap();
    fs::create_dir(t.spool()).unwrap();
    let file = t.spool().join("claude-safe.jsonl");
    symlink("/dev/full", &file).unwrap();
    t.record(&args, input, true);
    assert!(
        fs::symlink_metadata(&file)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    fs::remove_file(&file).unwrap();
    fs::create_dir(&file).unwrap();
    t.record(&args, input, true);
}

#[cfg(unix)]
#[test]
fn recorder_handles_unreadable_stdin_non_utf8_args_and_home_fallback() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt, process::Stdio};
    let t = Temp::new();
    fs::create_dir_all(t.0.join("codex")).unwrap();
    fs::write(
        t.0.join("codex/config.toml"),
        "[features]\ndaemon_auto_start = false\n",
    )
    .unwrap();
    let output = t
        .record_command(&["hook", "record", "--agent", "claude"], true)
        .stdin(Stdio::from(fs::File::open(&t.0).unwrap()))
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    let output = t
        .record_command(&["hook", "record", "--agent"], true)
        .arg(OsString::from_vec(vec![0xff]))
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    let mut child = t
        .record_command(&["hook", "record", "--agent", "codex"], true)
        .env_remove("XDG_STATE_HOME")
        .env_remove("HERDR_PANE_ID")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(br#"{"session_id":"fallback"}"#)
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    let record: Value = serde_json::from_str(
        &fs::read_to_string(t.0.join(".local/state/herdcats-bridge/requests/codex-fallback.jsonl"))
            .unwrap(),
    )
    .unwrap();
    assert!(record["pane"].is_null());
    assert_eq!(record["session_id"], "fallback");
}

#[test]
fn recorder_concurrent_appends_are_complete_json_lines() {
    use std::{io::Write, process::Stdio};
    let t = Temp::new();
    let mut children = vec![];
    for i in 0..16 {
        let mut child = t
            .record_command(&["hook", "record", "--agent", "claude"], true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(&json!({"session_id":"concurrent","i":i})).unwrap())
            .unwrap();
        children.push(child);
    }
    for child in children {
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
    let records = t.records("claude", "concurrent");
    assert_eq!(records.len(), 16);
    let mut ids: Vec<_> = records
        .iter()
        .map(|r| r["payload"]["i"].as_u64().unwrap())
        .collect();
    ids.sort();
    assert_eq!(ids, (0..16).collect::<Vec<_>>());
}

#[test]
fn recorder_marks_single_record_that_exceeds_spool_budget() {
    let t = Temp::new();
    fs::create_dir_all(t.0.join("codex")).unwrap();
    fs::write(
        t.0.join("codex/config.toml"),
        "[features]\ndaemon_auto_start = false\n",
    )
    .unwrap();
    let payload = json!({"session_id":"budget","data":"x".repeat(300*1024)});
    t.record(
        &["hook", "record", "--agent", "codex"],
        &serde_json::to_vec(&payload).unwrap(),
        true,
    );
    assert_eq!(
        t.records("codex", "budget")[0]["payload"],
        json!({"truncated":true})
    );
}

#[test]
fn prune_deletes_only_spool_files_strictly_older_than_one_day() {
    use std::{
        fs::FileTimes,
        time::{Duration, UNIX_EPOCH},
    };
    let t = Temp::new();
    let dir = t.spool();
    fs::create_dir_all(&dir).unwrap();
    let now = UNIX_EPOCH + Duration::from_secs(200_000);
    for (name, age) in [
        ("claude-old.jsonl", 86_401),
        ("codex-old.jsonl", 86_401),
        ("claude-boundary.jsonl", 86_400),
        ("codex-recent.jsonl", 1),
        ("notes.txt", 100_000),
        ("unknown-old.jsonl", 100_000),
        ("claude-bad.name.jsonl", 100_000),
        ("claude-.jsonl", 100_000),
        (".spool.tmp-1-0", 100_000),
    ] {
        let path = dir.join(name);
        fs::write(&path, "record").unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(now - Duration::from_secs(age)))
            .unwrap();
    }
    let future = dir.join("claude-future.jsonl");
    fs::write(&future, "future").unwrap();
    fs::File::options()
        .write(true)
        .open(&future)
        .unwrap()
        .set_times(FileTimes::new().set_modified(now + Duration::from_secs(1)))
        .unwrap();
    fs::create_dir(dir.join("claude-directory.jsonl")).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(dir.join("notes.txt"), dir.join("claude-link.jsonl")).unwrap();
    }
    herdcats_bridge::spool::prune(&dir, now).unwrap();
    assert!(!dir.join(".spool.tmp-1-0").exists());
    assert!(!dir.join("claude-old.jsonl").exists());
    assert!(!dir.join("codex-old.jsonl").exists());
    for name in [
        "claude-boundary.jsonl",
        "codex-recent.jsonl",
        "notes.txt",
        "unknown-old.jsonl",
        "claude-bad.name.jsonl",
        "claude-.jsonl",
        "claude-future.jsonl",
        "claude-directory.jsonl",
    ] {
        assert!(dir.join(name).exists(), "{name}");
    }
    #[cfg(unix)]
    {
        assert!(
            fs::symlink_metadata(dir.join("claude-link.jsonl"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
}

#[test]
fn prune_missing_directory_is_noop_and_symlink_directory_is_refused() {
    let t = Temp::new();
    let now = std::time::SystemTime::now();
    herdcats_bridge::spool::prune(&t.spool(), now).unwrap();
    assert!(!t.spool().exists());
    #[cfg(unix)]
    {
        let target = t.0.join("target");
        fs::create_dir(&target).unwrap();
        let old = target.join("claude-old.jsonl");
        fs::write(&old, "keep").unwrap();
        fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH))
            .unwrap();
        fs::create_dir_all(t.spool().parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&target, t.spool()).unwrap();
        assert!(herdcats_bridge::spool::prune(&t.spool(), now).is_err());
        assert_eq!(fs::read_to_string(old).unwrap(), "keep");
    }
}

#[test]
fn serve_prunes_at_start_and_ignores_cleanup_errors() {
    use std::process::Stdio;
    let t = Temp::new();
    fs::create_dir_all(t.spool()).unwrap();
    let old = t.spool().join("claude-old.jsonl");
    fs::write(&old, "old").unwrap();
    fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH))
        .unwrap();
    let serve = || {
        let output = Command::new(env!("CARGO_BIN_EXE_herdcats-bridge"))
            .args(["serve", "--stdio", "--socket"])
            .arg(t.0.join("unused.sock"))
            .env("HOME", &t.0)
            .env("XDG_STATE_HOME", t.0.join("state"))
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    };
    serve();
    assert!(!old.exists());
    assert!(t.spool().join(".spool.lock").is_file());
    fs::remove_dir_all(t.spool()).unwrap();
    fs::write(t.spool(), "not a directory").unwrap();
    serve();
    assert_eq!(fs::read_to_string(t.spool()).unwrap(), "not a directory");
}

#[test]
fn install_removes_stale_recorder_when_current_entry_exists() {
    for agent in ["claude", "codex"] {
        let t = Temp::new();
        t.run("install", agent);
        let p = t.file(agent);
        let mut v: Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
        let current = v["hooks"]["PermissionRequest"][0].clone();
        v["hooks"]["PermissionRequest"].as_array_mut().unwrap().push(json!({"hooks":[{"type":"command","command":format!("/old/bridge hook record --agent {agent}")}]}));
        fs::write(&p, serde_json::to_vec(&v).unwrap()).unwrap();
        t.run("install", agent);
        let v: Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
        assert_eq!(v["hooks"]["PermissionRequest"], json!([current]));
    }
}

#[test]
fn codex_recorder_requires_disabled_daemon() {
    let t = Temp::new();
    t.record(
        &["hook", "record", "--agent", "codex"],
        br#"{"session_id":"daemon"}"#,
        true,
    );
    assert!(!t.spool().exists());
}

#[test]
fn concurrent_trimming_retains_all_new_requests() {
    use std::{io::Write, process::Stdio};
    let t = Temp::new();
    for i in 0..64 {
        t.record(
            &["hook", "record", "--agent", "claude"],
            &serde_json::to_vec(&json!({"session_id":"trim","old":i})).unwrap(),
            true,
        );
    }
    let mut children = vec![];
    for i in 0..32 {
        let mut child = t
            .record_command(&["hook", "record", "--agent", "claude"], true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(&json!({"session_id":"trim","new":i})).unwrap())
            .unwrap();
        children.push(child);
    }
    for child in children {
        assert!(child.wait_with_output().unwrap().status.success());
    }
    let records = t.records("claude", "trim");
    assert_eq!(records.len(), 64);
    let mut ids: Vec<_> = records
        .iter()
        .filter_map(|r| r["payload"]["new"].as_u64())
        .collect();
    ids.sort();
    assert_eq!(ids, (0..32).collect::<Vec<_>>());
}

#[test]
fn codex_cleanup_migrates_downstream_trust_state() {
    for action in ["install", "uninstall"] {
        let t = Temp::new();
        t.run("install", "codex");
        let path = t.file("codex");
        let initial: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let recorder = initial["hooks"]["PermissionRequest"][0]["hooks"][0].clone();
        let stale = json!({"type":"command","command":"/old/bridge hook record --agent codex"});
        let doc = json!({"hooks":{"PermissionRequest":[
            {"hooks":[recorder,stale.clone(),{"type":"command","command":"echo first"}]},
            {"hooks":[stale]},
            {"hooks":[{"type":"command","command":"echo second"}]}
        ]}});
        fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
        let key = |g, h| format!("{}:permission_request:{g}:{h}", path.display());
        let config_path = path.parent().unwrap().join("config.toml");
        let config = format!(
            "# keep comment\nmodel = 'test'\n[hooks.state.{}]\nenabled = false\ntrusted_hash = 'first'\n[hooks.state.{}]\nenabled = true\ntrusted_hash = 'second'\n[hooks.state.{}]\ntrusted_hash = 'removed'\n[hooks.state.unrelated]\nenabled = false\n",
            serde_json::to_string(&key(0, 2)).unwrap(),
            serde_json::to_string(&key(2, 0)).unwrap(),
            serde_json::to_string(&key(1, 0)).unwrap()
        );
        fs::write(&config_path, &config).unwrap();
        let result = t.run(action, "codex");
        assert!(result.get("error").is_none(), "{result}");
        let text = fs::read_to_string(config_path).unwrap();
        let config: toml_edit::DocumentMut = text.parse().unwrap();
        let state = &config["hooks"]["state"];
        let first = if action == "install" { 1 } else { 0 };
        assert_eq!(
            state[&key(0, first)]["trusted_hash"].as_str(),
            Some("first")
        );
        assert_eq!(state[&key(0, first)]["enabled"].as_bool(), Some(false));
        assert_eq!(state[&key(1, 0)]["trusted_hash"].as_str(), Some("second"));
        assert_eq!(state[&key(1, 0)]["enabled"].as_bool(), Some(true));
        assert_eq!(state["unrelated"]["enabled"].as_bool(), Some(false));
        assert!(text.contains("# keep comment"));
        assert_eq!(config["model"].as_str(), Some("test"));
    }
}

#[test]
fn codex_migration_refuses_invalid_config_without_changing_hooks() {
    for action in ["install", "uninstall"] {
        let t = Temp::new();
        t.run("install", "codex");
        let path = t.file("codex");
        let mut doc: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        doc["hooks"]["PermissionRequest"].as_array_mut().unwrap().push(json!({"hooks":[{"type":"command","command":"/old/bridge hook record --agent codex"},{"type":"command","command":"echo keep"}]}));
        let original = serde_json::to_vec(&doc).unwrap();
        fs::write(&path, &original).unwrap();
        let config = path.parent().unwrap().join("config.toml");
        fs::write(&config, "[invalid").unwrap();
        assert_eq!(t.run(action, "codex")["error"], "hooks_config_invalid");
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::read_to_string(config).unwrap(), "[invalid");
    }
}

#[test]
fn concurrent_byte_trimming_retains_new_requests() {
    use std::{io::Write, process::Stdio};
    let t = Temp::new();
    let data = "x".repeat(60 * 1024);
    for i in 0..4 {
        t.record(
            &["hook", "record", "--agent", "claude"],
            &serde_json::to_vec(&json!({"session_id":"byte-race","old":i,"data":data})).unwrap(),
            true,
        );
    }
    let mut children = vec![];
    for i in 0..3 {
        let mut child = t
            .record_command(&["hook", "record", "--agent", "claude"], true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(
                &serde_json::to_vec(&json!({"session_id":"byte-race","new":i,"data":data}))
                    .unwrap(),
            )
            .unwrap();
        children.push(child);
    }
    for child in children {
        assert!(child.wait_with_output().unwrap().status.success());
    }
    let records = t.records("claude", "byte-race");
    assert_eq!(records.len(), 4);
    let mut ids: Vec<_> = records
        .iter()
        .filter_map(|r| r["payload"]["new"].as_u64())
        .collect();
    ids.sort();
    assert_eq!(ids, vec![0, 1, 2]);
}

#[test]
fn codex_migration_does_not_inherit_stale_destination_state() {
    let t = Temp::new();
    t.run("install", "codex");
    let path = t.file("codex");
    let mut doc: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    doc["hooks"]["PermissionRequest"]
        .as_array_mut()
        .unwrap()
        .push(json!({"hooks":[
            {"type":"command","command":"echo a"},
            {"type":"command","command":"echo b"},
            {"type":"command","command":"echo c"}
        ]}));
    fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
    let config_path = path.parent().unwrap().join("config.toml");
    let key = format!("{}:permission_request:0:2", path.display());
    fs::write(
        &config_path,
        format!(
            "[hooks.state.{}]\nenabled = false\ntrusted_hash = 'stale'\n",
            serde_json::to_string(&key).unwrap()
        ),
    )
    .unwrap();
    t.run("uninstall", "codex");
    let config: toml_edit::DocumentMut = fs::read_to_string(config_path).unwrap().parse().unwrap();
    assert!(
        config
            .get("hooks")
            .and_then(|h| h.get("state"))
            .and_then(|s| s.get(&key))
            .is_none()
    );
}

#[test]
fn spool_lock_storage_is_constant_across_sessions() {
    let t = Temp::new();
    for i in 0..8 {
        t.record(
            &["hook", "record", "--agent", "claude"],
            &serde_json::to_vec(&json!({"session_id":format!("session-{i}")})).unwrap(),
            true,
        );
    }
    let locks: Vec<_> = fs::read_dir(t.spool())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .filter(|name| name.to_string_lossy().ends_with(".lock"))
        .collect();
    assert_eq!(locks, vec![std::ffi::OsString::from(".spool.lock")]);
}

#[cfg(unix)]
#[test]
fn prune_waits_for_recorder_lock_and_rechecks_age() {
    use std::{
        os::fd::AsRawFd,
        sync::mpsc,
        time::{Duration, SystemTime},
    };
    let t = Temp::new();
    fs::create_dir_all(t.spool()).unwrap();
    let path = t.spool().join("claude-race.jsonl");
    fs::write(&path, "old").unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH))
        .unwrap();
    let lock = fs::File::create(t.spool().join(".spool.lock")).unwrap();
    assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) }, 0);
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let dir = t.spool();
    let thread = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        let result = herdcats_bridge::spool::prune(&dir, SystemTime::now());
        done_tx.send(result).unwrap();
    });
    started_rx.recv().unwrap();
    assert!(done_rx.recv_timeout(Duration::from_millis(100)).is_err());
    fs::write(&path, "fresh request").unwrap();
    assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_UN) }, 0);
    done_rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .unwrap();
    thread.join().unwrap();
    assert_eq!(fs::read_to_string(path).unwrap(), "fresh request");
}

#[test]
fn status_reports_mixed_and_duplicate_recorders_as_stale() {
    for agent in ["claude", "codex"] {
        for stale_first in [false, true] {
            for same_group in [false, true] {
                let t = Temp::new();
                t.run("install", agent);
                let path = t.file(agent);
                let initial: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                let current = initial["hooks"]["PermissionRequest"][0]["hooks"][0].clone();
                for extra in [
                    json!({"type":"command","command":format!("/old/bridge hook record --agent {agent}")}),
                    current.clone(),
                ] {
                    let handlers = if stale_first {
                        vec![extra, current.clone()]
                    } else {
                        vec![current.clone(), extra]
                    };
                    let groups = if same_group {
                        json!([{"hooks":handlers}])
                    } else {
                        json!([{"hooks":[handlers[0].clone()]},{"hooks":[handlers[1].clone()]}])
                    };
                    let bytes =
                        serde_json::to_vec(&json!({"hooks":{"PermissionRequest":groups}})).unwrap();
                    fs::write(&path, &bytes).unwrap();
                    assert_eq!(t.run("status", agent)["status"], "stale");
                    assert_eq!(fs::read(&path).unwrap(), bytes);
                    t.run("install", agent);
                    assert_eq!(
                        t.run("status", agent)["status"],
                        if agent == "claude" {
                            "installed"
                        } else {
                            "needs_trust"
                        }
                    );
                }
            }
        }
    }
}

#[test]
fn install_repairs_damaged_recorder_fields() {
    for agent in ["claude", "codex"] {
        for damage in [
            "matcher",
            "type",
            "timeout",
            "missing_timeout",
            "extra_field",
        ] {
            let t = Temp::new();
            t.run("install", agent);
            let path = t.file(agent);
            let expected: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            let mut damaged = expected.clone();
            let group = &mut damaged["hooks"]["PermissionRequest"][0];
            match damage {
                "matcher" => group["matcher"] = json!("Bash"),
                "type" => group["hooks"][0]["type"] = json!("prompt"),
                "timeout" => group["hooks"][0]["timeout"] = json!(0),
                "missing_timeout" => {
                    group["hooks"][0]
                        .as_object_mut()
                        .unwrap()
                        .shift_remove("timeout");
                }
                _ => group["hooks"][0]["async"] = json!(true),
            }
            let before = serde_json::to_vec(&damaged).unwrap();
            fs::write(&path, &before).unwrap();
            assert_eq!(
                t.run("status", agent)["status"],
                "stale",
                "{agent}/{damage}"
            );
            assert_eq!(fs::read(&path).unwrap(), before);
            t.run("install", agent);
            let actual: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(actual, expected, "{agent}/{damage}");
            let before = fs::read(&path).unwrap();
            t.run("install", agent);
            assert_eq!(fs::read(&path).unwrap(), before);
        }
    }
}

#[test]
fn repair_restrictive_group_preserves_unrelated_matcher() {
    for agent in ["claude", "codex"] {
        let t = Temp::new();
        t.run("install", agent);
        let path = t.file(agent);
        let mut doc: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let group = &mut doc["hooks"]["PermissionRequest"][0];
        group["matcher"] = json!("Bash");
        let foreign = json!({"type":"command","command":"echo unrelated","timeout":7});
        group["hooks"].as_array_mut().unwrap().push(foreign.clone());
        fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
        if agent == "codex" {
            let key = format!("{}:permission_request:1:0", path.display());
            fs::write(
                path.parent().unwrap().join("config.toml"),
                format!(
                    "[hooks.state.{}]\nenabled = false\ntrusted_hash = 'orphan'\n",
                    serde_json::to_string(&key).unwrap()
                ),
            )
            .unwrap();
        }
        assert_eq!(t.run("status", agent)["status"], "stale");
        t.run("install", agent);
        let actual: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(
            actual["hooks"]["PermissionRequest"][0],
            json!({"matcher":"Bash","hooks":[foreign]})
        );
        assert_eq!(
            actual["hooks"]["PermissionRequest"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            t.run("status", agent)["status"],
            if agent == "claude" {
                "installed"
            } else {
                "needs_trust"
            }
        );
    }
}

#[test]
fn codex_repair_clears_only_changed_recorder_trust() {
    for damage in ["command", "timeout", "matcher", "type", "async"] {
        for moved in [false, true] {
            if damage == "command" && moved {
                continue;
            }
            for enabled in [false, true] {
                let t = Temp::new();
                t.run("install", "codex");
                let path = t.file("codex");
                let mut doc: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                let group = &mut doc["hooks"]["PermissionRequest"][0];
                match damage {
                    "command" => {
                        group["hooks"][0]["command"] =
                            json!("/old/bridge hook record --agent codex")
                    }
                    "timeout" => group["hooks"][0]["timeout"] = json!(1),
                    "matcher" => group["matcher"] = json!("Bash"),
                    "type" => group["hooks"][0]["type"] = json!("prompt"),
                    _ => group["hooks"][0]["async"] = json!(true),
                }
                if moved {
                    doc["hooks"]["PermissionRequest"].as_array_mut().unwrap().insert(0,json!({"hooks":[{"type":"command","command":"/duplicate/bridge hook record --agent codex","timeout":10}]}));
                }
                doc["hooks"]["PermissionRequest"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"hooks":[{"type":"command","command":"echo foreign"}]}));
                fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
                let key = |g| format!("{}:permission_request:{g}:0", path.display());
                let old = usize::from(moved);
                let config_path = path.parent().unwrap().join("config.toml");
                fs::write(&config_path,format!("# preserve\n[hooks.state.{}]\nenabled = {enabled}\ntrusted_hash = 'old'\ntrusted_content_hash = 'content'\n[hooks.state.{}]\nenabled = false\ntrusted_hash = 'foreign'\n",serde_json::to_string(&key(old)).unwrap(),serde_json::to_string(&key(old+1)).unwrap())).unwrap();
                t.run("install", "codex");
                let config: toml_edit::DocumentMut =
                    fs::read_to_string(&config_path).unwrap().parse().unwrap();
                let state = &config["hooks"]["state"];
                assert!(
                    state[&key(0)].get("trusted_hash").is_none(),
                    "{damage}/{moved}/{enabled}"
                );
                assert!(state[&key(0)].get("trusted_content_hash").is_none());
                assert_eq!(state[&key(0)]["enabled"].as_bool(), Some(enabled));
                assert_eq!(state[&key(1)]["trusted_hash"].as_str(), Some("foreign"));
                assert_eq!(state[&key(1)]["enabled"].as_bool(), Some(false));
                assert_eq!(
                    t.run("status", "codex")["status"],
                    if enabled { "needs_trust" } else { "disabled" }
                );
                // Re-trusting a repaired, canonical entry survives idempotent install.
                let text = fs::read_to_string(&config_path).unwrap().replace(
                    &format!("[hooks.state.{}]", serde_json::to_string(&key(0)).unwrap()),
                    &format!(
                        "[hooks.state.{}]\ntrusted_hash = 'new'",
                        serde_json::to_string(&key(0)).unwrap()
                    ),
                );
                fs::write(&config_path, &text).unwrap();
                t.run("install", "codex");
                assert_eq!(fs::read_to_string(config_path).unwrap(), text);
            }
        }
    }
}

#[test]
fn prune_reclaims_only_regular_abandoned_trim_files() {
    let t = Temp::new();
    let dir = t.spool();
    fs::create_dir_all(&dir).unwrap();
    let temp = dir.join(".spool.tmp-42-0");
    fs::write(&temp, "left by interrupted trim").unwrap();
    fs::write(dir.join("unrelated.tmp"), "keep").unwrap();
    fs::create_dir(dir.join(".spool.tmp-42-1")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(dir.join("unrelated.tmp"), dir.join(".spool.tmp-42-2")).unwrap();
    herdcats_bridge::spool::prune(&dir, std::time::SystemTime::now()).unwrap();
    assert!(!temp.exists());
    assert_eq!(
        fs::read_to_string(dir.join("unrelated.tmp")).unwrap(),
        "keep"
    );
    assert!(dir.join(".spool.tmp-42-1").is_dir());
    #[cfg(unix)]
    assert!(
        fs::symlink_metadata(dir.join(".spool.tmp-42-2"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}
