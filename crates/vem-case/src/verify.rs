//! Re-hash evidence files and retained blobs; record drift as `hash_drift` anomalies (spec §5).

use crate::error::CaseError;
use crate::{blobs, Case};
use rusqlite::params;
use serde::Serialize;
use std::path::PathBuf;
use vem_core::hash::sha256_file;

#[derive(Debug, Default, Clone, Serialize)]
pub struct VerifyReport {
    pub files_checked: usize,
    pub drifted: Vec<String>,
    pub missing: Vec<String>,
    pub roots_unavailable: Vec<String>,
    pub blobs_checked: usize,
    pub blob_errors: Vec<String>,
}

pub fn verify(case: &mut Case) -> Result<VerifyReport, CaseError> {
    let mut report = VerifyReport::default();
    let files: Vec<(i64, i64, String, String, String)> = {
        let mut stmt = case.conn.prepare(
            "SELECT f.id, r.id, r.path, f.rel_path, f.sha256 FROM source_files f JOIN evidence_roots r ON r.id = f.root_id
             WHERE f.version = (SELECT MAX(version) FROM source_files g WHERE g.root_id = f.root_id AND g.rel_path = f.rel_path)
             ORDER BY r.id, f.rel_path",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    for (file_id, root_id, root_path, rel, expected) in files {
        let root = PathBuf::from(&root_path);
        if !root.is_dir() {
            if !report.roots_unavailable.contains(&root_path) {
                report.roots_unavailable.push(root_path.clone());
            }
            continue;
        }
        let abs = root.join(&rel);
        report.files_checked += 1;
        match sha256_file(&abs) {
            Ok((actual, _)) if actual == expected => {}
            Ok((actual, _)) => {
                report.drifted.push(rel.clone());
                case.conn.execute(
                    "INSERT INTO anomalies (root_id, source_file_id, kind, severity, message, details) VALUES (?1, ?2, 'hash_drift', 'error', ?3, ?4)",
                    params![root_id, file_id, format!("{rel} no longer matches its manifest hash"), serde_json::json!({ "expected": expected, "actual": actual }).to_string()],
                )?;
            }
            Err(e) => {
                report.missing.push(rel.clone());
                case.conn.execute(
                    "INSERT INTO anomalies (root_id, source_file_id, kind, severity, message, details) VALUES (?1, ?2, 'hash_drift', 'warning', ?3, ?4)",
                    params![root_id, file_id, format!("{rel} is missing or unreadable in the evidence root"), serde_json::json!({ "expected": expected, "error": e.to_string() }).to_string()],
                )?;
            }
        }
    }
    let shas: Vec<String> = {
        let mut stmt = case.conn.prepare("SELECT sha256 FROM blobs ORDER BY sha256")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect::<Result<_, _>>()?
    };
    for sha in shas {
        report.blobs_checked += 1;
        let ok = matches!(sha256_file(&blobs::path(&case.dir, &sha)), Ok((actual, _)) if actual == sha);
        if !ok {
            report.blob_errors.push(sha.clone());
            case.conn.execute(
                "INSERT INTO anomalies (root_id, kind, severity, message, details) VALUES ((SELECT MIN(id) FROM evidence_roots), 'hash_drift', 'error', ?1, ?2)",
                params![format!("retained blob {sha} is missing or corrupted"), serde_json::json!({ "blob": sha }).to_string()],
            )?;
        }
    }
    case.audit("verify", None, serde_json::to_value(&report)?)?;
    Ok(report)
}
