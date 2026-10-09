//! Read side of the case database. Every struct here is what the CLI prints and the web API will serialize.

use crate::error::CaseError;
use crate::evidence::decode_rel_path;
use crate::{blobs, Case};
use rusqlite::{params, params_from_iter, OptionalExtension, Row};
use serde::Serialize;
use serde_json::Value;
use std::io::{Read, Seek, SeekFrom};
use vem_core::hash::sha256_hex;

fn json(s: String) -> Value {
    serde_json::from_str(&s).unwrap_or(Value::Null)
}

#[derive(Debug, Clone, Serialize)]
pub struct RootRow { pub id: i64, pub path: String, pub label: String, pub host: Option<String>, pub user: Option<String>, pub os: Option<String>, pub harness: String, pub attached_at: String }

pub fn roots(case: &Case) -> Result<Vec<RootRow>, CaseError> {
    let mut stmt = case.conn.prepare("SELECT id, path, label, host, user, os, harness, attached_at FROM evidence_roots ORDER BY id")?;
    let rows = stmt.query_map([], |r| Ok(RootRow { id: r.get(0)?, path: r.get(1)?, label: r.get(2)?, host: r.get(3)?, user: r.get(4)?, os: r.get(5)?, harness: r.get(6)?, attached_at: r.get(7)? }))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[derive(Debug, Clone, Serialize)]
pub struct StoreRow { pub id: i64, pub root_id: i64, pub kind: String, pub generation: Option<String>, pub rel_path: String, pub discovery_method: String, pub status: String, pub file_count: i64 }

pub fn stores(case: &Case, root_id: i64) -> Result<Vec<StoreRow>, CaseError> {
    let mut stmt = case.conn.prepare(
        "SELECT s.id, s.root_id, s.kind, s.generation, s.rel_path, s.discovery_method, s.status, (SELECT COUNT(*) FROM source_files f WHERE f.store_id = s.id) FROM stores s WHERE s.root_id = ?1 ORDER BY s.id",
    )?;
    let rows = stmt.query_map([root_id], |r| Ok(StoreRow { id: r.get(0)?, root_id: r.get(1)?, kind: r.get(2)?, generation: r.get(3)?, rel_path: r.get(4)?, discovery_method: r.get(5)?, status: r.get(6)?, file_count: r.get(7)? }))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn absent_stores(case: &Case, root_id: i64) -> Result<Vec<String>, CaseError> {
    let mut stmt = case.conn.prepare("SELECT kind FROM absent_stores WHERE root_id = ?1 ORDER BY kind")?;
    let rows = stmt.query_map([root_id], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceFileRow { pub id: i64, pub root_id: i64, pub store_id: Option<i64>, pub rel_path: String, pub kind: String, pub link_target: Option<String>, pub version: i64, pub size: i64, pub sha256: String, pub mtime: Option<String>, pub retained: bool, pub parse_status: String, pub parse_error: Option<String>, pub record_count: i64, pub anomaly_count: i64 }

pub fn source_files(case: &Case, root_id: i64) -> Result<Vec<SourceFileRow>, CaseError> {
    let mut stmt = case.conn.prepare(
        "SELECT id, root_id, store_id, rel_path, size, sha256, mtime, retained, parse_status, parse_error, record_count, anomaly_count, kind, link_target, version FROM source_files WHERE root_id = ?1 ORDER BY rel_path, version",
    )?;
    let rows = stmt.query_map([root_id], |r| Ok(SourceFileRow { id: r.get(0)?, root_id: r.get(1)?, store_id: r.get(2)?, rel_path: r.get(3)?, size: r.get(4)?, sha256: r.get(5)?, mtime: r.get(6)?, retained: r.get::<_, i64>(7)? == 1, parse_status: r.get(8)?, parse_error: r.get(9)?, record_count: r.get(10)?, anomaly_count: r.get(11)?, kind: r.get(12)?, link_target: r.get(13)?, version: r.get(14)? }))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionRow {
    pub id: i64, pub root_id: i64, pub store_id: i64, pub harness: String, pub harness_session_id: String, pub kind: String,
    pub parent_session_id: Option<i64>, pub title: Option<String>, pub project_path: Option<String>, pub git_branch: Option<String>,
    pub harness_version: Option<String>, pub models: Vec<String>, pub first_ts: Option<String>, pub first_ts_origin: String,
    pub last_ts: Option<String>, pub last_ts_origin: String, pub message_count: i64, pub tool_call_count: i64, pub anomaly_count: i64, pub child_count: i64,
}

#[derive(Debug, Clone, Default)]
pub struct SessionFilter { pub root_id: Option<i64>, pub harness: Option<String>, pub kind: Option<String>, pub project_contains: Option<String> }

const SESSION_SELECT: &str = "SELECT s.id, st.root_id, s.store_id, st.harness, s.harness_session_id, s.kind, s.parent_session_id, s.title, s.project_path, s.git_branch, s.harness_version, s.models, s.first_ts, s.first_ts_origin, s.last_ts, s.last_ts_origin, s.message_count, s.tool_call_count,
    (SELECT COUNT(*) FROM anomalies a WHERE a.session_id = s.id), (SELECT COUNT(*) FROM sessions c WHERE c.parent_session_id = s.id)
    FROM sessions s JOIN stores st ON st.id = s.store_id";

fn session_row(r: &Row<'_>) -> rusqlite::Result<SessionRow> {
    let models: String = r.get(11)?;
    Ok(SessionRow {
        id: r.get(0)?, root_id: r.get(1)?, store_id: r.get(2)?, harness: r.get(3)?, harness_session_id: r.get(4)?, kind: r.get(5)?,
        parent_session_id: r.get(6)?, title: r.get(7)?, project_path: r.get(8)?, git_branch: r.get(9)?, harness_version: r.get(10)?,
        models: serde_json::from_str(&models).unwrap_or_default(), first_ts: r.get(12)?, first_ts_origin: r.get(13)?, last_ts: r.get(14)?,
        last_ts_origin: r.get(15)?, message_count: r.get(16)?, tool_call_count: r.get(17)?, anomaly_count: r.get(18)?, child_count: r.get(19)?,
    })
}

pub fn sessions(case: &Case, f: &SessionFilter) -> Result<Vec<SessionRow>, CaseError> {
    let mut sql = format!("{SESSION_SELECT} WHERE 1 = 1");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(r) = f.root_id { sql.push_str(" AND st.root_id = ?"); args.push(Box::new(r)); }
    if let Some(h) = &f.harness { sql.push_str(" AND st.harness = ?"); args.push(Box::new(h.clone())); }
    if let Some(k) = &f.kind { sql.push_str(" AND s.kind = ?"); args.push(Box::new(k.clone())); }
    if let Some(p) = &f.project_contains {
        let escaped = p.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
        sql.push_str(" AND s.project_path LIKE ? ESCAPE '\\'");
        args.push(Box::new(format!("%{escaped}%")));
    }
    sql.push_str(" ORDER BY s.first_ts DESC, s.id DESC");
    let mut stmt = case.conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter().map(|a| a.as_ref())), session_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn session(case: &Case, id: i64) -> Result<Option<SessionRow>, CaseError> {
    Ok(case.conn.query_row(&format!("{SESSION_SELECT} WHERE s.id = ?1"), [id], session_row).optional()?)
}

pub fn children(case: &Case, id: i64) -> Result<Vec<SessionRow>, CaseError> {
    let mut stmt = case.conn.prepare(&format!("{SESSION_SELECT} WHERE s.parent_session_id = ?1 ORDER BY s.first_ts, s.id"))?;
    let rows = stmt.query_map([id], session_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[derive(Debug, Clone, Serialize)]
pub struct BlockRow { pub id: i64, pub ordinal: i64, pub kind: String, pub text: Option<String>, pub payload: Value, pub tool_use_id: Option<String>, pub tool_call_id: Option<i64> }

#[derive(Debug, Clone, Serialize)]
pub struct MessageRow {
    pub id: i64, pub session_id: i64, pub ordinal: i64, pub role: String, pub harness_record_type: String, pub harness_uuid: Option<String>,
    pub parent_uuid: Option<String>, pub timestamp: Option<String>, pub ts_origin: String, pub model: Option<String>, pub provenance_id: i64,
    pub attributes: Value, pub blocks: Vec<BlockRow>,
}

pub fn messages(case: &Case, session_id: i64, include_meta: bool) -> Result<Vec<MessageRow>, CaseError> {
    let sql = format!(
        "SELECT id, session_id, ordinal, role, harness_record_type, harness_uuid, parent_uuid, timestamp, ts_origin, model, provenance_id, attributes FROM messages WHERE session_id = ?1 {} ORDER BY ordinal",
        if include_meta { "" } else { "AND role != 'meta'" }
    );
    let mut stmt = case.conn.prepare(&sql)?;
    let mut out: Vec<MessageRow> = stmt
        .query_map([session_id], |r| {
            let attrs: String = r.get(11)?;
            Ok(MessageRow { id: r.get(0)?, session_id: r.get(1)?, ordinal: r.get(2)?, role: r.get(3)?, harness_record_type: r.get(4)?, harness_uuid: r.get(5)?, parent_uuid: r.get(6)?, timestamp: r.get(7)?, ts_origin: r.get(8)?, model: r.get(9)?, provenance_id: r.get(10)?, attributes: json(attrs), blocks: Vec::new() })
        })?
        .collect::<Result<_, _>>()?;
    let mut bstmt = case.conn.prepare("SELECT id, ordinal, kind, text, payload, tool_use_id, tool_call_id FROM blocks WHERE message_id = ?1 ORDER BY ordinal")?;
    for m in &mut out {
        let blocks = bstmt.query_map([m.id], |r| {
            let payload: String = r.get(4)?;
            Ok(BlockRow { id: r.get(0)?, ordinal: r.get(1)?, kind: r.get(2)?, text: r.get(3)?, payload: json(payload), tool_use_id: r.get(5)?, tool_call_id: r.get(6)? })
        })?;
        m.blocks = blocks.collect::<Result<_, _>>()?;
    }
    Ok(out)
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolCallRow {
    pub id: i64, pub session_id: i64, pub name: String, pub category: String, pub input: Value, pub result_text: Option<String>,
    pub result_payload: Option<Value>, pub is_error: bool, pub started_ts: Option<String>, pub ended_ts: Option<String>, pub ts_origin: String,
    pub tool_use_block_id: i64, pub tool_result_block_id: Option<i64>,
}

pub fn tool_calls(case: &Case, session_id: i64) -> Result<Vec<ToolCallRow>, CaseError> {
    let mut stmt = case.conn.prepare("SELECT id, session_id, name, category, input, result_text, result_payload, is_error, started_ts, ended_ts, ts_origin, tool_use_block_id, tool_result_block_id FROM tool_calls WHERE session_id = ?1 ORDER BY id")?;
    let rows = stmt.query_map([session_id], |r| {
        let input: String = r.get(4)?;
        let payload: Option<String> = r.get(6)?;
        Ok(ToolCallRow { id: r.get(0)?, session_id: r.get(1)?, name: r.get(2)?, category: r.get(3)?, input: json(input), result_text: r.get(5)?, result_payload: payload.map(json), is_error: r.get::<_, i64>(7)? == 1, started_ts: r.get(8)?, ended_ts: r.get(9)?, ts_origin: r.get(10)?, tool_use_block_id: r.get(11)?, tool_result_block_id: r.get(12)? })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[derive(Debug, Clone, Serialize)]
pub struct ObservationRow {
    pub id: i64, pub session_id: i64, pub kind: String, pub path: Option<String>, pub command: Option<String>, pub before_blob: Option<String>,
    pub after_blob: Option<String>, pub timestamp: Option<String>, pub ts_origin: String, pub confidence: String, pub details: Value,
    pub derived_from_tool_call_id: Option<i64>, pub derived_from_block_id: Option<i64>, pub derived_from_provenance_id: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct ObservationFilter { pub root_id: Option<i64>, pub session_id: Option<i64>, pub kind: Option<String> }

pub fn observations(case: &Case, f: &ObservationFilter) -> Result<Vec<ObservationRow>, CaseError> {
    let mut sql = String::from("SELECT o.id, o.session_id, o.kind, o.path, o.command, o.before_blob, o.after_blob, o.timestamp, o.ts_origin, o.confidence, o.details, o.derived_from_tool_call_id, o.derived_from_block_id, o.derived_from_provenance_id FROM observations o JOIN sessions s ON s.id = o.session_id JOIN stores st ON st.id = s.store_id WHERE 1 = 1");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(r) = f.root_id { sql.push_str(" AND st.root_id = ?"); args.push(Box::new(r)); }
    if let Some(s) = f.session_id { sql.push_str(" AND o.session_id = ?"); args.push(Box::new(s)); }
    if let Some(k) = &f.kind { sql.push_str(" AND o.kind = ?"); args.push(Box::new(k.clone())); }
    sql.push_str(" ORDER BY o.timestamp, o.id");
    let mut stmt = case.conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter().map(|a| a.as_ref())), |r| {
        let details: String = r.get(10)?;
        Ok(ObservationRow { id: r.get(0)?, session_id: r.get(1)?, kind: r.get(2)?, path: r.get(3)?, command: r.get(4)?, before_blob: r.get(5)?, after_blob: r.get(6)?, timestamp: r.get(7)?, ts_origin: r.get(8)?, confidence: r.get(9)?, details: json(details), derived_from_tool_call_id: r.get(11)?, derived_from_block_id: r.get(12)?, derived_from_provenance_id: r.get(13)? })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[derive(Debug, Clone, Serialize)]
pub struct AnomalyRow { pub id: i64, pub root_id: i64, pub store_id: Option<i64>, pub source_file_id: Option<i64>, pub session_id: Option<i64>, pub kind: String, pub severity: String, pub byte_offset: Option<i64>, pub provenance_id: Option<i64>, pub message: String, pub details: Value }

#[derive(Debug, Clone, Default)]
pub struct AnomalyFilter { pub root_id: Option<i64>, pub kind: Option<String>, pub severity: Option<String> }

pub fn anomalies(case: &Case, f: &AnomalyFilter) -> Result<Vec<AnomalyRow>, CaseError> {
    let mut sql = String::from("SELECT id, root_id, store_id, source_file_id, session_id, kind, severity, byte_offset, message, details, provenance_id FROM anomalies WHERE 1 = 1");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(r) = f.root_id { sql.push_str(" AND root_id = ?"); args.push(Box::new(r)); }
    if let Some(k) = &f.kind { sql.push_str(" AND kind = ?"); args.push(Box::new(k.clone())); }
    if let Some(s) = &f.severity { sql.push_str(" AND severity = ?"); args.push(Box::new(s.clone())); }
    sql.push_str(" ORDER BY id");
    let mut stmt = case.conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter().map(|a| a.as_ref())), |r| {
        let details: String = r.get(9)?;
        Ok(AnomalyRow { id: r.get(0)?, root_id: r.get(1)?, store_id: r.get(2)?, source_file_id: r.get(3)?, session_id: r.get(4)?, kind: r.get(5)?, severity: r.get(6)?, byte_offset: r.get(7)?, provenance_id: r.get(10)?, message: r.get(8)?, details: json(details) })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaimRow { pub id: i64, pub session_id: i64, pub scheme: String, pub claimed_id: String, pub source_file_id: i64, pub join_status: String, pub matched_session_id: Option<i64> }

pub fn claims(case: &Case, session_id: i64) -> Result<Vec<ClaimRow>, CaseError> {
    let mut stmt = case.conn.prepare("SELECT id, session_id, scheme, claimed_id, source_file_id, join_status, matched_session_id FROM identity_claims WHERE session_id = ?1 ORDER BY id")?;
    let rows = stmt.query_map([session_id], |r| Ok(ClaimRow { id: r.get(0)?, session_id: r.get(1)?, scheme: r.get(2)?, claimed_id: r.get(3)?, source_file_id: r.get(4)?, join_status: r.get(5)?, matched_session_id: r.get(6)? }))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[derive(Debug, Clone, Serialize)]
pub struct ProvenanceRow {
    pub id: i64, pub source_file_id: i64, pub rel_path: String, pub rel_path_encoded: bool, pub file_sha256: String, pub root_path: String, pub retained: bool,
    pub byte_offset: i64, pub byte_length: i64, pub record_index: i64, pub content_sha256: String, pub parser_name: String, pub parser_version: String, pub origin: String,
}

pub fn provenance(case: &Case, id: i64) -> Result<Option<ProvenanceRow>, CaseError> {
    Ok(case
        .conn
        .query_row(
            "SELECT p.id, p.source_file_id, f.rel_path, f.sha256, r.path, f.retained, p.byte_offset, p.byte_length, p.record_index, p.content_sha256, p.parser_name, p.parser_version, p.origin, f.rel_path_encoded
             FROM provenance p JOIN source_files f ON f.id = p.source_file_id JOIN evidence_roots r ON r.id = f.root_id WHERE p.id = ?1",
            [id],
            |r| Ok(ProvenanceRow { id: r.get(0)?, source_file_id: r.get(1)?, rel_path: r.get(2)?, file_sha256: r.get(3)?, root_path: r.get(4)?, retained: r.get::<_, i64>(5)? == 1, byte_offset: r.get(6)?, byte_length: r.get(7)?, record_index: r.get(8)?, content_sha256: r.get(9)?, parser_name: r.get(10)?, parser_version: r.get(11)?, origin: r.get(12)?, rel_path_encoded: r.get(13)? }),
        )
        .optional()?)
}

/// The exact bytes a provenance row points at, from the retained copy when there is one.
pub fn raw_record(case: &Case, provenance_id: i64) -> Result<Vec<u8>, CaseError> {
    let p = provenance(case, provenance_id)?.ok_or_else(|| CaseError::Export(format!("no provenance row {provenance_id}")))?;
    let path = if p.retained { blobs::path(&case.dir, &p.file_sha256) } else { std::path::Path::new(&p.root_path).join(decode_rel_path(&p.rel_path, p.rel_path_encoded)) };
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(p.byte_offset as u64))?;
    let mut buf = vec![0u8; p.byte_length as usize];
    f.read_exact(&mut buf)?;
    if sha256_hex(&buf) != p.content_sha256 {
        return Err(CaseError::IntegrityMismatch(format!("record bytes of provenance {provenance_id} do not match its hash")));
    }
    Ok(buf)
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit { pub block_id: i64, pub message_id: i64, pub session_id: i64, pub tool_call_id: Option<i64>, pub snippet: String }

/// Case-wide full-text search over message blocks and tool calls (name, input, result). A tool-call hit
/// points at its `tool_use` block. The query is matched as one phrase.
pub fn search(case: &Case, query: &str, limit: usize) -> Result<Vec<SearchHit>, CaseError> {
    let mut stmt = case.conn.prepare(
        "SELECT block_id, message_id, session_id, tool_call_id, snippet FROM (
             SELECT b.id AS block_id, b.message_id AS message_id, m.session_id AS session_id, NULL AS tool_call_id,
                    snippet(blocks_fts, 0, '[', ']', '…', 12) AS snippet, blocks_fts.rank AS rank
             FROM blocks_fts JOIN blocks b ON b.id = blocks_fts.rowid JOIN messages m ON m.id = b.message_id
             WHERE blocks_fts MATCH ?1
             UNION ALL
             SELECT b.id, b.message_id, t.session_id, t.id,
                    snippet(tool_calls_fts, -1, '[', ']', '…', 12), tool_calls_fts.rank
             FROM tool_calls_fts JOIN tool_calls t ON t.id = tool_calls_fts.rowid JOIN blocks b ON b.id = t.tool_use_block_id
             WHERE tool_calls_fts MATCH ?1
         ) ORDER BY rank LIMIT ?2",
    )?;
    let quoted = format!("\"{}\"", query.replace('"', "\"\""));
    let rows = stmt.query_map(params![quoted, limit as i64], |r| Ok(SearchHit { block_id: r.get(0)?, message_id: r.get(1)?, session_id: r.get(2)?, tool_call_id: r.get(3)?, snippet: r.get(4)? }))?;
    Ok(rows.collect::<Result<_, _>>()?)
}
