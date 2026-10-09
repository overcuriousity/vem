//! Exports (spec §9): a common event list, written as Timesketch JSONL/CSV or Vestigo Parquet.

pub mod events;
pub mod parquet;
pub mod timesketch;

pub use events::{events, timestamp_desc, Event, EventProvenance, Scope};

use crate::error::CaseError;
use crate::{query, Case};
use std::path::{Path, PathBuf};

/// Refuses an export output path inside any attached evidence root (spec §3: nothing is ever created
/// under an evidence root). A symbolic link at `out` is resolved first.
pub fn check_output_path(case: &Case, out: &Path) -> Result<(), CaseError> {
    let target = if out.exists() {
        out.canonicalize()?
    } else {
        let parent = out.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        parent.canonicalize()?.join(out.file_name().unwrap_or_default())
    };
    for r in query::roots(case)? {
        let root = PathBuf::from(&r.path);
        if target.starts_with(&root) {
            return Err(CaseError::InsideEvidence { path: target, root });
        }
    }
    Ok(())
}

/// Refuses a scope naming a root or session that does not exist.
pub fn check_scope(case: &Case, scope: &Scope) -> Result<(), CaseError> {
    match *scope {
        Scope::Case => Ok(()),
        Scope::Root(id) => query::roots(case)?.iter().any(|r| r.id == id).then_some(()).ok_or(CaseError::NoSuchRoot(id)),
        Scope::Session(id) => query::session(case, id)?.map(|_| ()).ok_or(CaseError::NoSuchSession(id)),
    }
}
