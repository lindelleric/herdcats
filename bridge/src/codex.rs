use crate::{
    Result,
    adapter::Adapter,
    claude::{Entry, Transcript, bounded, entry_id},
    follower::{AgentType, Follower, MAX_RECORD, MAX_SOURCE},
    herdr::Pane,
    resolver::{open_beneath, session_id},
};
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStamp {
    pub dev: u64,
    pub ino: u64,
    pub len: u64,
    pub mtime: (i64, i64),
}

impl FileStamp {
    pub fn of(m: &fs::Metadata) -> Self {
        Self {
            dev: m.dev(),
            ino: m.ino(),
            len: m.len(),
            mtime: (m.mtime(), m.mtime_nsec()),
        }
    }
}

pub struct CodexResolver {
    pub root: PathBuf,
    anchor: fs::File,
    pub scan_count: Arc<AtomicUsize>,
    pub meta_read_count: Arc<AtomicUsize>,
}

impl Clone for CodexResolver {
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
            anchor: self.anchor.try_clone().expect("anchor clone"),
            scan_count: self.scan_count.clone(),
            meta_read_count: self.meta_read_count.clone(),
        }
    }
}

pub fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

pub fn parse_uuid_v7_timestamp_ms(thread_id: &str) -> Option<u64> {
    if thread_id.len() != 36 {
        return None;
    }
    let bytes = thread_id.as_bytes();
    if bytes[8] != b'-' || bytes[13] != b'-' || bytes[18] != b'-' || bytes[23] != b'-' {
        return None;
    }
    if bytes[14] != b'7' {
        return None;
    }
    let variant = bytes[19];
    if !matches!(variant, b'8' | b'9' | b'a' | b'b' | b'A' | b'B') {
        return None;
    }
    let p1 = u64::from_str_radix(std::str::from_utf8(&bytes[0..8]).ok()?, 16).ok()?;
    let p2 = u64::from_str_radix(std::str::from_utf8(&bytes[9..13]).ok()?, 16).ok()?;
    Some((p1 << 16) | p2)
}

pub fn v7_candidate_day_dirs(ts_ms: u64) -> Vec<(i32, u32, u32)> {
    let days = (ts_ms / 1000) as i64 / 86400;
    vec![
        civil_from_days(days - 1),
        civil_from_days(days),
        civil_from_days(days + 1),
    ]
}

impl CodexResolver {
    pub fn new(root: &Path) -> Result<Self> {
        if !root.is_absolute() {
            return Err("transcript_root_unavailable");
        }
        let root = root
            .canonicalize()
            .map_err(|_| "transcript_root_unavailable")?;
        if !root.is_dir() {
            return Err("transcript_root_unavailable");
        }
        let anchor = crate::resolver::open_absolute(&root, true)?;
        Ok(Self {
            root,
            anchor,
            scan_count: Arc::new(AtomicUsize::new(0)),
            meta_read_count: Arc::new(AtomicUsize::new(0)),
        })
    }

    pub fn resolve_relative(&self, thread_id: &str) -> Result<PathBuf> {
        // Validate strictly before touching the filesystem
        if !session_id(thread_id) {
            return Err("invalid_session_reference");
        }
        self.scan_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let thread_suffix = format!("-{thread_id}.jsonl");
        let thread_suffix_lower = format!("-{}.jsonl", thread_id.to_ascii_lowercase());

        let mut matches = Vec::new();
        let mut count = 0;

        if let Some(ts_ms) = parse_uuid_v7_timestamp_ms(thread_id) {
            // UUIDv7: look only in the UTC date of that timestamp plus/minus 1 day
            let candidates = v7_candidate_day_dirs(ts_ms);
            for (y, m, d) in candidates {
                let mut day_paths = vec![self.root.join(format!("{y:04}/{m:02}/{d:02}"))];
                if m < 10 || d < 10 {
                    day_paths.push(self.root.join(format!("{y:04}/{m}/{d}")));
                }
                for day_path in day_paths {
                    let day_meta = match fs::symlink_metadata(&day_path) {
                        Ok(meta) => meta,
                        Err(_) => continue,
                    };
                    if day_meta.file_type().is_symlink() || !day_meta.is_dir() {
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
                            if !matches.contains(&relative) {
                                matches.push(relative);
                            }
                        }
                    }
                }
            }
        } else {
            // Fall back to the bounded full walk only if the ID is not v7
            let root_entries =
                fs::read_dir(&self.root).map_err(|_| "transcript_root_unavailable")?;
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
        }

        match matches.len() {
            0 => Err("transcript_unavailable"),
            1 => Ok(matches.remove(0)),
            _ => Err("ambiguous_session"),
        }
    }

    pub fn read_first_record(file: &mut fs::File) -> Result<Value> {
        let mut reader = BufReader::new((&mut *file).take(MAX_RECORD as u64));
        let mut line = Vec::new();
        reader
            .read_until(b'\n', &mut line)
            .map_err(|_| "transcript_unavailable")?;
        if line.is_empty() {
            return Err("invalid_transcript");
        }
        if line.last() != Some(&b'\n') {
            if line.len() >= MAX_RECORD {
                return Err("source_too_large");
            }
            return Err("invalid_transcript");
        }
        serde_json::from_slice(&line).map_err(|_| "invalid_transcript")
    }

    pub fn verify_session_meta(
        first_record: &Value,
        thread_id: &str,
        pane: &Pane,
    ) -> Result<String> {
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
        Self::verify_pane_cwd(meta_cwd, pane)?;
        Ok(meta_cwd.to_string())
    }

    pub fn verify_pane_cwd(meta_cwd: &str, pane: &Pane) -> Result<()> {
        let pane_cwd = pane.foreground_cwd.as_deref().or(pane.cwd.as_deref());
        if pane_cwd.map(|p| p.trim_end_matches('/')) != Some(meta_cwd.trim_end_matches('/')) {
            return Err("session_cwd_mismatch");
        }
        Ok(())
    }

    pub fn open_and_verify(
        &self,
        relative_path: &Path,
        thread_id: &str,
        pane: &Pane,
    ) -> Result<fs::File> {
        // Confined open beneath the pinned root anchor: O_NOFOLLOW on every level
        let mut file = open_beneath(&self.anchor, relative_path, false)?;

        let meta = file.metadata().map_err(|_| "transcript_unavailable")?;
        if meta.len() > MAX_SOURCE {
            return Err("source_too_large");
        }

        self.meta_read_count.fetch_add(1, Ordering::SeqCst);
        let first_record = Self::read_first_record(&mut file)?;
        Self::verify_session_meta(&first_record, thread_id, pane)?;

        // Seek file back to 0 for the follower
        file.seek(SeekFrom::Start(0))
            .map_err(|_| "transcript_unavailable")?;

        Ok(file)
    }

    pub fn resolve(&self, thread_id: &str, pane: &Pane) -> Result<(String, fs::File)> {
        let relative_path = self.resolve_relative(thread_id)?;
        let file = self.open_and_verify(&relative_path, thread_id, pane)?;
        Ok((thread_id.to_string(), file))
    }
}

#[derive(Clone, Debug)]
struct CachedSession {
    rel: PathBuf,
    last_walk: Option<Instant>,
}

pub struct CodexAdapter {
    pub resolver: Option<CodexResolver>,
    cache: Mutex<std::collections::HashMap<String, CachedSession>>,
    clock: Arc<dyn Fn() -> Instant + Send + Sync>,
}

impl CodexAdapter {
    pub fn new(resolver: Option<CodexResolver>) -> Self {
        Self::with_clock(resolver, Arc::new(Instant::now))
    }

    pub fn with_clock(
        resolver: Option<CodexResolver>,
        clock: Arc<dyn Fn() -> Instant + Send + Sync>,
    ) -> Self {
        Self {
            resolver,
            cache: Mutex::new(std::collections::HashMap::new()),
            clock,
        }
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

        let thread_id = &reference.value;
        let is_v7 = parse_uuid_v7_timestamp_ms(thread_id).is_some();
        let now = (self.clock)();

        let (rel, walk_occurred) = if is_v7 {
            // For UUIDv7 ids: re-list the +-1 day directories on every lookup (they are cheap, 3 dirs)
            // and fail with ambiguous_session if more than one file matches.
            let rel = resolver.resolve_relative(thread_id)?;
            (rel, false)
        } else {
            // For non-v7 ids (full walk): redo the walk at most every 30 s while a cached path exists,
            // and fail closed as soon as a duplicate shows up.
            let cached_opt = self.cache.lock().unwrap().get(thread_id).cloned();
            match cached_opt {
                Some(cached) => {
                    let elapsed = cached
                        .last_walk
                        .map(|t| now.duration_since(t))
                        .unwrap_or(Duration::from_secs(31));
                    if elapsed < Duration::from_secs(30) {
                        (cached.rel, false)
                    } else {
                        match resolver.resolve_relative(thread_id) {
                            Ok(rel) => (rel, true),
                            Err(e) => {
                                self.cache.lock().unwrap().remove(thread_id);
                                return Err(e);
                            }
                        }
                    }
                }
                None => {
                    let rel = resolver.resolve_relative(thread_id)?;
                    (rel, true)
                }
            }
        };

        let mut file = match open_beneath(&resolver.anchor, &rel, false) {
            Ok(f) => f,
            Err(e) => {
                self.cache.lock().unwrap().remove(thread_id);
                if !is_v7 && !walk_occurred {
                    let fresh_rel = resolver.resolve_relative(thread_id)?;
                    let mut fresh_file = open_beneath(&resolver.anchor, &fresh_rel, false)?;
                    let fresh_meta = fresh_file
                        .metadata()
                        .map_err(|_| "transcript_unavailable")?;
                    if fresh_meta.len() > MAX_SOURCE {
                        return Err("source_too_large");
                    }
                    resolver.meta_read_count.fetch_add(1, Ordering::SeqCst);
                    let record = CodexResolver::read_first_record(&mut fresh_file)?;
                    CodexResolver::verify_session_meta(&record, thread_id, pane)?;
                    fresh_file
                        .seek(SeekFrom::Start(0))
                        .map_err(|_| "transcript_unavailable")?;
                    self.cache.lock().unwrap().insert(
                        thread_id.to_string(),
                        CachedSession {
                            rel: fresh_rel,
                            last_walk: Some(now),
                        },
                    );
                    return Ok((thread_id.to_string(), fresh_file));
                }
                return Err(e);
            }
        };

        let meta = file.metadata().map_err(|_| "transcript_unavailable")?;
        if meta.len() > MAX_SOURCE {
            return Err("source_too_large");
        }
        let cached_entry = self.cache.lock().unwrap().get(thread_id).cloned();
        // The first record authorizes the file, and an equal metadata stamp cannot prove
        // it is unchanged (same-length in-place rewrites on coarse-timestamp filesystems),
        // so it is verified on every open.
        resolver.meta_read_count.fetch_add(1, Ordering::SeqCst);
        let record = CodexResolver::read_first_record(&mut file)?;
        CodexResolver::verify_session_meta(&record, thread_id, pane)?;

        file.seek(SeekFrom::Start(0))
            .map_err(|_| "transcript_unavailable")?;

        let last_walk = if is_v7 {
            None
        } else if walk_occurred || cached_entry.as_ref().and_then(|c| c.last_walk).is_none() {
            Some(now)
        } else {
            cached_entry.and_then(|c| c.last_walk)
        };

        self.cache
            .lock()
            .unwrap()
            .insert(thread_id.to_string(), CachedSession { rel, last_walk });

        Ok((thread_id.to_string(), file))
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

fn has_visible_summary_text(summary: &Value) -> bool {
    if let Some(s) = summary.as_str() {
        return !s.trim().is_empty();
    }
    if let Some(text) = summary["text"].as_str() {
        return !text.trim().is_empty();
    }
    if let Some(arr) = summary.as_array() {
        return arr.iter().any(|item| {
            if let Some(text) = item.as_str() {
                !text.trim().is_empty()
            } else if let Some(text) = item["text"].as_str() {
                !text.trim().is_empty()
            } else {
                false
            }
        });
    }
    false
}

pub const CODEX_INJECTED_CONTEXT_PREFIXES: &[&str] = &[
    "# AGENTS.md instructions",
    "<environment_context>",
    "<user_instructions>",
    "<user_shell_command>",
    "<turn_aborted>",
    "<skill>",
    "<skill ",
    "<subagent>",
    "<subagent ",
    "<subagent_notification>",
];

pub fn is_codex_injected_text(text: &str) -> bool {
    let trimmed = text.trim_start();
    CODEX_INJECTED_CONTEXT_PREFIXES
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
}

pub fn is_codex_injected_context(content: &Value) -> bool {
    if let Some(text) = content.as_str() {
        return is_codex_injected_text(text);
    }
    if let Some(blocks) = content.as_array() {
        if blocks.is_empty() {
            return false;
        }
        return blocks.iter().all(|b| {
            if let Some(text) = b.as_str() {
                is_codex_injected_text(text)
            } else if matches!(
                b["type"].as_str(),
                Some("text" | "input_text" | "output_text")
            ) {
                b["text"].as_str().is_some_and(is_codex_injected_text)
            } else {
                false
            }
        });
    }
    false
}

#[derive(Debug)]
pub(crate) enum CodexRecordAction<'a> {
    SessionMeta {
        id: &'a str,
    },
    Skip,
    Message {
        id: &'a str,
        role: &'a str,
        text: String,
    },
    FunctionCall {
        call_id: &'a str,
        name: &'a str,
        args: String,
    },
    CustomToolCall {
        call_id: &'a str,
        name: &'a str,
        input: String,
    },
    ToolOutput {
        call_id: &'a str,
        output: String,
    },
    Error {
        id: Option<&'a str>,
        message: &'a str,
    },
    TurnAborted {
        turn_id: &'a str,
    },
    Unknown {
        label: &'a str,
    },
}

pub(crate) fn entry_key_part(s: &str) -> Result<&str> {
    if !s.is_empty() && s.len() <= 256 {
        Ok(s)
    } else {
        Err("invalid_transcript")
    }
}

fn optional_entry_key_part(val: &Value) -> Result<Option<&str>> {
    match val {
        Value::Null => Ok(None),
        Value::String(s) => entry_key_part(s).map(Some),
        _ => Err("invalid_transcript"),
    }
}

fn required_entry_key_part(val: &Value) -> Result<&str> {
    match val {
        Value::String(s) => entry_key_part(s),
        _ => Err("invalid_transcript"),
    }
}

pub(crate) fn classify_codex_record(record: &Value) -> Result<CodexRecordAction<'_>> {
    let kind = record["type"].as_str().unwrap_or("");
    match kind {
        "session_meta" => {
            let id = record["payload"]["id"]
                .as_str()
                .or_else(|| record["payload"]["session_id"].as_str())
                .unwrap_or("");
            Ok(CodexRecordAction::SessionMeta { id })
        }
        "turn_context"
        | "environment_context"
        | "world_state"
        | "token_usage_record"
        | "compacted"
        | "inter_agent_communication_metadata" => Ok(CodexRecordAction::Skip),
        "event_msg" => {
            let payload = &record["payload"];
            let event_type = payload["type"].as_str().unwrap_or("");
            match event_type {
                "turn_aborted" => {
                    let turn_id = match optional_entry_key_part(&payload["turn_id"])? {
                        Some(id) => id,
                        None => optional_entry_key_part(&payload["root_turn_id"])?.unwrap_or(""),
                    };
                    entry_id("turn_aborted", turn_id)?;
                    Ok(CodexRecordAction::TurnAborted { turn_id })
                }
                "thread_settings_applied"
                | "task_started"
                | "item_completed"
                | "task_complete"
                | "token_count"
                | "token_usage" => Ok(CodexRecordAction::Skip),
                _ => Ok(CodexRecordAction::Unknown {
                    label: if event_type.is_empty() {
                        "event_msg"
                    } else {
                        event_type
                    },
                }),
            }
        }
        "response_item" => {
            let payload = &record["payload"];
            let item_type = payload["type"].as_str().unwrap_or("");
            match item_type {
                "compaction" => Ok(CodexRecordAction::Skip),
                "reasoning" => {
                    if !has_visible_summary_text(&payload["summary"]) {
                        Ok(CodexRecordAction::Skip)
                    } else {
                        Ok(CodexRecordAction::Unknown { label: "reasoning" })
                    }
                }
                "message" => {
                    let role = payload["role"].as_str().unwrap_or("");
                    if role == "developer" || role == "system" {
                        return Ok(CodexRecordAction::Skip);
                    }
                    if !matches!(role, "user" | "assistant") {
                        return Ok(CodexRecordAction::Unknown { label: "message" });
                    }
                    if role == "user" && is_codex_injected_context(&payload["content"]) {
                        return Ok(CodexRecordAction::Skip);
                    }
                    let id = required_entry_key_part(&payload["id"])?;
                    entry_id("message", id)?;
                    let text = codex_text_content(&payload["content"]);
                    Ok(CodexRecordAction::Message { id, role, text })
                }
                "function_call" => {
                    let id = required_entry_key_part(&payload["id"])?;
                    let call_id = optional_entry_key_part(&payload["call_id"])?.unwrap_or(id);
                    entry_id("tool", call_id)?;
                    let name = required_entry_key_part(&payload["name"])?;
                    let args = if let Some(s) = payload["arguments"].as_str() {
                        s.to_owned()
                    } else if payload["arguments"].is_object() || payload["arguments"].is_array() {
                        payload["arguments"].to_string()
                    } else {
                        String::new()
                    };
                    Ok(CodexRecordAction::FunctionCall {
                        call_id,
                        name,
                        args,
                    })
                }
                "custom_tool_call" => {
                    let id = required_entry_key_part(&payload["id"])?;
                    let call_id = optional_entry_key_part(&payload["call_id"])?.unwrap_or(id);
                    entry_id("tool", call_id)?;
                    let name = required_entry_key_part(&payload["name"])?;
                    let input = if let Some(s) = payload["input"].as_str() {
                        s.to_owned()
                    } else if payload["input"].is_object() || payload["input"].is_array() {
                        payload["input"].to_string()
                    } else {
                        String::new()
                    };
                    Ok(CodexRecordAction::CustomToolCall {
                        call_id,
                        name,
                        input,
                    })
                }
                "function_call_output" | "custom_tool_call_output" => {
                    let call_id = required_entry_key_part(&payload["call_id"])?;
                    entry_id("tool", call_id)?;
                    let output = if let Some(s) = payload["output"].as_str() {
                        s.to_owned()
                    } else if payload["output"].is_object() || payload["output"].is_array() {
                        payload["output"].to_string()
                    } else {
                        String::new()
                    };
                    Ok(CodexRecordAction::ToolOutput { call_id, output })
                }
                "error" => {
                    let message = payload["message"].as_str().unwrap_or("Error");
                    let id = optional_entry_key_part(&payload["id"])?;
                    if let Some(id) = id {
                        entry_id("message", id)?;
                    }
                    Ok(CodexRecordAction::Error { id, message })
                }
                _ => Ok(CodexRecordAction::Unknown {
                    label: if item_type.is_empty() {
                        "response_item"
                    } else {
                        item_type
                    },
                }),
            }
        }
        _ => Ok(CodexRecordAction::Unknown {
            label: if kind.is_empty() {
                "Session record"
            } else {
                kind
            },
        }),
    }
}

fn error_entry_id(id: Option<&str>, offset: usize) -> Result<String> {
    match id {
        Some(id) => entry_id("message", id),
        None => entry_id("error", &offset.to_string()),
    }
}

impl Transcript {
    pub fn apply_codex(&mut self, record: &Value, thread_id: &str, offset: usize) -> Result<()> {
        let action = classify_codex_record(record)?;
        match action {
            CodexRecordAction::SessionMeta { id } => {
                if !id.eq_ignore_ascii_case(thread_id) {
                    return Err("session_mismatch");
                }
                Ok(())
            }
            CodexRecordAction::Skip => Ok(()),
            CodexRecordAction::Message { id, role, text } => {
                if !text.is_empty() {
                    let (text, truncated) = bounded(&text);
                    self.upsert(Entry {
                        id: entry_id("message", id)?,
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
            CodexRecordAction::FunctionCall {
                call_id,
                name,
                args,
            } => {
                let (name, name_truncated) = bounded(name);
                let (details, truncated) = bounded(&args);
                let key = entry_id("tool", call_id)?;
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
            CodexRecordAction::CustomToolCall {
                call_id,
                name,
                input,
            } => {
                let (name, name_truncated) = bounded(name);
                let (details, truncated) = bounded(&input);
                let key = entry_id("tool", call_id)?;
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
            CodexRecordAction::ToolOutput { call_id, output } => {
                let (text, truncated) = bounded(&output);
                let key = entry_id("tool", call_id)?;
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
            CodexRecordAction::Error { id, message } => {
                let (text, truncated) = bounded(message);
                self.upsert(Entry {
                    id: error_entry_id(id, offset)?,
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
            CodexRecordAction::TurnAborted { turn_id } => {
                for entry in self.entries.iter_mut() {
                    if entry.kind == "tool" && entry.status.as_deref() == Some("running") {
                        entry.status = Some("failed".into());
                    }
                }
                let id = entry_id("turn_aborted", turn_id)?;
                self.upsert(Entry {
                    id,
                    kind: "message".into(),
                    text: "Turn interrupted".into(),
                    role: Some("assistant".into()),
                    tool_name: None,
                    status: Some("failed".into()),
                    details: None,
                    truncated: false,
                    content_ref: None,
                    record_refs: Vec::new(),
                })
            }
            CodexRecordAction::Unknown { .. } => {
                self.unknown_records += 1;
                Ok(())
            }
        }
    }

    pub fn apply_codex_oversized(
        &mut self,
        part: &[u8],
        thread_id: &str,
        offset: usize,
    ) -> Result<()> {
        let row: Value = serde_json::from_slice(part).map_err(|_| "invalid_transcript")?;
        self.apply_codex(&row, thread_id, offset)
    }

    fn push_codex_record_fallback(
        &mut self,
        label: &str,
        part: &[u8],
        offset: usize,
        reference: String,
    ) -> Result<()> {
        if self
            .entries
            .iter()
            .any(|e| e.record_refs.contains(&reference))
        {
            return Ok(());
        }

        let (label_text, label_truncated) = bounded(label);
        let raw_str = std::str::from_utf8(part).unwrap_or("");
        let (details, details_truncated) = bounded(raw_str);
        self.upsert(Entry {
            id: entry_id("content", &format!("{offset}:0"))?,
            kind: "record".into(),
            text: label_text,
            role: None,
            tool_name: None,
            status: None,
            details: Some(details),
            truncated: details_truncated || label_truncated,
            content_ref: Some(reference.clone()),
            record_refs: vec![reference],
        })
    }

    pub fn preserve_codex_record(
        &mut self,
        part: &[u8],
        _thread_id: &str,
        offset: usize,
    ) -> Result<()> {
        let row: Value = serde_json::from_slice(part).map_err(|_| "invalid_transcript")?;
        let action = classify_codex_record(&row)?;
        let reference = format!("source:{offset}:{}", part.len());
        match action {
            CodexRecordAction::SessionMeta { .. } | CodexRecordAction::Skip => Ok(()),
            CodexRecordAction::Message { id, .. } => {
                let key = entry_id("message", id)?;
                if !self.attach_record(&key, &reference) {
                    self.push_codex_record_fallback("message", part, offset, reference)?;
                }
                Ok(())
            }
            CodexRecordAction::Error { id, .. } => {
                let key = error_entry_id(id, offset)?;
                if !self.attach_record(&key, &reference) {
                    self.push_codex_record_fallback("error", part, offset, reference)?;
                }
                Ok(())
            }
            CodexRecordAction::FunctionCall { call_id, .. }
            | CodexRecordAction::CustomToolCall { call_id, .. }
            | CodexRecordAction::ToolOutput { call_id, .. } => {
                let key = entry_id("tool", call_id)?;
                self.attach_record(&key, &reference);
                Ok(())
            }
            CodexRecordAction::TurnAborted { turn_id } => {
                let id = entry_id("turn_aborted", turn_id)?;
                self.attach_record(&id, &reference);
                Ok(())
            }
            CodexRecordAction::Unknown { label } => {
                self.push_codex_record_fallback(label, part, offset, reference)
            }
        }
    }
}
