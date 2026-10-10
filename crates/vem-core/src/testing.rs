//! `VecSink`: an in-memory `ParseSink` for adapter tests.

use crate::hash::sha256_hex;
use crate::model::*;
use crate::sink::{ParseSink, RootFileError};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct VecSink {
    pub sessions: Vec<(SessionHandle, SessionDraft)>,
    pub updates: Vec<(SessionHandle, SessionUpdate)>,
    pub messages: Vec<(SessionHandle, MessageHandle, MessageDraft)>,
    pub tool_calls: Vec<(SessionHandle, ToolCallHandle, ToolCallDraft)>,
    pub observations: Vec<(SessionHandle, ObservationDraft)>,
    pub claims: Vec<(SessionHandle, IdentityClaimDraft)>,
    pub anomalies: Vec<AnomalyDraft>,
    pub blobs: HashMap<String, Vec<u8>>,
    /// Files served by `read_root_file`, by path relative to the root.
    pub root_files: HashMap<PathBuf, Vec<u8>>,
    /// When set and a path is not in `root_files`, `read_root_file` reads it under this directory without
    /// following symbolic links (no manifest, so no hash check).
    pub live_root: Option<PathBuf>,
}

impl VecSink {
    pub fn messages_with_role(&self, role: Role) -> Vec<&MessageDraft> {
        self.messages
            .iter()
            .filter(|(_, _, m)| m.role == role)
            .map(|(_, _, m)| m)
            .collect()
    }
    pub fn anomalies_of(&self, kind: AnomalyKind) -> Vec<&AnomalyDraft> {
        self.anomalies.iter().filter(|a| a.kind == kind).collect()
    }
    pub fn observations_of(&self, kind: ObservationKind) -> Vec<&ObservationDraft> {
        self.observations
            .iter()
            .filter(|(_, o)| o.kind == kind)
            .map(|(_, o)| o)
            .collect()
    }
    pub fn session_draft(&self, h: SessionHandle) -> &SessionDraft {
        &self
            .sessions
            .iter()
            .find(|(x, _)| *x == h)
            .expect("session handle")
            .1
    }
}

impl ParseSink for VecSink {
    fn session(&mut self, draft: SessionDraft) -> SessionHandle {
        let h = SessionHandle(self.sessions.len() as i64 + 1);
        self.sessions.push((h, draft));
        h
    }
    fn update_session(&mut self, session: SessionHandle, update: SessionUpdate) {
        self.updates.push((session, update));
    }
    fn find_session(&self, harness_session_id: &str) -> Option<SessionHandle> {
        self.sessions
            .iter()
            .find(|(_, s)| s.harness_session_id == harness_session_id)
            .map(|(h, _)| *h)
    }
    fn find_source_file(&self, _rel_path: &std::path::Path) -> Option<SourceFileHandle> {
        None
    }
    fn find_unparsed_transcript(&self, _harness_session_id: &str) -> Option<SourceFileHandle> {
        None
    }
    fn message(&mut self, session: SessionHandle, draft: MessageDraft) -> MessageHandle {
        let h = MessageHandle(self.messages.len() as i64 + 1);
        self.messages.push((session, h, draft));
        h
    }
    fn tool_call(&mut self, session: SessionHandle, draft: ToolCallDraft) -> ToolCallHandle {
        let h = ToolCallHandle(self.tool_calls.len() as i64 + 1);
        self.tool_calls.push((session, h, draft));
        h
    }
    fn observation(&mut self, session: SessionHandle, draft: ObservationDraft) {
        self.observations.push((session, draft));
    }
    fn identity_claim(&mut self, session: SessionHandle, draft: IdentityClaimDraft) {
        self.claims.push((session, draft));
    }
    fn anomaly(&mut self, draft: AnomalyDraft) {
        self.anomalies.push(draft);
    }
    fn blob(&mut self, bytes: &[u8]) -> String {
        let sha = sha256_hex(bytes);
        self.blobs
            .entry(sha.clone())
            .or_insert_with(|| bytes.to_vec());
        sha
    }
    fn read_root_file(
        &self,
        rel_path: &Path,
        max_len: u64,
    ) -> Result<Option<Vec<u8>>, RootFileError> {
        if let Some(b) = self.root_files.get(rel_path) {
            return if b.len() as u64 > max_len {
                Err(RootFileError::TooLarge)
            } else {
                Ok(Some(b.clone()))
            };
        }
        let Some(root) = &self.live_root else {
            return Ok(None);
        };
        let mut p = root.clone();
        for c in rel_path.components() {
            p.push(c);
            match std::fs::symlink_metadata(&p) {
                Err(_) => return Ok(None),
                Ok(m) if m.file_type().is_symlink() => return Err(RootFileError::Symlink),
                Ok(_) => {}
            }
        }
        if !std::fs::symlink_metadata(&p)
            .map(|m| m.is_file())
            .unwrap_or(false)
        {
            return Ok(None);
        }
        let bytes = std::fs::read(&p).map_err(|e| RootFileError::Io(e.to_string()))?;
        if bytes.len() as u64 > max_len {
            Err(RootFileError::TooLarge)
        } else {
            Ok(Some(bytes))
        }
    }
}
