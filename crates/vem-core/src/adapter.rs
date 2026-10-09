//! Harness adapter contract (spec §6): identify a root, discover its stores, parse one file.

use crate::model::{Harness, SourceFileHandle};
use crate::sink::ParseSink;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identification {
    pub harness: Harness,
    /// Human-readable signatures that matched, recorded on the evidence root.
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreCandidate {
    /// e.g. `claude:projects`.
    pub kind: String,
    pub generation: Option<String>,
    /// Relative to the root.
    pub rel_path: PathBuf,
    /// Files belonging to this store, relative to the root, sorted.
    pub files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Discovery {
    pub stores: Vec<StoreCandidate>,
    /// Store kinds this harness normally has that were not found.
    pub absent: Vec<String>,
}

pub struct FileContext<'a> {
    pub root: &'a Path,
    pub store: &'a StoreCandidate,
    pub rel_path: &'a Path,
    pub abs_path: PathBuf,
    pub handle: SourceFileHandle,
    pub mtime: Option<SystemTime>,
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Invalid(String),
}

/// What `parse_file` did with a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseOutcome {
    /// A parser read the file; `records` is the number of records it read (lines, documents).
    Parsed { records: u64 },
    /// No parser understands this file; it stays inventoried (hashed and retained) only.
    NotParsed,
}

pub trait HarnessAdapter: Send + Sync {
    fn harness(&self) -> Harness;
    /// `Some` when `root` is this harness's directory, with the signatures that matched.
    fn identify(&self, root: &Path) -> Option<Identification>;
    fn discover(&self, root: &Path) -> Discovery;
    /// Parses one file of one store, emitting into `sink`. Must not panic on malformed input.
    /// Reads the file only through `ctx.abs_path`, which may be a retained copy rather than the evidence.
    fn parse_file(&self, ctx: &FileContext<'_>, sink: &mut dyn ParseSink) -> Result<ParseOutcome, ParseError>;
}
