use herdcats_bridge::{follower::read_tail, herdr::Client, hooks::Agent, spool::read_session};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, Seek, SeekFrom, Write},
    os::unix::{fs::symlink, net::UnixListener},
    path::PathBuf,
    thread,
};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "approval-input-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn spool_reader_is_bounded_read_only_and_skips_bad_lines() {
    let dir = Temp::new();
    let missing = dir.0.join("missing");
    assert!(read_session(&missing, Agent::Claude, "session").is_empty());
    assert!(!missing.exists());
    assert!(read_session(&dir.0, Agent::Claude, "session").is_empty());
    assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 0);
    let path = dir.0.join("claude-session.jsonl");
    fs::write(&path, b"{\"ok\":1}\nbad\n{\"ok\":2}\n{\"partial\":3}").unwrap();
    assert_eq!(
        read_session(&dir.0, Agent::Claude, "session"),
        vec![json!({"ok":1}), json!({"ok":2})]
    );
    fs::write(&path, vec![b' '; 256 * 1024 + 1]).unwrap();
    assert!(read_session(&dir.0, Agent::Claude, "session").is_empty());
    fs::remove_file(&path).unwrap();
    let target = dir.0.join("target");
    fs::write(&target, b"{}\n").unwrap();
    symlink(&target, &path).unwrap();
    assert!(read_session(&dir.0, Agent::Claude, "session").is_empty());
    let link = dir.0.join("link");
    symlink(&dir.0, &link).unwrap();
    assert!(read_session(&link, Agent::Claude, "session").is_empty());
    assert!(read_session(&dir.0, Agent::Claude, "../target").is_empty());
}

#[test]
fn spool_reader_keeps_at_most_the_record_cap() {
    let dir = Temp::new();
    let text: String = (0..65).map(|n| format!("{n}\n")).collect();
    fs::write(dir.0.join("codex-session.jsonl"), text).unwrap();
    let records = read_session(&dir.0, Agent::Codex, "session");
    assert_eq!(records.len(), 64);
    assert_eq!(records[0], json!(1));
    assert_eq!(records[63], json!(64));
}

#[test]
fn tail_reader_uses_open_file_and_only_complete_bounded_lines() {
    let dir = Temp::new();
    let path = dir.0.join("transcript");
    fs::write(&path, b"{\"first\":1}\nbad\n{\"last\":2}\n{\"partial\":3}").unwrap();
    let mut file = fs::File::open(&path).unwrap();
    file.seek(SeekFrom::End(0)).unwrap();
    assert_eq!(
        read_tail(file).unwrap(),
        vec![json!({"first":1}), json!({"last":2})]
    );
    let mut bytes = vec![b'x'; 4 * 1024 * 1024];
    bytes.extend_from_slice(b"\n{\"last\":2}\n{}");
    fs::write(&path, bytes).unwrap();
    assert_eq!(
        read_tail(fs::File::open(&path).unwrap()).unwrap(),
        vec![json!({"last":2})]
    );
    assert!(read_tail(fs::File::open(&dir.0).unwrap()).is_err());
}

#[test]
fn tail_drops_first_partial_line_even_when_it_is_valid_json() {
    let dir = Temp::new();
    let path = dir.0.join("transcript");
    let mut bytes = b"x{}\n".to_vec();
    bytes.resize(4 * 1024 * 1024 - 3, b'x');
    bytes.extend_from_slice(b"\n{}\n");
    // The 4 MiB window begins immediately after x, at a valid JSON fragment.
    fs::write(&path, bytes).unwrap();
    let file = fs::File::open(&path).unwrap();
    fs::remove_file(&path).unwrap();
    symlink(dir.0.join("missing"), &path).unwrap();
    assert_eq!(read_tail(file).unwrap(), vec![json!({})]);
}

fn exchange(method: &str, params: Value, response: Value) -> Client {
    let dir = Temp::new();
    let socket = dir.0.join("socket");
    let listener = UnixListener::bind(&socket).unwrap();
    let expected = json!({"id":method,"method":method,"params":params});
    thread::spawn(move || {
        let _dir = dir;
        let (mut stream, _) = listener.accept().unwrap();
        let mut line = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap(), expected);
        writeln!(stream, "{response}").unwrap();
    });
    Client { socket }
}

#[test]
fn herdr_visible_and_keys_exact_requests() {
    let client = exchange(
        "pane.read",
        json!({"pane_id":"p","source":"visible"}),
        json!({"id":"pane.read","result":{"read":{"text":"dialog\n"}}}),
    );
    assert_eq!(client.read_visible("p").unwrap(), "dialog\n");
    let client = exchange(
        "pane.send_keys",
        json!({"pane_id":"p","keys":["1","esc"]}),
        json!({"id":"pane.send_keys","result":{}}),
    );
    client.send_keys("p", &["1", "esc"]).unwrap();
}

#[test]
fn herdr_errors_have_stable_codes() {
    for (response, code) in [
        (
            json!({"id":"pane.read","error":{"code":"pane_not_found"}}),
            "pane_unavailable",
        ),
        (
            json!({"id":"pane.read","error":{"code":"other"}}),
            "herdr_unavailable",
        ),
        (
            json!({"id":"wrong","result":{"read":{"text":"x"}}}),
            "invalid_herdr_response",
        ),
        (
            json!({"id":"pane.read","result":{"read":{"text":1}}}),
            "invalid_herdr_response",
        ),
    ] {
        let client = exchange(
            "pane.read",
            json!({"pane_id":"p","source":"visible"}),
            response,
        );
        assert_eq!(client.read_visible("p"), Err(code));
    }
    let dir = Temp::new();
    assert_eq!(
        Client {
            socket: dir.0.join("missing")
        }
        .send_keys("p", &["1"]),
        Err("herdr_unavailable")
    );
}
