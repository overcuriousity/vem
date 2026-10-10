//! Examiner annotations (tag, bookmark, note) on case rows. Every create and delete writes an audit entry
//! holding the whole annotation, so the annotation history can be rebuilt from the audit log alone.

use crate::{Case, CaseError};
use rusqlite::{params, OptionalExtension, Row};
use serde::Serialize;

/// Annotatable row types and their tables.
pub const TARGET_TYPES: &[(&str, &str)] = &[
    ("session", "sessions"),
    ("message", "messages"),
    ("block", "blocks"),
    ("tool_call", "tool_calls"),
    ("observation", "observations"),
    ("anomaly", "anomalies"),
    ("source_file", "source_files"),
];
pub const KINDS: &[&str] = &["tag", "bookmark", "note"];
pub const MAX_VALUE_LEN: usize = 65536;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Annotation {
    pub id: i64,
    pub target_type: String,
    pub target_id: i64,
    pub kind: String,
    pub value: String,
    pub created_at: String,
}

const SELECT: &str = "SELECT id, target_type, target_id, kind, value, created_at FROM annotations";

fn row(r: &Row<'_>) -> rusqlite::Result<Annotation> {
    Ok(Annotation {
        id: r.get(0)?,
        target_type: r.get(1)?,
        target_id: r.get(2)?,
        kind: r.get(3)?,
        value: r.get(4)?,
        created_at: r.get(5)?,
    })
}

pub fn create(
    case: &Case,
    target_type: &str,
    target_id: i64,
    kind: &str,
    value: &str,
) -> Result<Annotation, CaseError> {
    let table = TARGET_TYPES
        .iter()
        .find(|(t, _)| *t == target_type)
        .map(|(_, tbl)| *tbl)
        .ok_or_else(|| {
            CaseError::Invalid(format!("unknown annotation target type {target_type:?}"))
        })?;
    if !KINDS.contains(&kind) {
        return Err(CaseError::Invalid(format!(
            "unknown annotation kind {kind:?} (tag, bookmark or note)"
        )));
    }
    if value.trim().is_empty() {
        return Err(CaseError::Invalid("annotation value is empty".into()));
    }
    if value.len() > MAX_VALUE_LEN {
        return Err(CaseError::Invalid(format!(
            "annotation value exceeds {MAX_VALUE_LEN} bytes"
        )));
    }
    let exists = case
        .conn
        .query_row(
            &format!("SELECT 1 FROM {table} WHERE id = ?1"),
            [target_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !exists {
        return Err(CaseError::Invalid(format!(
            "no {target_type} with id {target_id}"
        )));
    }
    let tx = case.conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO annotations (target_type, target_id, kind, value, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![target_type, target_id, kind, value, crate::case::now()],
    )?;
    let a = tx.query_row(
        &format!("{SELECT} WHERE id = ?1"),
        [tx.last_insert_rowid()],
        row,
    )?;
    case.audit(
        "annotation.create",
        Some(&format!("{target_type}:{target_id}")),
        serde_json::to_value(&a)?,
    )?;
    tx.commit()?;
    Ok(a)
}

pub fn list(
    case: &Case,
    target_type: Option<&str>,
    target_id: Option<i64>,
) -> Result<Vec<Annotation>, CaseError> {
    let mut stmt = case.conn.prepare(&format!("{SELECT} WHERE (?1 IS NULL OR target_type = ?1) AND (?2 IS NULL OR target_id = ?2) ORDER BY id"))?;
    let rows = stmt.query_map(params![target_type, target_id], row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Annotations on the message, on any of its blocks, and on any tool call whose use or result block is in it.
pub fn list_for_message(case: &Case, message_id: i64) -> Result<Vec<Annotation>, CaseError> {
    let mut stmt = case.conn.prepare(&format!(
        "{SELECT} WHERE (target_type = 'message' AND target_id = ?1)
            OR (target_type = 'block' AND target_id IN (SELECT id FROM blocks WHERE message_id = ?1))
            OR (target_type = 'tool_call' AND target_id IN (SELECT t.id FROM tool_calls t JOIN blocks b ON b.id = t.tool_use_block_id OR b.id = t.tool_result_block_id WHERE b.message_id = ?1))
         ORDER BY id"
    ))?;
    let rows = stmt.query_map([message_id], row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn delete(case: &Case, id: i64) -> Result<(), CaseError> {
    let a = case
        .conn
        .query_row(&format!("{SELECT} WHERE id = ?1"), [id], row)
        .optional()?
        .ok_or_else(|| CaseError::NotFound(format!("annotation {id}")))?;
    let tx = case.conn.unchecked_transaction()?;
    tx.execute("DELETE FROM annotations WHERE id = ?1", [id])?;
    case.audit(
        "annotation.delete",
        Some(&format!("{}:{}", a.target_type, a.target_id)),
        serde_json::to_value(&a)?,
    )?;
    tx.commit()?;
    Ok(())
}
