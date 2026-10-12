//! Post-ingest passes: per evidence root, parent links, identity-claim resolution and session bounds;
//! case-wide, the `cursor:` identity claims, which join sessions across roots.

use crate::error::CaseError;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::{BTreeSet, HashMap};

pub fn link_sessions(conn: &Connection, root_id: i64) -> Result<(), CaseError> {
    conn.execute(
        "UPDATE sessions SET parent_session_id = (
            SELECT p.id FROM sessions p JOIN stores ps ON ps.id = p.store_id
            WHERE ps.root_id = ?1 AND p.harness_session_id = sessions.parent_harness_session_id AND p.id != sessions.id
            ORDER BY p.id LIMIT 1)
         WHERE parent_harness_session_id IS NOT NULL AND parent_session_id IS NULL
           AND store_id IN (SELECT id FROM stores WHERE root_id = ?1)",
        [root_id],
    )?;

    let claims: Vec<(i64, i64, String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT c.id, c.session_id, c.scheme, c.claimed_id FROM identity_claims c
             JOIN sessions s ON s.id = c.session_id JOIN stores st ON st.id = s.store_id
             WHERE st.root_id = ?1 AND c.join_status = 'unmatched' AND c.scheme NOT LIKE 'cursor:%' ORDER BY c.id",
        )?;
        let rows = stmt.query_map([root_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?;
        rows.collect::<Result<_, _>>()?
    };
    for (claim_id, session_id, scheme, claimed_id) in claims {
        let hits: Vec<i64> = if scheme == "claude:spawning_tool_use_id" {
            let mut stmt = conn.prepare(
                "SELECT DISTINCT m.session_id FROM blocks b JOIN messages m ON m.id = b.message_id
                 JOIN sessions s ON s.id = m.session_id JOIN stores st ON st.id = s.store_id
                 WHERE st.root_id = ?1 AND b.kind = 'tool_use' AND b.tool_use_id = ?2",
            )?;
            let rows = stmt.query_map(params![root_id, claimed_id], |r| r.get(0))?;
            rows.collect::<Result<_, _>>()?
        } else {
            let mut stmt = conn.prepare(
                "SELECT s.id FROM sessions s JOIN stores st ON st.id = s.store_id
                 WHERE st.root_id = ?1 AND s.harness_session_id = ?2 AND s.id != ?3",
            )?;
            let rows = stmt.query_map(params![root_id, claimed_id, session_id], |r| r.get(0))?;
            rows.collect::<Result<_, _>>()?
        };
        match hits.len() {
            0 => {}
            1 => {
                conn.execute("UPDATE identity_claims SET join_status = 'matched', matched_session_id = ?2 WHERE id = ?1", params![claim_id, hits[0]])?;
                if scheme == "claude:spawning_tool_use_id" {
                    conn.execute("UPDATE sessions SET parent_session_id = ?2 WHERE id = ?1 AND parent_session_id IS NULL", params![session_id, hits[0]])?;
                } else if scheme == "claude:origin_session_id" {
                    conn.execute(
                        "UPDATE sessions SET kind = 'resumed' WHERE id = ?1 AND kind = 'primary'",
                        [session_id],
                    )?;
                }
            }
            _ => {
                conn.execute(
                    "UPDATE identity_claims SET join_status = 'ambiguous' WHERE id = ?1",
                    [claim_id],
                )?;
            }
        }
    }

    let unlinked: Vec<(i64, String, i64, Option<i64>)> = {
        let mut stmt = conn.prepare(
            "SELECT s.id, s.harness_session_id, st.id, s.primary_source_file_id FROM sessions s JOIN stores st ON st.id = s.store_id
             WHERE st.root_id = ?1 AND s.kind = 'subagent' AND s.parent_session_id IS NULL
               AND NOT EXISTS (SELECT 1 FROM anomalies a WHERE a.session_id = s.id AND a.kind = 'unlinked_subagent')",
        )?;
        let rows = stmt.query_map([root_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?;
        rows.collect::<Result<_, _>>()?
    };
    for (sid, hid, store_id, file_id) in unlinked {
        conn.execute(
            "INSERT INTO anomalies (root_id, store_id, source_file_id, session_id, kind, severity, message, details) VALUES (?1, ?2, ?3, ?4, 'unlinked_subagent', 'warning', ?5, '{}')",
            params![root_id, store_id, file_id, sid, format!("subagent session {hid} has no parent transcript in this root")],
        )?;
    }
    Ok(())
}

/// Re-evaluates every `cursor:` claim of the case. Candidates are the other sessions of `cursor` and
/// `cursor-ide` stores that make the same claim or whose `harness_session_id` is the claimed id: one is
/// `matched`, more are `ambiguous`, none is `unmatched`. Sessions are never merged. One transaction;
/// idempotent, so it runs after every ingest and a root attached later joins the earlier ones.
pub fn link_cursor_claims(conn: &Connection) -> Result<(), CaseError> {
    let tx = conn.unchecked_transaction()?;
    let sessions: Vec<(i64, String)> = {
        let mut stmt = tx.prepare(
            "SELECT s.id, s.harness_session_id FROM sessions s JOIN stores st ON st.id = s.store_id
             WHERE st.harness IN ('cursor', 'cursor-ide')",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let claims: Vec<(i64, i64, String, String)> = {
        let mut stmt = tx.prepare(
            "SELECT id, session_id, scheme, claimed_id FROM identity_claims
             WHERE scheme LIKE 'cursor:%' ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    let cursor: BTreeSet<i64> = sessions.iter().map(|(id, _)| *id).collect();
    let mut by_harness_id: HashMap<&str, Vec<i64>> = HashMap::new();
    for (id, hid) in &sessions {
        by_harness_id.entry(hid).or_default().push(*id);
    }
    let mut by_claim: HashMap<(&str, &str), Vec<i64>> = HashMap::new();
    for (_, sid, scheme, claimed) in &claims {
        if cursor.contains(sid) {
            by_claim.entry((scheme, claimed)).or_default().push(*sid);
        }
    }
    let mut update = tx.prepare(
        "UPDATE identity_claims SET join_status = ?2, matched_session_id = ?3 WHERE id = ?1",
    )?;
    for (id, sid, scheme, claimed) in &claims {
        let candidates: BTreeSet<i64> = by_claim
            .get(&(scheme.as_str(), claimed.as_str()))
            .into_iter()
            .chain(by_harness_id.get(claimed.as_str()))
            .flatten()
            .copied()
            .filter(|c| c != sid)
            .collect();
        let (status, matched) = match candidates.len() {
            0 => ("unmatched", None),
            1 => ("matched", candidates.first().copied()),
            _ => ("ambiguous", None),
        };
        update.execute(params![id, status, matched])?;
    }
    drop(update);
    tx.commit()?;
    Ok(())
}

pub fn finalize_sessions(conn: &Connection, root_id: i64) -> Result<(), CaseError> {
    let sessions: Vec<(i64, i64, Option<i64>)> = {
        let mut stmt = conn.prepare("SELECT s.id, s.bounds_from_adapter, s.primary_source_file_id FROM sessions s JOIN stores st ON st.id = s.store_id WHERE st.root_id = ?1")?;
        let rows = stmt.query_map([root_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    for (sid, fixed, file_id) in sessions {
        if fixed == 0 {
            let first: Option<(String, String)> = conn
                .query_row("SELECT timestamp, ts_origin FROM messages WHERE session_id = ?1 AND timestamp IS NOT NULL ORDER BY timestamp ASC, ordinal ASC LIMIT 1", [sid], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?;
            let last: Option<(String, String)> = conn
                .query_row("SELECT timestamp, ts_origin FROM messages WHERE session_id = ?1 AND timestamp IS NOT NULL ORDER BY timestamp DESC, ordinal DESC LIMIT 1", [sid], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?;
            let (f, fo, l, lo) = match (first, last) {
                (Some((f, fo)), Some((l, lo))) => (Some(f), fo, Some(l), lo),
                _ => {
                    let mtime: Option<String> = match file_id {
                        Some(id) => conn
                            .query_row("SELECT mtime FROM source_files WHERE id = ?1", [id], |r| {
                                r.get(0)
                            })
                            .optional()?
                            .flatten(),
                        None => None,
                    };
                    match mtime {
                        Some(m) => (
                            Some(m.clone()),
                            "file_mtime".to_string(),
                            Some(m),
                            "file_mtime".to_string(),
                        ),
                        None => (None, "absent".to_string(), None, "absent".to_string()),
                    }
                }
            };
            conn.execute("UPDATE sessions SET first_ts = ?2, first_ts_origin = ?3, last_ts = ?4, last_ts_origin = ?5 WHERE id = ?1", params![sid, f, fo, l, lo])?;
        }
        let models: Vec<String> = {
            let mut stmt = conn.prepare("SELECT DISTINCT model FROM messages WHERE session_id = ?1 AND model IS NOT NULL ORDER BY model")?;
            let rows = stmt.query_map([sid], |r| r.get(0))?;
            rows.collect::<Result<_, _>>()?
        };
        conn.execute(
            "UPDATE sessions SET message_count = (SELECT COUNT(*) FROM messages WHERE session_id = ?1 AND role != 'meta'),
                                 tool_call_count = (SELECT COUNT(*) FROM tool_calls WHERE session_id = ?1),
                                 models = ?2 WHERE id = ?1",
            params![sid, serde_json::to_string(&models)?],
        )?;
    }
    Ok(())
}
