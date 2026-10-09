//! One `Event` per message, tool call and observation, with provenance for Vestigo.

use crate::error::CaseError;
use crate::Case;
use rusqlite::params_from_iter;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Case,
    Root(i64),
    Session(i64),
}

#[derive(Debug, Clone, Serialize)]
pub struct EventProvenance {
    pub source_file: String,
    pub file_sha256: String,
    pub byte_offset: u64,
    pub content_sha256: String,
    pub file_size: u64,
    pub file_mtime: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub datetime: Option<String>,
    pub timestamp_desc: String,
    pub message: String,
    pub source: String,
    pub source_long: String,
    pub display_name: String,
    pub tags: Vec<String>,
    pub attributes: BTreeMap<String, String>,
    pub provenance: EventProvenance,
}

pub fn timestamp_desc(what: &str, origin: &str) -> String {
    match origin {
        "absent" => "No Timestamp".to_string(),
        "stored" => format!("{what} (stored)"),
        "stored_local_clock" => format!("{what} (stored, local clock)"),
        "file_mtime" => "Session File Modified (inferred)".to_string(),
        "neighbor_interpolated" => format!("{what} (interpolated, inferred)"),
        other => format!("{what} ({other})"),
    }
}

fn source_for(harness: &str) -> (String, String) {
    let upper = harness.replace('-', "_").to_uppercase();
    (format!("AI:{upper}"), harness.replace('-', "_"))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { format!("{}…", s.chars().take(max).collect::<String>()) }
}

fn scope_clause(scope: &Scope, args: &mut Vec<Box<dyn rusqlite::ToSql>>) -> String {
    match scope {
        Scope::Case => String::new(),
        Scope::Root(r) => { args.push(Box::new(*r)); " AND st.root_id = ?".to_string() }
        Scope::Session(s) => { args.push(Box::new(*s)); " AND s.id = ?".to_string() }
    }
}

fn put(attrs: &mut BTreeMap<String, String>, key: &str, v: Option<String>) {
    if let Some(v) = v {
        if !v.is_empty() {
            attrs.insert(key.to_string(), v);
        }
    }
}

/// `timestamp_desc` of an event whose record has no timestamp and which is placed at its session's start.
pub const SESSION_START_INFERRED: &str = "Session Start (inferred, record has no timestamp)";

struct Common {
    harness: String, root_label: String, session_db_id: i64, session_hid: String, session_kind: String, project_path: Option<String>,
    rel_path: String, file_sha: String, file_size: i64, file_mtime: Option<String>, byte_offset: i64, content_sha: String,
    session_first_ts: Option<String>,
}

impl Common {
    /// The event time and its description: the record's own timestamp; else the session's start, labelled
    /// as inferred and tagged `inferred` (timeline tools drop events without a time); else none.
    fn when(&self, ts: Option<String>, what: &str, origin: &str, tags: &mut Vec<String>) -> (Option<String>, String) {
        match (ts, &self.session_first_ts) {
            (Some(t), _) => (Some(t), timestamp_desc(what, origin)),
            (None, Some(start)) => {
                tags.push("inferred".to_string());
                (Some(start.clone()), SESSION_START_INFERRED.to_string())
            }
            (None, None) => (None, timestamp_desc(what, origin)),
        }
    }

    fn base(&self, record_kind: &str, ts_origin: &str) -> (BTreeMap<String, String>, EventProvenance, String) {
        let mut a = BTreeMap::new();
        put(&mut a, "harness", Some(self.harness.clone()));
        put(&mut a, "root_label", Some(self.root_label.clone()));
        put(&mut a, "session_db_id", Some(self.session_db_id.to_string()));
        put(&mut a, "session_id", Some(self.session_hid.clone()));
        put(&mut a, "session_kind", Some(self.session_kind.clone()));
        put(&mut a, "project_path", self.project_path.clone());
        put(&mut a, "record_kind", Some(record_kind.to_string()));
        put(&mut a, "ts_origin", Some(ts_origin.to_string()));
        put(&mut a, "evidence_file", Some(self.rel_path.clone()));
        put(&mut a, "evidence_file_sha256", Some(self.file_sha.clone()));
        put(&mut a, "evidence_byte_offset", Some(self.byte_offset.to_string()));
        put(&mut a, "evidence_record_sha256", Some(self.content_sha.clone()));
        let prov = EventProvenance { source_file: self.rel_path.clone(), file_sha256: self.file_sha.clone(), byte_offset: self.byte_offset as u64, content_sha256: self.content_sha.clone(), file_size: self.file_size as u64, file_mtime: self.file_mtime.clone() };
        (a, prov, format!("{}:{}", self.root_label, self.rel_path))
    }
}

const COMMON_COLS: &str = "st.harness, r.label, s.id, s.harness_session_id, s.kind, s.project_path, f.rel_path, f.sha256, f.size, f.mtime, p.byte_offset, p.content_sha256, s.first_ts";

fn common_from(r: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<Common> {
    Ok(Common {
        harness: r.get(offset)?, root_label: r.get(offset + 1)?, session_db_id: r.get(offset + 2)?, session_hid: r.get(offset + 3)?, session_kind: r.get(offset + 4)?,
        project_path: r.get(offset + 5)?, rel_path: r.get(offset + 6)?, file_sha: r.get(offset + 7)?, file_size: r.get(offset + 8)?, file_mtime: r.get(offset + 9)?,
        byte_offset: r.get(offset + 10)?, content_sha: r.get(offset + 11)?, session_first_ts: r.get(offset + 12)?,
    })
}

pub fn events(case: &Case, scope: &Scope) -> Result<Vec<Event>, CaseError> {
    let mut out = Vec::new();

    // Messages: text is the concatenation of the message's text blocks.
    {
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        let clause = scope_clause(scope, &mut args);
        let sql = format!(
            "SELECT m.id, m.role, m.harness_record_type, m.harness_uuid, m.timestamp, m.ts_origin, m.model,
                    (SELECT GROUP_CONCAT(b.text, char(10)) FROM blocks b WHERE b.message_id = m.id AND b.kind = 'text' AND b.text IS NOT NULL),
                    {COMMON_COLS}
             FROM messages m JOIN sessions s ON s.id = m.session_id JOIN stores st ON st.id = s.store_id JOIN evidence_roots r ON r.id = st.root_id
                  JOIN provenance p ON p.id = m.provenance_id JOIN source_files f ON f.id = p.source_file_id
             WHERE 1 = 1 {clause} ORDER BY m.id"
        );
        let mut stmt = case.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(args.iter().map(|a| a.as_ref())), |r| {
            let role: String = r.get(1)?;
            let record_type: String = r.get(2)?;
            let uuid: Option<String> = r.get(3)?;
            let ts: Option<String> = r.get(4)?;
            let origin: String = r.get(5)?;
            let model: Option<String> = r.get(6)?;
            let text: Option<String> = r.get(7)?;
            let c = common_from(r, 8)?;
            let (mut attrs, prov, display) = c.base("message", &origin);
            put(&mut attrs, "role", Some(role.clone()));
            put(&mut attrs, "record_type", Some(record_type.clone()));
            put(&mut attrs, "message_uuid", uuid);
            put(&mut attrs, "model", model);
            let (source, slug) = source_for(&c.harness);
            let body = text.unwrap_or_default();
            let message = if body.is_empty() { format!("[{}] {}: <{}>", c.harness, role, record_type) } else { format!("[{}] {}: {}", c.harness, role, truncate(&body, 4000)) };
            let mut tags = vec![c.harness.clone(), role.clone(), origin.clone()];
            let (datetime, desc) = c.when(ts, "Message Timestamp", &origin, &mut tags);
            Ok(Event { datetime, timestamp_desc: desc, message, source, source_long: format!("{slug}:message:{role}"), display_name: display, tags, attributes: attrs, provenance: prov })
        })?;
        out.extend(rows.collect::<Result<Vec<_>, _>>()?);
    }

    // Tool calls: one event at the start time; provenance is the tool_use block's record.
    {
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        let clause = scope_clause(scope, &mut args);
        let sql = format!(
            "SELECT t.id, t.name, t.category, t.input, t.started_ts, t.ts_origin, t.is_error, {COMMON_COLS}
             FROM tool_calls t JOIN sessions s ON s.id = t.session_id JOIN stores st ON st.id = s.store_id JOIN evidence_roots r ON r.id = st.root_id
                  JOIN blocks b ON b.id = t.tool_use_block_id JOIN provenance p ON p.id = b.provenance_id JOIN source_files f ON f.id = p.source_file_id
             WHERE 1 = 1 {clause} ORDER BY t.id"
        );
        let mut stmt = case.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(args.iter().map(|a| a.as_ref())), |r| {
            let name: String = r.get(1)?;
            let category: String = r.get(2)?;
            let input: String = r.get(3)?;
            let ts: Option<String> = r.get(4)?;
            let origin: String = r.get(5)?;
            let is_error: i64 = r.get(6)?;
            let c = common_from(r, 7)?;
            let (mut attrs, prov, display) = c.base("tool_call", &origin);
            put(&mut attrs, "tool_name", Some(name.clone()));
            put(&mut attrs, "tool_category", Some(category.clone()));
            put(&mut attrs, "tool_error", Some((is_error == 1).to_string()));
            let summary: String = serde_json::from_str::<serde_json::Value>(&input)
                .ok()
                .and_then(|v| ["command", "file_path", "url", "pattern", "description", "prompt"].iter().find_map(|k| v.get(k).and_then(|x| x.as_str()).map(String::from)))
                .unwrap_or_else(|| truncate(&input, 200));
            let (source, slug) = source_for(&c.harness);
            let mut tags = vec![c.harness.clone(), "tool_call".to_string(), category.clone(), origin.clone()];
            let (datetime, desc) = c.when(ts, "Tool Call Started", &origin, &mut tags);
            Ok(Event { datetime, timestamp_desc: desc, message: format!("[{}] tool {}: {}", c.harness, name, truncate(&summary, 500)), source, source_long: format!("{slug}:tool_call:{category}"), display_name: display, tags, attributes: attrs, provenance: prov })
        })?;
        out.extend(rows.collect::<Result<Vec<_>, _>>()?);
    }

    // Observations: provenance comes from the tool call's tool_use block, the block, or the record.
    {
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        let clause = scope_clause(scope, &mut args);
        let sql = format!(
            "SELECT o.id, o.kind, o.path, o.command, o.confidence, o.timestamp, o.ts_origin, {COMMON_COLS}
             FROM observations o JOIN sessions s ON s.id = o.session_id JOIN stores st ON st.id = s.store_id JOIN evidence_roots r ON r.id = st.root_id
                  JOIN provenance p ON p.id = COALESCE(o.derived_from_provenance_id,
                        (SELECT b.provenance_id FROM blocks b WHERE b.id = o.derived_from_block_id),
                        (SELECT b.provenance_id FROM tool_calls t JOIN blocks b ON b.id = t.tool_use_block_id WHERE t.id = o.derived_from_tool_call_id))
                  JOIN source_files f ON f.id = p.source_file_id
             WHERE 1 = 1 {clause} ORDER BY o.id"
        );
        let mut stmt = case.conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(args.iter().map(|a| a.as_ref())), |r| {
            let kind: String = r.get(1)?;
            let path: Option<String> = r.get(2)?;
            let command: Option<String> = r.get(3)?;
            let confidence: String = r.get(4)?;
            let ts: Option<String> = r.get(5)?;
            let origin: String = r.get(6)?;
            let c = common_from(r, 7)?;
            let (mut attrs, prov, display) = c.base("observation", &origin);
            put(&mut attrs, "observation_kind", Some(kind.clone()));
            put(&mut attrs, "path", path.clone());
            put(&mut attrs, "command", command.clone());
            put(&mut attrs, "confidence", Some(confidence.clone()));
            let what = command.clone().or(path.clone()).unwrap_or_default();
            let (source, slug) = source_for(&c.harness);
            let mut tags = vec![c.harness.clone(), "observation".to_string(), kind.clone(), confidence, origin.clone()];
            let (datetime, desc) = c.when(ts, "Observation Time", &origin, &mut tags);
            Ok(Event { datetime, timestamp_desc: desc, message: format!("[{}] {}: {}", c.harness, kind, truncate(&what, 500)), source, source_long: format!("{slug}:observation:{kind}"), display_name: display, tags, attributes: attrs, provenance: prov })
        })?;
        out.extend(rows.collect::<Result<Vec<_>, _>>()?);
    }

    out.sort_by(|a, b| match (&a.datetime, &b.datetime) {
        (Some(x), Some(y)) => x.cmp(y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    Ok(out)
}
