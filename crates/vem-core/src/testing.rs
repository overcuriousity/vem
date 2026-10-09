//! `VecSink`: an in-memory `ParseSink` for adapter tests.

use crate::hash::sha256_hex;
use crate::model::*;
use crate::sink::ParseSink;
use std::collections::HashMap;

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
}

impl VecSink {
    pub fn messages_with_role(&self, role: Role) -> Vec<&MessageDraft> {
        self.messages.iter().filter(|(_, _, m)| m.role == role).map(|(_, _, m)| m).collect()
    }
    pub fn anomalies_of(&self, kind: AnomalyKind) -> Vec<&AnomalyDraft> {
        self.anomalies.iter().filter(|a| a.kind == kind).collect()
    }
    pub fn observations_of(&self, kind: ObservationKind) -> Vec<&ObservationDraft> {
        self.observations.iter().filter(|(_, o)| o.kind == kind).map(|(_, o)| o).collect()
    }
    pub fn session_draft(&self, h: SessionHandle) -> &SessionDraft {
        &self.sessions.iter().find(|(x, _)| *x == h).expect("session handle").1
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
        self.sessions.iter().find(|(_, s)| s.harness_session_id == harness_session_id).map(|(h, _)| *h)
    }
    fn find_source_file(&self, _rel_path: &std::path::Path) -> Option<SourceFileHandle> {
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
        self.blobs.entry(sha.clone()).or_insert_with(|| bytes.to_vec());
        sha
    }
}
