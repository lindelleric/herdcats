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
}
pub struct Service {
    pub client: Client,
    pub resolver: Option<Resolver>,
    epoch: String,
    sequence: u64,
    generation: u64,
    initialized: bool,
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
    fn poll_inner(&self, watch: &mut Watch) -> Result<(Identity, bool, String)> {
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
            watch.follower = Follower::default();
        }
        watch
            .follower
            .refresh_file(resolver.open(&path)?, &session)?;
        let reset = watch.identity.as_ref() != Some(&identity)
            || watch.unavailable.is_some()
            || watch.follower.generation != watch.generation;
        Ok((identity, reset, pane.agent_status.clone()))
    }
    pub fn poll(&mut self, out: &mut impl Write) -> io::Result<()> {
        let Some(mut watch) = self.watch.take() else {
            return Ok(());
        };
        match self.poll_inner(&mut watch) {
            Err(code) => {
                if watch.unavailable != Some(code) {
                    self.generation += 1;
                    watch.identity = None;
                    watch.previous.clear();
                    watch.follower = Follower::default();
                    watch.unavailable = Some(code);
                    self.event(
                        out,
                        "timeline.unavailable",
                        json!({"pane_id":watch.pane_id,"generation":self.generation,"code":code}),
                    )?;
                }
            }
            Ok((identity, reset, status)) => {
                let data = &watch.follower.transcript;
                let evicted = watch
                    .previous
                    .iter()
                    .any(|old| !data.entries.iter().any(|new| new.id == old.id));
                if reset || evicted {
                    self.generation += 1;
                    let mut body = page(&data.entries, data.entries.len());
                    body["identity"] = json!(identity);
                    body["generation"] = json!(self.generation);
                    body["agent_status"] = json!(status);
                    body["unknown_records"] = json!(data.unknown_records);
                    body["dropped_entries"] = json!(data.dropped_entries);
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
                    if !changed.is_empty() || status != watch.status {
                        // Emit bounded update batches, with one sequence per batch.
                        if changed.is_empty() {
                            self.event(out, "timeline.updated", json!({"identity":identity,"generation":self.generation,"entries":[],"agent_status":status}))?;
                        } else {
                            let mut batch = Vec::new();
                            let mut size = 0;
                            for entry in changed {
                                let encoded = serde_json::to_vec(&entry)?.len() + 1;
                                if !batch.is_empty() && size + encoded > MAX_RECORD / 2 {
                                    self.event(out, "timeline.updated", json!({"identity":identity,"generation":self.generation,"entries":batch,"agent_status":status,"dropped_entries":data.dropped_entries}))?;
                                    batch.clear();
                                    size = 0;
                                }
                                batch.push(entry);
                                size += encoded;
                            }
                            if !batch.is_empty() {
                                self.event(out, "timeline.updated", json!({"identity":identity,"generation":self.generation,"entries":batch,"agent_status":status,"dropped_entries":data.dropped_entries}))?;
                            }
                        }
                    }
                }
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
                &json!({"id":id,"result":{"protocol":1,"bridge_version":env!("CARGO_PKG_VERSION"),"epoch":self.epoch,"capabilities":{"claude_timeline":self.resolver.is_some(),"codex_timeline":false,"git":false,"prompts":false,"approvals":false,"questions":false,"routed_hosts":false}}}),
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
                    follower: Follower::default(),
                    generation: 0,
                    previous: Vec::new(),
                    unavailable: None,
                    status: String::new(),
                });
                Self::write(out, &json!({"id":id,"result":{"watching":pane}}))?;
                self.poll(out)
            }
            "unwatch" => {
                self.watch = None;
                Self::write(out, &json!({"id":id,"result":{"watching":null}}))
            }
            "history" => {
                self.poll(out)?;
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
                    return Self::error(out, id, "invalid_cursor");
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
