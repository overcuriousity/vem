//! Line diff of two blobs for the file-operation views. Binary content is detected and never diffed.

use serde::Serialize;
use similar::{ChangeTag, TextDiff};

pub const CONTEXT: usize = 3;
pub const MAX_DIFF_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tag {
    Equal,
    Insert,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiffLine {
    pub tag: Tag,
    pub old_no: Option<usize>,
    pub new_no: Option<usize>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hunk {
    pub old_start: usize,
    pub old_lines: usize,
    pub new_start: usize,
    pub new_lines: usize,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiffResult {
    pub binary: bool,
    pub hunks: Vec<Hunk>,
}

/// Binary: a NUL byte in the first 8 KiB, or invalid UTF-8 anywhere except a sequence cut off at the end.
pub fn is_binary(bytes: &[u8]) -> bool {
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return true;
    }
    match std::str::from_utf8(bytes) {
        Ok(_) => false,
        Err(e) => e.error_len().is_some(),
    }
}

fn text_of(bytes: &[u8]) -> &str {
    match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => std::str::from_utf8(&bytes[..e.valid_up_to()]).unwrap_or_default(),
    }
}

/// Hunks with `CONTEXT` lines of context; line numbers are 1-based; `text` has its line ending removed.
pub fn diff_text(old: &str, new: &str) -> Vec<Hunk> {
    let diff = TextDiff::from_lines(old, new);
    let mut hunks = Vec::new();
    for group in diff.grouped_ops(CONTEXT) {
        let (Some(first), Some(last)) = (group.first(), group.last()) else {
            continue;
        };
        let mut lines = Vec::new();
        for op in &group {
            for change in diff.iter_changes(op) {
                let tag = match change.tag() {
                    ChangeTag::Equal => Tag::Equal,
                    ChangeTag::Insert => Tag::Insert,
                    ChangeTag::Delete => Tag::Delete,
                };
                let v = change.value();
                let text = v
                    .strip_suffix('\n')
                    .map(|t| t.strip_suffix('\r').unwrap_or(t))
                    .unwrap_or(v)
                    .to_string();
                lines.push(DiffLine {
                    tag,
                    old_no: change.old_index().map(|i| i + 1),
                    new_no: change.new_index().map(|i| i + 1),
                    text,
                });
            }
        }
        hunks.push(Hunk {
            old_start: first.old_range().start + 1,
            old_lines: last.old_range().end - first.old_range().start,
            new_start: first.new_range().start + 1,
            new_lines: last.new_range().end - first.new_range().start,
            lines,
        });
    }
    hunks
}

pub fn diff_bytes(old: &[u8], new: &[u8]) -> DiffResult {
    if is_binary(old) || is_binary(new) {
        return DiffResult {
            binary: true,
            hunks: Vec::new(),
        };
    }
    DiffResult {
        binary: false,
        hunks: diff_text(text_of(old), text_of(new)),
    }
}
