#![allow(dead_code)]
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use vem_core::hash::sha256_file;

pub fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/claude-code/basic")
        .canonicalize()
        .unwrap()
}

pub const S1: &str = "0f0f0f0f-0000-4000-8000-000000000001";
pub const S0: &str = "0f0f0f0f-0000-4000-8000-000000000000";

/// rel path -> (mtime nanos, sha256) for files, `(mtime, "dir")` for directories and `(0, "link:<target>")`
/// for symbolic links. Used to prove evidence was not touched: nothing created, modified or deleted.
pub fn tree_fingerprint(root: &Path) -> BTreeMap<String, (u128, String)> {
    let mut out = BTreeMap::new();
    for entry in walkdir::WalkDir::new(root)
        .follow_links(false)
        .sort_by_file_name()
    {
        let entry = entry.unwrap();
        let rel = entry
            .path()
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .to_string();
        let ft = entry.file_type();
        if ft.is_symlink() {
            out.insert(
                rel,
                (
                    0,
                    format!(
                        "link:{}",
                        std::fs::read_link(entry.path()).unwrap().display()
                    ),
                ),
            );
            continue;
        }
        let mtime = entry
            .metadata()
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let what = if ft.is_dir() {
            "dir".to_string()
        } else {
            sha256_file(entry.path()).unwrap().0
        };
        out.insert(rel, (mtime, what));
    }
    out
}

pub fn copy_dir(src: &Path, dst: &Path) {
    for entry in walkdir::WalkDir::new(src) {
        let entry = entry.unwrap();
        let rel = entry.path().strip_prefix(src).unwrap();
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target).unwrap();
        } else {
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

pub fn file_count(root: &Path) -> usize {
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter(|e| e.as_ref().unwrap().file_type().is_file())
        .count()
}
