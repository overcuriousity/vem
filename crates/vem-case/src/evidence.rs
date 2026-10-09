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
use vem_core::hash::sha256_file;
use vem_core::model::{format_system_time, Harness};

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
    pub unreadable: Vec<String>,
}

/// Forward-slash relative path string, whatever the host separator.
pub fn rel_string(p: &Path) -> String {
    p.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect::<Vec<_>>().join("/")
}

/// Immediate child directories of `path` that identify as a harness directory (one level only).
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

    let mut file_to_store: HashMap<String, i64> = HashMap::new();
    let mut stores = Vec::new();
    for s in &discovery.stores {
        tx.execute(
            "INSERT INTO stores (root_id, harness, kind, generation, rel_path, discovery_method, status) VALUES (?1, ?2, ?3, ?4, ?5, 'signature', 'pending')",
            params![root_id, identification.harness.as_str(), s.kind, s.generation, rel_string(&s.rel_path)],
        )?;
        let store_id = tx.last_insert_rowid();
        for f in &s.files {
            file_to_store.insert(rel_string(f), store_id);
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
    for entry in walkdir::WalkDir::new(&root).follow_links(false).sort_by_file_name() {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                unreadable.push(e.path().map(|p| p.display().to_string()).unwrap_or_default());
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = rel_string(entry.path().strip_prefix(&root).unwrap_or(entry.path()));
        let (sha, size) = match sha256_file(entry.path()) {
            Ok(x) => x,
            Err(_) => {
                unreadable.push(rel);
                continue;
            }
        };
        let meta = entry.metadata().ok();
        let mtime = meta.as_ref().and_then(|m| m.modified().ok()).map(format_system_time);
        let ctime = meta.as_ref().and_then(|m| m.created().ok()).map(format_system_time);
        let atime = meta.as_ref().and_then(|m| m.accessed().ok()).map(format_system_time);
        let store_id = file_to_store.get(&rel).copied();
        if store_id.is_none() {
            unclaimed += 1;
        }
        let retained = if opts.retain {
            blobs::put_file(&tx, &case_dir, entry.path(), &sha, size)?;
            1
        } else {
            0
        };
        tx.execute(
            "INSERT INTO source_files (root_id, store_id, rel_path, size, sha256, mtime, ctime, atime, retained, parse_status, version) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'unparsed', 1)",
            params![root_id, store_id, rel, size as i64, sha, mtime, ctime, atime, retained],
        )?;
        file_count += 1;
        total_bytes += size;
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
        unreadable,
    };
    case.audit(
        "evidence.attach",
        Some(&root.to_string_lossy()),
        serde_json::json!({ "root_id": root_id, "harness": report.harness, "file_count": file_count, "total_bytes": total_bytes, "retained": opts.retain, "unreadable": report.unreadable }),
    )?;
    Ok(report)
}
