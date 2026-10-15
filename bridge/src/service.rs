use crate::{
    Result,
    claude::Entry,
    follower::{Follower, MAX_RECORD},
    herdr::Client,
    resolver::Resolver,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, PartialEq, Serialize)]
struct Identity {
    pane_id: String,
    terminal_id: String,
    agent: String,
    native_session_id: String,
}
struct Watch {
    pane_id: String,
    identity: Option<Identity>,
    follower: Follower,
    generation: u64,
    previous: Vec<Entry>,
    unavailable: Option<&'static str>,
    status: String,
    unknown_records: usize,
    dropped_entries: usize,
    source_changing_count: u32,
    delayed: bool,
}
pub struct Service {
    pub client: Client,
    pub resolver: Option<Resolver>,
    epoch: String,
    sequence: u64,
    generation: u64,
    initialized: bool,
    include_content: bool,
    watch: Option<Watch>,
}
fn page(entries: &[Entry], before: usize) -> Value {
    let mut start = before.saturating_sub(100);
    // Leave room for envelope and identity; JSON escaping can expand source text.
    while start < before
        && serde_json::to_vec(&entries[start..before]).map_or(true, |v| v.len() > MAX_RECORD / 2)
    {
        start += 1;
    }
    json!({"entries":entries[start..before], "has_more":start>0,
        "before":entries.get(start).filter(|_|start<before).map(|e|e.id.as_str())})
}
impl Service {
    pub fn new(socket: PathBuf, root: PathBuf) -> Self {
        let epoch = format!(
            "{}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            std::process::id()
        );
        Self {
            client: Client { socket },
            resolver: Resolver::new(&root).ok(),
            epoch,
            sequence: 0,
            generation: 0,
            initialized: false,
            include_content: false,
            watch: None,
        }
    }
    fn write(out: &mut impl Write, frame: &Value) -> io::Result<()> {
        let bytes = serde_json::to_vec(frame)?;
        if bytes.len() > MAX_RECORD {
            return Err(io::Error::other("output_frame_too_large"));
        }
        out.write_all(&bytes)?;
        out.write_all(b"\n")?;
        out.flush()
    }
    fn error(out: &mut impl Write, id: Value, code: &str) -> io::Result<()> {
        Self::write(out, &json!({"id":id,"error":{"code":code}}))
    }
    fn event(&mut self, out: &mut impl Write, event: &str, data: Value) -> io::Result<()> {
        self.sequence += 1;
        Self::write(
            out,
            &json!({"event":event,"epoch":self.epoch,"seq":self.sequence,"data":data}),
        )
    }
    fn poll_inner(
        &self,
        watch: &mut Watch,
        refresh: &mut impl FnMut(&mut Follower, std::fs::File, &str) -> Result<bool>,
    ) -> Result<(Identity, bool, String)> {
        let snapshot = self.client.snapshot()?;
        let pane = snapshot
            .panes
            .iter()
            .find(|p| p.pane_id == watch.pane_id)
            .ok_or("pane_unavailable")?;
        if pane.agent.as_deref() != Some("claude") {
            return Err("unsupported_agent");
        }
        let reference = pane
            .agent_session
            .as_ref()
            .ok_or("session_reference_unavailable")?;
        if reference.agent != "claude" || reference.source != "herdr:claude" {
            return Err("unsupported_session_source");
        }
        let resolver = self
            .resolver
            .as_ref()
            .ok_or("transcript_root_unavailable")?;
        let (session, path) = resolver.resolve(&reference.kind, &reference.value)?;
        if pane.terminal_id.is_empty()
            || pane.terminal_id.len() > 256
            || pane.agent_status.len() > 256
        {
            return Err("invalid_herdr_response");
        }
        let identity = Identity {
            pane_id: pane.pane_id.clone(),
            terminal_id: pane.terminal_id.clone(),
            agent: "claude".into(),
            native_session_id: session.clone(),
        };
        if watch.identity.as_ref() != Some(&identity) {
            watch.follower = if self.include_content {
                Follower::with_content()
            } else {
                Follower::default()
            };
            // A transient read of a new occupant cannot authorize serving the
            // previous occupant's cached transcript.
            watch.identity = None;
        }
        refresh(&mut watch.follower, resolver.open(&path)?, &session)?;
        let reset = watch.identity.as_ref() != Some(&identity)
            || watch.unavailable.is_some()
            || watch.follower.generation != watch.generation;
        Ok((identity, reset, pane.agent_status.clone()))
    }
    pub fn poll(&mut self, out: &mut impl Write) -> io::Result<()> {
        self.poll_with_refresh(out, |follower, file, session| {
            follower.refresh_file(file, session)
        })
    }
    fn poll_with_refresh(
        &mut self,
        out: &mut impl Write,
        mut refresh: impl FnMut(&mut Follower, std::fs::File, &str) -> Result<bool>,
    ) -> io::Result<()> {
        let Some(mut watch) = self.watch.take() else {
            return Ok(());
        };
        match self.poll_inner(&mut watch, &mut refresh) {
            Err("source_changing") if watch.identity.is_some() && watch.unavailable.is_none() => {
                watch.source_changing_count += 1;
                if watch.source_changing_count >= 10 && !watch.delayed {
                    watch.delayed = true;
                    let identity = watch.identity.clone().unwrap();
                    let status = watch.status.clone();
                    self.event(
                        out,
                        "timeline.updated",
                        json!({
                            "identity": identity,
                            "generation": self.generation,
                            "entries": [],
                            "evicted_ids": [],
                            "agent_status": status,
                            "unknown_records": watch.unknown_records,
                            "dropped_entries": watch.dropped_entries,
                            "delayed": true
                        }),
                    )?;
                }
            }
            Err(code) => {
                watch.source_changing_count = 0;
                watch.delayed = false;
                if watch.unavailable != Some(code) {
                    self.generation += 1;
                    watch.identity = None;
                    watch.previous.clear();
                    watch.follower = if self.include_content {
                        Follower::with_content()
                    } else {
                        Follower::default()
                    };
                    watch.unavailable = Some(code);
                    self.event(
                        out,
                        "timeline.unavailable",
                        json!({"pane_id":watch.pane_id,"generation":self.generation,"code":code}),
                    )?;
                }
            }
            Ok((identity, reset, status)) => {
                let was_delayed = watch.delayed;
                watch.source_changing_count = 0;
                watch.delayed = false;
                let data = &watch.follower.transcript;
                let evicted: Vec<_> = watch
                    .previous
                    .iter()
                    .filter(|old| !data.entries.iter().any(|new| new.id == old.id))
                    .map(|old| old.id.clone())
                    .collect();
                if reset {
                    self.generation += 1;
                    let mut body = page(&data.entries, data.entries.len());
                    body["identity"] = json!(identity);
                    body["generation"] = json!(self.generation);
                    body["agent_status"] = json!(status);
                    body["unknown_records"] = json!(data.unknown_records);
                    body["dropped_entries"] = json!(data.dropped_entries);
                    body["delayed"] = json!(false);
                    self.event(out, "timeline.snapshot", body)?;
                } else {
                    let changed: Vec<_> = data
                        .entries
                        .iter()
                        .filter(|entry| {
                            watch.previous.iter().find(|old| old.id == entry.id) != Some(*entry)
                        })
                        .cloned()
                        .collect();
                    if !changed.is_empty()
                        || !evicted.is_empty()
                        || status != watch.status
                        || data.unknown_records != watch.unknown_records
                        || data.dropped_entries != watch.dropped_entries
                        || was_delayed
                    {
                        // Each batch reserves half the frame for the envelope.
                        // Evictions precede inserts, with <=1000 IDs in total.
                        let mut evictions = Vec::new();
                        let mut size = 0;
                        for id in evicted {
                            let encoded = serde_json::to_vec(&id)?.len() + 1;
                            if !evictions.is_empty() && size + encoded > MAX_RECORD / 2 {
                                self.event(out, "timeline.updated", json!({"identity":identity,"generation":self.generation,"entries":[],"evicted_ids":evictions,"agent_status":status,"unknown_records":data.unknown_records,"dropped_entries":data.dropped_entries,"delayed":false}))?;
                                evictions.clear();
                                size = 0;
                            }
                            evictions.push(id);
                            size += encoded;
                        }
                        let mut batch = Vec::new();
                        for entry in changed {
                            let encoded = serde_json::to_vec(&entry)?.len() + 1;
                            if (!batch.is_empty() || !evictions.is_empty())
                                && size + encoded > MAX_RECORD / 2
                            {
                                self.event(out, "timeline.updated", json!({"identity":identity,"generation":self.generation,"entries":batch,"evicted_ids":evictions,"agent_status":status,"unknown_records":data.unknown_records,"dropped_entries":data.dropped_entries,"delayed":false}))?;
                                batch.clear();
                                evictions.clear();
                                size = 0;
                            }
                            batch.push(entry);
                            size += encoded;
                        }
                        self.event(out, "timeline.updated", json!({"identity":identity,"generation":self.generation,"entries":batch,"evicted_ids":evictions,"agent_status":status,"unknown_records":data.unknown_records,"dropped_entries":data.dropped_entries,"delayed":false}))?;
                    }
                }
                watch.unknown_records = data.unknown_records;
                watch.dropped_entries = data.dropped_entries;
                watch.previous = data.entries.clone();
                watch.identity = Some(identity);
                watch.status = status;
                watch.unavailable = None;
                watch.generation = watch.follower.generation;
            }
        }
        self.watch = Some(watch);
        Ok(())
    }
    pub fn request(&mut self, bytes: &[u8], out: &mut impl Write) -> io::Result<()> {
        self.request_with_refresh(bytes, out, |follower, file, session| {
            follower.refresh_file(file, session)
        })
    }
    fn request_with_refresh(
        &mut self,
        bytes: &[u8],
        out: &mut impl Write,
        refresh: impl FnMut(&mut Follower, std::fs::File, &str) -> Result<bool>,
    ) -> io::Result<()> {
        let request: Value = match serde_json::from_slice(bytes) {
            Ok(value) => value,
            Err(_) => return Self::error(out, Value::Null, "invalid_json"),
        };
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        if !id.as_str().is_some_and(|s| !s.is_empty() && s.len() <= 128) {
            return Self::error(out, Value::Null, "invalid_request_id");
        }
        let params = &request["params"];
        let method = request["method"].as_str().unwrap_or("");
        if method == "initialize" {
            self.initialized = false;
            self.include_content = params["record_content"] == true;
            self.watch = None;
            if params["protocol"] != 1 {
                return Self::error(out, id, "protocol_mismatch");
            }
            if let Err(code) = self.client.snapshot() {
                return Self::error(out, id, code);
            }
            self.initialized = true;
            return Self::write(
                out,
                &json!({"id":id,"result":{"protocol":1,"bridge_version":env!("CARGO_PKG_VERSION"),"epoch":self.epoch,"capabilities":{"claude_timeline":self.resolver.is_some(),"record_content":true,"codex_timeline":false,"git":false,"prompts":false,"approvals":false,"questions":false,"routed_hosts":false}}}),
            );
        }
        if !self.initialized {
            return Self::error(out, id, "not_initialized");
        }
        match method {
            "watch" => {
                let Some(pane) = params["pane_id"]
                    .as_str()
                    .filter(|s| !s.is_empty() && s.len() <= 256)
                else {
                    return Self::error(out, id, "invalid_pane_id");
                };
                self.watch = Some(Watch {
                    pane_id: pane.into(),
                    identity: None,
                    follower: if self.include_content {
                        Follower::with_content()
                    } else {
                        Follower::default()
                    },
                    generation: 0,
                    previous: Vec::new(),
                    unavailable: None,
                    status: String::new(),
                    unknown_records: 0,
                    dropped_entries: 0,
                    source_changing_count: 0,
                    delayed: false,
                });
                Self::write(out, &json!({"id":id,"result":{"watching":pane}}))?;
                self.poll(out)
            }
            "unwatch" => {
                self.watch = None;
                Self::write(out, &json!({"id":id,"result":{"watching":null}}))
            }
            "content" => {
                if !self.include_content {
                    return Self::error(out, id, "unsupported_capability");
                }
                self.poll_with_refresh(out, refresh)?;
                let Some(watch) = &self.watch else {
                    return Self::error(out, id, "not_watching");
                };
                if watch.unavailable.is_some() || watch.identity.is_none() || watch.delayed {
                    return Self::error(out, id, "timeline_unavailable");
                }
                if params["generation"].as_u64() != Some(self.generation) {
                    return Self::error(out, id, "stale_cursor");
                }
                let Some(reference) = params["content_ref"].as_str().filter(|s| s.len() <= 128)
                else {
                    return Self::error(out, id, "content_unavailable");
                };
                let Some(offset) = params["offset"]
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                else {
                    return Self::error(out, id, "invalid_content_offset");
                };
                match watch.follower.read_content(reference, offset) {
                    Ok((bytes, total)) => {
                        let digits = b"0123456789abcdef";
                        let mut hex = String::with_capacity(bytes.len() * 2);
                        for byte in &bytes {
                            hex.push(digits[(byte >> 4) as usize] as char);
                            hex.push(digits[(byte & 15) as usize] as char);
                        }
                        Self::write(
                            out,
                            &json!({"id":id,"result":{"identity":watch.identity,"generation":self.generation,
                            "content_ref":reference,"offset":offset,"next_offset":offset + bytes.len(),"total":total,"hex":hex}}),
                        )
                    }
                    Err(code) => Self::error(out, id, code),
                }
            }
            "history" => {
                self.poll_with_refresh(out, refresh)?;
                let Some(watch) = &self.watch else {
                    return Self::error(out, id, "not_watching");
                };
                if watch.unavailable.is_some() || watch.identity.is_none() {
                    return Self::error(out, id, "timeline_unavailable");
                }
                if params["generation"].as_u64() != Some(self.generation) {
                    return Self::error(out, id, "stale_cursor");
                }
                let entries = &watch.follower.transcript.entries;
                let Some(before) = params["before"]
                    .as_str()
                    .and_then(|id| entries.iter().position(|e| e.id == id))
                else {
                    return Self::error(out, id, "stale_cursor");
                };
                let mut result = page(entries, before);
                result["identity"] = json!(watch.identity);
                result["generation"] = json!(self.generation);
                result["dropped_entries"] = json!(watch.follower.transcript.dropped_entries);
                Self::write(out, &json!({"id":id,"result":result}))
            }
            _ => Self::error(out, id, "unknown_method"),
        }
    }
}
pub fn serve(mut service: Service, interval: Duration) -> io::Result<()> {
    let (sender, receiver) = mpsc::sync_channel::<Result<Vec<u8>>>(16);
    thread::spawn(move || {
        let mut input = BufReader::new(io::stdin());
        loop {
            let mut bytes = Vec::new();
            match (&mut input)
                .take((MAX_RECORD + 1) as u64)
                .read_until(b'\n', &mut bytes)
            {
                Ok(0) => break,
                Ok(_) if bytes.len() > MAX_RECORD => {
                    let _ = sender.send(Err("input_frame_too_large"));
                    break;
                }
                Ok(_) if bytes.last() != Some(&b'\n') => {
                    let _ = sender.send(Err("incomplete_input_frame"));
                    break;
                }
                Ok(_) => {
                    if sender.send(Ok(bytes)).is_err() {
                        break;
                    }
                }
                Err(_) => {
                    let _ = sender.send(Err("input_unavailable"));
                    break;
                }
            }
        }
    });
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut deadline = Instant::now() + interval;
    loop {
        match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Ok(bytes)) => service.request(&bytes, &mut out)?,
            Ok(Err(code)) => {
                Service::error(&mut out, Value::Null, code)?;
                return Err(io::Error::other(code));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
        if Instant::now() >= deadline {
            service.poll(&mut out)?;
            deadline = Instant::now() + interval;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::net::UnixListener};
    const SESSION: &str = "16246b9d-4c3d-54fc-b6d3-2a765d418491";
    const OTHER: &str = "11111111-1111-4111-8111-111111111111";

    // Return snapshots synchronously for each explicit poll. The injected
    // refresh boundary makes source_changing independent of timing/file races.
    fn fixture() -> (Service, PathBuf, std::sync::Arc<std::sync::Mutex<String>>) {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "service-test-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        fs::create_dir_all(root.join("projects/one")).unwrap();
        for session in [SESSION, OTHER] {
            let text = (0..107).map(|i| format!("{}\n", json!({"type":"user","uuid":format!("m{i}"),"sessionId":session,"message":{"role":"user","content":format!("text{i}")}}))).collect::<String>();
            fs::write(root.join(format!("projects/one/{session}.jsonl")), text).unwrap();
        }
        let socket = root.join("h.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let session = std::sync::Arc::new(std::sync::Mutex::new(SESSION.to_owned()));
        let state = session.clone();
        // Cleanup wakes this blocking listener with an empty request.
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut line = String::new();
                if BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap()
                    == 0
                {
                    break;
                }
                let request: Value = serde_json::from_str(&line).unwrap();
                let snapshot = json!({"protocol":22,"version":"0.9.3","panes":[{"pane_id":"w1:p1","terminal_id":"terminal-one","workspace_id":"w1","tab_id":"w1:t1","agent":"claude","agent_status":"idle","revision":1,"agent_session":{"agent":"claude","source":"herdr:claude","kind":"id","value":state.lock().unwrap().clone()}}]});
                writeln!(stream, "{}", json!({"id":request["id"],"result":{"type":"session_snapshot","snapshot":snapshot}})).unwrap();
            }
        });
        let mut service = Service::new(socket, root.join("projects"));
        service.initialized = true;
        service
            .request(
                br#"{"id":"w","method":"watch","params":{"pane_id":"w1:p1"}}"#,
                &mut Vec::new(),
            )
            .unwrap();
        (service, root, session)
    }
    fn cleanup(service: &Service, root: PathBuf) {
        // Wake and terminate the fixture thread without a valid request.
        drop(std::os::unix::net::UnixStream::connect(&service.client.socket).unwrap());
        fs::remove_dir_all(root).unwrap();
    }
    fn frames(out: Vec<u8>) -> Vec<Value> {
        String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    #[test]
    fn changing_same_source_keeps_verified_history_and_generation() {
        let (mut service, root, _) = fixture();
        let generation = service.generation;
        let entries = service.watch.as_ref().unwrap().previous.clone();
        let mut output = Vec::new();
        service
            .poll_with_refresh(&mut output, |_, _, _| Err("source_changing"))
            .unwrap();
        assert!(
            output.is_empty(),
            "a retry must not clear the client timeline"
        );
        assert_eq!(service.generation, generation);
        assert_eq!(service.watch.as_ref().unwrap().previous, entries);
        service.request_with_refresh(json!({"id":"h","method":"history","params":{"generation":generation,"before":"message:m7"}}).to_string().as_bytes(), &mut output, |_, _, _| Err("source_changing")).unwrap();
        let replies = frames(output);
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0]["result"]["entries"].as_array().unwrap().len(), 7);
        assert_eq!(replies[0]["result"]["generation"], generation);
        cleanup(&service, root);
    }
    #[test]
    fn changing_new_identity_clears_previous_occupant_and_recovers_with_snapshot() {
        let (mut service, root, identity) = fixture();
        let generation = service.generation;
        *identity.lock().unwrap() = OTHER.to_owned();
        let mut output = Vec::new();
        service
            .poll_with_refresh(&mut output, |_, _, _| Err("source_changing"))
            .unwrap();
        let replies = frames(output);
        assert_eq!(replies[0]["event"], "timeline.unavailable");
        assert_eq!(replies[0]["data"]["code"], "source_changing");
        assert!(service.watch.as_ref().unwrap().identity.is_none());
        assert!(service.watch.as_ref().unwrap().previous.is_empty());
        assert!(
            service
                .watch
                .as_ref()
                .unwrap()
                .follower
                .transcript
                .entries
                .is_empty()
        );
        let mut output = Vec::new();
        service.request(json!({"id":"h","method":"history","params":{"generation":generation,"before":"message:m7"}}).to_string().as_bytes(), &mut output).unwrap();
        let replies = frames(output);
        assert_eq!(replies[0]["event"], "timeline.snapshot");
        assert_eq!(replies[0]["data"]["identity"]["native_session_id"], OTHER);
        assert_eq!(replies[1]["error"]["code"], "stale_cursor");
        cleanup(&service, root);
    }
    #[test]
    fn counter_only_changes_emit_an_update() {
        let (mut service, root, _) = fixture();
        let generation = service.generation;
        let mut output = Vec::new();
        service
            .poll_with_refresh(&mut output, |follower, _, _| {
                follower.transcript.unknown_records += 1;
                Ok(true)
            })
            .unwrap();
        let replies = frames(output);
        assert_eq!(replies[0]["event"], "timeline.updated");
        assert_eq!(replies[0]["data"]["entries"], json!([]));
        assert_eq!(replies[0]["data"]["unknown_records"], 1);
        assert_eq!(replies[0]["data"]["generation"], generation);
        cleanup(&service, root);
    }
    #[test]
    fn repeated_source_changing_signals_delay_while_keeping_history_and_recovers() {
        let (mut service, root, identity) = fixture();
        let generation = service.generation;
        let entries = service.watch.as_ref().unwrap().previous.clone();
        assert!(!entries.is_empty());

        for _ in 0..9 {
            let mut output = Vec::new();
            service
                .poll_with_refresh(&mut output, |_, _, _| Err("source_changing"))
                .unwrap();
            assert!(output.is_empty());
            assert_eq!(service.generation, generation);
            assert_eq!(service.watch.as_ref().unwrap().previous, entries);
        }

        let mut output = Vec::new();
        service
            .poll_with_refresh(&mut output, |_, _, _| Err("source_changing"))
            .unwrap();
        let replies = frames(output);
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0]["event"], "timeline.updated");
        assert_eq!(replies[0]["data"]["delayed"], true);
        assert_eq!(replies[0]["data"]["generation"], generation);
        assert_eq!(replies[0]["data"]["entries"], json!([]));
        assert_eq!(service.watch.as_ref().unwrap().previous, entries);

        let mut output = Vec::new();
        service
            .request_with_refresh(
                json!({"id":"h","method":"history","params":{"generation":generation,"before":"message:m7"}}).to_string().as_bytes(),
                &mut output,
                |_, _, _| Err("source_changing"),
            )
            .unwrap();
        let replies = frames(output);
        assert_eq!(replies[0]["result"]["entries"].as_array().unwrap().len(), 7);
        assert_eq!(replies[0]["result"]["generation"], generation);

        let mut output = Vec::new();
        service
            .poll_with_refresh(&mut output, |_, _, _| Ok(false))
            .unwrap();
        let replies = frames(output);
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0]["event"], "timeline.updated");
        assert_eq!(replies[0]["data"]["delayed"], false);
        assert_eq!(service.generation, generation);

        *identity.lock().unwrap() = OTHER.to_owned();
        let mut output = Vec::new();
        service
            .poll_with_refresh(&mut output, |_, _, _| Err("source_changing"))
            .unwrap();
        let replies = frames(output);
        assert_eq!(replies[0]["event"], "timeline.unavailable");
        assert_eq!(replies[0]["data"]["code"], "source_changing");
        assert!(service.watch.as_ref().unwrap().identity.is_none());
        assert!(service.watch.as_ref().unwrap().previous.is_empty());

        cleanup(&service, root);
    }
}
