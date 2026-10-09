//! Ingest: parse every unparsed source file of every store, one transaction per file, then link and finalize.

use crate::case::now;
use crate::error::CaseError;
use crate::link::{finalize_sessions, link_sessions};
use crate::sink::DbSink;
use crate::Case;
use rusqlite::params;
use serde::Serialize;
use std::path::PathBuf;
use vem_core::adapter::{FileContext, StoreCandidate};
use vem_core::model::{Harness, SourceFileHandle};

#[derive(Debug, Default, Clone, Serialize)]
pub struct IngestReport {
    pub files_parsed: usize,
    pub files_failed: usize,
    pub files_skipped: usize,
    pub sessions: usize,
    pub messages: usize,
    pub tool_calls: usize,
    pub observations: usize,
    pub anomalies: usize,
}

fn parse_ts_to_system_time(s: &str) -> Option<std::time::SystemTime> {
    chrono::DateTime::parse_from_rfc3339(s).ok().map(std::time::SystemTime::from)
}

pub fn ingest(case: &mut Case, root_filter: Option<i64>) -> Result<IngestReport, CaseError> {
    let roots: Vec<(i64, String, String)> = {
        let mut stmt = case.conn.prepare("SELECT id, path, harness FROM evidence_roots ORDER BY id")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let roots: Vec<_> = roots.into_iter().filter(|(id, _, _)| root_filter.map(|f| f == *id).unwrap_or(true)).collect();
    if let Some(f) = root_filter {
        if roots.is_empty() {
            return Err(CaseError::NoSuchRoot(f));
        }
    }
    let mut report = IngestReport::default();
    let case_dir = case.dir.clone();

    for (root_id, root_path, harness) in roots {
        let harness = Harness::parse(&harness).ok_or_else(|| CaseError::NoAdapter(harness.clone()))?;
        let adapter = vem_adapters::adapter_for(harness).ok_or_else(|| CaseError::NoAdapter(harness.to_string()))?;
        let root = PathBuf::from(&root_path);
        let stores: Vec<(i64, String, Option<String>, String)> = {
            let mut stmt = case.conn.prepare("SELECT id, kind, generation, rel_path FROM stores WHERE root_id = ?1 ORDER BY id")?;
            let rows = stmt.query_map([root_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            rows.collect::<Result<_, _>>()?
        };
        for (store_id, kind, generation, store_rel) in stores {
            let all_files: Vec<(i64, String, Option<String>, String)> = {
                let mut stmt = case.conn.prepare("SELECT id, rel_path, mtime, parse_status FROM source_files WHERE store_id = ?1 AND version = (SELECT MAX(version) FROM source_files g WHERE g.root_id = source_files.root_id AND g.rel_path = source_files.rel_path) ORDER BY rel_path")?;
                let rows = stmt.query_map([store_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
                rows.collect::<Result<_, _>>()?
            };
            let candidate = StoreCandidate {
                kind: kind.clone(),
                generation,
                rel_path: PathBuf::from(&store_rel),
                files: all_files.iter().map(|(_, rel, _, _)| PathBuf::from(rel)).collect(),
            };
            let mut store_records = 0i64;
            for (file_id, rel, mtime, status) in &all_files {
                if status != "unparsed" {
                    report.files_skipped += 1;
                    continue;
                }
                let rel_path = PathBuf::from(rel);
                let ctx = FileContext {
                    root: &root,
                    store: &candidate,
                    rel_path: &rel_path,
                    abs_path: root.join(&rel_path),
                    handle: SourceFileHandle(*file_id),
                    mtime: mtime.as_deref().and_then(parse_ts_to_system_time),
                };
                let tx = case.conn.transaction()?;
                let outcome = {
                    let mut sink = DbSink::new(&tx, &case_dir, root_id, store_id, *file_id);
                    let parsed = adapter.parse_file(&ctx, &mut sink);
                    let counts = sink.counts;
                    let sink_error = sink.error.take();
                    (parsed, counts, sink_error)
                };
                let (parsed, counts, sink_error) = outcome;
                if let Some(e) = sink_error {
                    return Err(e);
                }
                report.sessions += counts.sessions;
                report.messages += counts.messages;
                report.tool_calls += counts.tool_calls;
                report.observations += counts.observations;
                report.anomalies += counts.anomalies;
                store_records += counts.messages as i64;
                match parsed {
                    Ok(()) => {
                        tx.execute(
                            "UPDATE source_files SET parse_status = 'parsed', parse_error = NULL, record_count = ?2, anomaly_count = ?3, ingested_at = ?4 WHERE id = ?1",
                            params![file_id, counts.messages as i64, counts.anomalies as i64, now()],
                        )?;
                        report.files_parsed += 1;
                    }
                    Err(e) => {
                        tx.execute(
                            "UPDATE source_files SET parse_status = 'failed', parse_error = ?2, record_count = ?3, anomaly_count = ?4, ingested_at = ?5 WHERE id = ?1",
                            params![file_id, e.to_string(), counts.messages as i64, counts.anomalies as i64, now()],
                        )?;
                        report.files_failed += 1;
                    }
                }
                tx.commit()?;
            }
            let has_records: i64 = case.conn.query_row(
                "SELECT COUNT(*) FROM sessions WHERE store_id = ?1",
                [store_id],
                |r| r.get(0),
            )?;
            let status = if has_records > 0 || store_records > 0 { "parsed" } else { "inventoried" };
            case.conn.execute("UPDATE stores SET status = ?2 WHERE id = ?1", params![store_id, status])?;
        }
        link_sessions(&case.conn, root_id)?;
        finalize_sessions(&case.conn, root_id)?;
    }
    case.audit("ingest", None, serde_json::to_value(&report)?)?;
    Ok(report)
}
