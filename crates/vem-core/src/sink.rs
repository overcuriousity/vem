//! The sink an adapter emits canonical drafts into. The case database implements it with SQL
//! inserts; tests implement it with vectors.

use crate::model::*;
use std::path::Path;

pub trait ParseSink {
    fn session(&mut self, draft: SessionDraft) -> SessionHandle;
    fn update_session(&mut self, session: SessionHandle, update: SessionUpdate);
    /// Looks a session up by its harness id across everything parsed so far in this root.
    fn find_session(&self, harness_session_id: &str) -> Option<SessionHandle>;
    /// Looks a file up in this root's manifest by its path relative to the root (latest version).
    /// `None` when the manifest does not list it (or, in tests, when there is no manifest).
    fn find_source_file(&self, rel_path: &Path) -> Option<SourceFileHandle>;
    /// Looks up a main transcript `projects/<dir>/<harness_session_id>.jsonl` anywhere in this root's
    /// manifest (latest version) that has not been parsed yet (`unparsed` or `failed`).
    /// `None` when there is no such file (or, in tests, when there is no manifest).
    fn find_unparsed_transcript(&self, harness_session_id: &str) -> Option<SourceFileHandle>;
    fn message(&mut self, session: SessionHandle, draft: MessageDraft) -> MessageHandle;
    fn tool_call(&mut self, session: SessionHandle, draft: ToolCallDraft) -> ToolCallHandle;
    fn observation(&mut self, session: SessionHandle, draft: ObservationDraft);
    fn identity_claim(&mut self, session: SessionHandle, draft: IdentityClaimDraft);
    fn anomaly(&mut self, draft: AnomalyDraft);
    /// Stores content-addressed bytes and returns their SHA-256 hex.
    fn blob(&mut self, bytes: &[u8]) -> String;
}
