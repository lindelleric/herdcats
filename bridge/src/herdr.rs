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
        let frame = self.request(
            "snapshot", "session.snapshot", json!({}), "herdr_snapshot_too_large",
        )?;
        if frame["result"]["type"] != "session_snapshot" {
            return Err("invalid_herdr_response");
        }
        let snapshot: Snapshot = serde_json::from_value(frame["result"]["snapshot"].clone())
            .map_err(|_| "invalid_herdr_response")?;
        if snapshot.protocol != 22 {
            return Err("unsupported_herdr_protocol");
        }
        Ok(snapshot)
    }
    pub fn read_visible(&self, pane_id: &str) -> Result<String> {
        let frame = self.request(
            "pane.read", "pane.read", json!({"pane_id":pane_id,"source":"visible"}),
            "invalid_herdr_response",
        )?;
        frame["result"]["read"]["text"].as_str()
            .map(str::to_owned).ok_or("invalid_herdr_response")
    }

    pub fn send_keys(&self, pane_id: &str, keys: &[&str]) -> Result<()> {
        self.request(
            "pane.send_keys", "pane.send_keys", json!({"pane_id":pane_id,"keys":keys}),
            "invalid_herdr_response",
        )?;
        Ok(())
    }

    fn request(&self, id: &str, method: &str, params: Value, too_large: &'static str) -> Result<Value> {
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
            json!({"id":id,"method":method,"params":params})
        )
        .map_err(|_| "herdr_unavailable")?;
        let mut bytes = Vec::new();
        BufReader::new(stream)
            .take((MAX_RECORD + 1) as u64)
            .read_until(b'\n', &mut bytes)
            .map_err(|_| "herdr_unavailable")?;
        if bytes.len() > MAX_RECORD {
            return Err(too_large);
        }
        if bytes.last() != Some(&b'\n') {
            return Err("invalid_herdr_response");
        }
        let frame: Value = serde_json::from_slice(&bytes).map_err(|_| "invalid_herdr_response")?;
        if frame["id"] != id {
            return Err("invalid_herdr_response");
        }
        if let Some(error) = frame.get("error") {
            return Err(match error["code"].as_str() {
                Some("pane_not_found") => "pane_unavailable",
                Some(_) => "herdr_unavailable",
                None => "invalid_herdr_response",
            });
        }
        if !frame.get("result").is_some_and(Value::is_object) {
            return Err("invalid_herdr_response");
        }
        Ok(frame)
    }
}
