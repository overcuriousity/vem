//! Exports (spec §9): a common event list, written as Timesketch JSONL/CSV or Vestigo Parquet.

pub mod events;
pub mod parquet;
pub mod timesketch;

pub use events::{events, timestamp_desc, Event, EventProvenance, Scope};

use crate::error::CaseError;
use crate::{query, Case};
use serde::Serialize;
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
        let parent = p
            .parent()
            .filter(|q| !q.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .canonicalize()?;
        let full = parent.join(name);
        match std::fs::symlink_metadata(&full) {
            Ok(m) if m.file_type().is_symlink() => {
                let link = std::fs::read_link(&full)?;
                p = if link.is_absolute() {
                    link
                } else {
                    parent.join(link)
                };
            }
            _ => return Ok(full),
        }
    }
    Err(std::io::Error::other(format!(
        "too many levels of symbolic links at {}",
        out.display()
    )))
}

/// Refuses a scope naming a root or session that does not exist.
pub fn check_scope(case: &Case, scope: &Scope) -> Result<(), CaseError> {
    match *scope {
        Scope::Case => Ok(()),
        Scope::Root(id) => query::roots(case)?
            .iter()
            .any(|r| r.id == id)
            .then_some(())
            .ok_or(CaseError::NoSuchRoot(id)),
        Scope::Session(id) => query::session(case, id)?
            .map(|_| ())
            .ok_or(CaseError::NoSuchSession(id)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    TimesketchJsonl,
    TimesketchCsv,
    VestigoParquet,
}

impl Format {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TimesketchJsonl => "timesketch-jsonl",
            Self::TimesketchCsv => "timesketch-csv",
            Self::VestigoParquet => "vestigo-parquet",
        }
    }
    pub fn extension(&self) -> &'static str {
        match self {
            Self::TimesketchJsonl => "jsonl",
            Self::TimesketchCsv => "csv",
            Self::VestigoParquet => "parquet",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [
            Self::TimesketchJsonl,
            Self::TimesketchCsv,
            Self::VestigoParquet,
        ]
        .into_iter()
        .find(|f| f.as_str() == s)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportReport {
    pub name: String,
    pub format: String,
    pub output: String,
    pub events: usize,
    pub sha256: String,
}

pub fn scope_label(scope: &Scope) -> String {
    match scope {
        Scope::Case => "case".into(),
        Scope::Root(id) => format!("root-{id}"),
        Scope::Session(id) => format!("session-{id}"),
    }
}

/// `<case>/exports/<UTC yyyymmddThhmmssZ>-<scope>.<ext>`, with `-2`, `-3`… appended if taken.
pub fn default_output(case: &Case, format: Format, scope: &Scope) -> PathBuf {
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let base = format!("{stamp}-{}", scope_label(scope));
    let dir = case.dir.join("exports");
    let mut n = 1;
    loop {
        let name = if n == 1 {
            format!("{base}.{}", format.extension())
        } else {
            format!("{base}-{n}.{}", format.extension())
        };
        let p = dir.join(name);
        if !p.exists() {
            return p;
        }
        n += 1;
    }
}

/// Checks scope and output path, writes the export, hashes it and audits it. Refuses an empty scope.
pub fn run(
    case: &Case,
    format: Format,
    scope: &Scope,
    out: &Path,
) -> Result<ExportReport, CaseError> {
    check_scope(case, scope)?;
    check_output_path(case, out)?;
    let ev = events(case, scope)?;
    if ev.is_empty() {
        return Err(CaseError::Export(
            "nothing to export in this scope".to_string(),
        ));
    }
    match format {
        Format::TimesketchJsonl => {
            timesketch::write_jsonl(&ev, std::io::BufWriter::new(std::fs::File::create(out)?))?
        }
        Format::TimesketchCsv => {
            timesketch::write_csv(&ev, std::io::BufWriter::new(std::fs::File::create(out)?))?
        }
        Format::VestigoParquet => {
            parquet::write_parquet(&ev, out)?;
        }
    }
    let (sha256, _) = vem_core::hash::sha256_file(out)?;
    let report = ExportReport {
        name: out
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        format: format.as_str().to_string(),
        output: out.display().to_string(),
        events: ev.len(),
        sha256,
    };
    case.audit(
        "export",
        Some(&report.output),
        serde_json::to_value(&report)?,
    )?;
    Ok(report)
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportEntry {
    pub name: String,
    pub format: String,
    pub events: usize,
    pub sha256: String,
    pub ts: String,
    pub exists: bool,
}

/// Exports written into `<case>/exports/`, newest first, from the audit log.
pub fn list(case: &Case) -> Result<Vec<ExportEntry>, CaseError> {
    let dir = case.dir.join("exports").canonicalize()?;
    let mut out = Vec::new();
    for a in query::audit_log(case)?
        .into_iter()
        .rev()
        .filter(|a| a.action == "export")
    {
        let Some(output) = a.details.get("output").and_then(|v| v.as_str()) else {
            continue;
        };
        let p = Path::new(output);
        let in_dir = p
            .parent()
            .and_then(|d| d.canonicalize().ok())
            .is_some_and(|d| d == dir);
        if !in_dir {
            continue;
        }
        let s = |k: &str| {
            a.details
                .get(k)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };
        out.push(ExportEntry {
            name: p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            format: s("format"),
            events: a
                .details
                .get("events")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize,
            sha256: s("sha256"),
            ts: a.ts.clone(),
            exists: p.is_file(),
        });
    }
    Ok(out)
}

/// The path of the export named `name`, which must be exactly a regular file directly in `<case>/exports/`.
pub fn export_file(case: &Case, name: &str) -> Result<PathBuf, CaseError> {
    let not_found = || CaseError::NotFound(format!("export {name:?}"));
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        return Err(not_found());
    }
    let dir = case.dir.join("exports");
    let found = std::fs::read_dir(&dir)?
        .flatten()
        .any(|e| e.file_name().to_str() == Some(name) && e.file_type().is_ok_and(|t| t.is_file()));
    if found {
        Ok(dir.join(name))
    } else {
        Err(not_found())
    }
}
