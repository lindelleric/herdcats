//! Best-effort, bounded storage for non-deciding agent hook requests.
use crate::hooks::Agent;
pub use crate::hooks::resolve_spool_dir;
use serde::de::{IgnoredAny, MapAccess, Visitor};
use serde_json::{Value, json};
use std::{
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

const INPUT_LIMIT: usize = 1024 * 1024;
const BYTE_LIMIT: usize = 256 * 1024;
const RECORD_LIMIT: usize = 64;

fn valid_session_id(session: &str) -> bool {
    !session.is_empty()
        && session.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}

/// Read complete records without creating or modifying any spool state.
/// Any unavailable, unsafe, or oversized source yields no requests.
pub fn read_session(dir: &Path, agent: Agent, session_id: &str) -> Vec<Value> {
    fn read(dir: &Path, agent: Agent, session_id: &str) -> io::Result<Vec<Value>> {
        if !valid_session_id(session_id) {
            return Ok(Vec::new());
        }
        let metadata = fs::symlink_metadata(dir)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Ok(Vec::new());
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = options.open(dir.join(format!("{}-{session_id}.jsonl", agent.name())))?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.len() > BYTE_LIMIT as u64 {
            return Ok(Vec::new());
        }
        let mut bytes = Vec::new();
        file.take((BYTE_LIMIT + 1) as u64).read_to_end(&mut bytes)?;
        if bytes.len() > BYTE_LIMIT {
            return Ok(Vec::new());
        }
        let mut records = std::collections::VecDeque::new();
        for line in bytes.split_inclusive(|b| *b == b'\n') {
            if line.last() != Some(&b'\n') {
                continue;
            }
            if let Ok(record) = serde_json::from_slice(line) {
                if records.len() == RECORD_LIMIT {
                    records.pop_front();
                }
                records.push_back(record);
            }
        }
        Ok(records.into_iter().collect())
    }
    read(dir, agent, session_id).unwrap_or_default()
}

/// Called only behind the binary's silent panic boundary. Bad arguments and
/// non-Herdr sessions still consume stdin, but never create spool files.
pub fn record_cli() {
    let args: Vec<_> = std::env::args_os().skip(2).collect();
    let agent = match args.as_slice() {
        [action, flag, agent] if action == "record" && flag == "--agent" => {
            agent.to_str().and_then(Agent::parse)
        }
        _ => None,
    };
    let mut input = io::stdin().lock();
    if std::env::var_os("HERDR_ENV").as_deref() != Some(OsStr::new("1")) || agent.is_none() {
        let _ = io::copy(&mut input, &mut io::sink());
        return;
    }
    if let (Some(agent), Some(dir)) = (agent, resolve_spool_dir()) {
        if agent == Agent::Codex && !codex_recording_safe() {
            let _ = io::copy(&mut input, &mut io::sink());
            return;
        }
        let pane = std::env::var("HERDR_PANE_ID").ok();
        let _ = record_in_dir(agent, &dir, &mut input, pane.as_deref());
    } else {
        let _ = io::copy(&mut input, &mut io::sink());
    }
}

// Shared daemons inherit the launching pane's environment. Refuse to attribute
// their requests, including when process identity or configuration is unknown.
fn codex_recording_safe() -> bool {
    use crate::codex_config::{self, ProcessStatus};
    let Some(dir) = codex_config::resolve_codex_dir() else {
        return false;
    };
    let status = codex_config::daemon_status_in_dir(&dir);
    status.auto_start == Some(false)
        && status.daemon == ProcessStatus::NotRunning
        && status.updater == ProcessStatus::NotRunning
}

/// Record at most one request. All errors are returned to the silent caller.
pub fn record_in_dir(
    agent: Agent,
    dir: &Path,
    mut input: impl Read,
    pane: Option<&str>,
) -> io::Result<()> {
    let mut bytes = Vec::new();
    input
        .by_ref()
        .take((INPUT_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)?;
    let oversized = bytes.len() > INPUT_LIMIT;
    let (payload, session, event) = if oversized {
        io::copy(&mut input, &mut io::sink())?;
        // Only retain metadata fully present in the bounded prefix. The visitor
        // skips other fields without allocating their values, even on EOF.
        let mut metadata = Metadata::default();
        let mut decoder = serde_json::Deserializer::from_slice(&bytes[..INPUT_LIMIT]);
        let _ = serde::de::Deserializer::deserialize_map(&mut decoder, &mut metadata);
        let Some(session) = metadata.session else {
            return Ok(());
        };
        (json!({"truncated":true}), session, metadata.event)
    } else {
        let Ok(payload) = serde_json::from_slice::<Value>(&bytes) else {
            return Ok(());
        };
        let Some(session) = payload
            .get("session_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return Ok(());
        };
        let event = payload
            .get("hook_event_name")
            .and_then(Value::as_str)
            .map(str::to_owned);
        (payload, session, event)
    };
    if !valid_session_id(&session) {
        return Ok(());
    }
    let mut record = json!({
        "v":1,
        "t":SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64(),
        "agent":agent.name(), "event":event, "session_id":session, "pane":pane, "payload":payload
    });
    let mut line = serde_json::to_vec(&record)?;
    if line.len() + 1 > BYTE_LIMIT {
        // A single input can be below the stdin cap and still exceed the file
        // budget. Keep the newest request as a marker rather than evicting it.
        record["payload"] = json!({"truncated":true});
        line = serde_json::to_vec(&record)?;
    }
    if line.len() + 1 > BYTE_LIMIT {
        return Ok(());
    }
    line.push(b'\n');
    ensure_dir(dir)?;
    let path = dir.join(format!("{}-{session}.jsonl", agent.name()));
    // One stable lock serializes append/replace with prune and uninstall.
    // Its inode is retained, but storage stays constant across session IDs.
    let _lock = lock_existing(dir)?;
    let mut file = open_file(&path, false)?;
    file.write_all(&line)?;
    trim(dir, &path, &mut file)
}

/// Lock an existing spool directory without creating it on a cleanup/no-op path.
/// Never delete the lock file: waiters must always synchronize on one inode.
pub(crate) fn lock_existing(dir: &Path) -> io::Result<Option<SpoolLock>> {
    let metadata = match fs::symlink_metadata(dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(io::Error::other("spool_symlink_or_not_directory"));
    }
    SpoolLock::acquire(&dir.join(".spool.lock")).map(Some)
}

pub(crate) struct SpoolLock(File);
impl SpoolLock {
    fn acquire(path: &Path) -> io::Result<Self> {
        let file = open_file(path, false)?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            loop {
                // SAFETY: file owns a valid descriptor for the lock's lifetime.
                if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
                    break;
                }
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error);
                }
            }
        }
        #[cfg(not(unix))]
        return Err(io::Error::other("spool_lock_unsupported"));
        Ok(Self(file))
    }
}
impl Drop for SpoolLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // SAFETY: the descriptor remains valid until after Drop returns.
            unsafe {
                libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
            }
        }
    }
}

#[derive(Default)]
struct Metadata {
    session: Option<String>,
    event: Option<String>,
}
impl<'de> Visitor<'de> for &mut Metadata {
    type Value = ();
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a hook input object")
    }
    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<(), M::Error> {
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "session_id" => self.session = Some(map.next_value::<String>()?),
                "hook_event_name" => self.event = Some(map.next_value::<String>()?),
                _ => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        Ok(())
    }
}

fn ensure_dir(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)?;
    let metadata = fs::symlink_metadata(dir)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(io::Error::other("spool_symlink_or_not_directory"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn open_file(path: &Path, create_new: bool) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).append(true);
    if create_new {
        options.create_new(true);
    } else {
        options.create(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("spool_not_file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

fn trim(dir: &Path, path: &Path, file: &mut File) -> io::Result<()> {
    let len = file.metadata()?.len();
    // Inspect only a bounded tail even if an existing file is unexpectedly huge.
    let start = len.saturating_sub((BYTE_LIMIT + 1) as u64);
    file.seek(SeekFrom::Start(start))?;
    let mut tail = Vec::new();
    file.take((BYTE_LIMIT + 1) as u64).read_to_end(&mut tail)?;
    let mut begin = if start > 0 {
        match tail.iter().position(|b| *b == b'\n') {
            Some(n) => n + 1,
            None => tail.len(),
        }
    } else {
        0
    };
    let count = tail[begin..].iter().filter(|b| **b == b'\n').count();
    if start == 0 && len <= BYTE_LIMIT as u64 && count <= RECORD_LIMIT {
        return Ok(());
    }
    let mut remaining = count;
    while remaining > RECORD_LIMIT || tail.len() - begin > BYTE_LIMIT {
        let Some(n) = tail[begin..].iter().position(|b| *b == b'\n') else {
            begin = tail.len();
            break;
        };
        begin += n + 1;
        remaining = remaining.saturating_sub(1);
    }
    let mut counter = 0;
    let (temp, mut output) = loop {
        let temp = dir.join(format!(".spool.tmp-{}-{counter}", std::process::id()));
        match open_file(&temp, true) {
            Ok(file) => break (temp, file),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => counter += 1,
            Err(e) => return Err(e),
        }
    };
    let result = (|| {
        output.write_all(&tail[begin..])?;
        output.sync_all()?;
        drop(output);
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

/// Delete regular session spool files strictly older than 24 hours. Never follow
/// a symlinked spool directory or treat temporary/unrelated files as records.
pub fn prune(dir: &Path, now: SystemTime) -> io::Result<()> {
    let Some(_lock) = lock_existing(dir)? else {
        return Ok(());
    };
    let max_age = std::time::Duration::from_secs(24 * 60 * 60);
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        // Every trim holds this same lock while its temporary file exists.
        // Thus any regular trim temporary visible here is abandoned.
        if name.starts_with(".spool.tmp-") {
            if entry.file_type()?.is_file() {
                fs::remove_file(entry.path())?;
            }
            continue;
        }
        let Some(session) = name
            .strip_prefix("claude-")
            .or_else(|| name.strip_prefix("codex-"))
            .and_then(|name| name.strip_suffix(".jsonl"))
        else {
            continue;
        };
        if !valid_session_id(session) {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.is_file()
            && now
                .duration_since(metadata.modified()?)
                .is_ok_and(|age| age > max_age)
        {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}
