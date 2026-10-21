//! Manage non-deciding PermissionRequest hooks without shifting existing trust keys.
use crate::codex_config;
use serde_json::{Value, json};
use std::{
    fs,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Agent {
    Claude,
    Codex,
}
impl Agent {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }
    fn filename(self) -> &'static str {
        match self {
            Self::Claude => "settings.json",
            Self::Codex => "hooks.json",
        }
    }
}

#[derive(Debug)]
pub struct HooksResult(Value);
impl HooksResult {
    pub fn error(code: &'static str) -> Self {
        Self(json!({"error":code}))
    }
    fn state(state: &str, confirmed: bool) -> Self {
        Self(json!({"status":state,"spool_confirmed":confirmed}))
    }
    pub fn to_json(&self) -> String {
        self.0.to_string()
    }
}
type Result<T> = std::result::Result<T, &'static str>;

pub fn resolve_agent_dir(agent: Agent) -> Option<PathBuf> {
    match agent {
        Agent::Codex => codex_config::resolve_codex_dir(),
        Agent::Claude => std::env::var_os("CLAUDE_CONFIG_DIR")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".claude"))),
    }
}
pub fn resolve_spool_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/state")))
        .map(|p| p.join("herdcats-bridge/requests"))
}
fn dispatch(agent: Agent, action: fn(Agent, &Path, &Path) -> HooksResult) -> HooksResult {
    match (resolve_agent_dir(agent), resolve_spool_dir()) {
        (Some(dir), Some(spool)) => action(agent, &dir, &spool),
        _ => HooksResult::error("home_required"),
    }
}
pub fn install(agent: Agent) -> HooksResult {
    dispatch(agent, install_in_dir)
}
pub fn status(agent: Agent) -> HooksResult {
    dispatch(agent, status_in_dir)
}
pub fn uninstall(agent: Agent) -> HooksResult {
    dispatch(agent, uninstall_in_dir)
}

fn shell_quote(path: &str) -> String {
    if path
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/_-+.".contains(c))
    {
        path.into()
    } else {
        format!("'{}'", path.replace('\'', "'\\''"))
    }
}
pub fn recorder_command(agent: Agent) -> Result<String> {
    let exe = std::env::current_exe()
        .and_then(fs::canonicalize)
        .map_err(|_| "executable_unavailable")?;
    let exe = exe.to_str().ok_or("executable_unavailable")?;
    Ok(format!(
        "{} hook record --agent {}",
        shell_quote(exe),
        agent.name()
    ))
}
fn ours(handler: &Value, agent: Agent) -> bool {
    let Some(cmd) = handler.get("command").and_then(Value::as_str) else {
        return false;
    };
    let suffix = format!(" hook record --agent {}", agent.name());
    let Some(prefix) = cmd.strip_suffix(&suffix) else {
        return false;
    };
    // Accept only a single executable path, never shell pipelines or wrappers.
    let path = if prefix.starts_with('\'') && prefix.ends_with('\'') && prefix.len() >= 2 {
        prefix[1..prefix.len() - 1].replace("'\\''", "'")
    } else {
        prefix.to_string()
    };
    Path::new(&path).is_absolute() && shell_quote(&path) == prefix
}
fn recorder_handler(command: &str) -> Value {
    json!({"type":"command", "command":command, "timeout":10})
}
fn recorder_matcher_valid(agent: Agent, group: &Value) -> bool {
    match agent {
        Agent::Claude => group.get("matcher").and_then(Value::as_str) == Some("*"),
        Agent::Codex => group.get("matcher").is_none(),
    }
}
fn recorder_valid(agent: Agent, group: &Value, handler: &Value, command: &str) -> bool {
    recorder_matcher_valid(agent, group) && *handler == recorder_handler(command)
}
fn repair_matcher(agent: Agent, group: &mut Value) {
    if agent == Agent::Claude {
        group["matcher"] = json!("*");
    } else {
        group.as_object_mut().unwrap().shift_remove("matcher");
    }
}
fn read_document(path: &Path) -> Result<(Value, Option<fs::Metadata>)> {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => return Err("hooks_symlink"),
        Ok(m) if m.is_file() => Some(m),
        Ok(_) => return Err("hooks_read_failed"),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(_) => return Err("hooks_read_failed"),
    };
    let doc = if meta.is_some() {
        serde_json::from_slice(&fs::read(path).map_err(|_| "hooks_read_failed")?)
            .map_err(|_| "hooks_invalid_json")?
    } else {
        json!({})
    };
    if !doc.is_object() {
        return Err("hooks_invalid_json");
    }
    if let Some(hooks) = doc.get("hooks") {
        if !hooks.is_object() {
            return Err("hooks_invalid_json");
        }
        if let Some(groups) = hooks.get("PermissionRequest") {
            let groups = groups.as_array().ok_or("hooks_invalid_json")?;
            for group in groups {
                if !group.is_object() || !group.get("hooks").is_some_and(Value::is_array) {
                    return Err("hooks_invalid_json");
                }
            }
        }
    }
    Ok((doc, meta))
}
fn groups(doc: &Value) -> &[Value] {
    doc.get("hooks")
        .and_then(|h| h.get("PermissionRequest"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}
fn spool_files(agent: Agent, dir: &Path) -> io::Result<Vec<PathBuf>> {
    match fs::symlink_metadata(dir) {
        Ok(m) if m.file_type().is_symlink() => return Err(io::Error::other("spool_symlink")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e),
        _ => (),
    }
    let prefix = format!("{}-", agent.name());
    let mut files = vec![];
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if let Some(session) = name
            .strip_prefix(&prefix)
            .and_then(|s| s.strip_suffix(".jsonl"))
        {
            if !session.is_empty()
                && session
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
                && entry.file_type()?.is_file()
            {
                files.push(entry.path());
            }
        }
    }
    Ok(files)
}
fn spool_confirmed(agent: Agent, dir: &Path) -> bool {
    spool_files(agent, dir)
        .unwrap_or_default()
        .iter()
        .any(|path| {
            let Ok(file) = fs::File::open(path) else {
                return false;
            };
            io::BufReader::new(file)
                .lines()
                .map_while(std::result::Result::ok)
                .any(|line| {
                    serde_json::from_str::<Value>(&line).ok().is_some_and(|v| {
                        v.get("agent").and_then(Value::as_str) == Some(agent.name())
                    })
                })
        })
}
pub fn status_in_dir(agent: Agent, dir: &Path, spool: &Path) -> HooksResult {
    match status_inner(agent, dir, spool) {
        Ok(r) => r,
        Err(e) => HooksResult::error(e),
    }
}
fn status_inner(agent: Agent, dir: &Path, spool: &Path) -> Result<HooksResult> {
    let path = dir.join(agent.filename());
    let (doc, _) = read_document(&path)?;
    let command = recorder_command(agent)?;
    let confirmed = spool_confirmed(agent, spool);
    // Status drives cleanup decisions: inspect all owned entries before
    // returning the current recorder's trust state, regardless of their order.
    let owned: Vec<_> = groups(&doc)
        .iter()
        .flat_map(|group| {
            group["hooks"]
                .as_array()
                .unwrap()
                .iter()
                .map(move |handler| (group, handler))
        })
        .filter(|(_, handler)| ours(handler, agent))
        .collect();
    if owned.len() > 1
        || owned
            .iter()
            .any(|(group, handler)| !recorder_valid(agent, group, handler, &command))
    {
        return Ok(HooksResult::state("stale", confirmed));
    }
    for (g, group) in groups(&doc).iter().enumerate() {
        for (h, handler) in group["hooks"]
            .as_array()
            .ok_or("hooks_invalid_json")?
            .iter()
            .enumerate()
        {
            if !ours(handler, agent) {
                continue;
            }
            let state = if agent == Agent::Claude {
                "installed"
            } else {
                let absolute = if path.is_absolute() {
                    path.clone()
                } else {
                    std::env::current_dir()
                        .map_err(|_| "hooks_read_failed")?
                        .join(&path)
                };
                let key = format!("{}:permission_request:{g}:{h}", absolute.display());
                let text = match fs::read_to_string(dir.join("config.toml")) {
                    Ok(t) => t,
                    Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
                    Err(_) => return Err("hooks_config_unreadable"),
                };
                let config: toml_edit::DocumentMut =
                    text.parse().map_err(|_| "hooks_config_invalid")?;
                let table = config
                    .get("hooks")
                    .and_then(|i| i.get("state"))
                    .and_then(|i| i.get(&key))
                    .filter(|i| i.is_table_like());
                if table
                    .and_then(|i| i.get("enabled"))
                    .and_then(|i| i.as_bool())
                    == Some(false)
                {
                    "disabled"
                } else if table.and_then(|i| i.get("trusted_hash")).is_some() {
                    "installed"
                } else {
                    "needs_trust"
                }
            };
            return Ok(HooksResult::state(state, confirmed));
        }
    }
    Ok(HooksResult::state("missing", confirmed))
}
pub fn install_in_dir(agent: Agent, dir: &Path, spool: &Path) -> HooksResult {
    match install_inner(agent, dir, spool) {
        Ok(r) => r,
        Err(e) => HooksResult::error(e),
    }
}
fn install_inner(agent: Agent, dir: &Path, spool: &Path) -> Result<HooksResult> {
    let path = dir.join(agent.filename());
    let (mut doc, meta) = read_document(&path)?;
    let command = recorder_command(agent)?;
    let owned: Vec<_> = groups(&doc)
        .iter()
        .enumerate()
        .flat_map(|(g, group)| {
            group["hooks"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .map(move |(h, handler)| ((g, h), group, handler))
        })
        .filter(|(_, _, handler)| ours(handler, agent))
        .collect();
    if owned.len() == 1 && recorder_valid(agent, owned[0].1, owned[0].2, &command) {
        return Ok(HooksResult::state(
            "installed",
            spool_confirmed(agent, spool),
        ));
    }
    // A restrictive group shared with foreign handlers cannot be broadened.
    // Move its recorder to a new unrestricted group instead.
    let repairable = |group: &Value| {
        recorder_matcher_valid(agent, group)
            || group["hooks"]
                .as_array()
                .unwrap()
                .iter()
                .all(|h| ours(h, agent))
    };
    let selected = owned
        .iter()
        .find(|(_, group, h)| repairable(group) && h["command"].as_str() == Some(&command))
        .or_else(|| owned.iter().find(|(_, group, _)| repairable(group)))
        .map(|(position, _, _)| *position);
    let invalidate_selected = owned.iter().any(|(position, group, handler)| {
        Some(*position) == selected && !recorder_valid(agent, group, handler, &command)
    });
    let hooks = doc
        .as_object_mut()
        .ok_or("hooks_invalid_json")?
        .entry("hooks")
        .or_insert_with(|| json!({}));
    let groups = hooks
        .as_object_mut()
        .ok_or("hooks_invalid_json")?
        .entry("PermissionRequest")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or("hooks_invalid_json")?;
    let mut replaced = false;
    let mut positions = Vec::new();
    let mut invalidated = Vec::new();
    let mut old_group = 0;
    let mut new_group = 0;
    groups.retain_mut(|group| {
        let mut old_handler = 0;
        let mut new_handler = 0;
        // Retain the selected recorder when its group can safely be repaired.
        let handlers = group["hooks"].as_array_mut().unwrap();
        let before = handlers.len();
        handlers.retain_mut(|handler| {
            let old = (old_group, old_handler);
            old_handler += 1;
            let keep = if !ours(handler, agent) {
                true
            } else if selected == Some(old) {
                if invalidate_selected {
                    invalidated.push((new_group, new_handler));
                }
                *handler = recorder_handler(&command);
                replaced = true;
                true
            } else {
                false
            };
            positions.push((old, keep.then_some((new_group, new_handler))));
            if keep {
                new_handler += 1;
            }
            keep
        });
        let keep = !(handlers.len() != before && handlers.is_empty());
        if selected.is_some_and(|(g, _)| g == old_group) {
            repair_matcher(agent, group);
        }
        old_group += 1;
        if keep {
            new_group += 1;
        }
        keep
    });
    if !replaced {
        let mut group = json!({"hooks":[recorder_handler(&command)]});
        repair_matcher(agent, &mut group);
        groups.push(group);
    }
    let backup = write_hook_edit(agent, &path, &doc, meta.as_ref(), &positions, &invalidated)?;
    let mut result = HooksResult::state("installed", spool_confirmed(agent, spool));
    if let Some(backup) = backup {
        result.0["backup"] = json!(backup);
    }
    Ok(result)
}
pub fn uninstall_in_dir(agent: Agent, dir: &Path, spool: &Path) -> HooksResult {
    match uninstall_inner(agent, dir, spool) {
        Ok(r) => r,
        Err(e) => HooksResult::error(e),
    }
}
fn uninstall_inner(agent: Agent, dir: &Path, spool: &Path) -> Result<HooksResult> {
    let path = dir.join(agent.filename());
    let (mut doc, meta) = read_document(&path)?;
    let mut changed = false;
    let mut positions = Vec::new();
    if let Some(groups) = doc
        .get_mut("hooks")
        .and_then(|h| h.get_mut("PermissionRequest"))
        .and_then(Value::as_array_mut)
    {
        let mut old_group = 0;
        let mut new_group = 0;
        groups.retain_mut(|group| {
            let handlers = group["hooks"].as_array_mut().unwrap();
            let before = handlers.len();
            let mut old_handler = 0;
            let mut new_handler = 0;
            handlers.retain(|handler| {
                let keep = !ours(handler, agent);
                positions.push((
                    (old_group, old_handler),
                    keep.then_some((new_group, new_handler)),
                ));
                old_handler += 1;
                if keep {
                    new_handler += 1;
                }
                keep
            });
            let removed = handlers.len() != before;
            changed |= removed;
            let keep = !(removed && handlers.is_empty());
            old_group += 1;
            if keep {
                new_group += 1;
            }
            keep
        });
    }
    if changed {
        let hooks = doc.get_mut("hooks").and_then(Value::as_object_mut).unwrap();
        if hooks
            .get("PermissionRequest")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
        {
            hooks.shift_remove("PermissionRequest");
        }
    }
    let backup = if changed {
        write_hook_edit(agent, &path, &doc, meta.as_ref(), &positions, &[])?
    } else {
        None
    };
    let _spool_lock = crate::spool::lock_existing(spool).map_err(|_| "spool_cleanup_failed")?;
    for file in spool_files(agent, spool).map_err(|_| "spool_cleanup_failed")? {
        fs::remove_file(file).map_err(|_| "spool_cleanup_failed")?;
    }
    let mut result = HooksResult::state("missing", false);
    if let Some(backup) = backup {
        result.0["backup"] = json!(backup);
    }
    Ok(result)
}
type Position = (usize, usize);
type PositionChange = (Position, Option<Position>);

// Trust state belongs to a handler, not its transient array index. Snapshot
// every source value before moving keys so overlapping moves cannot clobber it.
fn write_hook_edit(
    agent: Agent,
    path: &Path,
    doc: &Value,
    meta: Option<&fs::Metadata>,
    positions: &[PositionChange],
    invalidated: &[Position],
) -> Result<Option<String>> {
    if agent != Agent::Codex {
        return write_document(agent, path, doc, meta);
    }
    let fresh: Vec<_> = groups(doc)
        .iter()
        .enumerate()
        .flat_map(|(g, group)| {
            group["hooks"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .map(move |(h, _)| (g, h))
        })
        .filter(|position| !positions.iter().any(|(_, new)| *new == Some(*position)))
        .collect();
    if fresh.is_empty()
        && invalidated.is_empty()
        && positions.iter().all(|(old, new)| Some(*old) == *new)
    {
        return write_document(agent, path, doc, meta);
    }
    let config_path = path
        .parent()
        .ok_or("hooks_write_failed")?
        .join("config.toml");
    let config_meta = match fs::symlink_metadata(&config_path) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => meta,
        Ok(_) => return Err("hooks_config_unreadable"),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return write_document(agent, path, doc, meta);
        }
        Err(_) => return Err("hooks_config_unreadable"),
    };
    let original = fs::read_to_string(&config_path).map_err(|_| "hooks_config_unreadable")?;
    let mut config: toml_edit::DocumentMut =
        original.parse().map_err(|_| "hooks_config_invalid")?;
    let Some(state) = config
        .get_mut("hooks")
        .and_then(|h| h.get_mut("state"))
        .and_then(|s| s.as_table_like_mut())
    else {
        return write_document(agent, path, doc, meta);
    };
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|_| "hooks_read_failed")?
            .join(path)
    };
    let key = |(g, h)| format!("{}:permission_request:{g}:{h}", absolute.display());
    let values: Vec<_> = positions
        .iter()
        .map(|(old, new)| (*new, state.remove(&key(*old))))
        .collect();
    // An orphaned destination must not become state for an unrelated handler
    // that had no source entry. Clear only after all source values are saved.
    for (_, new) in positions {
        if let Some(new) = new {
            state.remove(&key(*new));
        }
    }
    // New/relocated recorders have no source state and must not inherit an
    // orphaned trust or disabled entry at their newly allocated position.
    for position in fresh {
        state.remove(&key(position));
    }
    for (new, value) in values {
        if let (Some(new), Some(mut value)) = (new, value) {
            if invalidated.contains(&new) {
                // Trust fingerprints the handler definition and matcher. Keep
                // enabled/disabled and other state, but require fresh trust.
                if let Some(table) = value.as_table_like_mut() {
                    table.remove("trusted_hash");
                    table.remove("trusted_content_hash");
                }
            }
            state.insert(&key(new), value);
        }
    }
    let updated = config.to_string();
    if updated == original {
        return write_document(agent, path, doc, meta);
    }
    // Both files retain backups. Roll back config only if hooks were not
    // replaced; post-rename verification failure must retain the new layout.
    write_bytes_document(
        Agent::Claude,
        &config_path,
        updated.as_bytes(),
        Some(&config_meta),
    )?;
    match write_document(agent, path, doc, meta) {
        Ok(backup) => Ok(backup),
        Err("hooks_verification_failed") => Err("hooks_verification_failed"),
        Err(error) => {
            write_bytes_document(
                Agent::Claude,
                &config_path,
                original.as_bytes(),
                Some(&config_meta),
            )
            .map_err(|_| "hooks_config_rollback_failed")?;
            Err(error)
        }
    }
}

fn write_document(
    agent: Agent,
    path: &Path,
    doc: &Value,
    meta: Option<&fs::Metadata>,
) -> Result<Option<String>> {
    let mut bytes = serde_json::to_vec_pretty(doc).map_err(|_| "hooks_write_failed")?;
    bytes.push(b'\n');
    write_bytes_document(agent, path, &bytes, meta)
}
fn write_bytes_document(
    agent: Agent,
    path: &Path,
    bytes: &[u8],
    meta: Option<&fs::Metadata>,
) -> Result<Option<String>> {
    let filename = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("hooks_write_failed")?;
    let dir = path.parent().ok_or("hooks_write_failed")?;
    fs::create_dir_all(dir).map_err(|_| "hooks_write_failed")?;
    let backup = if let Some(meta) = meta {
        let timestamp = codex_config::utc_timestamp_string();
        let mut count = 0;
        loop {
            let suffix = if count == 0 {
                String::new()
            } else {
                format!("_{count}")
            };
            let name = format!("{}.bak.herdcats-{timestamp}{suffix}", filename);
            let backup_path = dir.join(&name);
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&backup_path) {
                Ok(mut file) => {
                    file.set_permissions(meta.permissions())
                        .map_err(|_| "hooks_write_failed")?;
                    let data = fs::read(path).map_err(|_| "hooks_read_failed")?;
                    file.write_all(&data)
                        .and_then(|_| file.sync_all())
                        .map_err(|_| "hooks_write_failed")?;
                    break Some(name);
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => count += 1,
                Err(_) => return Err("hooks_write_failed"),
            }
        }
    } else {
        None
    };
    let mut count = 0;
    let (temp, mut file) = loop {
        let temp = dir.join(format!(
            ".{}.tmp.herdcats-{}-{count}",
            filename,
            std::process::id()
        ));
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(if agent == Agent::Claude { 0o600 } else { 0o644 });
        }
        match opts.open(&temp) {
            Ok(f) => break (temp, f),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => count += 1,
            Err(_) => return Err("hooks_write_failed"),
        }
    };
    let write = (|| -> io::Result<()> {
        if let Some(meta) = meta {
            file.set_permissions(meta.permissions())?;
        } else {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(fs::Permissions::from_mode(if agent == Agent::Claude {
                    0o600
                } else {
                    0o644
                }))?;
            }
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        // Refuse a symlink substituted since the initial read, too.
        if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(io::Error::other("hooks_symlink"));
        }
        fs::rename(&temp, path)
    })();
    if write.is_err() {
        let _ = fs::remove_file(temp);
        return Err("hooks_write_failed");
    }
    let verified = fs::read(path).ok().is_some_and(|data| data == bytes);
    if !verified {
        return Err("hooks_verification_failed");
    }
    Ok(backup)
}
