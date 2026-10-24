use crate::{Result, claude::Transcript};
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
    path::Path,
};
pub const MAX_SOURCE: u64 = 64 * 1024 * 1024;
pub const MAX_RECORD: usize = 1024 * 1024;
/// Tail the already resolved and verified adapter file; no path lookup occurs.
/// Ignore the first possibly partial line and any unfinished final record.
pub fn read_tail(file: fs::File) -> Result<Vec<serde_json::Value>> {
    read_tail_within(file, 4 * 1024 * 1024)
}

/// Records whose lines lie entirely in the last `limit` bytes of the file.
pub fn read_tail_within(mut file: fs::File, limit: u64) -> Result<Vec<serde_json::Value>> {
    let metadata = file.metadata().map_err(|_| "transcript_unavailable")?;
    if !metadata.is_file() {
        return Err("transcript_unavailable");
    }
    let start = metadata.len().saturating_sub(limit);
    file.seek(SeekFrom::Start(start))
        .map_err(|_| "transcript_unavailable")?;
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes)
        .map_err(|_| "transcript_unavailable")?;
    let mut lines = bytes.split_inclusive(|b| *b == b'\n');
    if start > 0 {
        lines.next();
    }
    Ok(lines.filter(|line| line.last() == Some(&b'\n'))
        .filter_map(|line| serde_json::from_slice(line).ok())
        .collect())
}

#[derive(Clone, Debug, PartialEq)]
struct Stamp {
    dev: u64,
    ino: u64,
    len: u64,
    mtime: (i64, i64),
    ctime: (i64, i64),
}

impl Stamp {
    fn of(m: &fs::Metadata) -> Self {
        Self {
            dev: m.dev(),
            ino: m.ino(),
            len: m.len(),
            mtime: (m.mtime(), m.mtime_nsec()),
            ctime: (m.ctime(), m.ctime_nsec()),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum AgentType {
    #[default]
    Claude,
    Codex,
}

#[derive(Default)]
pub struct Follower {
    pub agent_type: AgentType,
    preserve_content: bool,
    pub transcript: Transcript,
    pub offset: u64,
    pub generation: u64,
    stamp: Option<Stamp>,
    bytes: Vec<u8>,
    session: String,
    buf: Vec<u8>,
}
impl Follower {
    pub fn for_agent(agent_type: AgentType) -> Self {
        Self {
            agent_type,
            ..Self::default()
        }
    }
    pub fn with_content() -> Self {
        Self {
            preserve_content: true,
            ..Self::default()
        }
    }
    pub fn with_content_for_agent(agent_type: AgentType) -> Self {
        Self {
            agent_type,
            preserve_content: true,
            ..Self::default()
        }
    }
    pub fn read_content(&self, reference: &str, offset: usize) -> Result<(Vec<u8>, usize)> {
        if !self.transcript.entries.iter().any(|entry| {
            entry.content_ref.as_deref() == Some(reference)
                || entry.record_refs.iter().any(|r| r == reference)
        }) {
            return Err("content_unavailable");
        }
        let mut parts = reference.split(':');
        if parts.next() != Some("source") {
            return Err("content_unavailable");
        }
        let start = parts
            .next()
            .and_then(|s| s.parse::<usize>().ok())
            .ok_or("content_unavailable")?;
        let length = parts
            .next()
            .and_then(|s| s.parse::<usize>().ok())
            .ok_or("content_unavailable")?;
        if parts.next().is_some() || offset > length {
            return Err("invalid_content_offset");
        }
        let end = start
            .checked_add(length)
            .filter(|end| *end <= self.bytes.len())
            .ok_or("content_unavailable")?;
        let source = &self.bytes[start..end];
        let next = offset.saturating_add(64 * 1024).min(source.len());
        Ok((source[offset..next].to_vec(), source.len()))
    }
    pub fn refresh(&mut self, path: &Path, session: &str) -> Result<bool> {
        self.refresh_file(crate::resolver::open_absolute(path, false)?, session)
    }
    pub fn refresh_file(&mut self, mut file: fs::File, session: &str) -> Result<bool> {
        let before = Stamp::of(&file.metadata().map_err(|_| "transcript_unavailable")?);
        if before.len > MAX_SOURCE {
            return Err("source_too_large");
        }
        // Retaining a bounded prefix lets us distinguish append from any rewrite,
        // including truncate/regrow between polls or edits earlier in the file.
        // Equal metadata stamps can hide same-length rewrites on filesystems with
        // coarse timestamps, so every poll rereads up to MAX_SOURCE bytes. This
        // bounded I/O cost is necessary to verify the cached content is current.
        // Reusing a preallocated buffer avoids bounded allocator churn across
        // frequent idle polls (which drove resident memory up to ~1.28 GB when
        // allocating and discarding large buffers repeatedly under the allocator,
        // rather than an unbounded memory leak). Steady-state memory footprint is
        // bounded by two retained buffers: `buf` (for reading) and `bytes` (prefix
        // cache). Their lengths are capped at MAX_SOURCE (64 MiB); Vec capacity may
        // round upward. Temporary parsing allocations add to that footprint.
        self.buf.clear();
        self.buf.reserve((before.len as usize).saturating_add(1));
        (&mut file)
            .take(MAX_SOURCE + 1)
            .read_to_end(&mut self.buf)
            .map_err(|_| "transcript_unavailable")?;
        if self.buf.len() as u64 > MAX_SOURCE {
            return Err("source_too_large");
        }
        let after = Stamp::of(&file.metadata().map_err(|_| "transcript_unavailable")?);
        if before != after {
            return Err("source_changing");
        }
        let reset = self
            .stamp
            .as_ref()
            .is_none_or(|old| old.dev != before.dev || old.ino != before.ino)
            || self.session != session
            || !self.buf.starts_with(&self.bytes);

        if !reset && self.buf.len() == self.bytes.len() {
            self.stamp = Some(before);
            return Ok(false);
        }

        let mut transcript = if reset {
            Transcript::default()
        } else {
            self.transcript.clone()
        };
        let start = if reset { 0 } else { self.offset as usize };
        let mut offset = start;
        for part in self.buf[start..].split_inclusive(|b| *b == b'\n') {
            if part.last() != Some(&b'\n') {
                break;
            }
            if part != b"\n" {
                let result = match self.agent_type {
                    AgentType::Claude => {
                        if part.len() > MAX_RECORD {
                            transcript.apply_oversized(part, session)
                        } else {
                            let row =
                                serde_json::from_slice(part).map_err(|_| "invalid_transcript")?;
                            transcript.apply(&row, session)
                        }
                    }
                    AgentType::Codex => {
                        if part.len() > MAX_RECORD {
                            transcript.apply_codex_oversized(part, session, offset)
                        } else {
                            let row =
                                serde_json::from_slice(part).map_err(|_| "invalid_transcript")?;
                            transcript.apply_codex(&row, session, offset)
                        }
                    }
                };
                // Valid but unfamiliar message shapes remain inspectable as raw
                // records. Ownership/security and resource-limit errors still fail.
                if let Err(code) = result {
                    if code != "invalid_transcript" || !self.preserve_content {
                        return Err(code);
                    }
                }
                if self.preserve_content {
                    match self.agent_type {
                        AgentType::Claude => transcript.preserve_record(part, session, offset)?,
                        AgentType::Codex => {
                            transcript.preserve_codex_record(part, session, offset)?
                        }
                    }
                }
            }
            offset += part.len();
        }
        let changed = reset
            || transcript.entries != self.transcript.entries
            || transcript.unknown_records != self.transcript.unknown_records;
        if reset {
            self.generation += 1;
            self.bytes.clear();
            self.bytes.extend_from_slice(&self.buf[..offset]);
        } else {
            self.bytes
                .extend_from_slice(&self.buf[self.bytes.len()..offset]);
        }
        self.offset = offset as u64;
        self.transcript = transcript;
        self.stamp = Some(before);
        self.session = session.to_owned();
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_length_rewrite_is_detected_when_metadata_stamps_collide() {
        struct TempFile(std::path::PathBuf);
        impl Drop for TempFile {
            fn drop(&mut self) {
                let _ = fs::remove_file(&self.0);
            }
        }
        let path = TempFile(std::env::temp_dir().join(format!(
            "hc-follower-stamp-collision-{}",
            std::process::id()
        )));
        let session = "16246b9d-4c3d-54fc-b6d3-2a765d418491";
        let record = |text| {
            format!(
                "{}\n",
                serde_json::json!({"type":"user","uuid":"same","sessionId":session,
                    "message":{"role":"user","content":text}})
            )
        };
        let old = record("old");
        let new = record("new");
        assert_eq!(old.len(), new.len());
        fs::write(&path.0, &old).unwrap();
        let source_path = path.0.canonicalize().unwrap();
        let mut follower = Follower::default();
        assert!(follower.refresh(&source_path, session).unwrap());
        assert_eq!(follower.transcript.entries[0].text, "old");
        let generation = follower.generation;
        fs::write(&path.0, &new).unwrap();
        // Model a filesystem whose timestamp resolution cannot distinguish these
        // same-length writes, while leaving the cached source bytes untouched.
        follower.stamp = Some(Stamp::of(&fs::metadata(&path.0).unwrap()));
        assert!(follower.refresh(&source_path, session).unwrap());
        assert_eq!(follower.transcript.entries.len(), 1);
        assert_eq!(follower.transcript.entries[0].text, "new");
        assert_eq!(follower.generation, generation + 1);
        assert!(!follower.refresh(&source_path, session).unwrap());
        assert_eq!(follower.generation, generation + 1);
    }
}
