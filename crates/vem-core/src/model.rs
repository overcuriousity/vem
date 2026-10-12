//! Canonical model shared by adapters and the case database (spec §4).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::SystemTime;

macro_rules! str_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident = $s:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum $name { $( #[serde(rename = $s)] $variant ),+ }
        impl $name {
            pub fn as_str(&self) -> &'static str { match self { $(Self::$variant => $s),+ } }
            pub fn parse(s: &str) -> Option<Self> { match s { $($s => Some(Self::$variant),)+ _ => None } }
            pub const ALL: &'static [$name] = &[$(Self::$variant),+];
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(self.as_str()) }
        }
    };
}

str_enum!(/// Which harness produced a store. `CursorIde` is Cursor's IDE application directory.
    Harness { ClaudeCode = "claude-code", Codex = "codex", Cursor = "cursor", CursorIde = "cursor-ide" });
str_enum!(/// Where a timestamp came from (spec §4).
    TsOrigin { Stored = "stored", StoredLocalClock = "stored_local_clock", FileMtime = "file_mtime", NeighborInterpolated = "neighbor_interpolated", Absent = "absent" });
str_enum!(Role { User = "user", Assistant = "assistant", System = "system", Tool = "tool", Meta = "meta" });
str_enum!(BlockKind { Text = "text", Thinking = "thinking", ToolUse = "tool_use", ToolResult = "tool_result", Image = "image", Attachment = "attachment", Other = "other" });
str_enum!(ToolCategory { Shell = "shell", FileRead = "file_read", FileWrite = "file_write", FileEdit = "file_edit", Search = "search", Web = "web", Agent = "agent", Mcp = "mcp", Other = "other" });
str_enum!(ObservationKind { CommandExecuted = "command_executed", FileRead = "file_read", FileWritten = "file_written", FileEdited = "file_edited", FileDeleted = "file_deleted", SubagentSpawned = "subagent_spawned", UrlReferenced = "url_referenced", SecretCandidate = "secret_candidate", PasteDetected = "paste_detected", UploadDetected = "upload_detected" });
str_enum!(Confidence { High = "high", Medium = "medium", Low = "low" });
str_enum!(/// `SidecarOnly`: a session known only from a sidecar (e.g. `history.jsonl`) whose transcript is gone.
    SessionKind { Primary = "primary", Subagent = "subagent", Resumed = "resumed", Forked = "forked", SidecarOnly = "sidecar_only" });
str_enum!(JoinStatus { Matched = "matched", Unmatched = "unmatched", Ambiguous = "ambiguous" });
str_enum!(AnomalyKind {
    TruncatedLine = "truncated_line", MalformedRecord = "malformed_record", UnknownRecordType = "unknown_record_type",
    UnknownStoreGeneration = "unknown_store_generation", MissingTimestamp = "missing_timestamp", OrphanedFile = "orphaned_file",
    SupersededFile = "superseded_file", ArchivedSession = "archived_session", UnlinkedSubagent = "unlinked_subagent",
    FolderDateClockMismatch = "folder_date_clock_mismatch", HashDrift = "hash_drift", EmptyStore = "empty_store",
    OversizedRecord = "oversized_record", UnpairedToolResult = "unpaired_tool_result", MissingTranscript = "missing_transcript",
    SuspiciousPath = "suspicious_path", NonUtf8Path = "non_utf8_path", SymlinkInEvidence = "symlink_in_evidence", InvalidUtf8 = "invalid_utf8",
    UnreadableFile = "unreadable_file",
});
str_enum!(Severity { Info = "info", Warning = "warning", Error = "error" });
str_enum!(ProvOrigin { Stored = "stored", Derived = "derived", Inferred = "inferred" });

/// The one timestamp format stored anywhere in vem: UTC, millisecond precision, `Z` suffix.
pub const TS_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.3fZ";

pub fn normalize_rfc3339(raw: &str) -> Option<String> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|d| d.with_timezone(&chrono::Utc).format(TS_FORMAT).to_string())
}

pub fn normalize_epoch_ms(ms: i64) -> Option<String> {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)
        .map(|d| d.format(TS_FORMAT).to_string())
}

pub fn format_system_time(t: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(t)
        .format(TS_FORMAT)
        .to_string()
}

/// A timestamp plus where it came from. `value` is always in `TS_FORMAT` when present.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timestamp {
    pub value: Option<String>,
    pub origin: TsOrigin,
}

impl Timestamp {
    pub fn absent() -> Self {
        Self {
            value: None,
            origin: TsOrigin::Absent,
        }
    }
    pub fn stored(raw: &str) -> Option<Self> {
        normalize_rfc3339(raw).map(|v| Self {
            value: Some(v),
            origin: TsOrigin::Stored,
        })
    }
    pub fn stored_epoch_ms(ms: i64) -> Option<Self> {
        normalize_epoch_ms(ms).map(|v| Self {
            value: Some(v),
            origin: TsOrigin::Stored,
        })
    }
    pub fn from_mtime(t: SystemTime) -> Self {
        Self {
            value: Some(format_system_time(t)),
            origin: TsOrigin::FileMtime,
        }
    }
    pub fn is_present(&self) -> bool {
        self.value.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceFileHandle(pub i64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionHandle(pub i64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageHandle(pub i64);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolCallHandle(pub i64);

/// Where a parsed row came from, down to the byte range (spec §5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub source_file: SourceFileHandle,
    pub byte_offset: u64,
    pub byte_length: u64,
    pub record_index: u64,
    pub content_sha256: String,
    pub parser_name: String,
    pub parser_version: String,
    pub origin: ProvOrigin,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionDraft {
    pub harness_session_id: String,
    pub kind: SessionKind,
    pub parent_harness_session_id: Option<String>,
    pub title: Option<String>,
    pub project_path: Option<String>,
    pub git_branch: Option<String>,
    pub harness_version: Option<String>,
    /// When `None`, the case computes bounds from message timestamps, then file mtime.
    pub first_ts: Option<Timestamp>,
    pub last_ts: Option<Timestamp>,
}

/// Fields an adapter learns after it has already emitted the session. Only `Some` fields are applied,
/// and they never overwrite a value already set (first seen wins), except `title` which last-seen wins.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionUpdate {
    pub title: Option<String>,
    pub project_path: Option<String>,
    pub git_branch: Option<String>,
    pub harness_version: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockDraft {
    pub kind: BlockKind,
    pub text: Option<String>,
    pub payload: Value,
    pub tool_use_id: Option<String>,
}

impl BlockDraft {
    pub fn text(s: &str) -> Self {
        Self {
            kind: BlockKind::Text,
            text: Some(s.to_string()),
            payload: Value::Null,
            tool_use_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageDraft {
    pub harness_record_type: String,
    pub harness_uuid: Option<String>,
    pub parent_uuid: Option<String>,
    pub role: Role,
    pub timestamp: Timestamp,
    pub model: Option<String>,
    pub attributes: serde_json::Map<String, Value>,
    pub blocks: Vec<BlockDraft>,
    pub provenance: Provenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockRef {
    pub message: MessageHandle,
    pub ordinal: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallDraft {
    pub name: String,
    pub category: ToolCategory,
    pub input: Value,
    pub tool_use: BlockRef,
    pub tool_result: Option<BlockRef>,
    pub result_text: Option<String>,
    pub result_payload: Option<Value>,
    pub is_error: bool,
    pub started: Timestamp,
    pub ended: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Derivation {
    ToolCall(ToolCallHandle),
    Block(BlockRef),
    Record(Provenance),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservationDraft {
    pub kind: ObservationKind,
    pub derived_from: Derivation,
    pub path: Option<String>,
    pub command: Option<String>,
    pub before_blob: Option<String>,
    pub after_blob: Option<String>,
    pub timestamp: Timestamp,
    pub confidence: Confidence,
    pub details: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdentityClaimDraft {
    pub scheme: String,
    pub claimed_id: String,
    pub source_file: SourceFileHandle,
    pub join_status: JoinStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnomalyDraft {
    pub kind: AnomalyKind,
    pub severity: Severity,
    pub source_file: Option<SourceFileHandle>,
    pub session: Option<SessionHandle>,
    pub byte_offset: Option<u64>,
    pub message: String,
    pub details: Value,
    /// The record the anomaly is about. Every record-level anomaly carries it (spec §5); file-level
    /// anomalies (orphaned file, oversized record whose bytes were not buffered) leave it `None`.
    pub provenance: Option<Provenance>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_offsets_to_utc_millis() {
        assert_eq!(
            Timestamp::stored("2026-09-30T12:00:00.000+02:00")
                .unwrap()
                .value
                .as_deref(),
            Some("2026-09-30T10:00:00.000Z")
        );
        assert_eq!(
            Timestamp::stored("2026-09-30T10:00:00Z")
                .unwrap()
                .value
                .as_deref(),
            Some("2026-09-30T10:00:00.000Z")
        );
        assert_eq!(
            Timestamp::stored("2026-09-30T10:00:00.123456Z")
                .unwrap()
                .value
                .as_deref(),
            Some("2026-09-30T10:00:00.123Z")
        );
    }

    #[test]
    fn rejects_unparseable_or_offsetless_timestamps() {
        assert!(Timestamp::stored("yesterday").is_none());
        assert!(Timestamp::stored("2026-09-30 10:00:00").is_none());
        assert!(Timestamp::stored("").is_none());
    }

    #[test]
    fn epoch_millis_and_mtime() {
        assert_eq!(
            Timestamp::stored_epoch_ms(1_790_000_000_000)
                .unwrap()
                .value
                .as_deref(),
            Some("2026-09-21T14:13:20.000Z")
        );
        let t = Timestamp::from_mtime(
            SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(1_790_000_000_500),
        );
        assert_eq!(t.origin, TsOrigin::FileMtime);
        assert_eq!(t.value.as_deref(), Some("2026-09-21T14:13:20.500Z"));
        assert_eq!(Timestamp::absent().origin, TsOrigin::Absent);
    }

    #[test]
    fn enums_round_trip_through_strings_and_serde() {
        assert_eq!(Harness::ClaudeCode.as_str(), "claude-code");
        assert_eq!(Harness::parse("cursor-ide"), Some(Harness::CursorIde));
        assert_eq!(AnomalyKind::parse("nope"), None);
        assert_eq!(serde_json::to_string(&Role::Tool).unwrap(), "\"tool\"");
        let k: ObservationKind = serde_json::from_str("\"file_edited\"").unwrap();
        assert_eq!(k, ObservationKind::FileEdited);
        assert_eq!(TsOrigin::StoredLocalClock.to_string(), "stored_local_clock");
    }
}
