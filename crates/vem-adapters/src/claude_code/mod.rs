//! Claude Code adapter: a collected `.claude` directory (spec §6.1).

pub mod discover;
pub mod sidecars;
pub mod tools;
pub mod transcript;

use std::path::Path;
use vem_core::adapter::{Discovery, FileContext, HarnessAdapter, Identification, ParseError};
use vem_core::model::Harness;
use vem_core::sink::ParseSink;

pub const STORE_PROJECTS: &str = "claude:projects";
pub const STORE_HISTORY: &str = "claude:history";
pub const STORE_FILE_HISTORY: &str = "claude:file-history";

/// (store kind, path relative to root). Order matters: `projects` first so sidecars can find sessions.
pub const EXPECTED_STORES: &[(&str, &str)] = &[
    (STORE_PROJECTS, "projects"),
    (STORE_HISTORY, "history.jsonl"),
    (STORE_FILE_HISTORY, "file-history"),
    ("claude:shell-snapshots", "shell-snapshots"),
    ("claude:todos", "todos"),
    ("claude:plans", "plans"),
    ("claude:paste-cache", "paste-cache"),
    ("claude:uploads", "uploads"),
    ("claude:settings", "settings.json"),
];

pub struct ClaudeCodeAdapter;

impl HarnessAdapter for ClaudeCodeAdapter {
    fn harness(&self) -> Harness {
        Harness::ClaudeCode
    }

    fn identify(&self, root: &Path) -> Option<Identification> {
        discover::identify(root)
    }

    fn discover(&self, root: &Path) -> Discovery {
        discover::discover(root)
    }

    fn parse_file(&self, ctx: &FileContext<'_>, sink: &mut dyn ParseSink) -> Result<(), ParseError> {
        match ctx.store.kind.as_str() {
            STORE_PROJECTS => {
                if transcript::classify_path(ctx.rel_path).is_some() {
                    transcript::parse_transcript(ctx, sink)
                } else {
                    Ok(()) // meta.json and tool-results/*.txt are inventoried and retained, not parsed
                }
            }
            STORE_HISTORY => sidecars::parse_history(ctx, sink),
            _ => Ok(()),
        }
    }
}
