use serde_json::{json, Value};
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
    use herdcats_bridge::hooks::{install_in_dir, status_in_dir, uninstall_in_dir, Agent};
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
    assert!(install_in_dir(Agent::Claude, &dir, &spool)
        .to_json()
        .contains("installed"));
    assert!(status_in_dir(Agent::Claude, &dir, &spool)
        .to_json()
        .contains("installed"));
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
        assert!(groups[1]["hooks"][1]["command"]
            .as_str()
            .unwrap()
            .ends_with(&format!(" hook record --agent {agent}")));
        assert!(!groups[1]["hooks"][1]["command"]
            .as_str()
            .unwrap()
            .contains("/old/"));
        assert_eq!(groups[2], json!({"hooks":[{"command":"echo keep"}]}));
        assert_eq!(groups[3], json!({"hooks":[]}));
        assert_eq!(groups[4], json!({"hooks":[foreign]}));
        let before = fs::read(&p).unwrap();
        t.run("install", agent);
        assert_eq!(fs::read(&p).unwrap(), before);
    }
}
