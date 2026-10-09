//! Content-addressed blob store: `<case>/blobs/<sha256>` plus a `blobs` index table.
//! Free functions take a `&Connection` so they work inside a transaction.

use crate::error::CaseError;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use vem_core::hash::sha256_hex;

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

/// Copies `src` (already hashed as `sha256`, `size` bytes) into the store.
pub fn put_file(conn: &Connection, case_dir: &Path, src: &Path, sha256: &str, size: u64) -> Result<(), CaseError> {
    write_atomically(&path(case_dir, sha256), |tmp| std::fs::copy(src, tmp).map(|_| ()))?;
    record(conn, sha256, size)?;
    Ok(())
}
