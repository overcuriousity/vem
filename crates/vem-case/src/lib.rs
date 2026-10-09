//! Case directory, SQLite database, ingest pipeline, verification, queries and exports.

pub mod blobs;
pub mod case;
pub mod db;
pub mod error;
pub mod evidence;
pub mod export;
pub mod ingest;
pub mod link;
pub mod query;
pub mod sink;
pub mod verify;

pub use case::{Case, CaseInfo};
pub use error::CaseError;

pub const TOOL_VERSION: &str = env!("CARGO_PKG_VERSION");
