//! Cursor adapters (spec §6.3): a collected `.cursor` directory (`cursor`) and Cursor's IDE application
//! directory (`cursor-ide`).

pub mod discover;
pub mod sqlite;

use std::path::Path;
use vem_core::adapter::{
    Discovery, FileContext, HarnessAdapter, Identification, ParseError, ParseOutcome,
};
use vem_core::model::Harness;
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
        _ctx: &FileContext<'_>,
        _sink: &mut dyn ParseSink,
    ) -> Result<ParseOutcome, ParseError> {
        Ok(ParseOutcome::NotParsed)
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
        _ctx: &FileContext<'_>,
        _sink: &mut dyn ParseSink,
    ) -> Result<ParseOutcome, ParseError> {
        Ok(ParseOutcome::NotParsed)
    }
}
