//! `DbSink`: the `ParseSink` that writes canonical drafts into the case database, one file per transaction.

use crate::blobs;
use crate::error::CaseError;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use crate::evidence::encode_rel_path;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use vem_core::model::*;
use vem_core::sink::ParseSink;

#[derive(Debug, Default, Clone, Copy)]
pub struct SinkCounts {
    pub sessions: usize,
    pub messages: usize,
    pub tool_calls: usize,
    pub observations: usize,
    pub claims: usize,
    pub anomalies: usize,
}

pub struct DbSink<'a> {
    conn: &'a Connection,
    case_dir: &'a Path,
    root_id: i64,
    store_id: i64,
    source_file_id: i64,
    next_ordinal: HashMap<i64, i64>,
    pub counts: SinkCounts,
    /// `name/version` of every parser whose provenance this sink stored (for the ingest audit entry).
    pub parsers: BTreeSet<String>,
    /// First error hit inside a sink method; surfaced by `ingest` after `parse_file` returns.
    pub error: Option<CaseError>,
}

impl<'a> DbSink<'a> {
    pub fn new(conn: &'a Connection, case_dir: &'a Path, root_id: i64, store_id: i64, source_file_id: i64) -> Self {
        Self { conn, case_dir, root_id, store_id, source_file_id, next_ordinal: HashMap::new(), counts: SinkCounts::default(), parsers: BTreeSet::new(), error: None }
    }

    fn fail<T: Default>(&mut self, r: Result<T, CaseError>) -> T {
        match r {
            Ok(v) => v,
            Err(e) => {
                if self.error.is_none() {
                    self.error = Some(e);
                }
                T::default()
            }
        }
    }

    fn insert_provenance(&mut self, p: &Provenance) -> Result<i64, CaseError> {
        self.parsers.insert(format!("{}/{}", p.parser_name, p.parser_version));
        self.conn.execute(
            "INSERT INTO provenance (source_file_id, byte_offset, byte_length, record_index, content_sha256, parser_name, parser_version, origin) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![p.source_file.0, p.byte_offset as i64, p.byte_length as i64, p.record_index as i64, p.content_sha256, p.parser_name, p.parser_version, p.origin.as_str()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    fn block_id(&self, r: &BlockRef) -> Result<Option<i64>, CaseError> {
        Ok(self
            .conn
            .query_row("SELECT id FROM blocks WHERE message_id = ?1 AND ordinal = ?2", params![r.message.0, r.ordinal as i64], |row| row.get(0))
            .optional()?)
    }

    fn ordinal_for(&mut self, session: SessionHandle) -> Result<i64, CaseError> {
        if let Some(n) = self.next_ordinal.get_mut(&session.0) {
            let v = *n;
            *n += 1;
            return Ok(v);
        }
        let start: i64 = self.conn.query_row("SELECT COALESCE(MAX(ordinal), -1) + 1 FROM messages WHERE session_id = ?1", [session.0], |r| r.get(0))?;
        self.next_ordinal.insert(session.0, start + 1);
        Ok(start)
    }

    fn try_session(&mut self, d: &SessionDraft) -> Result<i64, CaseError> {
        let (first, first_o, last, last_o, fixed) = match (&d.first_ts, &d.last_ts) {
            (Some(f), Some(l)) => (f.value.clone(), f.origin.as_str(), l.value.clone(), l.origin.as_str(), 1),
            _ => (None, "absent", None, "absent", 0),
        };
        self.conn.execute(
            "INSERT INTO sessions (store_id, harness_session_id, kind, parent_harness_session_id, title, project_path, git_branch, harness_version, first_ts, first_ts_origin, last_ts, last_ts_origin, bounds_from_adapter, primary_source_file_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![self.store_id, d.harness_session_id, d.kind.as_str(), d.parent_harness_session_id, d.title, d.project_path, d.git_branch, d.harness_version, first, first_o, last, last_o, fixed, self.source_file_id],
        )?;
        self.counts.sessions += 1;
        Ok(self.conn.last_insert_rowid())
    }

    fn try_update(&mut self, session: SessionHandle, u: &SessionUpdate) -> Result<(), CaseError> {
        self.conn.execute(
            "UPDATE sessions SET title = COALESCE(?2, title), project_path = COALESCE(project_path, ?3), git_branch = COALESCE(git_branch, ?4), harness_version = COALESCE(harness_version, ?5) WHERE id = ?1",
            params![session.0, u.title, u.project_path, u.git_branch, u.harness_version],
        )?;
        if let Some(model) = &u.model {
            let raw: String = self.conn.query_row("SELECT models FROM sessions WHERE id = ?1", [session.0], |r| r.get(0))?;
            let mut models: Vec<String> = serde_json::from_str(&raw).unwrap_or_default();
            if !models.contains(model) {
                models.push(model.clone());
                self.conn.execute("UPDATE sessions SET models = ?2 WHERE id = ?1", params![session.0, serde_json::to_string(&models)?])?;
            }
        }
        Ok(())
    }

    fn try_message(&mut self, session: SessionHandle, m: &MessageDraft) -> Result<i64, CaseError> {
        let prov_id = self.insert_provenance(&m.provenance)?;
        let ordinal = self.ordinal_for(session)?;
        self.conn.execute(
            "INSERT INTO messages (session_id, ordinal, role, harness_record_type, harness_uuid, parent_uuid, timestamp, ts_origin, model, provenance_id, attributes) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![session.0, ordinal, m.role.as_str(), m.harness_record_type, m.harness_uuid, m.parent_uuid, m.timestamp.value, m.timestamp.origin.as_str(), m.model, prov_id, Value::Object(m.attributes.clone()).to_string()],
        )?;
        let message_id = self.conn.last_insert_rowid();
        for (i, b) in m.blocks.iter().enumerate() {
            self.conn.execute(
                "INSERT INTO blocks (message_id, ordinal, kind, text, payload, tool_use_id, provenance_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![message_id, i as i64, b.kind.as_str(), b.text, b.payload.to_string(), b.tool_use_id, prov_id],
            )?;
        }
        self.counts.messages += 1;
        Ok(message_id)
    }

    fn try_tool_call(&mut self, session: SessionHandle, t: &ToolCallDraft) -> Result<i64, CaseError> {
        let use_id = self.block_id(&t.tool_use)?.unwrap_or(0);
        let result_id = match &t.tool_result {
            Some(r) => self.block_id(r)?,
            None => None,
        };
        self.conn.execute(
            "INSERT INTO tool_calls (session_id, tool_use_block_id, tool_result_block_id, name, category, input, result_text, result_payload, is_error, started_ts, ended_ts, ts_origin) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![session.0, use_id, result_id, t.name, t.category.as_str(), t.input.to_string(), t.result_text, t.result_payload.as_ref().map(|v| v.to_string()), t.is_error as i64, t.started.value, t.ended.value, t.started.origin.as_str()],
        )?;
        let id = self.conn.last_insert_rowid();
        self.conn.execute("UPDATE blocks SET tool_call_id = ?1 WHERE id = ?2 OR id = ?3", params![id, use_id, result_id])?;
        self.counts.tool_calls += 1;
        Ok(id)
    }

    fn try_observation(&mut self, session: SessionHandle, o: &ObservationDraft) -> Result<(), CaseError> {
        let (tc, blk, prov) = match &o.derived_from {
            Derivation::ToolCall(h) => (Some(h.0), None, None),
            Derivation::Block(r) => (None, self.block_id(r)?, None),
            Derivation::Record(p) => (None, None, Some(self.insert_provenance(p)?)),
        };
        self.conn.execute(
            "INSERT INTO observations (session_id, kind, derived_from_tool_call_id, derived_from_block_id, derived_from_provenance_id, path, command, before_blob, after_blob, timestamp, ts_origin, confidence, details) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![session.0, o.kind.as_str(), tc, blk, prov, o.path, o.command, o.before_blob, o.after_blob, o.timestamp.value, o.timestamp.origin.as_str(), o.confidence.as_str(), o.details.to_string()],
        )?;
        self.counts.observations += 1;
        Ok(())
    }

    fn try_claim(&mut self, session: SessionHandle, c: &IdentityClaimDraft) -> Result<(), CaseError> {
        self.conn.execute(
            "INSERT INTO identity_claims (session_id, scheme, claimed_id, source_file_id, join_status) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![session.0, c.scheme, c.claimed_id, c.source_file.0, c.join_status.as_str()],
        )?;
        self.counts.claims += 1;
        Ok(())
    }

    fn try_anomaly(&mut self, a: &AnomalyDraft) -> Result<(), CaseError> {
        let prov_id = match &a.provenance {
            Some(p) => Some(self.insert_provenance(p)?),
            None => None,
        };
        self.conn.execute(
            "INSERT INTO anomalies (root_id, store_id, source_file_id, session_id, kind, severity, byte_offset, provenance_id, message, details) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![self.root_id, self.store_id, a.source_file.map(|h| h.0).or(Some(self.source_file_id)), a.session.map(|h| h.0), a.kind.as_str(), a.severity.as_str(), a.byte_offset.map(|o| o as i64), prov_id, a.message, a.details.to_string()],
        )?;
        self.counts.anomalies += 1;
        Ok(())
    }
}

impl<'a> ParseSink for DbSink<'a> {
    fn session(&mut self, draft: SessionDraft) -> SessionHandle {
        let r = self.try_session(&draft);
        let id = self.fail(r);
        SessionHandle(id)
    }
    fn update_session(&mut self, session: SessionHandle, update: SessionUpdate) {
        let r = self.try_update(session, &update);
        self.fail(r);
    }
    fn find_session(&self, harness_session_id: &str) -> Option<SessionHandle> {
        self.conn
            .query_row(
                "SELECT s.id FROM sessions s JOIN stores st ON st.id = s.store_id WHERE st.root_id = ?1 AND s.harness_session_id = ?2 ORDER BY s.id LIMIT 1",
                params![self.root_id, harness_session_id],
                |r| r.get::<_, i64>(0),
            )
            .optional()
            .ok()
            .flatten()
            .map(SessionHandle)
    }
    fn find_source_file(&self, rel_path: &Path) -> Option<SourceFileHandle> {
        let (rel, encoded) = encode_rel_path(rel_path);
        self.conn
            .query_row(
                "SELECT id FROM source_files WHERE root_id = ?1 AND rel_path = ?2 AND rel_path_encoded = ?3 AND kind = 'file' ORDER BY version DESC LIMIT 1",
                params![self.root_id, rel, encoded],
                |r| r.get::<_, i64>(0),
            )
            .optional()
            .ok()
            .flatten()
            .map(SourceFileHandle)
    }
    fn message(&mut self, session: SessionHandle, draft: MessageDraft) -> MessageHandle {
        let r = self.try_message(session, &draft);
        let id = self.fail(r);
        MessageHandle(id)
    }
    fn tool_call(&mut self, session: SessionHandle, draft: ToolCallDraft) -> ToolCallHandle {
        let r = self.try_tool_call(session, &draft);
        let id = self.fail(r);
        ToolCallHandle(id)
    }
    fn observation(&mut self, session: SessionHandle, draft: ObservationDraft) {
        let r = self.try_observation(session, &draft);
        self.fail(r);
    }
    fn identity_claim(&mut self, session: SessionHandle, draft: IdentityClaimDraft) {
        let r = self.try_claim(session, &draft);
        self.fail(r);
    }
    fn anomaly(&mut self, draft: AnomalyDraft) {
        let r = self.try_anomaly(&draft);
        self.fail(r);
    }
    fn blob(&mut self, bytes: &[u8]) -> String {
        let r = blobs::put_bytes(self.conn, self.case_dir, bytes);
        self.fail(r)
    }
}
