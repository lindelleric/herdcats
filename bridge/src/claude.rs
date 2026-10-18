use crate::{Result, adapter::Adapter, follower::Follower, herdr::Pane, resolver::Resolver};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{borrow::Cow, collections::HashSet, fs::File};

pub struct ClaudeAdapter {
    pub resolver: Option<Resolver>,
}

impl ClaudeAdapter {
    pub fn new(resolver: Option<Resolver>) -> Self {
        Self { resolver }
    }
}

impl Adapter for ClaudeAdapter {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn supports(&self, agent: &str) -> bool {
        agent == "claude"
    }

    fn is_available(&self) -> bool {
        self.resolver.is_some()
    }

    fn resolve_and_open(&self, pane: &Pane) -> Result<(String, File)> {
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
        let file = resolver.open(&path)?;
        Ok((session, file))
    }

    fn new_follower(&self, include_content: bool) -> Follower {
        if include_content {
            Follower::with_content()
        } else {
            Follower::default()
        }
    }
}

pub const MAX_ENTRIES: usize = 1000;
const TEXT_LIMIT: usize = 8192;

/// Maximum Entry ID length in bytes allowed on the wire.
/// Matches the iOS client's limit in `AgentTimelineState.validate`:
/// `!$0.id.isEmpty && $0.id.utf8.count <= 264`
pub const MAX_ENTRY_ID: usize = 264;

/// Builds and validates a wire Entry ID.
///
/// Returns `Err("invalid_transcript")` if the resulting ID is empty or exceeds
/// `MAX_ENTRY_ID` (264 bytes), matching the client validation limit.
pub(crate) fn entry_id(prefix: &str, part: &str) -> Result<String> {
    let full = if prefix.is_empty() {
        part.to_string()
    } else if part.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}:{part}")
    };
    if full.is_empty() || full.len() > MAX_ENTRY_ID {
        return Err("invalid_transcript");
    }
    Ok(full)
}

pub(crate) fn is_routine_metadata(kind: &str) -> bool {
    matches!(
        kind,
        "attachment"
            | "system"
            | "file-history-snapshot"
            | "last-prompt"
            | "ai-title"
            | "mode"
            | "permission-mode"
            | "atis-latch"
    )
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub kind: String,
    pub text: String,
    pub role: Option<String>,
    pub tool_name: Option<String>,
    pub status: Option<String>,
    pub details: Option<String>,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub record_refs: Vec<String>,
}
#[derive(Default, Clone)]
pub struct Transcript {
    pub entries: Vec<Entry>,
    pub unknown_records: usize,
    pub dropped_entries: usize,
    seen: HashSet<String>,
}
pub(crate) fn bounded(text: &str) -> (String, bool) {
    let mut end = text.len().min(TEXT_LIMIT);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_owned(), end < text.len())
}
fn text_content(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        return text.to_owned();
    }
    value
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}
fn notification_field<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    text.split_once(&format!("<{tag}>"))?
        .1
        .split_once(&format!("</{tag}>"))
        .map(|(value, _)| value.trim())
        .filter(|value| !value.is_empty())
}

impl Transcript {
    pub(crate) fn upsert(&mut self, mut entry: Entry) -> Result<()> {
        if entry.id.is_empty() || entry.id.len() > MAX_ENTRY_ID {
            return Err("invalid_transcript");
        }
        if let Some(old) = self.entries.iter_mut().find(|old| old.id == entry.id) {
            if entry.record_refs.is_empty() {
                entry.record_refs = old.record_refs.clone();
            }
            *old = entry;
        } else {
            if self.seen.contains(&entry.id) {
                return Ok(());
            }
            if self.seen.len() >= 100_000 {
                return Err("transcript_identity_limit");
            }
            self.seen.insert(entry.id.clone());
            self.entries.push(entry);
        }
        if self.entries.len() > MAX_ENTRIES {
            self.entries.remove(0);
            self.dropped_entries += 1;
        }
        Ok(())
    }
    fn apply_notification(&mut self, source_id: &str, text: &str) -> Result<()> {
        let (summary, truncated) =
            bounded(notification_field(text, "summary").unwrap_or("Background task update"));
        let status = match notification_field(text, "status") {
            Some("completed") => "completed",
            Some("failed" | "killed" | "cancelled") => "failed",
            Some("running") => "running",
            _ => "unknown",
        };
        self.upsert(Entry {
            id: entry_id("task", source_id)?,
            kind: "tool".into(),
            text: summary,
            role: None,
            tool_name: Some("Background task".into()),
            status: Some(status.into()),
            details: None,
            truncated,
            content_ref: None,
            record_refs: Vec::new(),
        })
    }

    pub fn apply(&mut self, record: &Value, session: &str) -> Result<()> {
        let kind = record["type"].as_str().unwrap_or("");
        if !matches!(kind, "user" | "assistant") {
            if !is_routine_metadata(kind) {
                self.unknown_records += 1;
            }
            return Ok(());
        }
        if record["isSidechain"] == true || record["isMeta"] == true {
            return Ok(());
        }
        if record["sessionId"].as_str() != Some(session) {
            return Err("session_mismatch");
        }
        let source_id = record["uuid"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 256)
            .ok_or("invalid_transcript")?;
        let content = &record["message"]["content"];
        if !content.is_string() && !content.is_array() {
            return Err("invalid_transcript");
        }
        if let Some(blocks) = content.as_array() {
            for block in blocks {
                if !block.is_object() || block["type"].as_str().is_none() {
                    return Err("invalid_transcript");
                }
                if block["type"] == "text" && !block["text"].is_string() {
                    return Err("invalid_transcript");
                }
            }
        }
        let text = text_content(content);
        if kind == "user"
            && (record["origin"]["kind"] == "task-notification"
                || record["turnOrigin"] == "task_notification")
        {
            return self.apply_notification(source_id, &text);
        }
        if !text.is_empty() {
            let (text, truncated) = bounded(&text);
            self.upsert(Entry {
                id: entry_id("message", source_id)?,
                kind: "message".into(),
                text,
                role: Some(kind.into()),
                tool_name: None,
                status: None,
                details: None,
                truncated,
                content_ref: None,
                record_refs: Vec::new(),
            })?;
        }
        if let Some(blocks) = content.as_array() {
            for block in blocks {
                match block["type"].as_str() {
                    Some("tool_use") => {
                        let id = block["id"]
                            .as_str()
                            .filter(|s| !s.is_empty() && s.len() <= 256)
                            .ok_or("invalid_transcript")?;
                        let name = block["name"]
                            .as_str()
                            .filter(|s| !s.is_empty() && s.len() <= 256)
                            .ok_or("invalid_transcript")?;
                        let (name, name_truncated) = bounded(name);
                        let (details, truncated) = bounded(&block["input"].to_string());
                        let key = entry_id("tool", id)?;
                        // A repeated call record must not revert a completed tool.
                        let old = self.entries.iter().find(|e| e.id == key);
                        let status = old
                            .and_then(|e| e.status.clone())
                            .unwrap_or_else(|| "running".into());
                        let text = old.map(|e| e.text.clone()).unwrap_or_default();
                        let output_truncated = old.is_some_and(|e| e.truncated);
                        self.upsert(Entry {
                            id: key,
                            kind: "tool".into(),
                            text,
                            role: None,
                            tool_name: Some(name),
                            status: Some(status),
                            details: Some(details),
                            truncated: truncated || name_truncated || output_truncated,
                            content_ref: None,
                            record_refs: Vec::new(),
                        })?;
                    }
                    Some("tool_result") => {
                        let id = block["tool_use_id"]
                            .as_str()
                            .filter(|s| !s.is_empty() && s.len() <= 256)
                            .ok_or("invalid_transcript")?;
                        let key = entry_id("tool", id)?;
                        if let Some(old) = self.entries.iter_mut().find(|e| e.id == key) {
                            let (text, truncated) = bounded(&text_content(&block["content"]));
                            old.text = text;
                            old.status = Some(
                                if block["is_error"] == true {
                                    "failed"
                                } else {
                                    "completed"
                                }
                                .into(),
                            );
                            old.truncated |= truncated;
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    pub fn apply_oversized(&mut self, part: &[u8], session: &str) -> Result<()> {
        let header: RecordHeader =
            serde_json::from_slice(part).map_err(|_| "invalid_transcript")?;
        let kind = header.kind.unwrap_or("");
        if !matches!(kind, "user" | "assistant") {
            if !is_routine_metadata(kind) {
                self.unknown_records += 1;
            }
            return Ok(());
        }
        if header.is_sidechain == Some(true) || header.is_meta == Some(true) {
            return Ok(());
        }
        if header.session_id != Some(session) {
            return Err("session_mismatch");
        }
        let source_id = header
            .uuid
            .filter(|s| !s.is_empty() && s.len() <= 256)
            .ok_or("invalid_transcript")?;

        let payload: OversizedRecordPayload =
            serde_json::from_slice(part).map_err(|_| "invalid_transcript")?;

        let mut has_omission = false;
        let mut msg_text_parts = Vec::new();
        let trimmed_content = payload.message.content.get().trim();

        let blocks_opt: Option<Vec<OversizedBlockPayload>> = if trimmed_content.starts_with('"') {
            let text: Cow<'_, str> =
                serde_json::from_str(trimmed_content).map_err(|_| "invalid_transcript")?;
            msg_text_parts.push(text);
            None
        } else if trimmed_content.starts_with('[') {
            let blocks: Vec<OversizedBlockPayload> =
                serde_json::from_str(trimmed_content).map_err(|_| "invalid_transcript")?;
            for block in &blocks {
                if block.block_type == "text" {
                    if let Some(t) = &block.text {
                        msg_text_parts.push(t.clone());
                    } else {
                        return Err("invalid_transcript");
                    }
                }
            }
            Some(blocks)
        } else {
            return Err("invalid_transcript");
        };

        let full_text = msg_text_parts.join("\n");
        if kind == "user"
            && (header.origin.as_ref().and_then(|origin| origin.kind) == Some("task-notification")
                || header.turn_origin == Some("task_notification"))
        {
            return self.apply_notification(source_id, &full_text);
        }
        if !full_text.is_empty() {
            let (text, truncated) = bounded(&full_text);
            self.upsert(Entry {
                id: entry_id("message", source_id)?,
                kind: "message".into(),
                text,
                role: Some(kind.into()),
                tool_name: None,
                status: None,
                details: None,
                truncated,
                content_ref: None,
                record_refs: Vec::new(),
            })?;
        }

        if let Some(blocks) = blocks_opt {
            for block in blocks {
                match block.block_type {
                    "text" => {}
                    "tool_use" => {
                        let id = block
                            .id
                            .filter(|s| !s.is_empty() && s.len() <= 256)
                            .ok_or("invalid_transcript")?;
                        let name = block
                            .name
                            .filter(|s| !s.is_empty() && s.len() <= 256)
                            .ok_or("invalid_transcript")?;
                        let (name, name_truncated) = bounded(name);
                        let input_str = block.input.map(|v| v.get()).unwrap_or("null");
                        let (details, truncated) = bounded(input_str);
                        let key = entry_id("tool", id)?;
                        let old = self.entries.iter().find(|e| e.id == key);
                        let status = old
                            .and_then(|e| e.status.clone())
                            .unwrap_or_else(|| "running".into());
                        let text = old.map(|e| e.text.clone()).unwrap_or_default();
                        let output_truncated = old.is_some_and(|e| e.truncated);
                        self.upsert(Entry {
                            id: key,
                            kind: "tool".into(),
                            text,
                            role: None,
                            tool_name: Some(name),
                            status: Some(status),
                            details: Some(details),
                            truncated: truncated || name_truncated || output_truncated,
                            content_ref: None,
                            record_refs: Vec::new(),
                        })?;
                    }
                    "tool_result" => {
                        let id = block
                            .tool_use_id
                            .filter(|s| !s.is_empty() && s.len() <= 256)
                            .ok_or("invalid_transcript")?;
                        let (res_text, sub_omission) = block
                            .content
                            .map(|r| parse_tool_result_content(r.get()))
                            .unwrap_or_default();
                        if sub_omission {
                            has_omission = true;
                        }
                        let key = entry_id("tool", id)?;
                        if let Some(old) = self.entries.iter_mut().find(|e| e.id == key) {
                            let (text, truncated) = bounded(&res_text);
                            old.text = text;
                            let is_error =
                                block.is_error.map(|r| r.get() == "true").unwrap_or(false);
                            old.status = Some(if is_error { "failed" } else { "completed" }.into());
                            old.truncated |= truncated;
                        }
                    }
                    "image" => {
                        has_omission = true;
                    }
                    _ => {
                        // Tolerates unknown block kinds (e.g. progress, web_search_tool_result)
                        // without failing the transcript, counting them as omitted blocks.
                        has_omission = true;
                    }
                }
            }
        }

        if has_omission {
            self.unknown_records += 1;
        }

        Ok(())
    }
}

#[derive(Deserialize)]
struct RecordHeader<'a> {
    #[serde(default, borrow)]
    origin: Option<RecordOrigin<'a>>,
    #[serde(rename = "turnOrigin", default, borrow)]
    turn_origin: Option<&'a str>,
    #[serde(rename = "type", default, borrow)]
    kind: Option<&'a str>,
    #[serde(default, borrow)]
    uuid: Option<&'a str>,
    #[serde(rename = "sessionId", default, borrow)]
    session_id: Option<&'a str>,
    #[serde(rename = "isSidechain", default)]
    is_sidechain: Option<bool>,
    #[serde(rename = "isMeta", default)]
    is_meta: Option<bool>,
}

#[derive(Deserialize)]
struct RecordOrigin<'a> {
    #[serde(default, borrow)]
    kind: Option<&'a str>,
}

#[derive(Deserialize)]
struct OversizedRecordPayload<'a> {
    #[serde(borrow)]
    message: OversizedMessagePayload<'a>,
}

#[derive(Deserialize)]
struct OversizedMessagePayload<'a> {
    #[serde(borrow)]
    content: &'a serde_json::value::RawValue,
}

#[derive(Deserialize)]
struct OversizedBlockPayload<'a> {
    #[serde(rename = "type", borrow)]
    block_type: &'a str,
    #[serde(default, borrow)]
    text: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    id: Option<&'a str>,
    #[serde(default, borrow)]
    name: Option<&'a str>,
    #[serde(default, borrow)]
    input: Option<&'a serde_json::value::RawValue>,
    #[serde(default, borrow)]
    tool_use_id: Option<&'a str>,
    #[serde(default, borrow)]
    content: Option<&'a serde_json::value::RawValue>,
    #[serde(default, borrow)]
    is_error: Option<&'a serde_json::value::RawValue>,
}

fn parse_tool_result_content(raw: &str) -> (String, bool) {
    let trimmed = raw.trim();
    if trimmed.starts_with('"') {
        if let Ok(s) = serde_json::from_str::<Cow<'_, str>>(trimmed) {
            return (s.into_owned(), false);
        }
    } else if trimmed.starts_with('[') {
        let mut texts = Vec::new();
        let mut has_omission = false;
        if let Ok(values) = serde_json::from_str::<Vec<&serde_json::value::RawValue>>(trimmed) {
            for v in values {
                let v_trimmed = v.get().trim();
                if v_trimmed.starts_with('{') {
                    #[derive(Deserialize)]
                    struct SubBlock<'a> {
                        #[serde(rename = "type", default, borrow)]
                        block_type: Option<&'a str>,
                        #[serde(default, borrow)]
                        text: Option<Cow<'a, str>>,
                    }
                    if let Ok(sub) = serde_json::from_str::<SubBlock>(v_trimmed) {
                        if sub.block_type == Some("text") {
                            if let Some(t) = sub.text {
                                texts.push(t.into_owned());
                            }
                        } else {
                            has_omission = true;
                        }
                    } else {
                        has_omission = true;
                    }
                }
            }
            return (texts.join("\n"), has_omission);
        }
    }
    (String::new(), false)
}
