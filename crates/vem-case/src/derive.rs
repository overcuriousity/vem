//! Root-level derivations run after every store of a root is ingested.

use crate::error::CaseError;
use rusqlite::{params, Connection};

/// A `claude:paste-cache` file that no `paste_detected` observation of the root references is pasted
/// content whose prompt is gone from `history.jsonl`: an `orphaned_file` anomaly (info). Idempotent.
pub fn flag_unreferenced_pastes(conn: &Connection, root_id: i64) -> Result<usize, CaseError> {
    let files: Vec<(i64, i64, String)> = {
        let mut stmt = conn.prepare(
            "SELECT f.id, f.store_id, f.rel_path FROM source_files f JOIN stores st ON st.id = f.store_id
             WHERE f.root_id = ?1 AND st.kind = 'claude:paste-cache' AND f.kind = 'file'
               AND f.version = (SELECT MAX(version) FROM source_files g WHERE g.root_id = f.root_id AND g.rel_path = f.rel_path AND g.rel_path_encoded = f.rel_path_encoded)
             ORDER BY f.rel_path",
        )?;
        let rows = stmt.query_map([root_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut added = 0;
    for (file_id, store_id, rel) in files {
        let name = rel.rsplit('/').next().unwrap_or(&rel);
        let stem = name.strip_suffix(".txt").unwrap_or(name);
        let referenced: bool = conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM observations o JOIN sessions s ON s.id = o.session_id JOIN stores st ON st.id = s.store_id, json_each(o.details, '$.pastes') p
                            WHERE st.root_id = ?1 AND o.kind = 'paste_detected' AND json_extract(p.value, '$.content_hash') = ?2)",
            params![root_id, stem],
            |r| r.get(0),
        )?;
        let flagged: bool = conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM anomalies WHERE source_file_id = ?1 AND kind = 'orphaned_file' AND json_extract(details, '$.reason') = 'unreferenced_paste')",
            [file_id],
            |r| r.get(0),
        )?;
        if referenced || flagged {
            continue;
        }
        conn.execute(
            "INSERT INTO anomalies (root_id, store_id, source_file_id, kind, severity, message, details) VALUES (?1, ?2, ?3, 'orphaned_file', 'info', ?4, ?5)",
            params![
                root_id, store_id, file_id,
                format!("{rel} holds pasted content that no prompt in history.jsonl references; the prompt history may have been cleared"),
                serde_json::json!({ "reason": "unreferenced_paste", "content_hash": stem }).to_string()
            ],
        )?;
        added += 1;
    }
    Ok(added)
}
