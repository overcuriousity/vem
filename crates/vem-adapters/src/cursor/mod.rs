//! Cursor adapters (spec §6.3): a collected `.cursor` directory (`cursor`) and Cursor's IDE application
//! directory (`cursor-ide`).

pub mod discover;
pub mod history;
pub mod kv;
pub mod sqlite;
pub mod tools;
pub mod transcript;
pub mod workspace;

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::Path;
use vem_core::adapter::{
    Discovery, FileContext, HarnessAdapter, Identification, ParseError, ParseOutcome,
};
use vem_core::model::{
    AnomalyDraft, AnomalyKind, Harness, Provenance, SessionHandle, Severity, SourceFileHandle,
};
use vem_core::sink::ParseSink;

// Store kinds of harness `cursor` (a `.cursor` directory).
pub const STORE_AGENT_TRANSCRIPTS: &str = "cursor:agent-transcripts";
pub const STORE_CHAT_STORE: &str = "cursor:chat-store";
pub const STORE_ACP_SESSIONS: &str = "cursor:acp-sessions";
pub const STORE_AI_TRACKING: &str = "cursor:ai-tracking";
pub const STORE_PROMPT_HISTORY: &str = "cursor:prompt-history";
// Store kinds of harness `cursor-ide` (Cursor's application directory).
pub const STORE_STATE_VSCDB: &str = "cursor:state-vscdb";
pub const STORE_WORKSPACE_STORAGE: &str = "cursor:workspace-storage";
pub const STORE_LOCAL_HISTORY: &str = "cursor:local-history";

pub use sqlite::RECORD_CAP;

/// A record-level anomaly about the record `prov`, or a file-level one when `prov` is `None`.
pub(crate) fn record_anomaly(
    file: SourceFileHandle,
    session: Option<SessionHandle>,
    kind: AnomalyKind,
    severity: Severity,
    prov: Option<&Provenance>,
    message: String,
    details: Value,
) -> AnomalyDraft {
    AnomalyDraft {
        kind,
        severity,
        source_file: Some(file),
        session,
        byte_offset: prov.filter(|p| p.locator.is_none()).map(|p| p.byte_offset),
        message,
        details,
        provenance: prov.cloned(),
    }
}

/// Unknown JSON fields of one file, counted per (record kind, field) and reported once each.
#[derive(Default)]
pub(crate) struct FieldCensus {
    seen: BTreeMap<(String, String), (u64, String, Provenance)>,
}

impl FieldCensus {
    pub fn note(&mut self, record_kind: &str, field: &str, locator: &str, prov: &Provenance) {
        self.seen
            .entry((record_kind.to_string(), field.to_string()))
            .or_insert_with(|| (0, locator.to_string(), prov.clone()))
            .0 += 1;
    }

    /// One `unknown_field` (Info) per (record kind, field), citing the first record that had it.
    pub fn finish(
        &mut self,
        file: SourceFileHandle,
        session: Option<SessionHandle>,
        sink: &mut dyn ParseSink,
    ) {
        for ((record_kind, field), (count, first, prov)) in std::mem::take(&mut self.seen) {
            sink.anomaly(record_anomaly(
                file,
                session,
                AnomalyKind::UnknownField,
                Severity::Info,
                Some(&prov),
                format!("{count} {record_kind} record(s) carry the unknown field {field:?}; kept in attributes"),
                json!({ "record_kind": record_kind, "field": field, "count": count, "first_locator": first }),
            ));
        }
    }
}

pub struct CursorAdapter;

impl HarnessAdapter for CursorAdapter {
    fn harness(&self) -> Harness {
        Harness::Cursor
    }

    fn identify(&self, root: &Path) -> Option<Identification> {
        discover::identify_cursor(root)
    }

    fn discover(&self, root: &Path) -> Discovery {
        discover::discover_cursor(root)
    }

    fn parse_file(
        &self,
        ctx: &FileContext<'_>,
        sink: &mut dyn ParseSink,
    ) -> Result<ParseOutcome, ParseError> {
        match ctx.store.kind.as_str() {
            // Legacy `.txt` transcripts and `agent-tools/*.txt` sidecars are inventoried, not parsed.
            STORE_AGENT_TRANSCRIPTS => transcript::parse_transcript(ctx, sink),
            _ => Ok(ParseOutcome::NotParsed),
        }
    }
}

pub struct CursorIdeAdapter;

impl HarnessAdapter for CursorIdeAdapter {
    fn harness(&self) -> Harness {
        Harness::CursorIde
    }

    fn identify(&self, root: &Path) -> Option<Identification> {
        discover::identify_ide(root)
    }

    fn discover(&self, root: &Path) -> Discovery {
        discover::discover_ide(root)
    }

    fn parse_file(
        &self,
        ctx: &FileContext<'_>,
        sink: &mut dyn ParseSink,
    ) -> Result<ParseOutcome, ParseError> {
        let name = ctx.rel_path.file_name().and_then(|n| n.to_str());
        match ctx.store.kind.as_str() {
            // `-wal` and `-shm` are read with the db (WAL-applied view), not on their own.
            STORE_STATE_VSCDB if name == Some("state.vscdb") => kv::parse_state_vscdb(ctx, sink),
            STORE_WORKSPACE_STORAGE => workspace::parse_workspace_file(ctx, sink),
            STORE_LOCAL_HISTORY => history::parse_entries(ctx, sink),
            _ => Ok(ParseOutcome::NotParsed),
        }
    }
}
