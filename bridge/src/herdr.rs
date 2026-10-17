use crate::{Result, follower::MAX_RECORD};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    time::Duration,
};

#[derive(Clone, Deserialize)]
pub struct SessionRef {
    pub agent: String,
    pub source: String,
    pub kind: String,
    pub value: String,
}
#[derive(Clone, Deserialize)]
pub struct Pane {
    pub pane_id: String,
    pub terminal_id: String,
    pub agent: Option<String>,
    pub agent_status: String,
    pub agent_session: Option<SessionRef>,
    #[serde(default)]
    pub foreground_cwd: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
}
#[derive(Deserialize)]
pub struct Snapshot {
    pub protocol: u32,
    pub panes: Vec<Pane>,
}
pub struct Client {
    pub socket: PathBuf,
}
impl Client {
    pub fn snapshot(&self) -> Result<Snapshot> {
        let mut stream = UnixStream::connect(&self.socket).map_err(|_| "herdr_unavailable")?;
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|_| "herdr_unavailable")?;
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(|_| "herdr_unavailable")?;
        writeln!(
            stream,
            "{}",
            json!({"id":"snapshot","method":"session.snapshot","params":{}})
        )
        .map_err(|_| "herdr_unavailable")?;
        let mut bytes = Vec::new();
        BufReader::new(stream)
            .take((MAX_RECORD + 1) as u64)
            .read_until(b'\n', &mut bytes)
            .map_err(|_| "herdr_unavailable")?;
        if bytes.len() > MAX_RECORD {
            return Err("herdr_snapshot_too_large");
        }
        if bytes.last() != Some(&b'\n') {
            return Err("invalid_herdr_response");
        }
        let frame: Value = serde_json::from_slice(&bytes).map_err(|_| "invalid_herdr_response")?;
        if frame["id"] != "snapshot" || frame["result"]["type"] != "session_snapshot" {
            return Err("invalid_herdr_response");
        }
        let snapshot: Snapshot = serde_json::from_value(frame["result"]["snapshot"].clone())
            .map_err(|_| "invalid_herdr_response")?;
        if snapshot.protocol != 22 {
            return Err("unsupported_herdr_protocol");
        }
        Ok(snapshot)
    }
}
