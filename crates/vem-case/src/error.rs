use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum CaseError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("directory is not empty, refusing to create a case in it: {0}")]
    AlreadyExists(PathBuf),
    #[error("not a case directory (no case.db): {0}")]
    NotACase(PathBuf),
    #[error("case schema version {0} is newer than this tool supports")]
    SchemaTooNew(i64),
    #[error("evidence path is not recognized as a harness directory: {path}{hint}")]
    Unrecognized { path: PathBuf, hint: String },
    #[error("evidence path matches several harnesses ({0}); pass --harness to choose")]
    Ambiguous(String),
    #[error("no evidence root with id {0}")]
    NoSuchRoot(i64),
    #[error("no adapter for harness {0}")]
    NoAdapter(String),
    #[error("export: {0}")]
    Export(String),
    #[error("integrity: {0}")]
    IntegrityMismatch(String),
    #[error("refusing: {path} is inside evidence root {root}; nothing may be created under an evidence root")]
    InsideEvidence { path: PathBuf, root: PathBuf },
    #[error("refusing to attach {root}: it overlaps the case directory {case}")]
    EvidenceOverlapsCase { root: PathBuf, case: PathBuf },
    #[error("no session with id {0}")]
    NoSuchSession(i64),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid: {0}")]
    Invalid(String),
}
