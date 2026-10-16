use crate::{
    Result,
    adapter::Adapter,
    claude::{Entry, Transcript, bounded},
    follower::AgentType,
    follower::Follower,
    herdr::Pane,
    resolver::{open_beneath, session_id},
};
use serde::Deserialize;
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader, Seek, SeekFrom},
    path::{Path, PathBuf},
};

pub struct CodexResolver {
    pub root: PathBuf,
    anchor: fs::File,
}

impl Clone for CodexResolver {
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
            anchor: self.anchor.try_clone().expect("anchor clone"),
        }
    }
}

impl CodexResolver {
    pub fn new(root: &Path) -> Result<Self> {
        let root = root
            .canonicalize()
            .map_err(|_| "transcript_root_unavailable")?;
        if !root.is_dir() {
            return Err("transcript_root_unavailable");
        }
        let anchor = crate::resolver::open_absolute(&root, true)?;
        Ok(Self { root, anchor })
    }

    pub fn resolve(&self, thread_id: &str, pane: &Pane) -> Result<(String, fs::File)> {
        // Validate strictly before touching the filesystem
        if !session_id(thread_id) {
            return Err("invalid_session_reference");
        }
        let thread_suffix = format!("-{thread_id}.jsonl");
        let thread_suffix_lower = format!("-{}.jsonl", thread_id.to_ascii_lowercase());

        let mut matches = Vec::new();
        let mut count = 0;

        // Traverse YYYY/MM/DD/rollout-*-<thread-id>.jsonl
        let root_entries = fs::read_dir(&self.root).map_err(|_| "transcript_root_unavailable")?;
        for year_entry in root_entries {
            count += 1;
            if count > 50000 {
                return Err("transcript_index_too_large");
            }
            let year_entry = year_entry.map_err(|_| "transcript_root_unavailable")?;
            let year_path = year_entry.path();
            let year_meta =
                fs::symlink_metadata(&year_path).map_err(|_| "transcript_unavailable")?;
            if year_meta.file_type().is_symlink() || !year_meta.is_dir() {
                continue;
            }
            let year_name = year_entry.file_name();
            let year_str = year_name.to_str().unwrap_or("");
            if year_str.len() != 4 || !year_str.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }

            let month_entries = match fs::read_dir(&year_path) {
                Ok(entries) => entries,
                Err(_) => continue,
            };
            for month_entry in month_entries {
                count += 1;
                if count > 50000 {
                    return Err("transcript_index_too_large");
                }
                let month_entry = match month_entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                let month_path = month_entry.path();
                let month_meta = match fs::symlink_metadata(&month_path) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if month_meta.file_type().is_symlink() || !month_meta.is_dir() {
                    continue;
                }
                let month_name = month_entry.file_name();
                let month_str = month_name.to_str().unwrap_or("");
                if month_str.is_empty()
                    || month_str.len() > 2
                    || !month_str.chars().all(|c| c.is_ascii_digit())
                {
                    continue;
                }

                let day_entries = match fs::read_dir(&month_path) {
                    Ok(entries) => entries,
                    Err(_) => continue,
                };
                for day_entry in day_entries {
                    count += 1;
                    if count > 50000 {
                        return Err("transcript_index_too_large");
                    }
                    let day_entry = match day_entry {
                        Ok(e) => e,
                        Err(_) => continue,
                    };
                    let day_path = day_entry.path();
                    let day_meta = match fs::symlink_metadata(&day_path) {
                        Ok(m) => m,
                        Err(_) => continue,
                    };
                    if day_meta.file_type().is_symlink() || !day_meta.is_dir() {
                        continue;
                    }
                    let day_name = day_entry.file_name();
                    let day_str = day_name.to_str().unwrap_or("");
                    if day_str.is_empty()
                        || day_str.len() > 2
                        || !day_str.chars().all(|c| c.is_ascii_digit())
                    {
                        continue;
                    }

                    let file_entries = match fs::read_dir(&day_path) {
                        Ok(entries) => entries,
                        Err(_) => continue,
                    };
                    for file_entry in file_entries {
                        count += 1;
                        if count > 50000 {
                            return Err("transcript_index_too_large");
                        }
                        let file_entry = match file_entry {
                            Ok(e) => e,
                            Err(_) => continue,
                        };
                        let file_path = file_entry.path();
                        let file_meta = match fs::symlink_metadata(&file_path) {
                            Ok(m) => m,
                            Err(_) => continue,
                        };
                        if file_meta.file_type().is_symlink() || !file_meta.is_file() {
                            continue;
                        }
                        let file_name = file_entry.file_name();
                        let file_str = file_name.to_str().unwrap_or("");
                        let file_str_lower = file_str.to_ascii_lowercase();
                        if file_str.starts_with("rollout-")
                            && (file_str.ends_with(&thread_suffix)
                                || file_str_lower.ends_with(&thread_suffix_lower))
                        {
                            let relative = match file_path.strip_prefix(&self.root) {
                                Ok(r) => r.to_path_buf(),
                                Err(_) => continue,
                            };
                            matches.push(relative);
                        }
                    }
                }
            }
        }

        let relative_path = match matches.len() {
            0 => return Err("transcript_unavailable"),
            1 => matches.remove(0),
            _ => return Err("ambiguous_session"),
        };

        // Confined open beneath the pinned root anchor: O_NOFOLLOW on every level
        let mut file = open_beneath(&self.anchor, &relative_path, false)?;

        // Inspect record 0
        let mut reader = BufReader::new(&mut file);
        let mut line = Vec::new();
        reader
            .read_until(b'\n', &mut line)
            .map_err(|_| "transcript_unavailable")?;
        if line.is_empty() || line.last() != Some(&b'\n') {
            return Err("invalid_transcript");
        }
        let first_record: Value =
            serde_json::from_slice(&line).map_err(|_| "invalid_transcript")?;
        if first_record["type"].as_str() != Some("session_meta") {
            return Err("invalid_transcript");
        }
        let meta_id = first_record["payload"]["id"]
            .as_str()
            .or_else(|| first_record["payload"]["session_id"].as_str())
            .ok_or("invalid_transcript")?;
        if !meta_id.eq_ignore_ascii_case(thread_id) {
            return Err("session_mismatch");
        }
        let meta_cwd = first_record["payload"]["cwd"]
            .as_str()
            .ok_or("invalid_transcript")?;
        let pane_cwd = pane.foreground_cwd.as_deref().or(pane.cwd.as_deref());
        if pane_cwd.map(|p| p.trim_end_matches('/')) != Some(meta_cwd.trim_end_matches('/')) {
            return Err("session_cwd_mismatch");
        }

        // Seek file back to 0 for the follower
        file.seek(SeekFrom::Start(0))
            .map_err(|_| "transcript_unavailable")?;

        Ok((thread_id.to_string(), file))
    }
}

pub struct CodexAdapter {
    pub resolver: Option<CodexResolver>,
}

impl CodexAdapter {
    pub fn new(resolver: Option<CodexResolver>) -> Self {
        Self { resolver }
    }
}

impl Adapter for CodexAdapter {
    fn name(&self) -> &'static str {
        "codex"
    }

    fn supports(&self, agent: &str) -> bool {
        agent == "codex" && self.resolver.is_some()
    }

    fn is_available(&self) -> bool {
        self.resolver.is_some()
    }

    fn resolve_and_open(&self, pane: &Pane) -> Result<(String, fs::File)> {
        if pane.agent.as_deref() != Some("codex") {
            return Err("unsupported_agent");
        }
        let reference = pane
            .agent_session
            .as_ref()
            .ok_or("session_reference_unavailable")?;
        if reference.agent != "codex" || reference.source != "herdr:codex" {
            return Err("unsupported_session_source");
        }
        if reference.kind != "id" || !session_id(&reference.value) {
            return Err("invalid_session_reference");
        }
        let resolver = self
            .resolver
            .as_ref()
            .ok_or("transcript_root_unavailable")?;
        resolver.resolve(&reference.value, pane)
    }

    fn new_follower(&self, include_content: bool) -> Follower {
        if include_content {
            Follower::with_content_for_agent(AgentType::Codex)
        } else {
            Follower::for_agent(AgentType::Codex)
        }
    }
}

fn codex_text_content(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        return text.to_owned();
    }
    value
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter(|b| {
                    matches!(
                        b["type"].as_str(),
                        Some("text" | "input_text" | "output_text")
                    )
                })
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

impl Transcript {
    pub fn apply_codex(&mut self, record: &Value, thread_id: &str) -> Result<()> {
        let kind = record["type"].as_str().unwrap_or("");
        match kind {
            "session_meta" => {
                let id = record["payload"]["id"]
                    .as_str()
                    .or_else(|| record["payload"]["session_id"].as_str())
                    .unwrap_or("");
                if !id.eq_ignore_ascii_case(thread_id) {
                    return Err("session_mismatch");
                }
                Ok(())
            }
            "response_item" => {
                let payload = &record["payload"];
                let item_type = payload["type"].as_str().unwrap_or("");
                match item_type {
                    "message" => {
                        let role = payload["role"].as_str().unwrap_or("");
                        if role == "developer" || role == "system" {
                            return Ok(());
                        }
                        if !matches!(role, "user" | "assistant") {
                            self.unknown_records += 1;
                            return Ok(());
                        }
                        let id = payload["id"]
                            .as_str()
                            .filter(|s| !s.is_empty() && s.len() <= 256)
                            .ok_or("invalid_transcript")?;
                        let text = codex_text_content(&payload["content"]);
                        if !text.is_empty() {
                            let (text, truncated) = bounded(&text);
                            self.upsert(Entry {
                                id: format!("message:{id}"),
                                kind: "message".into(),
                                text,
                                role: Some(role.into()),
                                tool_name: None,
                                status: None,
                                details: None,
                                truncated,
                                content_ref: None,
                                record_refs: Vec::new(),
                            })?;
                        }
                        Ok(())
                    }
                    "function_call" => {
                        let id = payload["id"]
                            .as_str()
                            .filter(|s| !s.is_empty() && s.len() <= 256)
                            .ok_or("invalid_transcript")?;
                        let call_id = payload["call_id"].as_str().unwrap_or(id);
                        let name = payload["name"]
                            .as_str()
                            .filter(|s| !s.is_empty() && s.len() <= 256)
                            .ok_or("invalid_transcript")?;
                        let (name, name_truncated) = bounded(name);
                        let args = if let Some(s) = payload["arguments"].as_str() {
                            s.to_owned()
                        } else if payload["arguments"].is_object()
                            || payload["arguments"].is_array()
                        {
                            payload["arguments"].to_string()
                        } else {
                            String::new()
                        };
                        let (details, truncated) = bounded(&args);
                        let key = format!("tool:{call_id}");
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
                        })
                    }
                    "custom_tool_call" => {
                        let id = payload["id"]
                            .as_str()
                            .filter(|s| !s.is_empty() && s.len() <= 256)
                            .ok_or("invalid_transcript")?;
                        let call_id = payload["call_id"].as_str().unwrap_or(id);
                        let name = payload["name"]
                            .as_str()
                            .filter(|s| !s.is_empty() && s.len() <= 256)
                            .ok_or("invalid_transcript")?;
                        let (name, name_truncated) = bounded(name);
                        let input = if let Some(s) = payload["input"].as_str() {
                            s.to_owned()
                        } else if payload["input"].is_object() || payload["input"].is_array() {
                            payload["input"].to_string()
                        } else {
                            String::new()
                        };
                        let (details, truncated) = bounded(&input);
                        let key = format!("tool:{call_id}");
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
                        })
                    }
                    "function_call_output" | "custom_tool_call_output" => {
                        let call_id = payload["call_id"]
                            .as_str()
                            .filter(|s| !s.is_empty() && s.len() <= 256)
                            .ok_or("invalid_transcript")?;
                        let output = if let Some(s) = payload["output"].as_str() {
                            s.to_owned()
                        } else if payload["output"].is_object() || payload["output"].is_array() {
                            payload["output"].to_string()
                        } else {
                            String::new()
                        };
                        let (text, truncated) = bounded(&output);
                        let key = format!("tool:{call_id}");
                        if let Some(old) = self.entries.iter_mut().find(|e| e.id == key) {
                            old.text = text;
                            old.status = Some("completed".into());
                            old.truncated |= truncated;
                        } else {
                            self.upsert(Entry {
                                id: key,
                                kind: "tool".into(),
                                text,
                                role: None,
                                tool_name: None,
                                status: Some("completed".into()),
                                details: None,
                                truncated,
                                content_ref: None,
                                record_refs: Vec::new(),
                            })?;
                        }
                        Ok(())
                    }
                    "error" => {
                        let msg = payload["message"].as_str().unwrap_or("Error");
                        let (text, truncated) = bounded(msg);
                        let id = payload["id"].as_str().unwrap_or("error");
                        self.upsert(Entry {
                            id: format!("message:{id}"),
                            kind: "message".into(),
                            text,
                            role: Some("assistant".into()),
                            tool_name: None,
                            status: Some("failed".into()),
                            details: None,
                            truncated,
                            content_ref: None,
                            record_refs: Vec::new(),
                        })
                    }
                    _ => {
                        self.unknown_records += 1;
                        Ok(())
                    }
                }
            }
            "event_msg" => {
                let event_type = record["payload"]["type"].as_str().unwrap_or("");
                match event_type {
                    "turn_aborted" => {
                        for entry in self.entries.iter_mut() {
                            if entry.kind == "tool" && entry.status.as_deref() == Some("running") {
                                entry.status = Some("failed".into());
                            }
                        }
                        Ok(())
                    }
                    "task_started" | "item_completed" | "task_complete" | "token_count"
                    | "token_usage" => Ok(()),
                    _ => {
                        self.unknown_records += 1;
                        Ok(())
                    }
                }
            }
            "turn_context" | "environment_context" | "world_state" | "token_usage_record" => Ok(()),
            _ => {
                self.unknown_records += 1;
                Ok(())
            }
        }
    }

    pub fn apply_codex_oversized(&mut self, part: &[u8], thread_id: &str) -> Result<()> {
        #[derive(Deserialize)]
        struct RecordHeader<'a> {
            #[serde(rename = "type", default, borrow)]
            kind: Option<&'a str>,
            #[serde(default, borrow)]
            payload: Option<PayloadHeader<'a>>,
        }

        #[derive(Deserialize)]
        struct PayloadHeader<'a> {
            #[serde(rename = "type", default, borrow)]
            item_type: Option<&'a str>,
            #[serde(default, borrow)]
            id: Option<&'a str>,
            #[serde(rename = "session_id", default, borrow)]
            session_id: Option<&'a str>,
            #[serde(default, borrow)]
            call_id: Option<&'a str>,
            #[serde(default, borrow)]
            role: Option<&'a str>,
            #[serde(default, borrow)]
            name: Option<&'a str>,
            #[serde(default, borrow)]
            content: Option<&'a serde_json::value::RawValue>,
            #[serde(default, borrow)]
            arguments: Option<&'a serde_json::value::RawValue>,
            #[serde(default, borrow)]
            input: Option<&'a serde_json::value::RawValue>,
            #[serde(default, borrow)]
            output: Option<&'a serde_json::value::RawValue>,
            #[serde(default, borrow)]
            message: Option<&'a str>,
        }

        let header: RecordHeader =
            serde_json::from_slice(part).map_err(|_| "invalid_transcript")?;
        let kind = header.kind.unwrap_or("");
        match kind {
            "session_meta" => {
                let p = header.payload.ok_or("invalid_transcript")?;
                let id = p.id.or(p.session_id).unwrap_or("");
                if !id.eq_ignore_ascii_case(thread_id) {
                    return Err("session_mismatch");
                }
                Ok(())
            }
            "response_item" => {
                let p = header.payload.ok_or("invalid_transcript")?;
                let item_type = p.item_type.unwrap_or("");
                match item_type {
                    "message" => {
                        let role = p.role.unwrap_or("");
                        if role == "developer" || role == "system" {
                            return Ok(());
                        }
                        if !matches!(role, "user" | "assistant") {
                            self.unknown_records += 1;
                            return Ok(());
                        }
                        let id =
                            p.id.filter(|s| !s.is_empty() && s.len() <= 256)
                                .ok_or("invalid_transcript")?;

                        let content_raw = p.content.ok_or("invalid_transcript")?.get().trim();
                        let full_text: String = if content_raw.starts_with('"') {
                            serde_json::from_str(content_raw).map_err(|_| "invalid_transcript")?
                        } else if content_raw.starts_with('[') {
                            #[derive(Deserialize)]
                            struct SubBlock<'a> {
                                #[serde(rename = "type", default, borrow)]
                                block_type: Option<&'a str>,
                                #[serde(default, borrow)]
                                text: Option<std::borrow::Cow<'a, str>>,
                            }
                            let blocks: Vec<SubBlock> = serde_json::from_str(content_raw)
                                .map_err(|_| "invalid_transcript")?;
                            blocks
                                .into_iter()
                                .filter(|b| {
                                    matches!(
                                        b.block_type,
                                        Some("text" | "input_text" | "output_text")
                                    )
                                })
                                .filter_map(|b| b.text.map(|t| t.into_owned()))
                                .collect::<Vec<_>>()
                                .join("\n")
                        } else {
                            String::new()
                        };

                        if !full_text.is_empty() {
                            let (text, truncated) = bounded(&full_text);
                            self.upsert(Entry {
                                id: format!("message:{id}"),
                                kind: "message".into(),
                                text,
                                role: Some(role.into()),
                                tool_name: None,
                                status: None,
                                details: None,
                                truncated,
                                content_ref: None,
                                record_refs: Vec::new(),
                            })?;
                        }
                        Ok(())
                    }
                    "function_call" => {
                        let id =
                            p.id.filter(|s| !s.is_empty() && s.len() <= 256)
                                .ok_or("invalid_transcript")?;
                        let call_id = p.call_id.unwrap_or(id);
                        let name = p
                            .name
                            .filter(|s| !s.is_empty() && s.len() <= 256)
                            .ok_or("invalid_transcript")?;
                        let (name, name_truncated) = bounded(name);
                        let args_str = p.arguments.map(|v| v.get()).unwrap_or("");
                        let (details, truncated) = bounded(args_str);
                        let key = format!("tool:{call_id}");
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
                        })
                    }
                    "custom_tool_call" => {
                        let id =
                            p.id.filter(|s| !s.is_empty() && s.len() <= 256)
                                .ok_or("invalid_transcript")?;
                        let call_id = p.call_id.unwrap_or(id);
                        let name = p
                            .name
                            .filter(|s| !s.is_empty() && s.len() <= 256)
                            .ok_or("invalid_transcript")?;
                        let (name, name_truncated) = bounded(name);
                        let input_str = p.input.map(|v| v.get()).unwrap_or("");
                        let (details, truncated) = bounded(input_str);
                        let key = format!("tool:{call_id}");
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
                        })
                    }
                    "function_call_output" | "custom_tool_call_output" => {
                        let call_id = p
                            .call_id
                            .filter(|s| !s.is_empty() && s.len() <= 256)
                            .ok_or("invalid_transcript")?;
                        let output_str = p.output.map(|v| v.get()).unwrap_or("");
                        let (text, truncated) = bounded(output_str);
                        let key = format!("tool:{call_id}");
                        if let Some(old) = self.entries.iter_mut().find(|e| e.id == key) {
                            old.text = text;
                            old.status = Some("completed".into());
                            old.truncated |= truncated;
                        } else {
                            self.upsert(Entry {
                                id: key,
                                kind: "tool".into(),
                                text,
                                role: None,
                                tool_name: None,
                                status: Some("completed".into()),
                                details: None,
                                truncated,
                                content_ref: None,
                                record_refs: Vec::new(),
                            })?;
                        }
                        Ok(())
                    }
                    "error" => {
                        let msg = p.message.unwrap_or("Error");
                        let (text, truncated) = bounded(msg);
                        let id = p.id.unwrap_or("error");
                        self.upsert(Entry {
                            id: format!("message:{id}"),
                            kind: "message".into(),
                            text,
                            role: Some("assistant".into()),
                            tool_name: None,
                            status: Some("failed".into()),
                            details: None,
                            truncated,
                            content_ref: None,
                            record_refs: Vec::new(),
                        })
                    }
                    _ => {
                        self.unknown_records += 1;
                        Ok(())
                    }
                }
            }
            "event_msg" => {
                let p = header.payload.ok_or("invalid_transcript")?;
                let event_type = p.item_type.unwrap_or("");
                match event_type {
                    "turn_aborted" => {
                        for entry in self.entries.iter_mut() {
                            if entry.kind == "tool" && entry.status.as_deref() == Some("running") {
                                entry.status = Some("failed".into());
                            }
                        }
                        Ok(())
                    }
                    "task_started" | "item_completed" | "task_complete" | "token_count"
                    | "token_usage" => Ok(()),
                    _ => {
                        self.unknown_records += 1;
                        Ok(())
                    }
                }
            }
            "turn_context" | "environment_context" | "world_state" | "token_usage_record" => Ok(()),
            _ => {
                self.unknown_records += 1;
                Ok(())
            }
        }
    }

    pub fn preserve_codex_record(
        &mut self,
        part: &[u8],
        _thread_id: &str,
        offset: usize,
    ) -> Result<()> {
        let row: Value = serde_json::from_slice(part).map_err(|_| "invalid_transcript")?;
        let kind = row["type"].as_str().unwrap_or("");
        if matches!(
            kind,
            "session_meta"
                | "turn_context"
                | "environment_context"
                | "world_state"
                | "token_usage_record"
        ) {
            return Ok(());
        }
        if kind == "event_msg" {
            let event_type = row["payload"]["type"].as_str().unwrap_or("");
            if matches!(
                event_type,
                "task_started" | "item_completed" | "task_complete" | "token_count" | "token_usage"
            ) {
                return Ok(());
            }
        }
        let reference = format!("source:{offset}:{}", part.len());
        if kind == "response_item" {
            let payload = &row["payload"];
            let item_type = payload["type"].as_str().unwrap_or("");
            if item_type == "message" {
                let role = payload["role"].as_str().unwrap_or("");
                if role == "developer" || role == "system" {
                    return Ok(());
                }
                if let Some(id) = payload["id"].as_str() {
                    if self.attach_record(&format!("message:{id}"), &reference) {
                        return Ok(());
                    }
                }
            } else if matches!(
                item_type,
                "function_call"
                    | "custom_tool_call"
                    | "function_call_output"
                    | "custom_tool_call_output"
            ) {
                let call_id = payload["call_id"]
                    .as_str()
                    .or_else(|| payload["id"].as_str());
                if let Some(id) = call_id {
                    if self.attach_record(&format!("tool:{id}"), &reference) {
                        return Ok(());
                    }
                }
            }
        }
        Ok(())
    }
}
