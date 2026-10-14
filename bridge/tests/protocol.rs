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
                        stream
                            .set_read_timeout(Some(Duration::from_millis(500)))
                            .unwrap();
                        let mut line = String::new();
                        if BufReader::new(stream.try_clone().unwrap())
                            .read_line(&mut line)
                            .is_err()
                        {
                            continue;
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
    fn transcript(&self, session: &str, text: &str) {
        fs::write(
            self.root
                .join("projects/one")
                .join(format!("{session}.jsonl")),
            text,
        )
        .unwrap();
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
            .args(["--poll-ms", "50"])
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
        assert_eq!(x["result"]["capabilities"]["approvals"], false);
        assert_eq!(x["result"]["capabilities"]["codex_timeline"], false);
        for capability in ["git", "prompts", "approvals", "questions", "routed_hosts"] {
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
    server.state.lock().unwrap()["panes"][0]["agent"] = json!("codex");
    let unsupported = peer.next();
    assert_eq!(unsupported["data"]["code"], "unsupported_agent");
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
