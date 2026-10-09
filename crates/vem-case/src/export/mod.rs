//! Exports (spec §9): a common event list, written as Timesketch JSONL/CSV or Vestigo Parquet.

pub mod events;
pub mod parquet;
pub mod timesketch;

pub use events::{events, timestamp_desc, Event, EventProvenance, Scope};

use crate::error::CaseError;
use crate::{query, Case};
use std::path::{Path, PathBuf};

/// Refuses an export output path inside any attached evidence root (spec §3: nothing is ever created
/// under an evidence root). The parent is canonicalized and `out` itself is checked with
/// `symlink_metadata`, so a symbolic link at `out` (even a dangling one, which `File::create` would
/// follow) is resolved to the path that would actually be written.
pub fn check_output_path(case: &Case, out: &Path) -> Result<(), CaseError> {
    let target = resolve_output_path(out)?;
    for r in query::roots(case)? {
        let root = PathBuf::from(&r.path);
        if target.starts_with(&root) {
            return Err(CaseError::InsideEvidence { path: target, root });
        }
    }
    Ok(())
}

/// The path a write to `out` lands on: canonical parent plus file name, following symbolic links at
/// the final component without requiring their target to exist.
fn resolve_output_path(out: &Path) -> std::io::Result<PathBuf> {
    let mut p = out.to_path_buf();
    for _ in 0..40 {
        let Some(name) = p.file_name().map(|n| n.to_os_string()) else {
            return p.canonicalize();
        };
        let parent = p.parent().filter(|q| !q.as_os_str().is_empty()).unwrap_or(Path::new(".")).canonicalize()?;
        let full = parent.join(name);
        match std::fs::symlink_metadata(&full) {
            Ok(m) if m.file_type().is_symlink() => {
                let link = std::fs::read_link(&full)?;
                p = if link.is_absolute() { link } else { parent.join(link) };
            }
            _ => return Ok(full),
        }
    }
    Err(std::io::Error::other(format!("too many levels of symbolic links at {}", out.display())))
}

/// Refuses a scope naming a root or session that does not exist.
pub fn check_scope(case: &Case, scope: &Scope) -> Result<(), CaseError> {
    match *scope {
        Scope::Case => Ok(()),
        Scope::Root(id) => query::roots(case)?.iter().any(|r| r.id == id).then_some(()).ok_or(CaseError::NoSuchRoot(id)),
        Scope::Session(id) => query::session(case, id)?.map(|_| ()).ok_or(CaseError::NoSuchSession(id)),
    }
}
