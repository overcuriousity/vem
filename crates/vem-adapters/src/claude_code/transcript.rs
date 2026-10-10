//! Parser for `projects/<cwd>/<session>.jsonl` transcripts and their `subagents/` siblings (spec §6.1).

use super::discover::is_transcript_name;
use serde_json::{json, Map, Value};
use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Component, Path};
use vem_core::adapter::{FileContext, ParseError, ParseOutcome};
use vem_core::hash::sha256_hex;
use vem_core::jsonl::{JsonlReader, RawRecord, DEFAULT_MAX_LEN};
use vem_core::model::*;
use vem_core::sink::{ParseSink, RootFileError};

pub const PARSER_NAME: &str = "claude_code.transcript";
pub const PARSER_VERSION: &str = "1";

/// Record types that are session bookkeeping, not conversation. They become `meta` messages.
const KNOWN_META: &[&str] = &[
    "attachment",
    "summary",
    "ai-title",
    "custom-title",
    "last-prompt",
    "mode",
    "permission-mode",
    "atis-latch",
    "bridge-session",
    "file-history-snapshot",
    "file-history-delta",
    "queue-operation",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptPath {
    pub session_id: String,
    pub is_subagent: bool,
    pub parent_session_id: Option<String>,
    /// `orphaned` and/or `superseded` when the file name carries those markers.
    pub flags: Vec<&'static str>,
}

/// `projects/<cwd>/<uuid>.jsonl[.orphaned-N]` or `projects/<cwd>/<uuid>/subagents/agent-<id>.jsonl`.
pub fn classify_path(rel: &Path) -> Option<TranscriptPath> {
    let name = rel.file_name()?.to_str()?;
    if !is_transcript_name(name) {
        return None;
    }
    let session_id = name.split(".jsonl").next()?.to_string();
    let mut flags = Vec::new();
    if name.contains(".orphaned-") {
        flags.push("orphaned");
    }
    if name.contains(".superseded-") {
        flags.push("superseded");
    }
    let comps: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    let n = comps.len();
    let is_subagent = n >= 2 && comps[n - 2] == "subagents";
    let parent_session_id = if is_subagent && n >= 3 {
        Some(comps[n - 3].clone())
    } else {
        None
    };
    Some(TranscriptPath {
        session_id,
        is_subagent,
        parent_session_id,
        flags,
    })
}

pub fn provenance(handle: SourceFileHandle, rec: &RawRecord) -> Provenance {
    Provenance {
        source_file: handle,
        byte_offset: rec.offset,
        byte_length: rec.length,
        record_index: rec.index,
        content_sha256: sha256_hex(&rec.bytes),
        parser_name: PARSER_NAME.to_string(),
        parser_version: PARSER_VERSION.to_string(),
        origin: ProvOrigin::Stored,
    }
}

pub fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Text of a `content` value: a string, or the `text` of each item in an array, joined by newlines.
pub fn flatten_text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|i| i.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

pub fn block_from_item(item: &Value) -> BlockDraft {
    let kind = item.get("type").and_then(Value::as_str).unwrap_or("");
    match kind {
        "text" => BlockDraft {
            kind: BlockKind::Text,
            text: str_field(item, "text"),
            payload: item.clone(),
            tool_use_id: None,
        },
        "thinking" => BlockDraft {
            kind: BlockKind::Thinking,
            text: str_field(item, "thinking"),
            payload: item.clone(),
            tool_use_id: None,
        },
        "tool_use" => BlockDraft {
            kind: BlockKind::ToolUse,
            text: None,
            payload: item.clone(),
            tool_use_id: str_field(item, "id"),
        },
        "tool_result" => BlockDraft {
            kind: BlockKind::ToolResult,
            text: Some(flatten_text(item.get("content"))),
            payload: item.clone(),
            tool_use_id: str_field(item, "tool_use_id"),
        },
        "image" => BlockDraft {
            kind: BlockKind::Image,
            text: None,
            payload: item.clone(),
            tool_use_id: None,
        },
        _ => BlockDraft {
            kind: BlockKind::Other,
            text: None,
            payload: item.clone(),
            tool_use_id: None,
        },
    }
}

pub fn blocks_from_content(content: Option<&Value>) -> Vec<BlockDraft> {
    match content {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::String(s)) => vec![BlockDraft::text(s)],
        Some(Value::Array(items)) => items.iter().map(block_from_item).collect(),
        Some(other) => vec![BlockDraft {
            kind: BlockKind::Other,
            text: None,
            payload: other.clone(),
            tool_use_id: None,
        }],
    }
}

pub(crate) struct PendingToolUse {
    pub message: MessageHandle,
    pub ordinal: u32,
    pub name: String,
    pub input: Value,
    pub started: Timestamp,
}

pub(crate) struct TranscriptState {
    pub handle: SourceFileHandle,
    pub session: SessionHandle,
    pub session_id: String,
    pub version_sent: bool,
    pub cwd_sent: bool,
    pub branch_sent: bool,
    pub models_sent: BTreeSet<String>,
    pub session_ids: BTreeSet<String>,
    pub origin_ids: BTreeSet<String>,
    pub pending: HashMap<String, PendingToolUse>,
}

impl TranscriptState {
    /// A record-level anomaly about the record at `at`, or a file-level one when `at` is `None`.
    fn anomaly(
        &self,
        sink: &mut dyn ParseSink,
        kind: AnomalyKind,
        severity: Severity,
        at: Option<&Provenance>,
        message: String,
        details: Value,
    ) {
        sink.anomaly(AnomalyDraft {
            kind,
            severity,
            source_file: Some(self.handle),
            session: Some(self.session),
            byte_offset: at.map(|p| p.byte_offset),
            message,
            details,
            provenance: at.cloned(),
        });
    }

    fn note_session_fields(&mut self, v: &Value, sink: &mut dyn ParseSink) {
        let mut update = SessionUpdate::default();
        if !self.version_sent {
            if let Some(ver) = str_field(v, "version") {
                update.harness_version = Some(ver);
                self.version_sent = true;
            }
        }
        if !self.cwd_sent {
            if let Some(cwd) = str_field(v, "cwd") {
                update.project_path = Some(cwd);
                self.cwd_sent = true;
            }
        }
        if !self.branch_sent {
            if let Some(b) = str_field(v, "gitBranch") {
                update.git_branch = Some(b);
                self.branch_sent = true;
            }
        }
        if let Some(sid) = str_field(v, "sessionId") {
            self.session_ids.insert(sid);
        }
        if let Some(sid) = str_field(v, "session_id") {
            self.origin_ids.insert(sid);
        }
        if update != SessionUpdate::default() {
            sink.update_session(self.session, update);
        }
    }

    fn timestamp_of(
        &self,
        v: &Value,
        prov: &Provenance,
        conversation: bool,
        sink: &mut dyn ParseSink,
    ) -> Timestamp {
        let raw = v.get("timestamp").and_then(Value::as_str);
        let ts = raw.and_then(Timestamp::stored);
        match ts {
            Some(t) => t,
            None => {
                if conversation {
                    self.anomaly(
                        sink,
                        AnomalyKind::MissingTimestamp,
                        Severity::Warning,
                        Some(prov),
                        match raw {
                            Some(r) => {
                                format!("conversation record has unparseable timestamp {r:?}")
                            }
                            None => "conversation record has no timestamp".to_string(),
                        },
                        json!({ "record_type": v.get("type") }),
                    );
                }
                Timestamp::absent()
            }
        }
    }

    pub fn record(&mut self, v: Value, prov: Provenance, sink: &mut dyn ParseSink) {
        let rtype = v
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        self.note_session_fields(&v, sink);
        match rtype.as_str() {
            "user" | "assistant" | "system" => self.conversation_record(&rtype, v, prov, sink),
            "ai-title" | "custom-title" => {
                let title = str_field(&v, "aiTitle")
                    .or_else(|| str_field(&v, "customTitle"))
                    .or_else(|| str_field(&v, "title"));
                if title.is_some() {
                    sink.update_session(
                        self.session,
                        SessionUpdate {
                            title,
                            ..Default::default()
                        },
                    );
                }
                self.meta_record(&rtype, v, prov, Vec::new(), sink);
            }
            "summary" => {
                let title = str_field(&v, "summary");
                if title.is_some() {
                    sink.update_session(
                        self.session,
                        SessionUpdate {
                            title,
                            ..Default::default()
                        },
                    );
                }
                self.meta_record(&rtype, v, prov, Vec::new(), sink);
            }
            "attachment" => {
                let blocks = vec![BlockDraft {
                    kind: BlockKind::Attachment,
                    text: v
                        .get("attachment")
                        .and_then(|a| a.get("content"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    payload: v.get("attachment").cloned().unwrap_or(Value::Null),
                    tool_use_id: None,
                }];
                self.meta_record(&rtype, v, prov, blocks, sink);
            }
            "file-history-delta" => {
                self.file_history_delta(&v, &prov, sink);
                self.meta_record(&rtype, v, prov, Vec::new(), sink);
            }
            t if KNOWN_META.contains(&t) => self.meta_record(&rtype, v, prov, Vec::new(), sink),
            other => {
                self.anomaly(
                    sink,
                    AnomalyKind::UnknownRecordType,
                    Severity::Info,
                    Some(&prov),
                    format!("unknown record type {other:?} kept as meta message"),
                    json!({ "record_type": other }),
                );
                self.meta_record(&rtype, v, prov, Vec::new(), sink);
            }
        }
    }

    fn attributes_of(v: &Value) -> Map<String, Value> {
        let mut attrs = Map::new();
        if let Value::Object(obj) = v {
            for (k, val) in obj {
                if k != "message" && k != "toolUseResult" {
                    attrs.insert(k.clone(), val.clone());
                }
            }
        }
        attrs
    }

    fn meta_record(
        &mut self,
        rtype: &str,
        v: Value,
        prov: Provenance,
        blocks: Vec<BlockDraft>,
        sink: &mut dyn ParseSink,
    ) {
        let timestamp = self.timestamp_of(&v, &prov, false, sink);
        let attributes = match &v {
            Value::Object(obj) => obj.clone(),
            other => {
                let mut m = Map::new();
                m.insert("value".to_string(), other.clone());
                m
            }
        };
        sink.message(
            self.session,
            MessageDraft {
                harness_record_type: rtype.to_string(),
                harness_uuid: str_field(&v, "uuid"),
                parent_uuid: str_field(&v, "parentUuid"),
                role: Role::Meta,
                timestamp,
                model: None,
                attributes,
                blocks,
                provenance: prov,
            },
        );
    }

    fn conversation_record(
        &mut self,
        rtype: &str,
        v: Value,
        prov: Provenance,
        sink: &mut dyn ParseSink,
    ) {
        let msg = v.get("message");
        let blocks = blocks_from_content(msg.and_then(|m| m.get("content")));
        let only_tool_results =
            !blocks.is_empty() && blocks.iter().all(|b| b.kind == BlockKind::ToolResult);
        let role = match (rtype, only_tool_results) {
            ("user", true) => Role::Tool,
            ("user", false) => Role::User,
            ("assistant", _) => Role::Assistant,
            _ => Role::System,
        };
        let timestamp = self.timestamp_of(&v, &prov, true, sink);
        let model = msg
            .and_then(|m| m.get("model"))
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(m) = &model {
            if self.models_sent.insert(m.clone()) {
                sink.update_session(
                    self.session,
                    SessionUpdate {
                        model: Some(m.clone()),
                        ..Default::default()
                    },
                );
            }
        }
        let tool_use_result = v.get("toolUseResult").cloned();
        let attributes = Self::attributes_of(&v);
        let handle = sink.message(
            self.session,
            MessageDraft {
                harness_record_type: rtype.to_string(),
                harness_uuid: str_field(&v, "uuid"),
                parent_uuid: str_field(&v, "parentUuid"),
                role,
                timestamp: timestamp.clone(),
                model,
                attributes,
                blocks: blocks.clone(),
                provenance: prov.clone(),
            },
        );
        super::tools::pair_blocks(
            self,
            handle,
            &blocks,
            tool_use_result.as_ref(),
            &timestamp,
            &prov,
            sink,
        );
    }

    /// A `file-history-delta` records that the harness backed up `trackingPath` before changing it.
    /// The backup lives at `file-history/<session>/<backupFileName>`; when present it is the before-content.
    /// `backupFileName` is evidence data, so it must be one plain file name and the backup must be a regular
    /// file strictly inside that directory (no `..`, no separators, no absolute path, no symbolic link);
    /// anything else is a `suspicious_path` anomaly and the backup is not read.
    pub fn file_history_delta(&mut self, v: &Value, prov: &Provenance, sink: &mut dyn ParseSink) {
        let tracking = str_field(v, "trackingPath");
        let backup = v.get("backup").cloned().unwrap_or(Value::Null);
        let backup_name = str_field(&backup, "backupFileName");
        let real_parent = str_field(&backup, "realParentDir");
        let path = match (&real_parent, &tracking) {
            (Some(dir), Some(t)) => Some(join_evidence_path(dir, t)),
            (None, Some(t)) => Some(t.clone()),
            _ => None,
        };
        let before_blob = match &backup_name {
            None => None,
            Some(name) => match read_backup(&*sink, &self.session_id, name) {
                Ok(Some(bytes)) => Some(sink.blob(&bytes)),
                Ok(None) => None,
                Err((kind, severity, reason)) => {
                    self.anomaly(
                        sink,
                        kind,
                        severity,
                        Some(prov),
                        format!("file-history backup {name:?} was not read: {reason}"),
                        json!({ "backupFileName": name, "reason": reason }),
                    );
                    None
                }
            },
        };
        let timestamp = v
            .get("timestamp")
            .and_then(Value::as_str)
            .or_else(|| backup.get("backupTime").and_then(Value::as_str))
            .and_then(Timestamp::stored)
            .unwrap_or_else(Timestamp::absent);
        sink.observation(
            self.session,
            ObservationDraft {
                kind: ObservationKind::FileEdited,
                derived_from: Derivation::Record(prov.clone()),
                path,
                command: None,
                before_blob,
                after_blob: None,
                timestamp,
                confidence: Confidence::Medium,
                details: json!({
                    "source": "file-history-delta",
                    "trackingPath": tracking,
                    "backupFileName": backup_name,
                    "version": backup.get("version").cloned().unwrap_or(Value::Null),
                    "backupTime": backup.get("backupTime").cloned().unwrap_or(Value::Null),
                    "messageId": str_field(v, "messageId"),
                }),
            },
        );
    }

    pub fn finish(&mut self, sink: &mut dyn ParseSink) {
        super::tools::flush_unfinished(self, sink);
        for sid in std::mem::take(&mut self.session_ids) {
            let join_status = if sid == self.session_id {
                JoinStatus::Matched
            } else {
                JoinStatus::Unmatched
            };
            sink.identity_claim(
                self.session,
                IdentityClaimDraft {
                    scheme: "claude:sessionId".to_string(),
                    claimed_id: sid,
                    source_file: self.handle,
                    join_status,
                },
            );
        }
        for sid in std::mem::take(&mut self.origin_ids) {
            if sid == self.session_id {
                continue;
            }
            sink.identity_claim(
                self.session,
                IdentityClaimDraft {
                    scheme: "claude:origin_session_id".to_string(),
                    claimed_id: sid,
                    source_file: self.handle,
                    join_status: JoinStatus::Unmatched,
                },
            );
        }
    }
}

pub fn parse_transcript(
    ctx: &FileContext<'_>,
    sink: &mut dyn ParseSink,
) -> Result<ParseOutcome, ParseError> {
    let path = classify_path(ctx.rel_path).ok_or_else(|| {
        ParseError::Invalid(format!("not a transcript path: {}", ctx.rel_path.display()))
    })?;
    let file = File::open(&ctx.abs_path)?;
    let session = sink.session(SessionDraft {
        harness_session_id: path.session_id.clone(),
        kind: if path.is_subagent {
            SessionKind::Subagent
        } else {
            SessionKind::Primary
        },
        parent_harness_session_id: path.parent_session_id.clone(),
        title: None,
        project_path: None,
        git_branch: None,
        harness_version: None,
        first_ts: None,
        last_ts: None,
    });
    let mut state = TranscriptState {
        handle: ctx.handle,
        session,
        session_id: path.session_id.clone(),
        version_sent: false,
        cwd_sent: false,
        branch_sent: false,
        models_sent: BTreeSet::new(),
        session_ids: BTreeSet::new(),
        origin_ids: BTreeSet::new(),
        pending: HashMap::new(),
    };
    let file_name = ctx
        .rel_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    for flag in &path.flags {
        let (kind, what) = match *flag {
            "orphaned" => (
                AnomalyKind::OrphanedFile,
                "set aside as orphaned by the harness",
            ),
            _ => (
                AnomalyKind::SupersededFile,
                "set aside as superseded by the harness",
            ),
        };
        state.anomaly(
            sink,
            kind,
            Severity::Info,
            None,
            format!("transcript {file_name} was {what}; parsed anyway"),
            json!({ "file_name": file_name }),
        );
    }
    let reader = JsonlReader::new(BufReader::new(file));
    let mut records = 0u64;
    for rec in reader {
        let rec = rec?;
        records += 1;
        if rec.oversized {
            // The record's bytes were not buffered past the cap, so there is no record hash to cite.
            sink.anomaly(AnomalyDraft {
                kind: AnomalyKind::OversizedRecord,
                severity: Severity::Warning,
                source_file: Some(ctx.handle),
                session: Some(state.session),
                byte_offset: Some(rec.offset),
                message: format!(
                    "record of {} bytes exceeds the size cap and was skipped",
                    rec.length
                ),
                details: json!({ "length": rec.length }),
                provenance: None,
            });
            continue;
        }
        let prov = provenance(ctx.handle, &rec);
        let (text, lossy) = decode_record(&rec.bytes);
        if lossy {
            state.anomaly(
                sink,
                AnomalyKind::InvalidUtf8,
                Severity::Info,
                Some(&prov),
                format!("line {} is not valid UTF-8; decoded lossily (hashes and offsets are on the original bytes)", rec.index),
                json!({ "length": rec.length }),
            );
        }
        match serde_json::from_str::<Value>(&text) {
            Ok(v) => state.record(v, prov, sink),
            Err(e) => {
                let (kind, severity, msg) = if rec.terminated {
                    (
                        AnomalyKind::MalformedRecord,
                        Severity::Error,
                        format!("line {} is not valid JSON: {e}", rec.index),
                    )
                } else {
                    (
                        AnomalyKind::TruncatedLine,
                        Severity::Warning,
                        format!(
                            "final line {} is truncated (no newline, not valid JSON): {e}",
                            rec.index
                        ),
                    )
                };
                state.anomaly(
                    sink,
                    kind,
                    severity,
                    Some(&prov),
                    msg,
                    json!({ "length": rec.length, "terminated": rec.terminated }),
                );
            }
        }
    }
    state.finish(sink);
    Ok(ParseOutcome::Parsed { records })
}

pub const META_PARSER_NAME: &str = "claude_code.subagent_meta";
pub const META_PARSER_VERSION: &str = "1";

/// `projects/<cwd>/<session>/subagents/agent-<id>.meta.json`.
pub fn is_subagent_meta(rel: &Path) -> bool {
    let name = rel.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let parent = rel
        .parent()
        .and_then(Path::file_name)
        .and_then(|n| n.to_str())
        .unwrap_or("");
    name.ends_with(".meta.json") && parent == "subagents"
}

/// Text of a record for JSON parsing. Invalid UTF-8 is decoded lossily (spec §10); the flag says so.
pub fn decode_record(bytes: &[u8]) -> (Cow<'_, str>, bool) {
    match std::str::from_utf8(bytes) {
        Ok(s) => (Cow::Borrowed(s), false),
        Err(_) => (String::from_utf8_lossy(bytes), true),
    }
}

/// Reads at most `cap` bytes of `path`; `Ok(None)` when the file is larger than `cap`.
pub fn read_capped(path: &Path, cap: u64) -> std::io::Result<Option<Vec<u8>>> {
    let mut buf = Vec::new();
    File::open(path)?.take(cap + 1).read_to_end(&mut buf)?;
    Ok(if buf.len() as u64 > cap {
        None
    } else {
        Some(buf)
    })
}

/// `realParentDir` + the base name of `trackingPath`, splitting on both separators because the
/// evidence may come from Windows, and joining with the separator the evidence uses.
pub fn join_evidence_path(dir: &str, tracking: &str) -> String {
    let base = tracking
        .rsplit(['/', '\\'])
        .next()
        .filter(|b| !b.is_empty())
        .unwrap_or(tracking);
    let sep = if dir.contains('\\') && !dir.contains('/') {
        '\\'
    } else {
        '/'
    };
    format!("{}{}{}", dir.trim_end_matches(['/', '\\']), sep, base)
}

/// Why a name taken from evidence data cannot be used as one path component, if it cannot.
pub(crate) fn unsafe_component(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        return Some("empty name");
    }
    if name.contains('/') || name.contains('\\') {
        return Some("contains a path separator");
    }
    if name.contains('\0') {
        return Some("contains a NUL byte");
    }
    let mut comps = Path::new(name).components();
    match (comps.next(), comps.next()) {
        (Some(Component::Normal(_)), None) => None,
        _ => Some("is not a single plain file name (e.g. `.`, `..` or a root)"),
    }
}

/// Anomaly for a manifest file that could not be served: a link is suspicious, an oversized file is
/// `oversized_record`, a hash mismatch or a vanished file is `hash_drift`.
pub(crate) fn root_file_anomaly(e: &RootFileError) -> (AnomalyKind, Severity, String) {
    match e {
        RootFileError::Symlink => (
            AnomalyKind::SuspiciousPath,
            Severity::Warning,
            "the file or a directory on its path is a symbolic link".into(),
        ),
        RootFileError::TooLarge => (
            AnomalyKind::OversizedRecord,
            Severity::Warning,
            "the file exceeds the size cap".into(),
        ),
        RootFileError::HashMismatch { expected, actual } => (
            AnomalyKind::HashDrift,
            Severity::Error,
            format!("the bytes hash to {actual}, the manifest says {expected}"),
        ),
        RootFileError::Io(err) => (
            AnomalyKind::HashDrift,
            Severity::Warning,
            format!("listed in the manifest but unreadable and not retained: {err}"),
        ),
    }
}

/// The backup `file-history/<session>/<name>`, read through the manifest (retained copy first). `Ok(None)`
/// when it simply was not collected.
fn read_backup(
    sink: &dyn ParseSink,
    session_id: &str,
    name: &str,
) -> Result<Option<Vec<u8>>, (AnomalyKind, Severity, String)> {
    let suspicious = |r: &str| {
        (
            AnomalyKind::SuspiciousPath,
            Severity::Warning,
            r.to_string(),
        )
    };
    if let Some(r) = unsafe_component(name) {
        return Err(suspicious(&format!("backupFileName {r}")));
    }
    if let Some(r) = unsafe_component(session_id) {
        return Err(suspicious(&format!("session id {r}")));
    }
    let rel = Path::new("file-history").join(session_id).join(name);
    sink.read_root_file(&rel, DEFAULT_MAX_LEN as u64)
        .map_err(|e| root_file_anomaly(&e))
}

/// `agent-<id>.meta.json` next to a subagent transcript, parsed as its own source file so its provenance
/// is its own bytes: title from `description`, claim on `toolUseId`, the document kept as a meta message.
/// Ingest order (`agent-<id>.jsonl` sorts before `agent-<id>.meta.json`) means the session already exists.
pub fn parse_subagent_meta(
    ctx: &FileContext<'_>,
    sink: &mut dyn ParseSink,
) -> Result<ParseOutcome, ParseError> {
    let name = ctx
        .rel_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    let session_id = name.strip_suffix(".meta.json").unwrap_or(&name).to_string();
    let Some(bytes) = read_capped(&ctx.abs_path, DEFAULT_MAX_LEN as u64)? else {
        sink.anomaly(AnomalyDraft {
            kind: AnomalyKind::OversizedRecord,
            severity: Severity::Warning,
            source_file: Some(ctx.handle),
            session: sink.find_session(&session_id),
            byte_offset: Some(0),
            message: format!("subagent meta file {name} exceeds the size cap and was skipped"),
            details: json!({}),
            provenance: None,
        });
        return Ok(ParseOutcome::Parsed { records: 0 });
    };
    let prov = Provenance {
        source_file: ctx.handle,
        byte_offset: 0,
        byte_length: bytes.len() as u64,
        record_index: 0,
        content_sha256: sha256_hex(&bytes),
        parser_name: META_PARSER_NAME.to_string(),
        parser_version: META_PARSER_VERSION.to_string(),
        origin: ProvOrigin::Stored,
    };
    let session = sink.find_session(&session_id);
    let anomaly = |sink: &mut dyn ParseSink, kind, severity, message: String, details: Value| {
        sink.anomaly(AnomalyDraft {
            kind,
            severity,
            source_file: Some(ctx.handle),
            session,
            byte_offset: Some(0),
            message,
            details,
            provenance: Some(prov.clone()),
        })
    };
    let (text, lossy) = decode_record(&bytes);
    if lossy {
        anomaly(
            sink,
            AnomalyKind::InvalidUtf8,
            Severity::Info,
            format!("subagent meta file {name} is not valid UTF-8; decoded lossily"),
            json!({}),
        );
    }
    let meta = match serde_json::from_str::<Value>(&text) {
        Ok(v) => v,
        Err(e) => {
            anomaly(
                sink,
                AnomalyKind::MalformedRecord,
                Severity::Warning,
                format!("subagent meta file {name} is not valid JSON: {e}"),
                json!({}),
            );
            return Ok(ParseOutcome::Parsed { records: 1 });
        }
    };
    let Some(session) = session else {
        let transcript = ctx.rel_path.with_file_name(format!("{session_id}.jsonl"));
        if sink.find_source_file(&transcript).is_some() {
            // The transcript is in the manifest but has not been parsed (it failed); retry on the next ingest.
            return Err(ParseError::Invalid(format!(
                "subagent transcript {} has not been parsed yet",
                transcript.display()
            )));
        }
        anomaly(
            sink,
            AnomalyKind::MissingTranscript,
            Severity::Warning,
            format!("subagent meta file {name} has no transcript in this root"),
            json!({ "sessionId": session_id, "subagent_meta": meta }),
        );
        return Ok(ParseOutcome::Parsed { records: 1 });
    };
    let title = str_field(&meta, "description");
    if title.is_some() {
        sink.update_session(
            session,
            SessionUpdate {
                title,
                ..Default::default()
            },
        );
    }
    if let Some(tool_use_id) = str_field(&meta, "toolUseId") {
        sink.identity_claim(
            session,
            IdentityClaimDraft {
                scheme: "claude:spawning_tool_use_id".to_string(),
                claimed_id: tool_use_id,
                source_file: ctx.handle,
                join_status: JoinStatus::Unmatched,
            },
        );
    }
    let mut attrs = Map::new();
    attrs.insert("subagent_meta".to_string(), meta);
    sink.message(
        session,
        MessageDraft {
            harness_record_type: "subagent-meta".to_string(),
            harness_uuid: None,
            parent_uuid: None,
            role: Role::Meta,
            timestamp: Timestamp::absent(),
            model: None,
            attributes: attrs,
            blocks: Vec::new(),
            provenance: prov,
        },
    );
    Ok(ParseOutcome::Parsed { records: 1 })
}
