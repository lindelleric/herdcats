use crate::{Result, claude::Transcript};
use std::{fs, io::Read, os::unix::fs::MetadataExt, path::Path};
const MAX_SOURCE: u64 = 64 * 1024 * 1024;
pub const MAX_RECORD: usize = 1024 * 1024;
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
#[derive(Default)]
pub struct Follower {
    pub transcript: Transcript,
    pub offset: u64,
    pub generation: u64,
    stamp: Option<Stamp>,
    bytes: Vec<u8>,
    session: String,
}
impl Follower {
    pub fn refresh(&mut self, path: &Path, session: &str) -> Result<bool> {
        self.refresh_file(crate::resolver::open_absolute(path, false)?, session)
    }
    pub fn refresh_file(&mut self, mut file: fs::File, session: &str) -> Result<bool> {
        let before = Stamp::of(&file.metadata().map_err(|_| "transcript_unavailable")?);
        if before.len > MAX_SOURCE {
            return Err("source_too_large");
        }
        if self.stamp.as_ref() == Some(&before) && self.session == session {
            return Ok(false);
        }
        // Retaining a bounded prefix lets us distinguish append from any rewrite,
        // including truncate/regrow between polls or edits earlier in the file.
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_SOURCE + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "transcript_unavailable")?;
        if bytes.len() as u64 > MAX_SOURCE {
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
            || !bytes.starts_with(&self.bytes);
        let mut transcript = if reset {
            Transcript::default()
        } else {
            self.transcript.clone()
        };
        let start = if reset { 0 } else { self.offset as usize };
        let mut offset = start;
        for part in bytes[start..].split_inclusive(|b| *b == b'\n') {
            if part.len() > MAX_RECORD {
                return Err("record_too_large");
            }
            if part.last() != Some(&b'\n') {
                break;
            }
            if part != b"\n" {
                let row = serde_json::from_slice(part).map_err(|_| "invalid_transcript")?;
                transcript.apply(&row, session)?;
            }
            offset += part.len();
        }
        let changed = reset
            || transcript.entries != self.transcript.entries
            || transcript.unknown_records != self.transcript.unknown_records;
        if reset {
            self.generation += 1;
        }
        self.offset = offset as u64;
        self.transcript = transcript;
        self.bytes = bytes;
        self.stamp = Some(before);
        self.session = session.to_owned();
        Ok(changed)
    }
}
