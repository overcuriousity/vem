#![allow(dead_code)]
use std::path::{Path, PathBuf};
use vem_adapters::cursor::{CursorAdapter, CursorIdeAdapter};
use vem_core::adapter::{FileContext, HarnessAdapter, ParseOutcome};
use vem_core::model::SourceFileHandle;
use vem_core::testing::VecSink;

pub const T1: &str = "1c1c1c1c-0000-4000-8000-000000000001";
pub const T1_FILE: &str = "projects/home-alice-proj/agent-transcripts/1c1c1c1c-0000-4000-8000-000000000001/1c1c1c1c-0000-4000-8000-000000000001.jsonl";
pub const SUB_FILE: &str = "projects/home-alice-proj/agent-transcripts/1c1c1c1c-0000-4000-8000-000000000001/subagents/1c1c1c1c-0000-4000-8000-0000000000a1.jsonl";

pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/cursor")
        .join(name)
        .canonicalize()
        .unwrap()
}

/// Copies `src` into `dst` recursively (regular files and directories only).
pub fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        let to = dst.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &to);
        } else {
            std::fs::copy(e.path(), &to).unwrap();
        }
    }
}

/// Parses `rel` of `root` with the adapter whose discovery lists it, as source file `handle`.
pub fn parse_with(
    adapter: &dyn HarnessAdapter,
    root: &Path,
    rel: &str,
    handle: i64,
    sink: &mut VecSink,
) -> ParseOutcome {
    if sink.live_root.is_none() {
        sink.live_root = Some(root.to_path_buf());
    }
    with_ctx(adapter, root, rel, handle, |ctx| {
        adapter.parse_file(ctx, sink).expect("parse ok")
    })
}

/// Calls `f` with the `FileContext` of `rel` in the store of `adapter`'s discovery that lists it.
pub fn with_ctx<T>(
    adapter: &dyn HarnessAdapter,
    root: &Path,
    rel: &str,
    handle: i64,
    f: impl FnOnce(&FileContext<'_>) -> T,
) -> T {
    let discovery = adapter.discover(root);
    let rel_path = PathBuf::from(rel);
    let store = discovery
        .stores
        .iter()
        .find(|s| s.files.contains(&rel_path))
        .unwrap_or_else(|| panic!("no store claims {rel}"));
    let abs_path = root.join(&rel_path);
    let mtime = std::fs::metadata(&abs_path).and_then(|m| m.modified()).ok();
    let sha256 = vem_core::hash::sha256_file(&abs_path)
        .map(|(s, _)| s)
        .unwrap_or_default();
    let ctx = FileContext {
        root,
        store,
        rel_path: &rel_path,
        abs_path,
        handle: SourceFileHandle(handle),
        mtime,
        sha256: &sha256,
    };
    f(&ctx)
}

pub fn parse_cursor(root: &Path, rel: &str, sink: &mut VecSink) -> ParseOutcome {
    parse_with(&CursorAdapter, root, rel, 1, sink)
}

pub fn parse_ide(root: &Path, rel: &str, handle: i64, sink: &mut VecSink) -> ParseOutcome {
    parse_with(&CursorIdeAdapter, root, rel, handle, sink)
}

/// A `.cursor`-shaped temp root with `agent-transcripts/<id>/<id>.jsonl` holding `body` verbatim.
pub fn temp_transcript(id: &str, body: &str) -> (tempfile::TempDir, String) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp
        .path()
        .join("projects/home-bob-x/agent-transcripts")
        .join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{id}.jsonl")), body).unwrap();
    let rel = format!("projects/home-bob-x/agent-transcripts/{id}/{id}.jsonl");
    (tmp, rel)
}
