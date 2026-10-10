//! Read side of the case database. Every struct here is what the CLI prints and the web API will serialize.

use crate::error::CaseError;
use crate::evidence::decode_rel_path;
use crate::{blobs, Case};
use rusqlite::{params, params_from_iter, OptionalExtension, Row};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use vem_core::hash::sha256_hex;

fn json(s: String) -> Value {
    serde_json::from_str(&s).unwrap_or(Value::Null)
}

#[derive(Debug, Clone, Serialize)]
pub struct RootRow {
    pub id: i64,
    pub path: String,
    pub label: String,
    pub host: Option<String>,
    pub user: Option<String>,
    pub os: Option<String>,
    pub harness: String,
    pub attached_at: String,
}

pub fn roots(case: &Case) -> Result<Vec<RootRow>, CaseError> {
    let mut stmt = case.conn.prepare("SELECT id, path, label, host, user, os, harness, attached_at FROM evidence_roots ORDER BY id")?;
    let rows = stmt.query_map([], |r| {
        Ok(RootRow {
            id: r.get(0)?,
            path: r.get(1)?,
            label: r.get(2)?,
            host: r.get(3)?,
            user: r.get(4)?,
            os: r.get(5)?,
            harness: r.get(6)?,
            attached_at: r.get(7)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[derive(Debug, Clone, Serialize)]
pub struct StoreRow {
    pub id: i64,
    pub root_id: i64,
    pub kind: String,
    pub generation: Option<String>,
    pub rel_path: String,
    pub discovery_method: String,
    pub status: String,
    pub file_count: i64,
}

pub fn stores(case: &Case, root_id: i64) -> Result<Vec<StoreRow>, CaseError> {
    let mut stmt = case.conn.prepare(
        "SELECT s.id, s.root_id, s.kind, s.generation, s.rel_path, s.discovery_method, s.status, (SELECT COUNT(*) FROM source_files f WHERE f.store_id = s.id) FROM stores s WHERE s.root_id = ?1 ORDER BY s.id",
    )?;
    let rows = stmt.query_map([root_id], |r| {
        Ok(StoreRow {
            id: r.get(0)?,
            root_id: r.get(1)?,
            kind: r.get(2)?,
            generation: r.get(3)?,
            rel_path: r.get(4)?,
            discovery_method: r.get(5)?,
            status: r.get(6)?,
            file_count: r.get(7)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn absent_stores(case: &Case, root_id: i64) -> Result<Vec<String>, CaseError> {
    let mut stmt = case
        .conn
        .prepare("SELECT kind FROM absent_stores WHERE root_id = ?1 ORDER BY kind")?;
    let rows = stmt.query_map([root_id], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[derive(Debug, Clone, Serialize)]
pub struct CaseTotals {
    pub sessions: i64,
    pub messages: i64,
    pub tool_calls: i64,
    pub observations: i64,
    pub anomalies_info: i64,
    pub anomalies_warning: i64,
    pub anomalies_error: i64,
}

pub fn totals(case: &Case) -> Result<CaseTotals, CaseError> {
    Ok(case.conn.query_row(
        "SELECT (SELECT COUNT(*) FROM sessions), (SELECT COUNT(*) FROM messages), (SELECT COUNT(*) FROM tool_calls), (SELECT COUNT(*) FROM observations),
                (SELECT COUNT(*) FROM anomalies WHERE severity = 'info'), (SELECT COUNT(*) FROM anomalies WHERE severity = 'warning'), (SELECT COUNT(*) FROM anomalies WHERE severity = 'error')",
        [],
        |r| Ok(CaseTotals { sessions: r.get(0)?, messages: r.get(1)?, tool_calls: r.get(2)?, observations: r.get(3)?, anomalies_info: r.get(4)?, anomalies_warning: r.get(5)?, anomalies_error: r.get(6)? }),
    )?)
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditRow {
    pub id: i64,
    pub ts: String,
    pub action: String,
    pub target: Option<String>,
    pub details: Value,
}

fn audit_row(r: &Row<'_>) -> rusqlite::Result<AuditRow> {
    let d: String = r.get(4)?;
    Ok(AuditRow {
        id: r.get(0)?,
        ts: r.get(1)?,
        action: r.get(2)?,
        target: r.get(3)?,
        details: json(d),
    })
}

pub fn audit_log(case: &Case) -> Result<Vec<AuditRow>, CaseError> {
    let mut stmt = case
        .conn
        .prepare("SELECT id, ts, action, target, details FROM audit_log ORDER BY id")?;
    let rows = stmt.query_map([], audit_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[derive(Debug, Clone, Serialize)]
pub struct FailedFile {
    pub id: i64,
    pub rel_path: String,
    pub parse_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IngestStatus {
    pub counts: BTreeMap<String, i64>,
    pub failed: Vec<FailedFile>,
    pub last_ingest: Option<AuditRow>,
}

/// Files of a root per `parse_status` (every version), the failed ones with their error, and the last
/// ingest audit entry of the case.
pub fn ingest_status(case: &Case, root_id: i64) -> Result<IngestStatus, CaseError> {
    let mut counts = BTreeMap::new();
    let mut stmt = case.conn.prepare(
        "SELECT parse_status, COUNT(*) FROM source_files WHERE root_id = ?1 GROUP BY parse_status",
    )?;
    for row in stmt.query_map([root_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
    })? {
        let (k, v) = row?;
        counts.insert(k, v);
    }
    let mut stmt = case.conn.prepare("SELECT id, rel_path, parse_error FROM source_files WHERE root_id = ?1 AND parse_status = 'failed' ORDER BY rel_path")?;
    let failed = stmt
        .query_map([root_id], |r| {
            Ok(FailedFile {
                id: r.get(0)?,
                rel_path: r.get(1)?,
                parse_error: r.get(2)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    let last_ingest = case
        .conn
        .query_row("SELECT id, ts, action, target, details FROM audit_log WHERE action = 'ingest' ORDER BY id DESC LIMIT 1", [], audit_row)
        .optional()?;
    Ok(IngestStatus {
        counts,
        failed,
        last_ingest,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct RootOverview {
    pub root: RootRow,
    pub identification: Vec<String>,
    pub stores: Vec<StoreRow>,
    pub absent: Vec<String>,
    pub ingest: IngestStatus,
}

pub fn root_overviews(case: &Case) -> Result<Vec<RootOverview>, CaseError> {
    let mut out = Vec::new();
    for root in roots(case)? {
        let ident: String = case.conn.query_row(
            "SELECT identification FROM evidence_roots WHERE id = ?1",
            [root.id],
            |r| r.get(0),
        )?;
        out.push(RootOverview {
            identification: serde_json::from_str(&ident).unwrap_or_default(),
            stores: stores(case, root.id)?,
            absent: absent_stores(case, root.id)?,
            ingest: ingest_status(case, root.id)?,
            root,
        });
    }
    Ok(out)
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceFileRow {
    pub id: i64,
    pub root_id: i64,
    pub store_id: Option<i64>,
    pub rel_path: String,
    pub kind: String,
    pub link_target: Option<String>,
    pub version: i64,
    pub size: i64,
    pub sha256: String,
    pub mtime: Option<String>,
    pub retained: bool,
    pub parse_status: String,
    pub parse_error: Option<String>,
    pub record_count: i64,
    pub anomaly_count: i64,
}

pub fn source_files(case: &Case, root_id: i64) -> Result<Vec<SourceFileRow>, CaseError> {
    let mut stmt = case.conn.prepare(
        "SELECT id, root_id, store_id, rel_path, size, sha256, mtime, retained, parse_status, parse_error, record_count, anomaly_count, kind, link_target, version FROM source_files WHERE root_id = ?1 ORDER BY rel_path, version",
    )?;
    let rows = stmt.query_map([root_id], |r| {
        Ok(SourceFileRow {
            id: r.get(0)?,
            root_id: r.get(1)?,
            store_id: r.get(2)?,
            rel_path: r.get(3)?,
            size: r.get(4)?,
            sha256: r.get(5)?,
            mtime: r.get(6)?,
            retained: r.get::<_, i64>(7)? == 1,
            parse_status: r.get(8)?,
            parse_error: r.get(9)?,
            record_count: r.get(10)?,
            anomaly_count: r.get(11)?,
            kind: r.get(12)?,
            link_target: r.get(13)?,
            version: r.get(14)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionRow {
    pub id: i64,
    pub root_id: i64,
    pub store_id: i64,
    pub harness: String,
    pub harness_session_id: String,
    pub kind: String,
    pub parent_session_id: Option<i64>,
    pub title: Option<String>,
    pub project_path: Option<String>,
    pub git_branch: Option<String>,
    pub harness_version: Option<String>,
    pub models: Vec<String>,
    pub first_ts: Option<String>,
    pub first_ts_origin: String,
    pub last_ts: Option<String>,
    pub last_ts_origin: String,
    pub message_count: i64,
    pub tool_call_count: i64,
    pub anomaly_count: i64,
    pub child_count: i64,
}

#[derive(Debug, Clone, Default)]
pub struct SessionFilter {
    pub root_id: Option<i64>,
    pub harness: Option<String>,
    pub kind: Option<String>,
    pub project_contains: Option<String>,
    /// Sessions that end at or after `from` / start at or before `to`. A date-only value (`YYYY-MM-DD`)
    /// covers the whole UTC day. Sessions with no timestamp at all are excluded when either is set.
    pub from: Option<String>,
    pub to: Option<String>,
    pub has_children: bool,
    pub has_anomalies: bool,
}

fn day_bound(v: &str, end: bool) -> String {
    if v.len() == 10 {
        format!(
            "{v}T{}",
            if end {
                "23:59:59.999Z"
            } else {
                "00:00:00.000Z"
            }
        )
    } else {
        v.to_string()
    }
}

const SESSION_SELECT: &str = "SELECT s.id, st.root_id, s.store_id, st.harness, s.harness_session_id, s.kind, s.parent_session_id, s.title, s.project_path, s.git_branch, s.harness_version, s.models, s.first_ts, s.first_ts_origin, s.last_ts, s.last_ts_origin, s.message_count, s.tool_call_count,
    (SELECT COUNT(*) FROM anomalies a WHERE a.session_id = s.id), (SELECT COUNT(*) FROM sessions c WHERE c.parent_session_id = s.id)
    FROM sessions s JOIN stores st ON st.id = s.store_id";

fn session_row(r: &Row<'_>) -> rusqlite::Result<SessionRow> {
    let models: String = r.get(11)?;
    Ok(SessionRow {
        id: r.get(0)?,
        root_id: r.get(1)?,
        store_id: r.get(2)?,
        harness: r.get(3)?,
        harness_session_id: r.get(4)?,
        kind: r.get(5)?,
        parent_session_id: r.get(6)?,
        title: r.get(7)?,
        project_path: r.get(8)?,
        git_branch: r.get(9)?,
        harness_version: r.get(10)?,
        models: serde_json::from_str(&models).unwrap_or_default(),
        first_ts: r.get(12)?,
        first_ts_origin: r.get(13)?,
        last_ts: r.get(14)?,
        last_ts_origin: r.get(15)?,
        message_count: r.get(16)?,
        tool_call_count: r.get(17)?,
        anomaly_count: r.get(18)?,
        child_count: r.get(19)?,
    })
}

pub fn sessions(case: &Case, f: &SessionFilter) -> Result<Vec<SessionRow>, CaseError> {
    let mut sql = format!("{SESSION_SELECT} WHERE 1 = 1");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(r) = f.root_id {
        sql.push_str(" AND st.root_id = ?");
        args.push(Box::new(r));
    }
    if let Some(h) = &f.harness {
        sql.push_str(" AND st.harness = ?");
        args.push(Box::new(h.clone()));
    }
    if let Some(k) = &f.kind {
        sql.push_str(" AND s.kind = ?");
        args.push(Box::new(k.clone()));
    }
    if let Some(p) = &f.project_contains {
        let escaped = p
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        sql.push_str(" AND s.project_path LIKE ? ESCAPE '\\'");
        args.push(Box::new(format!("%{escaped}%")));
    }
    if let Some(from) = &f.from {
        sql.push_str(" AND COALESCE(s.last_ts, s.first_ts) >= ?");
        args.push(Box::new(day_bound(from, false)));
    }
    if let Some(to) = &f.to {
        sql.push_str(" AND COALESCE(s.first_ts, s.last_ts) <= ?");
        args.push(Box::new(day_bound(to, true)));
    }
    if f.has_children {
        sql.push_str(" AND EXISTS (SELECT 1 FROM sessions c WHERE c.parent_session_id = s.id)");
    }
    if f.has_anomalies {
        sql.push_str(" AND EXISTS (SELECT 1 FROM anomalies a WHERE a.session_id = s.id)");
    }
    sql.push_str(" ORDER BY s.first_ts DESC, s.id DESC");
    let mut stmt = case.conn.prepare(&sql)?;
    let rows = stmt.query_map(
        params_from_iter(args.iter().map(|a| a.as_ref())),
        session_row,
    )?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn session(case: &Case, id: i64) -> Result<Option<SessionRow>, CaseError> {
    Ok(case
        .conn
        .query_row(
            &format!("{SESSION_SELECT} WHERE s.id = ?1"),
            [id],
            session_row,
        )
        .optional()?)
}

pub fn children(case: &Case, id: i64) -> Result<Vec<SessionRow>, CaseError> {
    let mut stmt = case.conn.prepare(&format!(
        "{SESSION_SELECT} WHERE s.parent_session_id = ?1 ORDER BY s.first_ts, s.id"
    ))?;
    let rows = stmt.query_map([id], session_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The parent chain of a session, root first. Stops at 64 hops or on a cycle.
pub fn ancestors(case: &Case, id: i64) -> Result<Vec<SessionRow>, CaseError> {
    let mut chain = Vec::new();
    let mut seen = std::collections::HashSet::from([id]);
    let mut cur = session(case, id)?.and_then(|s| s.parent_session_id);
    while let Some(pid) = cur {
        if !seen.insert(pid) || chain.len() >= 64 {
            break;
        }
        let Some(p) = session(case, pid)? else { break };
        cur = p.parent_session_id;
        chain.push(p);
    }
    chain.reverse();
    Ok(chain)
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionDetail {
    pub session: SessionRow,
    pub ancestors: Vec<SessionRow>,
    pub children: Vec<SessionRow>,
    pub claims: Vec<ClaimRow>,
}

pub fn session_detail(case: &Case, id: i64) -> Result<Option<SessionDetail>, CaseError> {
    let Some(s) = session(case, id)? else {
        return Ok(None);
    };
    Ok(Some(SessionDetail {
        ancestors: ancestors(case, id)?,
        children: children(case, id)?,
        claims: claims(case, id)?,
        session: s,
    }))
}

#[derive(Debug, Clone, Serialize)]
pub struct BlockRow {
    pub id: i64,
    pub ordinal: i64,
    pub kind: String,
    pub text: Option<String>,
    pub payload: Value,
    pub tool_use_id: Option<String>,
    pub tool_call_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MessageRow {
    pub id: i64,
    pub session_id: i64,
    pub ordinal: i64,
    pub role: String,
    pub harness_record_type: String,
    pub harness_uuid: Option<String>,
    pub parent_uuid: Option<String>,
    pub timestamp: Option<String>,
    pub ts_origin: String,
    pub model: Option<String>,
    pub provenance_id: i64,
    /// Origin of the message's provenance: `stored`, `derived` or `inferred`.
    pub origin: String,
    pub attributes: Value,
    pub blocks: Vec<BlockRow>,
}

const MESSAGE_SELECT: &str = "SELECT m.id, m.session_id, m.ordinal, m.role, m.harness_record_type, m.harness_uuid, m.parent_uuid, m.timestamp, m.ts_origin, m.model, m.provenance_id, m.attributes, p.origin
    FROM messages m JOIN provenance p ON p.id = m.provenance_id";

fn message_row(r: &Row<'_>) -> rusqlite::Result<MessageRow> {
    let attrs: String = r.get(11)?;
    Ok(MessageRow {
        id: r.get(0)?,
        session_id: r.get(1)?,
        ordinal: r.get(2)?,
        role: r.get(3)?,
        harness_record_type: r.get(4)?,
        harness_uuid: r.get(5)?,
        parent_uuid: r.get(6)?,
        timestamp: r.get(7)?,
        ts_origin: r.get(8)?,
        model: r.get(9)?,
        provenance_id: r.get(10)?,
        origin: r.get(12)?,
        attributes: json(attrs),
        blocks: Vec::new(),
    })
}

fn load_blocks(case: &Case, msgs: &mut [MessageRow]) -> Result<(), CaseError> {
    let mut bstmt = case.conn.prepare("SELECT id, ordinal, kind, text, payload, tool_use_id, tool_call_id FROM blocks WHERE message_id = ?1 ORDER BY ordinal")?;
    for m in msgs {
        let blocks = bstmt.query_map([m.id], |r| {
            let payload: String = r.get(4)?;
            Ok(BlockRow {
                id: r.get(0)?,
                ordinal: r.get(1)?,
                kind: r.get(2)?,
                text: r.get(3)?,
                payload: json(payload),
                tool_use_id: r.get(5)?,
                tool_call_id: r.get(6)?,
            })
        })?;
        m.blocks = blocks.collect::<Result<_, _>>()?;
    }
    Ok(())
}

pub fn messages(
    case: &Case,
    session_id: i64,
    include_meta: bool,
) -> Result<Vec<MessageRow>, CaseError> {
    let sql = format!(
        "{MESSAGE_SELECT} WHERE m.session_id = ?1 {} ORDER BY m.ordinal",
        if include_meta {
            ""
        } else {
            "AND m.role != 'meta'"
        }
    );
    let mut stmt = case.conn.prepare(&sql)?;
    let mut out: Vec<MessageRow> = stmt
        .query_map([session_id], message_row)?
        .collect::<Result<_, _>>()?;
    load_blocks(case, &mut out)?;
    Ok(out)
}

pub fn message(case: &Case, id: i64) -> Result<Option<MessageRow>, CaseError> {
    let Some(m) = case
        .conn
        .query_row(
            &format!("{MESSAGE_SELECT} WHERE m.id = ?1"),
            [id],
            message_row,
        )
        .optional()?
    else {
        return Ok(None);
    };
    let mut v = vec![m];
    load_blocks(case, &mut v)?;
    Ok(v.pop())
}

pub fn message_id_for_block(case: &Case, block_id: i64) -> Result<Option<i64>, CaseError> {
    Ok(case
        .conn
        .query_row(
            "SELECT message_id FROM blocks WHERE id = ?1",
            [block_id],
            |r| r.get(0),
        )
        .optional()?)
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolCallRow {
    pub id: i64,
    pub session_id: i64,
    pub name: String,
    pub category: String,
    pub input: Value,
    pub result_text: Option<String>,
    pub result_payload: Option<Value>,
    pub is_error: bool,
    pub started_ts: Option<String>,
    pub ended_ts: Option<String>,
    pub ts_origin: String,
    pub tool_use_block_id: i64,
    pub tool_result_block_id: Option<i64>,
}

fn tool_call_row(r: &Row<'_>) -> rusqlite::Result<ToolCallRow> {
    let input: String = r.get(4)?;
    let payload: Option<String> = r.get(6)?;
    Ok(ToolCallRow {
        id: r.get(0)?,
        session_id: r.get(1)?,
        name: r.get(2)?,
        category: r.get(3)?,
        input: json(input),
        result_text: r.get(5)?,
        result_payload: payload.map(json),
        is_error: r.get::<_, i64>(7)? == 1,
        started_ts: r.get(8)?,
        ended_ts: r.get(9)?,
        ts_origin: r.get(10)?,
        tool_use_block_id: r.get(11)?,
        tool_result_block_id: r.get(12)?,
    })
}

pub fn tool_calls(case: &Case, session_id: i64) -> Result<Vec<ToolCallRow>, CaseError> {
    let mut stmt = case.conn.prepare(&format!(
        "SELECT {TOOL_CALL_COLS} FROM tool_calls t WHERE t.session_id = ?1 ORDER BY t.id"
    ))?;
    let rows = stmt.query_map([session_id], tool_call_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[derive(Debug, Clone, Serialize)]
pub struct ObservationRow {
    pub id: i64,
    pub session_id: i64,
    pub kind: String,
    pub path: Option<String>,
    pub command: Option<String>,
    pub before_blob: Option<String>,
    pub after_blob: Option<String>,
    pub timestamp: Option<String>,
    pub ts_origin: String,
    pub confidence: String,
    pub details: Value,
    pub derived_from_tool_call_id: Option<i64>,
    pub derived_from_block_id: Option<i64>,
    pub derived_from_provenance_id: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct ObservationFilter {
    pub root_id: Option<i64>,
    pub session_id: Option<i64>,
    pub kind: Option<String>,
    pub kinds: Vec<String>,
}

fn observation_row(r: &Row<'_>) -> rusqlite::Result<ObservationRow> {
    let details: String = r.get(10)?;
    Ok(ObservationRow {
        id: r.get(0)?,
        session_id: r.get(1)?,
        kind: r.get(2)?,
        path: r.get(3)?,
        command: r.get(4)?,
        before_blob: r.get(5)?,
        after_blob: r.get(6)?,
        timestamp: r.get(7)?,
        ts_origin: r.get(8)?,
        confidence: r.get(9)?,
        details: json(details),
        derived_from_tool_call_id: r.get(11)?,
        derived_from_block_id: r.get(12)?,
        derived_from_provenance_id: r.get(13)?,
    })
}

pub fn observations(case: &Case, f: &ObservationFilter) -> Result<Vec<ObservationRow>, CaseError> {
    let mut sql = format!("SELECT {OBSERVATION_COLS} FROM observations o JOIN sessions s ON s.id = o.session_id JOIN stores st ON st.id = s.store_id WHERE 1 = 1");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(r) = f.root_id {
        sql.push_str(" AND st.root_id = ?");
        args.push(Box::new(r));
    }
    if let Some(s) = f.session_id {
        sql.push_str(" AND o.session_id = ?");
        args.push(Box::new(s));
    }
    if let Some(k) = &f.kind {
        sql.push_str(" AND o.kind = ?");
        args.push(Box::new(k.clone()));
    }
    if !f.kinds.is_empty() {
        sql.push_str(&format!(
            " AND o.kind IN ({})",
            vec!["?"; f.kinds.len()].join(", ")
        ));
        for k in &f.kinds {
            args.push(Box::new(k.clone()));
        }
    }
    sql.push_str(" ORDER BY o.timestamp, o.id");
    let mut stmt = case.conn.prepare(&sql)?;
    let rows = stmt.query_map(
        params_from_iter(args.iter().map(|a| a.as_ref())),
        observation_row,
    )?;
    Ok(rows.collect::<Result<_, _>>()?)
}

const TOOL_CALL_COLS: &str = "t.id, t.session_id, t.name, t.category, t.input, t.result_text, t.result_payload, t.is_error, t.started_ts, t.ended_ts, t.ts_origin, t.tool_use_block_id, t.tool_result_block_id";
const OBSERVATION_COLS: &str = "o.id, o.session_id, o.kind, o.path, o.command, o.before_blob, o.after_blob, o.timestamp, o.ts_origin, o.confidence, o.details, o.derived_from_tool_call_id, o.derived_from_block_id, o.derived_from_provenance_id";

/// Tool calls whose `tool_use` or `tool_result` block belongs to the message.
pub fn tool_calls_for_message(case: &Case, message_id: i64) -> Result<Vec<ToolCallRow>, CaseError> {
    let mut stmt = case.conn.prepare(&format!(
        "SELECT {TOOL_CALL_COLS} FROM tool_calls t WHERE t.tool_use_block_id IN (SELECT id FROM blocks WHERE message_id = ?1)
            OR t.tool_result_block_id IN (SELECT id FROM blocks WHERE message_id = ?1) ORDER BY t.id"
    ))?;
    let rows = stmt.query_map([message_id], tool_call_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Observations derived from the message's record, one of its blocks, or one of its tool calls.
pub fn observations_for_message(
    case: &Case,
    message_id: i64,
) -> Result<Vec<ObservationRow>, CaseError> {
    let mut stmt = case.conn.prepare(&format!(
        "SELECT {OBSERVATION_COLS} FROM observations o WHERE
            o.derived_from_block_id IN (SELECT id FROM blocks WHERE message_id = ?1)
         OR o.derived_from_tool_call_id IN (SELECT t.id FROM tool_calls t JOIN blocks b ON b.id = t.tool_use_block_id OR b.id = t.tool_result_block_id WHERE b.message_id = ?1)
         OR o.derived_from_provenance_id = (SELECT provenance_id FROM messages WHERE id = ?1)
         ORDER BY o.id"
    ))?;
    let rows = stmt.query_map([message_id], observation_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[derive(Debug, Clone, Serialize)]
pub struct ActivityRow {
    #[serde(flatten)]
    pub observation: ObservationRow,
    /// The message the observation came from: its block's message, its tool call's `tool_use` message, or
    /// the message whose record it was derived from. `None` for sidecar records that are not messages.
    pub message_id: Option<i64>,
    pub session_title: Option<String>,
    pub harness_session_id: String,
}

pub fn activity(case: &Case, f: &ObservationFilter) -> Result<Vec<ActivityRow>, CaseError> {
    let obs = observations(case, f)?;
    let mut stmt = case.conn.prepare(
        "SELECT COALESCE(
             (SELECT b.message_id FROM blocks b WHERE b.id = ?1),
             (SELECT b.message_id FROM tool_calls t JOIN blocks b ON b.id = t.tool_use_block_id WHERE t.id = ?2),
             (SELECT m.id FROM messages m WHERE m.provenance_id = ?3)),
           s.title, s.harness_session_id FROM sessions s WHERE s.id = ?4",
    )?;
    let mut out = Vec::with_capacity(obs.len());
    for o in obs {
        let (message_id, session_title, harness_session_id) = stmt.query_row(
            params![
                o.derived_from_block_id,
                o.derived_from_tool_call_id,
                o.derived_from_provenance_id,
                o.session_id
            ],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        out.push(ActivityRow {
            observation: o,
            message_id,
            session_title,
            harness_session_id,
        });
    }
    Ok(out)
}

#[derive(Debug, Clone, Serialize)]
pub struct AnomalyRow {
    pub id: i64,
    pub root_id: i64,
    pub store_id: Option<i64>,
    pub source_file_id: Option<i64>,
    pub session_id: Option<i64>,
    pub kind: String,
    pub severity: String,
    pub byte_offset: Option<i64>,
    pub provenance_id: Option<i64>,
    pub message: String,
    pub details: Value,
}

#[derive(Debug, Clone, Default)]
pub struct AnomalyFilter {
    pub root_id: Option<i64>,
    pub kind: Option<String>,
    pub severity: Option<String>,
    pub session_id: Option<i64>,
}

pub fn anomalies(case: &Case, f: &AnomalyFilter) -> Result<Vec<AnomalyRow>, CaseError> {
    let mut sql = String::from("SELECT id, root_id, store_id, source_file_id, session_id, kind, severity, byte_offset, message, details, provenance_id FROM anomalies WHERE 1 = 1");
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(r) = f.root_id {
        sql.push_str(" AND root_id = ?");
        args.push(Box::new(r));
    }
    if let Some(k) = &f.kind {
        sql.push_str(" AND kind = ?");
        args.push(Box::new(k.clone()));
    }
    if let Some(s) = &f.severity {
        sql.push_str(" AND severity = ?");
        args.push(Box::new(s.clone()));
    }
    if let Some(s) = f.session_id {
        sql.push_str(" AND session_id = ?");
        args.push(Box::new(s));
    }
    sql.push_str(" ORDER BY id");
    let mut stmt = case.conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter().map(|a| a.as_ref())), |r| {
        let details: String = r.get(9)?;
        Ok(AnomalyRow {
            id: r.get(0)?,
            root_id: r.get(1)?,
            store_id: r.get(2)?,
            source_file_id: r.get(3)?,
            session_id: r.get(4)?,
            kind: r.get(5)?,
            severity: r.get(6)?,
            byte_offset: r.get(7)?,
            provenance_id: r.get(10)?,
            message: r.get(8)?,
            details: json(details),
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[derive(Debug, Clone, Serialize)]
pub struct ClaimRow {
    pub id: i64,
    pub session_id: i64,
    pub scheme: String,
    pub claimed_id: String,
    pub source_file_id: i64,
    pub join_status: String,
    pub matched_session_id: Option<i64>,
}

pub fn claims(case: &Case, session_id: i64) -> Result<Vec<ClaimRow>, CaseError> {
    let mut stmt = case.conn.prepare("SELECT id, session_id, scheme, claimed_id, source_file_id, join_status, matched_session_id FROM identity_claims WHERE session_id = ?1 ORDER BY id")?;
    let rows = stmt.query_map([session_id], |r| {
        Ok(ClaimRow {
            id: r.get(0)?,
            session_id: r.get(1)?,
            scheme: r.get(2)?,
            claimed_id: r.get(3)?,
            source_file_id: r.get(4)?,
            join_status: r.get(5)?,
            matched_session_id: r.get(6)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[derive(Debug, Clone, Serialize)]
pub struct ProvenanceRow {
    pub id: i64,
    pub source_file_id: i64,
    pub rel_path: String,
    pub rel_path_encoded: bool,
    pub file_sha256: String,
    pub root_path: String,
    pub retained: bool,
    pub byte_offset: i64,
    pub byte_length: i64,
    pub record_index: i64,
    pub content_sha256: String,
    pub parser_name: String,
    pub parser_version: String,
    pub origin: String,
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
    let p = provenance(case, provenance_id)?
        .ok_or_else(|| CaseError::NotFound(format!("provenance {provenance_id}")))?;
    let path = if p.retained {
        blobs::path(&case.dir, &p.file_sha256)
    } else {
        std::path::Path::new(&p.root_path).join(decode_rel_path(&p.rel_path, p.rel_path_encoded))
    };
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(p.byte_offset as u64))?;
    let mut buf = vec![0u8; p.byte_length as usize];
    f.read_exact(&mut buf)?;
    if sha256_hex(&buf) != p.content_sha256 {
        return Err(CaseError::IntegrityMismatch(format!(
            "record bytes of provenance {provenance_id} do not match its hash"
        )));
    }
    Ok(buf)
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub block_id: i64,
    pub message_id: i64,
    pub session_id: i64,
    pub tool_call_id: Option<i64>,
    pub snippet: String,
    pub session_title: Option<String>,
    pub message_ordinal: i64,
}

/// Case-wide full-text search over message blocks and tool calls (name, input, result). A tool-call hit
/// points at its `tool_use` block. The query is matched as one phrase.
pub fn search(case: &Case, query: &str, limit: usize) -> Result<Vec<SearchHit>, CaseError> {
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = case.conn.prepare(
        "SELECT h.block_id, h.message_id, h.session_id, h.tool_call_id, h.snippet, s.title, m.ordinal FROM (
             SELECT b.id AS block_id, b.message_id AS message_id, m.session_id AS session_id, NULL AS tool_call_id,
                    snippet(blocks_fts, 0, '[', ']', '…', 12) AS snippet, blocks_fts.rank AS rank
             FROM blocks_fts JOIN blocks b ON b.id = blocks_fts.rowid JOIN messages m ON m.id = b.message_id
             WHERE blocks_fts MATCH ?1
             UNION ALL
             SELECT b.id, b.message_id, t.session_id, t.id,
                    snippet(tool_calls_fts, -1, '[', ']', '…', 12), tool_calls_fts.rank
             FROM tool_calls_fts JOIN tool_calls t ON t.id = tool_calls_fts.rowid JOIN blocks b ON b.id = t.tool_use_block_id
             WHERE tool_calls_fts MATCH ?1
         ) h JOIN messages m ON m.id = h.message_id JOIN sessions s ON s.id = h.session_id ORDER BY h.rank LIMIT ?2",
    )?;
    let quoted = format!("\"{}\"", query.replace('"', "\"\""));
    let rows = stmt.query_map(params![quoted, limit as i64], |r| {
        Ok(SearchHit {
            block_id: r.get(0)?,
            message_id: r.get(1)?,
            session_id: r.get(2)?,
            tool_call_id: r.get(3)?,
            snippet: r.get(4)?,
            session_title: r.get(5)?,
            message_ordinal: r.get(6)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub const RAW_WINDOW_MAX: u64 = 65536;

#[derive(Debug, Clone, Serialize)]
pub struct RawWindowBytes {
    pub total_length: u64,
    pub offset: u64,
    pub bytes: Vec<u8>,
}

/// A window of a record's raw bytes. The whole record is read and verified against its hash first
/// (`IntegrityMismatch` otherwise); the window is clamped to the record and to `RAW_WINDOW_MAX`.
pub fn raw_window(
    case: &Case,
    provenance_id: i64,
    offset: u64,
    len: u64,
) -> Result<RawWindowBytes, CaseError> {
    let full = raw_record(case, provenance_id)?;
    let total = full.len() as u64;
    let start = offset.min(total);
    let end = start.saturating_add(len.min(RAW_WINDOW_MAX)).min(total);
    Ok(RawWindowBytes {
        total_length: total,
        offset: start,
        bytes: full[start as usize..end as usize].to_vec(),
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct BlobBytes {
    pub sha256: String,
    pub size: u64,
    pub bytes: Vec<u8>,
    pub truncated: bool,
}

/// The first `cap` bytes of a retained blob. `None` if the case has no such blob.
pub fn blob_bytes(case: &Case, sha: &str, cap: usize) -> Result<Option<BlobBytes>, CaseError> {
    if sha.len() != 64
        || !sha
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(CaseError::Invalid(format!("not a sha256: {sha:?}")));
    }
    let Some(size) = case
        .conn
        .query_row("SELECT size FROM blobs WHERE sha256 = ?1", [sha], |r| {
            r.get::<_, i64>(0)
        })
        .optional()?
    else {
        return Ok(None);
    };
    let mut f = std::fs::File::open(blobs::path(&case.dir, sha))?;
    let mut bytes = Vec::with_capacity(cap.min(size as usize));
    (&mut f).take(cap as u64).read_to_end(&mut bytes)?;
    Ok(Some(BlobBytes {
        sha256: sha.to_string(),
        size: size as u64,
        truncated: (bytes.len() as u64) < size as u64,
        bytes,
    }))
}
