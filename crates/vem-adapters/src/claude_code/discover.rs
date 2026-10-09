//! Identification by content signature and store discovery for a `.claude` directory.

use super::EXPECTED_STORES;
use std::io::Read;
use std::path::{Path, PathBuf};
use vem_core::adapter::{Discovery, Identification, StoreCandidate};
use vem_core::model::Harness;

const SIGNATURE_BYTES: usize = 16 * 1024;

fn head(path: &Path) -> Vec<u8> {
    let mut buf = Vec::new();
    if let Ok(f) = std::fs::File::open(path) {
        let _ = f.take(SIGNATURE_BYTES as u64).read_to_end(&mut buf);
    }
    buf
}

fn contains(hay: &[u8], needle: &str) -> bool {
    hay.windows(needle.len()).any(|w| w == needle.as_bytes())
}

pub fn is_transcript_name(name: &str) -> bool {
    name.ends_with(".jsonl") || name.contains(".jsonl.orphaned-") || name.contains(".jsonl.superseded-")
}

/// First transcript under `projects/` whose head carries both `parentUuid` and `sessionId`.
fn first_transcript_signature(root: &Path) -> Option<PathBuf> {
    let projects = root.join("projects");
    if !projects.is_dir() {
        return None;
    }
    for entry in walkdir::WalkDir::new(&projects).max_depth(4).sort_by_file_name().into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        if !is_transcript_name(&name) {
            continue;
        }
        let h = head(entry.path());
        if contains(&h, "\"parentUuid\"") && contains(&h, "\"sessionId\"") {
            return entry.path().strip_prefix(root).ok().map(Path::to_path_buf);
        }
    }
    None
}

fn history_signature(root: &Path) -> bool {
    let h = head(&root.join("history.jsonl"));
    contains(&h, "\"display\"") && contains(&h, "\"sessionId\"")
}

pub fn identify(root: &Path) -> Option<Identification> {
    let mut evidence = Vec::new();
    if let Some(p) = first_transcript_signature(root) {
        evidence.push(format!("projects transcript carrying parentUuid and sessionId: {}", p.display()));
    }
    if history_signature(root) {
        evidence.push("history.jsonl carrying display and sessionId".to_string());
    }
    if root.join("file-history").is_dir() {
        evidence.push("file-history directory".to_string());
    }
    if evidence.is_empty() {
        None
    } else {
        Some(Identification { harness: Harness::ClaudeCode, evidence })
    }
}

/// All regular files under `root/rel`, as paths relative to `root`, sorted. A file path yields itself.
pub fn list_files(root: &Path, rel: &Path) -> Vec<PathBuf> {
    let abs = root.join(rel);
    let mut out = Vec::new();
    if abs.is_file() {
        out.push(rel.to_path_buf());
        return out;
    }
    for entry in walkdir::WalkDir::new(&abs).follow_links(false).into_iter().flatten() {
        if entry.file_type().is_file() {
            if let Ok(r) = entry.path().strip_prefix(root) {
                out.push(r.to_path_buf());
            }
        }
    }
    out.sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));
    out
}

/// The newest harness version seen in the first versioned record of any transcript.
fn detect_generation(root: &Path, files: &[PathBuf]) -> Option<String> {
    let key = |v: &str| v.split('.').map(|p| p.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    let mut best: Option<String> = None;
    for f in files {
        let Some(name) = f.file_name().map(|n| n.to_string_lossy().to_string()) else { continue };
        if !is_transcript_name(&name) {
            continue;
        }
        let h = head(&root.join(f));
        for line in h.split(|&b| b == b'\n') {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(line) {
                if let Some(ver) = v.get("version").and_then(|x| x.as_str()) {
                    if best.as_deref().is_none_or(|b| key(ver) > key(b)) {
                        best = Some(ver.to_string());
                    }
                    break;
                }
            }
        }
    }
    best
}

pub fn discover(root: &Path) -> Discovery {
    let mut d = Discovery::default();
    for (kind, rel) in EXPECTED_STORES {
        let rel_path = PathBuf::from(rel);
        if !root.join(&rel_path).exists() {
            d.absent.push(kind.to_string());
            continue;
        }
        let files = list_files(root, &rel_path);
        let generation = if *kind == super::STORE_PROJECTS { detect_generation(root, &files) } else { None };
        d.stores.push(StoreCandidate { kind: kind.to_string(), generation, rel_path, files });
    }
    d
}
