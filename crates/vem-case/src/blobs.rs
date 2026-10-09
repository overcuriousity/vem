//! Content-addressed blob store: `<case>/blobs/<sha256>` plus a `blobs` index table.
//! Free functions take a `&Connection` so they work inside a transaction.

use crate::error::CaseError;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use vem_core::hash::{copy_and_hash, sha256_hex};

pub fn path(case_dir: &Path, sha256: &str) -> PathBuf {
    case_dir.join("blobs").join(sha256)
}

fn record(conn: &Connection, sha256: &str, size: u64) -> Result<(), CaseError> {
    conn.execute("INSERT OR IGNORE INTO blobs (sha256, size) VALUES (?1, ?2)", params![sha256, size as i64])?;
    Ok(())
}

fn write_atomically(target: &Path, write: impl FnOnce(&Path) -> std::io::Result<()>) -> std::io::Result<()> {
    if target.exists() {
        return Ok(());
    }
    let tmp = target.with_extension(format!("tmp.{}", std::process::id()));
    write(&tmp)?;
    match std::fs::rename(&tmp, target) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            if target.exists() { Ok(()) } else { Err(e) }
        }
    }
}

pub fn put_bytes(conn: &Connection, case_dir: &Path, bytes: &[u8]) -> Result<String, CaseError> {
    let sha = sha256_hex(bytes);
    write_atomically(&path(case_dir, &sha), |tmp| std::fs::write(tmp, bytes))?;
    record(conn, &sha, bytes.len() as u64)?;
    Ok(sha)
}

/// Copies `src` into the store, hashing the bytes as they are copied, so the blob's name is the hash of
/// exactly the bytes stored (no window between hashing and copying). Returns `(sha256, size)`.
pub fn put_file(conn: &Connection, case_dir: &Path, src: &Path) -> Result<(String, u64), CaseError> {
    let blobs = case_dir.join("blobs");
    let tmp = blobs.join(format!("incoming.tmp.{}", std::process::id()));
    let copied = copy_and_hash(src, &tmp);
    let (sha, size) = match copied {
        Ok(x) => x,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e.into());
        }
    };
    let target = path(case_dir, &sha);
    if target.exists() {
        std::fs::remove_file(&tmp)?;
    } else {
        std::fs::rename(&tmp, &target)?;
    }
    record(conn, &sha, size)?;
    Ok((sha, size))
}
