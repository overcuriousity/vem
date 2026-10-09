//! Attaching a collected harness directory: identify by content, discover stores, hash every file,
//! retain copies (spec §3, §5).

use crate::case::now;
use crate::error::CaseError;
use crate::{blobs, Case};
use rusqlite::params;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use vem_core::adapter::Identification;
use vem_core::hash::{sha256_file, sha256_hex};
use vem_core::model::{format_system_time, AnomalyKind, Harness, TS_FORMAT};

#[derive(Debug, Clone)]
pub struct AttachOptions {
    pub label: String,
    pub host: Option<String>,
    pub user: Option<String>,
    pub os: Option<String>,
    pub harness: Option<Harness>,
    pub retain: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoreSummary {
    pub id: i64,
    pub kind: String,
    pub generation: Option<String>,
    pub rel_path: String,
    pub file_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct AttachReport {
    pub root_id: i64,
    pub harness: Harness,
    pub evidence: Vec<String>,
    pub stores: Vec<StoreSummary>,
    pub absent: Vec<String>,
    pub file_count: usize,
    pub unclaimed_files: usize,
    pub total_bytes: u64,
    /// Symbolic links found in the root: recorded in the manifest, never followed.
    pub symlinks: Vec<String>,
    pub unreadable: Vec<String>,
}

/// Forward-slash relative path string, whatever the host separator. Lossless: see `encode_rel_path`.
pub fn rel_string(p: &Path) -> String {
    encode_rel_path(p).0
}

/// Lossless string form of a relative path: components joined by `/`. A path that is valid UTF-8 is
/// stored as-is (`false`). Otherwise every byte of an invalid UTF-8 sequence and every `%` is
/// percent-encoded as `%XX` (`true`); `decode_rel_path` reverses it.
pub fn encode_rel_path(p: &Path) -> (String, bool) {
    let parts: Vec<&std::ffi::OsStr> = p.components().map(|c| c.as_os_str()).collect();
    if let Some(utf8) = parts.iter().map(|c| c.to_str()).collect::<Option<Vec<&str>>>() {
        return (utf8.join("/"), false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        (parts.iter().map(|c| percent_encode_invalid(c.as_bytes())).collect::<Vec<_>>().join("/"), true)
    }
    #[cfg(not(unix))]
    {
        (parts.iter().map(|c| c.to_string_lossy().to_string()).collect::<Vec<_>>().join("/"), false)
    }
}

#[cfg(unix)]
fn percent_encode_invalid(bytes: &[u8]) -> String {
    fn push_text(out: &mut String, s: &str) {
        for ch in s.chars() {
            if ch == '%' {
                out.push_str("%25");
            } else {
                out.push(ch);
            }
        }
    }
    let mut out = String::new();
    let mut rest = bytes;
    loop {
        match std::str::from_utf8(rest) {
            Ok(s) => {
                push_text(&mut out, s);
                return out;
            }
            Err(e) => {
                let (valid, after) = rest.split_at(e.valid_up_to());
                push_text(&mut out, std::str::from_utf8(valid).unwrap_or_default());
                let bad = e.error_len().unwrap_or(after.len());
                for b in &after[..bad] {
                    out.push_str(&format!("%{b:02X}"));
                }
                rest = &after[bad..];
            }
        }
    }
}

/// The filesystem path for a stored `rel_path` (see `encode_rel_path`).
pub fn decode_rel_path(rel: &str, encoded: bool) -> PathBuf {
    if !encoded {
        return PathBuf::from(rel);
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let b = rel.as_bytes();
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if b[i] == b'%' && i + 2 < b.len() {
                if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                    out.push((h * 16 + l) as u8);
                    i += 3;
                    continue;
                }
            }
            out.push(b[i]);
            i += 1;
        }
        PathBuf::from(std::ffi::OsString::from_vec(out))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(rel)
    }
}

fn ts(t: std::io::Result<std::time::SystemTime>) -> Option<String> {
    t.ok().map(format_system_time)
}

/// Inode change time (Unix `st_ctime`); not available elsewhere.
fn change_time(m: &std::fs::Metadata) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        chrono::DateTime::from_timestamp(m.ctime(), m.ctime_nsec() as u32).map(|d| d.format(TS_FORMAT).to_string())
    }
    #[cfg(not(unix))]
    {
        let _ = m;
        None
    }
}

pub fn child_candidates(path: &Path) -> Vec<(PathBuf, Vec<Identification>)> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(path) else { return out };
    let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    for d in dirs {
        let ids = vem_adapters::identify_root(&d);
        if !ids.is_empty() {
            out.push((d, ids));
        }
    }
    out
}

fn identify(root: &Path, forced: Option<Harness>) -> Result<Identification, CaseError> {
    let ids = vem_adapters::identify_root(root);
    match (forced, ids.len()) {
        (Some(h), _) => Ok(ids
            .into_iter()
            .find(|i| i.harness == h)
            .unwrap_or(Identification { harness: h, evidence: vec!["forced by --harness; no content signature matched".to_string()] })),
        (None, 1) => Ok(ids.into_iter().next().expect("one")),
        (None, 0) => {
            let children = child_candidates(root);
            let hint = if children.is_empty() {
                String::new()
            } else {
                let names: Vec<String> = children
                    .iter()
                    .map(|(p, ids)| format!("{} ({})", p.display(), ids.iter().map(|i| i.harness.as_str()).collect::<Vec<_>>().join(", ")))
                    .collect();
                format!("; it contains harness directories you can attach individually: {}", names.join("; "))
            };
            Err(CaseError::Unrecognized { path: root.to_path_buf(), hint })
        }
        (None, _) => Err(CaseError::Ambiguous(ids.iter().map(|i| i.harness.as_str()).collect::<Vec<_>>().join(", "))),
    }
}

pub fn attach(case: &mut Case, path: &Path, opts: AttachOptions) -> Result<AttachReport, CaseError> {
    let root = path.canonicalize()?;
    let identification = identify(&root, opts.harness)?;
    let adapter = vem_adapters::adapter_for(identification.harness)
        .ok_or_else(|| CaseError::NoAdapter(identification.harness.to_string()))?;
    let discovery = adapter.discover(&root);
    let case_dir = case.dir.clone();
    let tx = case.conn.transaction()?;

    tx.execute(
        "INSERT INTO evidence_roots (path, label, host, user, os, harness, attached_at, identification) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            root.to_string_lossy().to_string(),
            opts.label,
            opts.host,
            opts.user,
            opts.os,
            identification.harness.as_str(),
            now(),
            serde_json::to_string(&identification.evidence)?,
        ],
    )?;
    let root_id = tx.last_insert_rowid();

    let mut file_to_store: HashMap<(String, bool), i64> = HashMap::new();
    let mut stores = Vec::new();
    for s in &discovery.stores {
        tx.execute(
            "INSERT INTO stores (root_id, harness, kind, generation, rel_path, discovery_method, status) VALUES (?1, ?2, ?3, ?4, ?5, 'signature', 'pending')",
            params![root_id, identification.harness.as_str(), s.kind, s.generation, rel_string(&s.rel_path)],
        )?;
        let store_id = tx.last_insert_rowid();
        for f in &s.files {
            file_to_store.insert(encode_rel_path(f), store_id);
        }
        stores.push(StoreSummary { id: store_id, kind: s.kind.clone(), generation: s.generation.clone(), rel_path: rel_string(&s.rel_path), file_count: s.files.len() });
    }
    for kind in &discovery.absent {
        tx.execute("INSERT INTO absent_stores (root_id, kind) VALUES (?1, ?2)", params![root_id, kind])?;
    }

    let mut file_count = 0usize;
    let mut unclaimed = 0usize;
    let mut total_bytes = 0u64;
    let mut unreadable = Vec::new();
    let mut symlinks = Vec::new();
    let anomaly = |tx: &rusqlite::Transaction<'_>, file_id: i64, kind: AnomalyKind, message: String, details: serde_json::Value| {
        tx.execute(
            "INSERT INTO anomalies (root_id, source_file_id, kind, severity, message, details) VALUES (?1, ?2, ?3, 'info', ?4, ?5)",
            params![root_id, file_id, kind.as_str(), message, details.to_string()],
        )
    };
    for entry in walkdir::WalkDir::new(&root).follow_links(false).sort_by_file_name() {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                unreadable.push(e.path().map(|p| p.display().to_string()).unwrap_or_default());
                continue;
            }
        };
        let ft = entry.file_type();
        if !ft.is_file() && !ft.is_symlink() {
            continue;
        }
        let (rel, encoded) = encode_rel_path(entry.path().strip_prefix(&root).unwrap_or(entry.path()));
        // Timestamps are taken before the file is read, so atime is not vem's own read.
        let meta = std::fs::symlink_metadata(entry.path()).ok();
        let mtime = meta.as_ref().and_then(|m| ts(m.modified()));
        let ctime = meta.as_ref().and_then(change_time);
        let btime = meta.as_ref().and_then(|m| ts(m.created()));
        let atime = meta.as_ref().and_then(|m| ts(m.accessed()));
        let store_id = file_to_store.get(&(rel.clone(), encoded)).copied();
        let file_id = if ft.is_symlink() {
            let Ok(target) = std::fs::read_link(entry.path()) else {
                unreadable.push(rel);
                continue;
            };
            let target_text = target.to_string_lossy().to_string();
            tx.execute(
                "INSERT INTO source_files (root_id, store_id, rel_path, rel_path_encoded, kind, link_target, size, sha256, mtime, ctime, btime, atime, retained, parse_status, version) VALUES (?1, ?2, ?3, ?4, 'symlink', ?5, 0, ?6, ?7, ?8, ?9, ?10, 0, 'inventoried', 1)",
                params![root_id, store_id, rel, encoded, target_text, sha256_hex(target.as_os_str().as_encoded_bytes()), mtime, ctime, btime, atime],
            )?;
            let id = tx.last_insert_rowid();
            anomaly(&tx, id, AnomalyKind::SymlinkInEvidence, format!("{rel} is a symbolic link to {target_text}; recorded, not followed"), serde_json::json!({ "target": target_text }))?;
            symlinks.push(rel.clone());
            id
        } else {
            if std::fs::File::open(entry.path()).is_err() {
                unreadable.push(rel);
                continue;
            }
            let (sha, size, retained) = if opts.retain {
                let (sha, size) = blobs::put_file(&tx, &case_dir, entry.path())?;
                (sha, size, 1)
            } else {
                match sha256_file(entry.path()) {
                    Ok((sha, size)) => (sha, size, 0),
                    Err(_) => {
                        unreadable.push(rel);
                        continue;
                    }
                }
            };
            if store_id.is_none() {
                unclaimed += 1;
            }
            tx.execute(
                "INSERT INTO source_files (root_id, store_id, rel_path, rel_path_encoded, kind, size, sha256, mtime, ctime, btime, atime, retained, parse_status, version) VALUES (?1, ?2, ?3, ?4, 'file', ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'unparsed', 1)",
                params![root_id, store_id, rel, encoded, size as i64, sha, mtime, ctime, btime, atime, retained],
            )?;
            file_count += 1;
            total_bytes += size;
            tx.last_insert_rowid()
        };
        if encoded {
            anomaly(&tx, file_id, AnomalyKind::NonUtf8Path, format!("the name {rel} is not valid UTF-8; stored percent-encoded"), serde_json::json!({ "rel_path": rel }))?;
        }
    }
    tx.commit()?;

    let report = AttachReport {
        root_id,
        harness: identification.harness,
        evidence: identification.evidence,
        stores,
        absent: discovery.absent,
        file_count,
        unclaimed_files: unclaimed,
        total_bytes,
        symlinks,
        unreadable,
    };
    case.audit(
        "evidence.attach",
        Some(&root.to_string_lossy()),
        serde_json::json!({ "root_id": root_id, "harness": report.harness, "file_count": file_count, "total_bytes": total_bytes, "retained": opts.retain, "unreadable": report.unreadable }),
    )?;
    Ok(report)
}
