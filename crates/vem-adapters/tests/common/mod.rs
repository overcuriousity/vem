#![allow(dead_code)]
use std::path::{Path, PathBuf};
use vem_adapters::claude_code::ClaudeCodeAdapter;
use vem_core::adapter::{FileContext, HarnessAdapter};
use vem_core::model::SourceFileHandle;
use vem_core::testing::VecSink;

pub fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/claude-code/basic")
        .canonicalize()
        .unwrap()
}

pub const S1: &str = "0f0f0f0f-0000-4000-8000-000000000001";
pub const S0: &str = "0f0f0f0f-0000-4000-8000-000000000000";
pub const S1_FILE: &str = "projects/-home-alice-proj/0f0f0f0f-0000-4000-8000-000000000001.jsonl";
pub const S0_FILE: &str = "projects/-home-alice-proj/0f0f0f0f-0000-4000-8000-000000000000.jsonl";
pub const SUB_FILE: &str = "projects/-home-alice-proj/0f0f0f0f-0000-4000-8000-000000000001/subagents/agent-0123456789abcdef.jsonl";
pub const SUB_META_FILE: &str = "projects/-home-alice-proj/0f0f0f0f-0000-4000-8000-000000000001/subagents/agent-0123456789abcdef.meta.json";
pub const ORPHAN_FILE: &str =
    "projects/-home-alice-proj/22222222-0000-4000-8000-000000000002.jsonl.orphaned-1759221000000";

/// Parses one file of `root` into `sink`, giving it source-file handle 1.
pub fn parse_file_into(root: &Path, rel: &str, sink: &mut VecSink) {
    parse_file_with_handle(root, rel, SourceFileHandle(1), sink);
}

/// Parses one file of `root` into `sink` as source file `handle`.
pub fn parse_file_with_handle(
    root: &Path,
    rel: &str,
    handle: SourceFileHandle,
    sink: &mut VecSink,
) {
    let adapter = ClaudeCodeAdapter;
    let discovery = adapter.discover(root);
    let rel_path = PathBuf::from(rel);
    let store = discovery
        .stores
        .iter()
        .find(|s| s.files.contains(&rel_path))
        .unwrap_or_else(|| panic!("no store claims {rel}"));
    let abs_path = root.join(&rel_path);
    let mtime = std::fs::metadata(&abs_path).and_then(|m| m.modified()).ok();
    let ctx = FileContext {
        root,
        store,
        rel_path: &rel_path,
        abs_path,
        handle,
        mtime,
    };
    adapter.parse_file(&ctx, sink).expect("parse ok");
}

pub fn parse_fixture(rel: &str) -> VecSink {
    let mut sink = VecSink::default();
    parse_file_into(&fixture_root(), rel, &mut sink);
    sink
}

/// Writes `lines` as a transcript named `<session>.jsonl` under a temporary `.claude`-shaped root.
pub fn temp_root_with_transcript(
    session: &str,
    lines: &[&str],
    trailing_newline: bool,
) -> (tempfile::TempDir, String) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("projects").join("-tmp-proj");
    std::fs::create_dir_all(&dir).unwrap();
    let mut body = lines.join("\n");
    if trailing_newline {
        body.push('\n');
    }
    let rel = format!("projects/-tmp-proj/{session}.jsonl");
    std::fs::write(tmp.path().join(&rel), body).unwrap();
    (tmp, rel)
}
