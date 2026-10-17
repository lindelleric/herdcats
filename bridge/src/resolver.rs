use crate::Result;
use std::{
    fs,
    path::{Path, PathBuf},
};
pub struct Resolver {
    pub root: PathBuf,
    anchor: fs::File,
}

impl Clone for Resolver {
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
            anchor: self.anchor.try_clone().expect("anchor clone"),
        }
    }
}

pub(crate) fn session_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}
impl Resolver {
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
        let anchor = open_absolute(&root, true)?;
        Ok(Self { root, anchor })
    }
    pub fn open(&self, path: &Path) -> Result<fs::File> {
        let relative = path
            .strip_prefix(&self.root)
            .map_err(|_| "unsafe_transcript_path")?;
        open_beneath(&self.anchor, relative, false)
    }
    fn allowed(&self, path: &Path) -> Result<PathBuf> {
        if fs::symlink_metadata(path)
            .map_err(|_| "transcript_unavailable")?
            .file_type()
            .is_symlink()
        {
            return Err("unsafe_transcript_path");
        }
        let path = path.canonicalize().map_err(|_| "transcript_unavailable")?;
        if !path.starts_with(&self.root) || !path.is_file() {
            return Err("unsafe_transcript_path");
        }
        Ok(path)
    }
    pub fn resolve(&self, kind: &str, value: &str) -> Result<(String, PathBuf)> {
        if kind == "path" {
            let path = self.allowed(Path::new(value))?;
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .filter(|id| session_id(id))
                .ok_or("invalid_session_reference")?;
            return Ok((id.to_owned(), path));
        }
        if kind != "id" || !session_id(value) {
            return Err("invalid_session_reference");
        }
        let file_name = format!("{value}.jsonl");
        let mut matches = Vec::new();
        let mut count = 0;
        for project in fs::read_dir(&self.root).map_err(|_| "transcript_root_unavailable")? {
            count += 1;
            if count > 10000 {
                return Err("transcript_index_too_large");
            }
            let project = project.map_err(|_| "transcript_root_unavailable")?;
            // Claude project roots are direct children, not a recursive cwd guess.
            let candidate = project.path().join(&file_name);
            if candidate.exists() {
                matches.push(self.allowed(&candidate)?);
            }
        }
        match matches.len() {
            0 => Err("transcript_unavailable"),
            1 => Ok((value.to_owned(), matches.remove(0))),
            _ => Err("ambiguous_session"),
        }
    }
}

// Walk through pinned directory descriptors. O_NOFOLLOW applies at every level,
// so renaming a parent and substituting a symlink cannot redirect a read.
pub(crate) fn open_beneath(anchor: &fs::File, path: &Path, directory: bool) -> Result<fs::File> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::ffi::OsStrExt,
        },
        path::Component,
    };
    let mut current = anchor.try_clone().map_err(|_| "transcript_unavailable")?;
    let parts: Vec<_> = path.components().collect();
    for (i, part) in parts.iter().enumerate() {
        let Component::Normal(name) = part else {
            return Err("unsafe_transcript_path");
        };
        let name = CString::new(name.as_bytes()).map_err(|_| "unsafe_transcript_path")?;
        let flags = libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | if i + 1 < parts.len() || directory {
                libc::O_DIRECTORY
            } else {
                0
            };
        // SAFETY: current owns a valid descriptor; name is NUL terminated. The
        // successful fresh descriptor is transferred immediately to File.
        let fd = unsafe { libc::openat(current.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err("unsafe_transcript_path");
        }
        current = unsafe { fs::File::from_raw_fd(fd) };
    }
    if !directory
        && !current
            .metadata()
            .map_err(|_| "transcript_unavailable")?
            .is_file()
    {
        return Err("unsafe_transcript_path");
    }
    Ok(current)
}
pub(crate) fn open_absolute(path: &Path, directory: bool) -> Result<fs::File> {
    let relative = path
        .strip_prefix("/")
        .map_err(|_| "unsafe_transcript_path")?;
    let anchor = fs::File::open("/").map_err(|_| "transcript_unavailable")?;
    open_beneath(&anchor, relative, directory)
}
