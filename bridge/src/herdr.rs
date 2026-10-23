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
        self.read_visible_with_timeout(pane_id, Duration::from_secs(2))
    }
    pub fn read_visible_with_timeout(&self, pane_id: &str, timeout: Duration) -> Result<String> {
        let frame = self.request_with_timeout(
            "pane.read", "pane.read", json!({"pane_id":pane_id,"source":"visible"}),
            "invalid_herdr_response", timeout,
        )?;
        frame["result"]["read"]["text"].as_str()
            .map(str::to_owned).ok_or("invalid_herdr_response")
    }
    /// The visible screen with ANSI styling kept, to tell faint placeholders
    /// from typed text.
    pub fn read_visible_ansi(&self, pane_id: &str) -> Result<String> {
        let frame = self.request(
            "pane.read", "pane.read",
            json!({"pane_id":pane_id,"source":"visible","format":"ansi"}),
            "invalid_herdr_response",
        )?;
        frame["result"]["read"]["text"].as_str()
            .map(str::to_owned).ok_or("invalid_herdr_response")
    }
    pub fn send_text(&self, pane_id: &str, text: &str) -> Result<()> {
        self.request("pane.send_text", "pane.send_text", json!({"pane_id":pane_id,"text":text}),
            "invalid_herdr_response")?;
        Ok(())
    }

    pub fn send_keys(&self, pane_id: &str, keys: &[&str]) -> Result<()> {
        self.request(
            "pane.send_keys", "pane.send_keys", json!({"pane_id":pane_id,"keys":keys}),
            "invalid_herdr_response",
        )?;
        Ok(())
    }

    /// Submits one prompt to an idle agent through Herdr's agent-aware prompt
    /// path — the same submission route the iOS app uses for composed
    /// messages. Slash commands typed this way land in the composer intact.
    pub fn agent_prompt(&self, pane_id: &str, text: &str) -> Result<()> {
        self.request(
            "agent.prompt", "agent.prompt", json!({"target":pane_id,"text":text}),
            "herdr_prompt_too_large",
        )?;
        Ok(())
    }

    fn request(&self, id: &str, method: &str, params: Value, too_large: &'static str) -> Result<Value> {
        self.request_with_timeout(id, method, params, too_large, Duration::from_secs(2))
    }
    fn request_with_timeout(&self, id: &str, method: &str, params: Value, too_large: &'static str, timeout: Duration) -> Result<Value> {
        if timeout.is_zero() { return Err("herdr_unavailable"); }
        let mut stream = UnixStream::connect(&self.socket).map_err(|_| "herdr_unavailable")?;
        // Set timeouts before sending: on macOS a second setsockopt after a
        // fast peer has replied and closed can fail with EINVAL despite unread data.
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|_| "herdr_unavailable")?;
        stream
            .set_write_timeout(Some(timeout))
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
