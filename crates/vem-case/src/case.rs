//! A case: one directory holding `case.db`, `blobs/` and `exports/`.

use crate::error::CaseError;
use crate::{blobs, db, TOOL_VERSION};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use vem_core::model::TS_FORMAT;

pub fn now() -> String {
    chrono::Utc::now().format(TS_FORMAT).to_string()
}

#[derive(Debug, Clone, Serialize)]
pub struct CaseInfo {
    pub name: String,
    pub examiner: Option<String>,
    pub created_at: String,
    pub tool_version: String,
}

pub struct Case {
    pub dir: PathBuf,
    pub conn: Connection,
}

impl Case {
    pub fn create(dir: &Path, name: &str, examiner: Option<&str>) -> Result<Case, CaseError> {
        if dir.exists() && std::fs::read_dir(dir)?.next().is_some() {
            return Err(CaseError::AlreadyExists(dir.to_path_buf()));
        }
        std::fs::create_dir_all(dir.join("blobs"))?;
        std::fs::create_dir_all(dir.join("exports"))?;
        let conn = db::open(&dir.join("case.db"))?;
        conn.execute(
            "INSERT INTO case_info (id, name, examiner, created_at, tool_version) VALUES (1, ?1, ?2, ?3, ?4)",
            params![name, examiner, now(), TOOL_VERSION],
        )?;
        let case = Case { dir: dir.to_path_buf(), conn };
        case.audit("case.create", Some(name), serde_json::json!({ "examiner": examiner, "tool_version": TOOL_VERSION }))?;
        Ok(case)
    }

    pub fn open(dir: &Path) -> Result<Case, CaseError> {
        let db_path = dir.join("case.db");
        if !db_path.is_file() {
            return Err(CaseError::NotACase(dir.to_path_buf()));
        }
        let conn = db::open(&db_path)?;
        Ok(Case { dir: dir.to_path_buf(), conn })
    }

    pub fn info(&self) -> Result<CaseInfo, CaseError> {
        Ok(self.conn.query_row(
            "SELECT name, examiner, created_at, tool_version FROM case_info WHERE id = 1",
            [],
            |r| Ok(CaseInfo { name: r.get(0)?, examiner: r.get(1)?, created_at: r.get(2)?, tool_version: r.get(3)? }),
        )?)
    }

    pub fn audit(&self, action: &str, target: Option<&str>, details: Value) -> Result<(), CaseError> {
        self.conn.execute(
            "INSERT INTO audit_log (ts, action, target, details) VALUES (?1, ?2, ?3, ?4)",
            params![now(), action, target, details.to_string()],
        )?;
        Ok(())
    }

    pub fn blob_path(&self, sha256: &str) -> PathBuf {
        blobs::path(&self.dir, sha256)
    }

    pub fn put_blob(&self, bytes: &[u8]) -> Result<String, CaseError> {
        blobs::put_bytes(&self.conn, &self.dir, bytes)
    }

    pub fn has_blob(&self, sha256: &str) -> Result<bool, CaseError> {
        Ok(self
            .conn
            .query_row("SELECT 1 FROM blobs WHERE sha256 = ?1", params![sha256], |_| Ok(()))
            .optional()?
            .is_some())
    }
}
