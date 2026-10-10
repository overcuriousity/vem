//! Ingest: parse every unparsed (or previously failed) source file of every store, one transaction per
//! file, then link and finalize.

use crate::case::now;
use crate::error::CaseError;
use crate::evidence::decode_rel_path;
use crate::link::{finalize_sessions, link_sessions};
use crate::sink::DbSink;
use crate::{blobs, Case, TOOL_VERSION};
use rusqlite::params;
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use vem_core::adapter::{FileContext, ParseOutcome, StoreCandidate};
use vem_core::hash::sha256_file;
use vem_core::model::{format_system_time, Harness, SourceFileHandle};

#[derive(Debug, Default, Clone, Serialize)]
pub struct IngestReport {
    pub files_parsed: usize,
    /// Files no parser understands: hashed and retained, listed in the inventory, not parsed.
    pub files_inventoried: usize,
    pub files_failed: usize,
    pub files_skipped: usize,
    /// Files whose content changed after attach; each was ingested as a new `source_files` version.
    pub files_drifted: usize,
    pub sessions: usize,
    pub messages: usize,
    pub tool_calls: usize,
    pub observations: usize,
    pub anomalies: usize,
    pub tool_version: String,
    /// `name/version` of every parser used in this run.
    pub parsers: BTreeSet<String>,
    /// Version of the secret-candidate rule set applied at ingest (`secrets::RULESET_VERSION`).
    pub secret_rules: String,
}

fn parse_ts_to_system_time(s: &str) -> Option<std::time::SystemTime> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(std::time::SystemTime::from)
}

struct FileRow {
    id: i64,
    rel: String,
    encoded: bool,
    mtime: Option<String>,
    status: String,
    sha256: String,
    retained: bool,
}

enum Resolved {
    /// Parse `path` (the retained copy when there is one) as source file `file_id`.
    Ready {
        file_id: i64,
        path: PathBuf,
        mtime: Option<String>,
    },
    /// The evidence file cannot be read and there is no retained copy.
    Unavailable(String),
}

/// Re-hashes the evidence file before parsing (spec §5). Unchanged: parse it, from the retained copy when
/// there is one, so the parser reads exactly the bytes `raw_record` will serve. Changed: record `hash_drift`,
/// add a new `source_files` version with the new hash (retaining it too) and parse that version.
/// Unreadable: parse the retained copy if there is one, so a detached case still ingests.
fn resolve(
    case: &mut Case,
    f: &FileRow,
    live: &Path,
    report: &mut IngestReport,
) -> Result<Resolved, CaseError> {
    let case_dir = case.dir.clone();
    let retained_path = |sha: &str| blobs::path(&case_dir, sha);
    match sha256_file(live) {
        Ok((sha, _)) if sha == f.sha256 => Ok(Resolved::Ready {
            file_id: f.id,
            path: if f.retained {
                retained_path(&f.sha256)
            } else {
                live.to_path_buf()
            },
            mtime: f.mtime.clone(),
        }),
        Ok(_) => {
            let tx = case.conn.transaction()?;
            let fresh = if f.retained {
                blobs::put_file(&tx, &case_dir, live)
            } else {
                sha256_file(live).map_err(CaseError::from)
            };
            let (new_sha, new_size) = match fresh {
                Ok(x) => x,
                Err(e) => return Ok(Resolved::Unavailable(e.to_string())),
            };
            let meta = std::fs::symlink_metadata(live).ok();
            let mtime = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .map(format_system_time);
            let atime = meta
                .as_ref()
                .and_then(|m| m.accessed().ok())
                .map(format_system_time);
            tx.execute(
                "INSERT INTO source_files (root_id, store_id, rel_path, rel_path_encoded, kind, size, sha256, mtime, atime, retained, parse_status, version)
                 SELECT root_id, store_id, rel_path, rel_path_encoded, 'file', ?2, ?3, ?4, ?5, retained, 'unparsed', version + 1 FROM source_files WHERE id = ?1",
                params![f.id, new_size as i64, new_sha, mtime, atime],
            )?;
            let new_id = tx.last_insert_rowid();
            let version: i64 = tx.query_row(
                "SELECT version FROM source_files WHERE id = ?1",
                [new_id],
                |r| r.get(0),
            )?;
            tx.execute(
                "UPDATE source_files SET parse_status = 'superseded', parse_error = ?2 WHERE id = ?1",
                params![f.id, format!("content changed after attach; ingested as version {version} (source file {new_id})")],
            )?;
            tx.execute(
                "INSERT INTO anomalies (root_id, store_id, source_file_id, kind, severity, message, details) SELECT root_id, store_id, id, 'hash_drift', 'error', ?2, ?3 FROM source_files WHERE id = ?1",
                params![
                    f.id,
                    format!("{} changed after attach; ingested as version {version}", f.rel),
                    serde_json::json!({ "expected": f.sha256, "actual": new_sha, "new_source_file_id": new_id, "version": version }).to_string()
                ],
            )?;
            tx.commit()?;
            report.files_drifted += 1;
            report.anomalies += 1;
            Ok(Resolved::Ready {
                file_id: new_id,
                path: if f.retained {
                    retained_path(&new_sha)
                } else {
                    live.to_path_buf()
                },
                mtime,
            })
        }
        Err(e) => {
            let copy = retained_path(&f.sha256);
            if f.retained && copy.is_file() {
                Ok(Resolved::Ready {
                    file_id: f.id,
                    path: copy,
                    mtime: f.mtime.clone(),
                })
            } else {
                Ok(Resolved::Unavailable(format!(
                    "io error: evidence file unavailable and no retained copy: {e}"
                )))
            }
        }
    }
}

fn mark_failed(case: &Case, file_id: i64, error: &str) -> Result<(), CaseError> {
    case.conn.execute(
        "UPDATE source_files SET parse_status = 'failed', parse_error = ?2, record_count = 0, anomaly_count = 0, ingested_at = ?3 WHERE id = ?1",
        params![file_id, error, now()],
    )?;
    Ok(())
}

/// Parses every `unparsed` or `failed` file. Each file is one transaction: a file that fails is rolled back
/// (no partial rows), marked `failed` with its error, and retried on the next ingest.
pub fn ingest(case: &mut Case, root_filter: Option<i64>) -> Result<IngestReport, CaseError> {
    let roots: Vec<(i64, String, String)> = {
        let mut stmt = case
            .conn
            .prepare("SELECT id, path, harness FROM evidence_roots ORDER BY id")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let roots: Vec<_> = roots
        .into_iter()
        .filter(|(id, _, _)| root_filter.map(|f| f == *id).unwrap_or(true))
        .collect();
    if let Some(f) = root_filter {
        if roots.is_empty() {
            return Err(CaseError::NoSuchRoot(f));
        }
    }
    let mut report = IngestReport {
        tool_version: TOOL_VERSION.to_string(),
        secret_rules: crate::secrets::RULESET_VERSION.to_string(),
        ..Default::default()
    };
    let case_dir = case.dir.clone();

    for (root_id, root_path, harness) in roots {
        let harness =
            Harness::parse(&harness).ok_or_else(|| CaseError::NoAdapter(harness.clone()))?;
        let adapter = vem_adapters::adapter_for(harness)
            .ok_or_else(|| CaseError::NoAdapter(harness.to_string()))?;
        let root = PathBuf::from(&root_path);
        let stores: Vec<(i64, String, Option<String>, String)> = {
            let mut stmt = case.conn.prepare(
                "SELECT id, kind, generation, rel_path FROM stores WHERE root_id = ?1 ORDER BY id",
            )?;
            let rows = stmt.query_map([root_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })?;
            rows.collect::<Result<_, _>>()?
        };
        for (store_id, kind, generation, store_rel) in stores {
            let all_files: Vec<FileRow> = {
                let mut stmt = case.conn.prepare(
                    "SELECT id, rel_path, rel_path_encoded, mtime, parse_status, sha256, retained FROM source_files
                     WHERE store_id = ?1 AND kind = 'file'
                       AND version = (SELECT MAX(version) FROM source_files g WHERE g.root_id = source_files.root_id AND g.rel_path = source_files.rel_path AND g.rel_path_encoded = source_files.rel_path_encoded)
                     ORDER BY rel_path",
                )?;
                let rows = stmt.query_map([store_id], |r| {
                    Ok(FileRow {
                        id: r.get(0)?,
                        rel: r.get(1)?,
                        encoded: r.get(2)?,
                        mtime: r.get(3)?,
                        status: r.get(4)?,
                        sha256: r.get(5)?,
                        retained: r.get::<_, i64>(6)? == 1,
                    })
                })?;
                rows.collect::<Result<_, _>>()?
            };
            let candidate = StoreCandidate {
                kind: kind.clone(),
                generation,
                rel_path: PathBuf::from(&store_rel),
                files: all_files
                    .iter()
                    .map(|f| decode_rel_path(&f.rel, f.encoded))
                    .collect(),
            };
            let mut store_records = 0i64;
            for f in &all_files {
                if f.status != "unparsed" && f.status != "failed" {
                    report.files_skipped += 1;
                    continue;
                }
                let rel_path = decode_rel_path(&f.rel, f.encoded);
                let (file_id, parse_path, mtime) =
                    match resolve(case, f, &root.join(&rel_path), &mut report)? {
                        Resolved::Ready {
                            file_id,
                            path,
                            mtime,
                        } => (file_id, path, mtime),
                        Resolved::Unavailable(err) => {
                            mark_failed(case, f.id, &err)?;
                            report.files_failed += 1;
                            continue;
                        }
                    };
                let ctx = FileContext {
                    root: &root,
                    store: &candidate,
                    rel_path: &rel_path,
                    abs_path: parse_path,
                    handle: SourceFileHandle(file_id),
                    mtime: mtime.as_deref().and_then(parse_ts_to_system_time),
                };
                let tx = case.conn.transaction()?;
                let (parsed, counts, sink_error, parsers, created) = {
                    let mut sink = DbSink::new(&tx, &case_dir, root_id, store_id, file_id);
                    let parsed = adapter.parse_file(&ctx, &mut sink);
                    (
                        parsed,
                        sink.counts,
                        sink.error.take(),
                        std::mem::take(&mut sink.parsers),
                        std::mem::take(&mut sink.created_blobs),
                    )
                };
                if let Some(e) = sink_error {
                    drop(tx);
                    remove_orphan_blobs(case, &created)?;
                    return Err(e);
                }
                match parsed {
                    Ok(outcome) => {
                        let (status, records) = match outcome {
                            ParseOutcome::Parsed { records } => ("parsed", records),
                            ParseOutcome::NotParsed => ("inventoried", 0),
                        };
                        tx.execute(
                            "UPDATE source_files SET parse_status = ?2, parse_error = NULL, record_count = ?3, anomaly_count = ?4, ingested_at = ?5 WHERE id = ?1",
                            params![file_id, status, records as i64, counts.anomalies as i64, now()],
                        )?;
                        tx.commit()?;
                        match outcome {
                            ParseOutcome::Parsed { .. } => report.files_parsed += 1,
                            ParseOutcome::NotParsed => report.files_inventoried += 1,
                        }
                        report.sessions += counts.sessions;
                        report.messages += counts.messages;
                        report.tool_calls += counts.tool_calls;
                        report.observations += counts.observations;
                        report.anomalies += counts.anomalies;
                        report.parsers.extend(parsers);
                        store_records += counts.messages as i64;
                    }
                    Err(e) => {
                        drop(tx); // roll back: a failed file leaves no partial rows
                        remove_orphan_blobs(case, &created)?;
                        mark_failed(case, file_id, &e.to_string())?;
                        report.files_failed += 1;
                    }
                }
            }
            let has_records: i64 = case.conn.query_row(
                "SELECT COUNT(*) FROM sessions WHERE store_id = ?1",
                [store_id],
                |r| r.get(0),
            )?;
            let status = if has_records > 0 || store_records > 0 {
                "parsed"
            } else {
                "inventoried"
            };
            case.conn.execute(
                "UPDATE stores SET status = ?2 WHERE id = ?1",
                params![store_id, status],
            )?;
        }
        link_sessions(&case.conn, root_id)?;
        finalize_sessions(&case.conn, root_id)?;
        report.anomalies += crate::derive::flag_unreferenced_pastes(&case.conn, root_id)?;
    }
    case.audit("ingest", None, serde_json::to_value(&report)?)?;
    Ok(report)
}

/// Deletes blob files written by a rolled-back parse that no `blobs` row references. Returns how many.
pub fn remove_orphan_blobs(case: &Case, shas: &[String]) -> Result<usize, CaseError> {
    let mut removed = 0;
    for sha in shas {
        let indexed: bool = case.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM blobs WHERE sha256 = ?1)",
            [sha],
            |r| r.get(0),
        )?;
        if !indexed && std::fs::remove_file(blobs::path(&case.dir, sha)).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}
