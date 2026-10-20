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
#[test]
fn executable_with_spaces_is_shell_quoted_and_backups_do_not_collide() {
    let t = Temp::new();
    let bin = t.0.join("bridge's executable");
    fs::copy(env!("CARGO_BIN_EXE_herdcats-bridge"), &bin).unwrap();
    let dir = t.0.join("claude");
    let output = Command::new(&bin)
        .args(["hooks", "install", "--agent", "claude"])
        .env("CLAUDE_CONFIG_DIR", &dir)
        .env("XDG_STATE_HOME", t.0.join("state"))
        .output()
        .unwrap();
    assert!(output.status.success());
    let p = dir.join("settings.json");
    let v: Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
    let command = v["hooks"]["PermissionRequest"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    let executable = command.strip_suffix(" hook record --agent claude").unwrap();
    let output = Command::new("sh")
        .args(["-c", &format!("{executable} --version-json")])
        .output()
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
        fs::write(&p, serde_json::to_vec(&json!({"hooks":{"PermissionRequest":[
            {"hooks":[{"command":"echo before"}]},
            {"matcher":"*", "hooks":[{"command":"echo first"},stale("/old/one"),stale("/old/two")]},
            {"hooks":[stale("/old/three"),{"command":"echo keep"}]},
            {"hooks":[stale("/old/four")]},
            {"hooks":[]},
            {"hooks":[foreign.clone()]}
        ]}})).unwrap()).unwrap();
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
