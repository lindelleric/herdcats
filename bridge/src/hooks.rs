//! Manage non-deciding PermissionRequest hooks without shifting existing trust keys.
use crate::codex_config;
use serde_json::{json, Value};
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
    let mut stale = false;
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
            if handler["command"].as_str() != Some(&command) {
                stale = true;
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
    Ok(HooksResult::state(
        if stale { "stale" } else { "missing" },
        confirmed,
    ))
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
    if groups(&doc).iter().any(|g| {
        g["hooks"].as_array().is_some_and(|hs| {
            hs.iter()
                .any(|h| h.get("command").and_then(Value::as_str) == Some(&command))
        })
    }) {
        return Ok(HooksResult::state(
            "installed",
            spool_confirmed(agent, spool),
        ));
    }
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
    groups.retain_mut(|group| {
        // Keep the first recorder in place so its trust key stays stable.
        let handlers = group["hooks"].as_array_mut().unwrap();
        let before = handlers.len();
        handlers.retain_mut(|handler| {
            if !ours(handler, agent) {
                return true;
            }
            if replaced {
                return false;
            }
            handler["command"] = Value::String(command.clone());
            replaced = true;
            true
        });
        !(handlers.len() != before && handlers.is_empty())
    });
    if !replaced {
        let mut group = json!({"hooks":[{"type":"command", "command":command,"timeout":10}]});
        if agent == Agent::Claude {
            group
                .as_object_mut()
                .unwrap()
                .insert("matcher".into(), json!("*"));
        }
        groups.push(group);
    }
    let backup = write_document(agent, &path, &doc, meta.as_ref())?;
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
    if let Some(groups) = doc
        .get_mut("hooks")
        .and_then(|h| h.get_mut("PermissionRequest"))
        .and_then(Value::as_array_mut)
    {
        groups.retain_mut(|group| {
            let handlers = group["hooks"].as_array_mut().unwrap();
            let before = handlers.len();
            handlers.retain(|h| !ours(h, agent));
            let removed = handlers.len() != before;
            changed |= removed;
            !(removed && handlers.is_empty())
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
        write_document(agent, &path, &doc, meta.as_ref())?
    } else {
        None
    };
    for file in spool_files(agent, spool).map_err(|_| "spool_cleanup_failed")? {
        fs::remove_file(file).map_err(|_| "spool_cleanup_failed")?;
    }
    let mut result = HooksResult::state("missing", false);
    if let Some(backup) = backup {
        result.0["backup"] = json!(backup);
    }
    Ok(result)
}
fn write_document(
    agent: Agent,
    path: &Path,
    doc: &Value,
    meta: Option<&fs::Metadata>,
) -> Result<Option<String>> {
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
            let name = format!("{}.bak.herdcats-{timestamp}{suffix}", agent.filename());
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
    let mut bytes = serde_json::to_vec_pretty(doc).map_err(|_| "hooks_write_failed")?;
    bytes.push(b'\n');
    let mut count = 0;
    let (temp, mut file) = loop {
        let temp = dir.join(format!(
            ".{}.tmp.herdcats-{}-{count}",
            agent.filename(),
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
        file.write_all(&bytes)?;
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
    let verified = fs::read(path).ok().is_some_and(|data| {
        data == bytes && serde_json::from_slice::<Value>(&data).is_ok_and(|v| v == *doc)
    });
    if !verified {
        return Err("hooks_verification_failed");
    }
    Ok(backup)
}
