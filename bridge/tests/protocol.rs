use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixListener,
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    time::Duration,
};
const SESSION: &str = "16246b9d-4c3d-54fc-b6d3-2a765d418491";
const OTHER: &str = "11111111-1111-4111-8111-111111111111";
struct Server {
    root: PathBuf,
    socket: PathBuf,
    state: Arc<Mutex<Value>>,
    stop: Arc<AtomicBool>,
    task: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "hcb-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(root.join("projects/one")).unwrap();
        let socket = root.join("h.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let state = Arc::new(Mutex::new(
            json!({"protocol":22,"version":"0.9.3","panes":[{"pane_id":"w1:p1","terminal_id":"terminal-one","workspace_id":"w1","tab_id":"w1:t1","agent":"claude","agent_status":"idle","revision":1,"agent_session":{"agent":"claude","source":"herdr:claude","kind":"id","value":SESSION}}]}),
        ));
        let stop = Arc::new(AtomicBool::new(false));
        let copy = state.clone();
        let done = stop.clone();
        let task = thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // A peer can exit with a snapshot connection queued.
                        // macOS may reject timeout setup on that closed socket.
                        if stream.set_nonblocking(false).is_err()
                            || stream
                                .set_read_timeout(Some(Duration::from_millis(500)))
                                .is_err()
                        {
                            continue;
                        }
                        let mut line = String::new();
                        match BufReader::new(stream.try_clone().unwrap()).read_line(&mut line) {
                            Ok(0) | Err(_) => continue,
                            Ok(_) => {}
                        }
                        let request: Value = serde_json::from_str(&line).unwrap();
                        assert_eq!(request["method"], "session.snapshot");
                        let frame = json!({"id":request["id"],"result":{"type":"session_snapshot","snapshot":copy.lock().unwrap().clone()}});
                        let _ = writeln!(stream, "{frame}");
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            root,
            socket,
            state,
            stop,
            task: Some(task),
        }
    }
    // Never truncate in place: fs::write empties the file before writing, and
    // the bridge's watcher can snapshot that empty file. Growth is appended
    // like a live transcript; anything else is replaced via write-then-rename.
    fn transcript(&self, session: &str, text: &str) {
        let dir = self.root.join("projects/one");
        let path = dir.join(format!("{session}.jsonl"));
        if let Ok(current) = fs::read_to_string(&path) {
            if let Some(suffix) = text.strip_prefix(current.as_str()) {
                fs::OpenOptions::new()
                    .append(true)
                    .open(&path)
                    .unwrap()
                    .write_all(suffix.as_bytes())
                    .unwrap();
                return;
            }
        }
        let tmp = dir.join(format!(".{session}.jsonl.tmp"));
        fs::write(&tmp, text).unwrap();
        fs::rename(&tmp, &path).unwrap();
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.task.take().unwrap().join().unwrap();
        let _ = fs::remove_dir_all(&self.root);
    }
}
struct Peer {
    child: Child,
    input: Option<ChildStdin>,
    frames: Receiver<Value>,
}
impl Peer {
    fn new(server: &Server) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_herdcats-bridge"))
            .args(["serve", "--stdio", "--socket"])
            .arg(&server.socket)
            .arg("--claude-root")
            .arg(server.root.join("projects"))
            .arg("--codex-root")
            .arg(server.root.join("codex"))
            .args(["--poll-ms", "50"])
            // Daemon status is read from CODEX_HOME, not from --codex-root;
            // point it at an empty temp home so the host's real Codex daemon
            // never leaks into assertions.
            .env("CODEX_HOME", server.root.join("codex-home"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let input = child.stdin.take();
        let output = child.stdout.take().unwrap();
        let (tx, frames) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else { break };
                let value = serde_json::from_str(&line).unwrap();
                if tx.send(value).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            input,
            frames,
        }
    }
    fn send(&mut self, value: Value) {
        writeln!(self.input.as_mut().unwrap(), "{value}").unwrap();
    }
    fn next(&self) -> Value {
        self.frames
            .recv_timeout(Duration::from_secs(5))
            .expect("bridge emitted no protocol frame")
    }
    fn initialize(&mut self) {
        self.send(json!({"id":"init","method":"initialize","params":{"protocol":1}}));
        let x = self.next();
        assert_eq!(x["id"], "init");
        assert_eq!(x["result"]["capabilities"]["claude_timeline"], true);
        assert_eq!(x["result"]["capabilities"]["codex_daemon_stop"], true);
        assert_eq!(x["result"]["capabilities"]["codex_daemon_config"], true);
        assert_eq!(x["result"]["capabilities"]["approvals"], true);
        assert_eq!(x["result"]["capabilities"]["questions"], true);
        for capability in ["git", "prompts", "routed_hosts"] {
            assert_eq!(x["result"]["capabilities"][capability], false);
        }
    }
    fn watch(&mut self) -> Value {
        self.send(json!({"id":"watch","method":"watch","params":{"pane_id":"w1:p1"}}));
        assert_eq!(self.next()["id"], "watch");
        self.next()
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn row(session: &str, id: &str, text: &str) -> String {
    format!(
        "{}\n",
        json!({"type":"user","uuid":id,"sessionId":session,"message":{"role":"user","content":text}})
    )
}

#[test]
fn snapshot_server_survives_a_client_disconnecting_before_its_request() {
    let server = Server::new();
    drop(std::os::unix::net::UnixStream::connect(&server.socket).unwrap());
    let snapshot = herdcats_bridge::herdr::Client {
        socket: server.socket.clone(),
    }
    .snapshot();
    drop(server);
    assert_eq!(snapshot.unwrap().panes[0].pane_id, "w1:p1");
}

#[test]
fn handshake_watch_append_and_eof_work_over_real_stdio_and_socket() {
    let server = Server::new();
    server.transcript(SESSION, &row(SESSION, "first", "hello"));
    let mut peer = Peer::new(&server);
    peer.initialize();
    let first = peer.watch();
    assert_eq!(first["event"], "timeline.snapshot");
    assert_eq!(first["data"]["identity"]["native_session_id"], SESSION);
    assert_eq!(first["data"]["entries"][0]["text"], "hello");
    server.transcript(
        SESSION,
        &(row(SESSION, "first", "hello") + &row(SESSION, "second", "猫")),
    );
    let update = peer.next();
    assert_eq!(update["event"], "timeline.updated");
    assert_eq!(update["data"]["entries"].as_array().unwrap().len(), 1);
    assert_eq!(update["data"]["entries"][0]["text"], "猫");
    assert!(update["seq"].as_u64().unwrap() > first["seq"].as_u64().unwrap());
    peer.input.take();
    assert!(peer.child.wait().unwrap().success());
}
#[test]
fn occupant_change_resets_identity_and_clears_previous_entries() {
    let server = Server::new();
    server.transcript(SESSION, &row(SESSION, "first", "old secret"));
    server.transcript(OTHER, &row(OTHER, "other", "new session"));
    let mut peer = Peer::new(&server);
    peer.initialize();
    let first = peer.watch();
    server.state.lock().unwrap()["panes"][0]["agent_session"]["value"] = json!(OTHER);
    let reset = peer.next();
    assert_eq!(reset["event"], "timeline.snapshot");
    assert_eq!(reset["data"]["identity"]["native_session_id"], OTHER);
    assert_eq!(reset["data"]["entries"][0]["text"], "new session");
    assert_ne!(reset["data"]["generation"], first["data"]["generation"]);
}
#[test]
fn missing_reference_and_unsupported_agent_never_guess_a_transcript() {
    let server = Server::new();
    server.transcript(SESSION, &row(SESSION, "first", "secret"));
    server.state.lock().unwrap()["panes"][0]
        .as_object_mut()
        .unwrap()
        .remove("agent_session");
    let mut peer = Peer::new(&server);
    peer.initialize();
    let missing = peer.watch();
    assert_eq!(missing["event"], "timeline.unavailable");
    assert_eq!(missing["data"]["code"], "session_reference_unavailable");
    assert!(!missing.to_string().contains("secret"));
    assert!(missing["data"].get("codex_daemon").is_none());
    server.state.lock().unwrap()["panes"][0]["agent"] = json!("codex");
    let unsupported = peer.next();
    assert_eq!(unsupported["data"]["code"], "unsupported_agent");
}
#[test]
fn timeline_unavailable_carries_codex_daemon_for_codex_and_not_for_claude() {
    let server = Server::new();
    fs::create_dir_all(server.root.join("codex")).unwrap();
    server.state.lock().unwrap()["panes"][0]
        .as_object_mut()
        .unwrap()
        .remove("agent_session");
    let mut peer = Peer::new(&server);
    peer.initialize();

    // 1. Claude pane with missing reference: codex_daemon NOT present
    let claude_missing = peer.watch();
    assert_eq!(claude_missing["event"], "timeline.unavailable");
    assert_eq!(
        claude_missing["data"]["code"],
        "session_reference_unavailable"
    );
    assert!(claude_missing["data"].get("codex_daemon").is_none());

    // 2. Switch to Codex pane with missing reference: codex_daemon IS present.
    // Unwatch before switching, or the live watch can emit an unavailable
    // frame for the switch ahead of the unwatch reply.
    peer.send(json!({"id":"unwatch","method":"unwatch","params":{}}));
    let unwatched = peer.next();
    assert_eq!(unwatched["id"], "unwatch");
    assert_eq!(unwatched["result"]["watching"], Value::Null);
    server.state.lock().unwrap()["panes"][0]["agent"] = json!("codex");
    peer.send(json!({"id":"watch-codex","method":"watch","params":{"pane_id":"w1:p1"}}));
    assert_eq!(peer.next()["id"], "watch-codex");
    let codex_missing = peer.next();
    assert_eq!(codex_missing["event"], "timeline.unavailable");
    assert_eq!(
        codex_missing["data"]["code"],
        "session_reference_unavailable"
    );
    assert_eq!(codex_missing["data"]["codex_daemon"], "not_running");
}

#[test]
fn unavailable_agent_flip_with_same_code_reemits_and_resets_generation() {
    let server = Server::new();
    fs::create_dir_all(server.root.join("codex")).unwrap();
    server.state.lock().unwrap()["panes"][0]
        .as_object_mut()
        .unwrap()
        .remove("agent_session");
    server.state.lock().unwrap()["panes"][0]["agent"] = json!("codex");
    let mut peer = Peer::new(&server);
    peer.initialize();

    // 1. Codex occupant with a missing session reference: codex_daemon present.
    let codex_missing = peer.watch();
    assert_eq!(codex_missing["event"], "timeline.unavailable");
    assert_eq!(
        codex_missing["data"]["code"],
        "session_reference_unavailable"
    );
    assert_eq!(codex_missing["data"]["codex_daemon"], "not_running");

    // 2. Unchanged occupant + code + daemon status: nothing new is emitted.
    assert!(
        peer.frames.recv_timeout(Duration::from_millis(300)).is_err(),
        "unchanged unavailable state must not re-emit"
    );

    // 3. Occupant flips codex -> claude with the SAME code: a fresh
    // timeline.unavailable WITHOUT the Codex-only codex_daemon field, and a
    // new generation for the new occupant.
    server.state.lock().unwrap()["panes"][0]["agent"] = json!("claude");
    let claude_missing = peer.next();
    assert_eq!(claude_missing["event"], "timeline.unavailable");
    assert_eq!(
        claude_missing["data"]["code"],
        "session_reference_unavailable"
    );
    assert!(claude_missing["data"].get("codex_daemon").is_none());
    assert_ne!(
        claude_missing["data"]["generation"],
        codex_missing["data"]["generation"]
    );

    // 4. Occupant flips back claude -> codex: a fresh event WITH codex_daemon
    // and another generation bump.
    server.state.lock().unwrap()["panes"][0]["agent"] = json!("codex");
    let codex_again = peer.next();
    assert_eq!(codex_again["event"], "timeline.unavailable");
    assert_eq!(
        codex_again["data"]["code"],
        "session_reference_unavailable"
    );
    assert_eq!(codex_again["data"]["codex_daemon"], "not_running");
    assert_ne!(
        codex_again["data"]["generation"],
        claude_missing["data"]["generation"]
    );

    // 5. Unchanged again: silence.
    assert!(
        peer.frames.recv_timeout(Duration::from_millis(300)).is_err(),
        "unchanged unavailable state must not re-emit"
    );
}
#[test]
fn history_pages_are_bounded_and_stale_cursors_are_rejected() {
    let server = Server::new();
    let text = (0..107)
        .map(|i| row(SESSION, &format!("m{i}"), &format!("text{i}")))
        .collect::<String>();
    server.transcript(SESSION, &text);
    let mut peer = Peer::new(&server);
    peer.initialize();
    let snap = peer.watch();
    let entries = snap["data"]["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 100);
    assert_eq!(entries[0]["text"], "text7");
    assert_eq!(snap["data"]["has_more"], true);
    peer.send(json!({"id":"page","method":"history","params":{"generation":snap["data"]["generation"],"before":"message:m7"}}));
    let page = peer.next();
    assert_eq!(page["result"]["entries"].as_array().unwrap().len(), 7);
    assert_eq!(page["result"]["entries"][0]["text"], "text0");
    server.transcript(SESSION, &row(SESSION, "replacement", "fresh"));
    let reset = peer.next();
    assert_eq!(reset["event"], "timeline.snapshot");
    peer.send(json!({"id":"stale","method":"history","params":{"generation":snap["data"]["generation"],"before":"message:m7"}}));
    assert_eq!(peer.next()["error"]["code"], "stale_cursor");
}
#[test]
fn protocol_mismatch_requires_reinitialization_and_actions_are_not_methods() {
    let server = Server::new();
    let mut peer = Peer::new(&server);
    peer.send(json!({"id":"bad","method":"initialize","params":{"protocol":99}}));
    assert_eq!(peer.next()["error"]["code"], "protocol_mismatch");
    peer.send(json!({"id":"watch","method":"watch","params":{"pane_id":"w1:p1"}}));
    assert_eq!(peer.next()["error"]["code"], "not_initialized");
    peer.initialize();
    peer.send(json!({"id":"action","method":"approve","params":{"command":"secret"}}));
    assert_eq!(peer.next()["error"]["code"], "unknown_method");
}
#[test]
fn changed_file_failure_hides_old_data_and_recovery_resnapshots() {
    let server = Server::new();
    server.transcript(SESSION, &row(SESSION, "first", "old secret"));
    let mut peer = Peer::new(&server);
    peer.initialize();
    peer.watch();
    server.transcript(SESSION, "{bad}\n");
    let error = peer.next();
    assert_eq!(error["event"], "timeline.unavailable");
    assert_eq!(error["data"]["code"], "invalid_transcript");
    assert!(!error.to_string().contains("old secret"));
    server.transcript(SESSION, &row(SESSION, "first", "recovered"));
    let recovery = peer.next();
    assert_eq!(recovery["event"], "timeline.snapshot");
    assert_eq!(recovery["data"]["entries"][0]["text"], "recovered");
}
#[test]
fn escaped_tool_output_updates_fit_frame_budget_without_closing_stream() {
    let server = Server::new();
    server.transcript(SESSION, "");
    let mut peer = Peer::new(&server);
    peer.initialize();
    peer.watch();
    let mut text = String::new();
    for i in 0..20 {
        text += &format!(
            "{}\n",
            json!({"type":"assistant","uuid":format!("a{i}"),"sessionId":SESSION,"message":{"content":[{"type":"tool_use","id":format!("t{i}"),"name":"Read","input":{"text":"\u{0}".repeat(1400)}}]}})
        );
        text += &format!(
            "{}\n",
            json!({"type":"user","uuid":format!("r{i}"),"sessionId":SESSION,"message":{"content":[{"type":"tool_result","tool_use_id":format!("t{i}"),"content":"\u{0}".repeat(8192)}]}})
        );
    }
    server.transcript(SESSION, &text);
    let mut received = 0;
    while received < 20 {
        let frame = peer.next();
        assert!(frame.to_string().len() <= 1024 * 1024);
        received += frame["data"]["entries"].as_array().unwrap().len();
    }
    peer.send(json!({"id":"stop","method":"unwatch","params":{}}));
    assert_eq!(peer.next()["id"], "stop");
}
#[test]
fn bounded_retention_updates_keep_generation_and_validate_retained_cursors() {
    let server = Server::new();
    let mut text = (0..1000)
        .map(|i| row(SESSION, &format!("m{i}"), "text"))
        .collect::<String>();
    server.transcript(SESSION, &text);
    let mut peer = Peer::new(&server);
    peer.initialize();
    let before = peer.watch();
    text += &row(SESSION, "m1000", "latest");
    server.transcript(SESSION, &text);
    let after = peer.next();
    assert_eq!(after["event"], "timeline.updated");
    assert_eq!(after["data"]["dropped_entries"], 1);
    assert_eq!(after["data"]["generation"], before["data"]["generation"]);
    assert_eq!(after["data"]["evicted_ids"], json!(["message:m0"]));
    peer.send(json!({"id":"retained","method":"history","params":{"generation":before["data"]["generation"],"before":"message:m900"}}));
    assert_eq!(
        peer.next()["result"]["entries"].as_array().unwrap().len(),
        100
    );
    peer.send(json!({"id":"evicted","method":"history","params":{"generation":before["data"]["generation"],"before":"message:m0"}}));
    assert_eq!(peer.next()["error"]["code"], "stale_cursor");
    text += &row(SESSION, "m1001", "latest again");
    server.transcript(SESSION, &text);
    let next = peer.next();
    assert_eq!(next["event"], "timeline.updated");
    assert_eq!(next["data"]["generation"], before["data"]["generation"]);
    assert_eq!(next["data"]["evicted_ids"], json!(["message:m1"]));
}
#[test]
fn requests_do_not_starve_occupant_refresh() {
    let server = Server::new();
    server.transcript(SESSION, &row(SESSION, "old", "old"));
    server.transcript(OTHER, &row(OTHER, "new", "new"));
    let mut peer = Peer::new(&server);
    peer.initialize();
    peer.watch();
    server.state.lock().unwrap()["panes"][0]["agent_session"]["value"] = json!(OTHER);
    let start = std::time::Instant::now();
    let mut observed = false;
    while start.elapsed() < Duration::from_millis(350) {
        peer.send(json!({"id":"traffic","method":"unknown"}));
        for frame in peer.frames.try_iter() {
            if frame["data"]["identity"]["native_session_id"] == OTHER {
                observed = true;
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert!(observed, "refresh starved while requests continued");
}
#[test]
fn history_rechecks_identity_before_returning_cached_content() {
    use herdcats_bridge::service::Service;
    let server = Server::new();
    server.transcript(SESSION, &row(SESSION, "old", "old"));
    server.transcript(OTHER, &row(OTHER, "new", "new"));
    let mut service = Service::new(server.socket.clone(), server.root.join("projects"));
    let mut output = Vec::new();
    service
        .request(
            br#"{"id":"i","method":"initialize","params":{"protocol":1}}"#,
            &mut output,
        )
        .unwrap();
    output.clear();
    service
        .request(
            br#"{"id":"w","method":"watch","params":{"pane_id":"w1:p1"}}"#,
            &mut output,
        )
        .unwrap();
    let frames: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    server.state.lock().unwrap()["panes"][0]["agent_session"]["value"] = json!(OTHER);
    let mut output = Vec::new();
    service.request(json!({"id":"h","method":"history","params":{"generation":frames[1]["data"]["generation"],"before":"message:old"}}).to_string().as_bytes(),&mut output).unwrap();
    let frames: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(frames.last().unwrap()["error"]["code"], "stale_cursor");
}
#[test]
fn oversized_agent_status_is_rejected_before_event_serialization() {
    let server = Server::new();
    server.transcript(SESSION, &row(SESSION, "a", "text"));
    server.state.lock().unwrap()["panes"][0]["agent_status"] = json!("x".repeat(300));
    let mut peer = Peer::new(&server);
    peer.initialize();
    let frame = peer.watch();
    assert_eq!(frame["data"]["code"], "invalid_herdr_response");
}

#[test]
fn content_reads_are_lossless_generation_bound_and_revalidated_against_live_occupant() {
    let server = Server::new();
    let block =
        json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":"AAAA"}});
    server.transcript(
        SESSION,
        &format!(
            "{}\n",
            json!({"type":"user","uuid":"image","sessionId":SESSION,"message":{"content":[block]}})
        ),
    );
    let mut peer = Peer::new(&server);
    peer.send(
        json!({"id":"init","method":"initialize","params":{"protocol":1,"record_content":true}}),
    );
    assert_eq!(
        peer.next()["result"]["capabilities"]["record_content"],
        true
    );
    let snap = peer.watch();
    let reference = snap["data"]["entries"][0]["content_ref"].clone();
    let generation = snap["data"]["generation"].clone();
    peer.send(json!({"id":"content","method":"content","params":{"generation":generation,"content_ref":reference,"offset":0}}));
    let result = peer.next();
    assert_eq!(result["id"], "content");
    let hex = result["result"]["hex"].as_str().unwrap();
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), block);
    peer.send(json!({"id":"forged","method":"content","params":{"generation":generation,"content_ref":"source:0:1","offset":0}}));
    assert_eq!(peer.next()["error"]["code"], "content_unavailable");
    peer.send(json!({"id":"stale","method":"content","params":{"generation":9999,"content_ref":reference,"offset":0}}));
    assert_eq!(peer.next()["error"]["code"], "stale_cursor");
    server.state.lock().unwrap()["panes"][0]["agent_session"]["value"] = json!(OTHER);
    server.transcript(OTHER, &row(OTHER, "other", "unrelated"));
    peer.send(json!({"id":"old","method":"content","params":{"generation":generation,"content_ref":reference,"offset":0}}));
    loop {
        let frame = peer.next();
        if frame["id"] == "old" {
            assert_eq!(frame["error"]["code"], "stale_cursor");
            break;
        }
    }
}

#[test]
fn daemon_status_resolves_from_configured_codex_home_not_symlink_target() {
    assert_daemon_status_uses_configured_home(true);
}

#[test]
fn daemon_status_keeps_codex_home_when_transcript_root_is_unrelated() {
    assert_daemon_status_uses_configured_home(false);
}

fn assert_daemon_status_uses_configured_home(symlink_sessions: bool) {
    use herdcats_bridge::codex_config::{
        ProcessStatus, check_process, current_boot_id, process_start_time,
    };
    use herdcats_bridge::service::Service;
    use std::os::unix::{fs::PermissionsExt, process::CommandExt};

    let server = Server::new();
    // Codex occupant with no session reference: the pane reports unavailable
    // and the event carries the cached codex_daemon status.
    {
        let mut state = server.state.lock().unwrap();
        let pane = &mut state["panes"][0];
        pane.as_object_mut().unwrap().remove("agent_session");
        pane["agent"] = json!("codex");
    }

    // The transcript root may be symlinked or mounted independently of the
    // configured home; daemon discovery must always use the home.
    static N: AtomicU64 = AtomicU64::new(0);
    let base = std::env::temp_dir().join(format!(
        "hcb-symlink-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let home = base.join("codex-home");
    let target = base.join("other-volume");
    let packages_bin = home.join("packages").join("app-server-daemon").join("bin");
    fs::create_dir_all(&packages_bin).unwrap();
    fs::create_dir_all(target.join("sessions-real")).unwrap();
    let transcript_root = if symlink_sessions {
        std::os::unix::fs::symlink(target.join("sessions-real"), home.join("sessions")).unwrap();
        home.join("sessions")
    } else {
        target.join("sessions-real")
    };
    fs::create_dir_all(target.join("app-server-daemon")).unwrap();
    fs::write(
        target.join("app-server-daemon/daemon.pid"),
        r#"{"pid":2147483647}"#,
    )
    .unwrap();

    // A live, verified managed daemon recorded under the CONFIGURED home.
    let script = packages_bin.join("daemon.sh");
    fs::write(&script, "#!/bin/sh\nwhile true; do sleep 0.05; done\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    // Guard the detached dummy's entire process group even on assertion failure.
    struct OrphanGuard {
        outer: Child,
        pgid: libc::pid_t,
        base: PathBuf,
    }
    impl Drop for OrphanGuard {
        fn drop(&mut self) {
            unsafe {
                libc::kill(-self.pgid, libc::SIGKILL);
            }
            let _ = self.outer.wait();
            let _ = fs::remove_dir_all(&self.base);
        }
    }
    let record = base.join("daemon-record");
    let outer = Command::new("/bin/sh")
        .arg("-c")
        .arg(format!(
            "/bin/sh '{}' & echo $! > '{}'",
            script.display(),
            record.display()
        ))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .unwrap();
    let mut dummy = OrphanGuard {
        pgid: outer.id() as libc::pid_t,
        outer,
        base: base.clone(),
    };
    dummy.outer.wait().unwrap();
    let pid: libc::pid_t = fs::read_to_string(&record).unwrap().trim().parse().unwrap();
    let (sec, usec) = process_start_time(pid).expect("could not read dummy start time");
    let pid_file = home.join("app-server-daemon").join("daemon.pid");
    fs::create_dir_all(pid_file.parent().unwrap()).unwrap();
    let mut identity = json!({"startSeconds": sec});
    if let Some(u) = usec {
        identity["startMicroseconds"] = json!(u);
    }
    if let Some(b) = current_boot_id() {
        identity["bootId"] = json!(b);
    }
    fs::write(
        &pid_file,
        json!({"pid": pid, "processIdentity": identity}).to_string(),
    )
    .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while check_process(&pid_file, &home) != ProcessStatus::Running {
        if std::time::Instant::now() >= deadline {
            panic!("dummy never classified running");
        }
        thread::sleep(Duration::from_millis(25));
    }

    // The resolver canonicalizes home/sessions to the symlink target; the
    // daemon lookup must still use the configured home.
    let mut service = Service::new(server.socket.clone(), server.root.join("projects"))
        .with_codex_home(home.clone())
        .with_codex_root(transcript_root);
    let mut output = Vec::new();
    service
        .request(
            br#"{"id":"i","method":"initialize","params":{"protocol":1}}"#,
            &mut output,
        )
        .unwrap();
    output.clear();
    service
        .request(
            br#"{"id":"w","method":"watch","params":{"pane_id":"w1:p1"}}"#,
            &mut output,
        )
        .unwrap();
    let frames: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    drop(dummy);
    let unavailable = frames
        .iter()
        .find(|f| f["event"] == "timeline.unavailable")
        .expect("no timeline.unavailable emitted");
    assert_eq!(unavailable["data"]["code"], "session_reference_unavailable");
    assert_eq!(
        unavailable["data"]["codex_daemon"], "running",
        "daemon status must come from the configured home, not the symlink target"
    );
}

#[test]
fn protocol_documents_approval_capability_request_and_event_fields() {
    let protocol: Value = serde_json::from_str(include_str!("../protocol.json")).unwrap();
    assert_eq!(protocol["protocol"], 1);
    assert_eq!(protocol["bridge_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(protocol["capabilities"]["approvals"], true);
    let initialize = protocol["requests"].as_array().unwrap().iter()
        .find(|request| request["method"] == "initialize").unwrap();
    assert!(initialize["params"]["approvals"].as_str().unwrap().contains("false by default"));
    assert!(protocol["approval_policy"]["capability"].as_str().unwrap().contains("initialize params.approvals"));
    assert_eq!(protocol["capabilities"]["prompts"], false);
    assert_eq!(protocol["capabilities"]["questions"], true);
    assert!(initialize["params"]["questions"].as_str().unwrap().contains("false by default"));
    let respond = protocol["requests"].as_array().unwrap().iter()
        .find(|request| request["method"] == "interaction.respond").unwrap();
    for field in ["request_id", "option", "label_lines"] {
        assert!(respond["params"].get(field).is_some(), "missing response param: {field}");
    }
    assert_eq!(respond["result_fields"], json!(["request_id", "sent", "confirmed"]));
    assert_eq!(protocol["events"]["interaction.pending"]["fields"], json!([
        "identity", "generation", "kind", "request_id", "agent", "tool_name", "command",
        "description", "justification", "environment", "title", "options", "selected"
    ]));
    assert_eq!(protocol["events"]["interaction.resolved"]["fields"], json!([
        "identity", "generation", "request_id"
    ]));
    for field in ["number", "label_lines", "key"] {
        assert!(protocol["events"]["interaction.pending"]["option_fields"].get(field).is_some());
    }
    for error in ["stale", "mismatch", "unsupported_capability"] {
        assert!(protocol["errors"].as_array().unwrap().contains(&json!(error)));
        assert!(respond["errors"].get(error).is_some());
    }
}

#[test]
fn protocol_documents_model_control_capability_requests_and_event() {
    let protocol: Value = serde_json::from_str(include_str!("../protocol.json")).unwrap();
    assert_eq!(protocol["capabilities"]["model_control"], true);
    let initialize = protocol["requests"].as_array().unwrap().iter()
        .find(|request| request["method"] == "initialize").unwrap();
    assert!(initialize["params"]["model_control"].as_str().unwrap().contains("false by default"));
    let status = protocol["requests"].as_array().unwrap().iter()
        .find(|request| request["method"] == "model.status").unwrap();
    assert_eq!(status["result_fields"], json!([
        "identity", "generation", "agent", "model", "effort", "models", "efforts"
    ]));
    let set = protocol["requests"].as_array().unwrap().iter()
        .find(|request| request["method"] == "model.set").unwrap();
    for field in ["model", "effort", "expect"] {
        assert!(set["params"].get(field).is_some(), "missing model.set param: {field}");
    }
    assert_eq!(set["result_fields"], json!([
        "identity", "generation", "applied", "confirmed", "error"
    ]));
    for error in ["unsupported_capability", "agent_busy", "stale", "picker_mismatch", "confirm_timeout"] {
        assert!(set["errors"].get(error).is_some(), "missing model.set error: {error}");
    }
    assert!(set["validation"].as_array().unwrap().iter().any(|rule| {
        rule.as_str().unwrap().contains("session-only")
    }), "model.set must document the Codex session-only key");
    assert_eq!(protocol["events"]["model.updated"]["fields"], json!([
        "identity", "generation", "model", "effort"
    ]));
    for error in ["agent_busy", "confirm_timeout", "invalid_effort", "invalid_request", "picker_mismatch"] {
        assert!(protocol["errors"].as_array().unwrap().contains(&json!(error)));
    }
}
