//! Parser for `projects/<cwd>/<session>.jsonl` transcripts and their `subagents/` siblings (spec §6.1).

use super::discover::is_transcript_name;
use serde_json::{json, Map, Value};
use std::collections::{BTreeSet, HashMap};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use vem_core::adapter::{FileContext, ParseError};
use vem_core::hash::sha256_hex;
use vem_core::jsonl::{JsonlReader, RawRecord};
use vem_core::model::*;
use vem_core::sink::ParseSink;

pub const PARSER_NAME: &str = "claude_code.transcript";
pub const PARSER_VERSION: &str = "1";

/// Record types that are session bookkeeping, not conversation. They become `meta` messages.
const KNOWN_META: &[&str] = &[
    "attachment", "summary", "ai-title", "custom-title", "last-prompt", "mode", "permission-mode",
    "atis-latch", "bridge-session", "file-history-snapshot", "file-history-delta", "queue-operation",
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
    let comps: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
    let n = comps.len();
    let is_subagent = n >= 2 && comps[n - 2] == "subagents";
    let parent_session_id = if is_subagent && n >= 3 { Some(comps[n - 3].clone()) } else { None };
    Some(TranscriptPath { session_id, is_subagent, parent_session_id, flags })
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
        "text" => BlockDraft { kind: BlockKind::Text, text: str_field(item, "text"), payload: item.clone(), tool_use_id: None },
        "thinking" => BlockDraft { kind: BlockKind::Thinking, text: str_field(item, "thinking"), payload: item.clone(), tool_use_id: None },
        "tool_use" => BlockDraft { kind: BlockKind::ToolUse, text: None, payload: item.clone(), tool_use_id: str_field(item, "id") },
        "tool_result" => BlockDraft {
            kind: BlockKind::ToolResult,
            text: Some(flatten_text(item.get("content"))),
            payload: item.clone(),
            tool_use_id: str_field(item, "tool_use_id"),
        },
        "image" => BlockDraft { kind: BlockKind::Image, text: None, payload: item.clone(), tool_use_id: None },
        _ => BlockDraft { kind: BlockKind::Other, text: None, payload: item.clone(), tool_use_id: None },
    }
}

pub fn blocks_from_content(content: Option<&Value>) -> Vec<BlockDraft> {
    match content {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::String(s)) => vec![BlockDraft::text(s)],
        Some(Value::Array(items)) => items.iter().map(block_from_item).collect(),
        Some(other) => vec![BlockDraft { kind: BlockKind::Other, text: None, payload: other.clone(), tool_use_id: None }],
    }
}

pub(crate) struct PendingToolUse {
    pub message: MessageHandle,
    pub ordinal: u32,
    pub name: String,
    pub input: Value,
    pub started: Timestamp,
}

#[allow(dead_code)] // root, session_id, pending are read by Tasks 7-9
pub(crate) struct TranscriptState<'a> {
    pub root: &'a Path,
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

impl<'a> TranscriptState<'a> {
    fn anomaly(&self, sink: &mut dyn ParseSink, kind: AnomalyKind, severity: Severity, offset: Option<u64>, message: String, details: Value) {
        sink.anomaly(AnomalyDraft {
            kind,
            severity,
            source_file: Some(self.handle),
            session: Some(self.session),
            byte_offset: offset,
            message,
            details,
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

    fn timestamp_of(&self, v: &Value, prov: &Provenance, conversation: bool, sink: &mut dyn ParseSink) -> Timestamp {
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
                        Some(prov.byte_offset),
                        match raw {
                            Some(r) => format!("conversation record has unparseable timestamp {r:?}"),
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
        let rtype = v.get("type").and_then(Value::as_str).unwrap_or("").to_string();
        self.note_session_fields(&v, sink);
        match rtype.as_str() {
            "user" | "assistant" | "system" => self.conversation_record(&rtype, v, prov, sink),
            "ai-title" | "custom-title" => {
                let title = str_field(&v, "aiTitle").or_else(|| str_field(&v, "customTitle")).or_else(|| str_field(&v, "title"));
                if title.is_some() {
                    sink.update_session(self.session, SessionUpdate { title, ..Default::default() });
                }
                self.meta_record(&rtype, v, prov, Vec::new(), sink);
            }
            "summary" => {
                let title = str_field(&v, "summary");
                if title.is_some() {
                    sink.update_session(self.session, SessionUpdate { title, ..Default::default() });
                }
                self.meta_record(&rtype, v, prov, Vec::new(), sink);
            }
            "attachment" => {
                let blocks = vec![BlockDraft {
                    kind: BlockKind::Attachment,
                    text: v.get("attachment").and_then(|a| a.get("content")).and_then(Value::as_str).map(str::to_string),
                    payload: v.get("attachment").cloned().unwrap_or(Value::Null),
                    tool_use_id: None,
                }];
                self.meta_record(&rtype, v, prov, blocks, sink);
            }
            // FILE HISTORY (Task 9): "file-history-delta" gains an observation here.
            t if KNOWN_META.contains(&t) => self.meta_record(&rtype, v, prov, Vec::new(), sink),
            other => {
                self.anomaly(
                    sink,
                    AnomalyKind::UnknownRecordType,
                    Severity::Info,
                    Some(prov.byte_offset),
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

    fn meta_record(&mut self, rtype: &str, v: Value, prov: Provenance, blocks: Vec<BlockDraft>, sink: &mut dyn ParseSink) {
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

    fn conversation_record(&mut self, rtype: &str, v: Value, prov: Provenance, sink: &mut dyn ParseSink) {
        let msg = v.get("message");
        let blocks = blocks_from_content(msg.and_then(|m| m.get("content")));
        let only_tool_results = !blocks.is_empty() && blocks.iter().all(|b| b.kind == BlockKind::ToolResult);
        let role = match (rtype, only_tool_results) {
            ("user", true) => Role::Tool,
            ("user", false) => Role::User,
            ("assistant", _) => Role::Assistant,
            _ => Role::System,
        };
        let timestamp = self.timestamp_of(&v, &prov, true, sink);
        let model = msg.and_then(|m| m.get("model")).and_then(Value::as_str).map(str::to_string);
        if let Some(m) = &model {
            if self.models_sent.insert(m.clone()) {
                sink.update_session(self.session, SessionUpdate { model: Some(m.clone()), ..Default::default() });
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
        super::tools::pair_blocks(self, handle, &blocks, tool_use_result.as_ref(), &timestamp, &prov, sink);
    }

    pub fn finish(&mut self, sink: &mut dyn ParseSink) {
        super::tools::flush_unfinished(self, sink);
        for sid in std::mem::take(&mut self.session_ids) {
            let join_status = if sid == self.session_id { JoinStatus::Matched } else { JoinStatus::Unmatched };
            sink.identity_claim(self.session, IdentityClaimDraft { scheme: "claude:sessionId".to_string(), claimed_id: sid, source_file: self.handle, join_status });
        }
        for sid in std::mem::take(&mut self.origin_ids) {
            if sid == self.session_id {
                continue;
            }
            sink.identity_claim(self.session, IdentityClaimDraft { scheme: "claude:origin_session_id".to_string(), claimed_id: sid, source_file: self.handle, join_status: JoinStatus::Unmatched });
        }
    }
}

pub fn parse_transcript(ctx: &FileContext<'_>, sink: &mut dyn ParseSink) -> Result<(), ParseError> {
    let path = classify_path(ctx.rel_path)
        .ok_or_else(|| ParseError::Invalid(format!("not a transcript path: {}", ctx.rel_path.display())))?;
    let session = sink.session(SessionDraft {
        harness_session_id: path.session_id.clone(),
        kind: if path.is_subagent { SessionKind::Subagent } else { SessionKind::Primary },
        parent_harness_session_id: path.parent_session_id.clone(),
        title: None,
        project_path: None,
        git_branch: None,
        harness_version: None,
        first_ts: None,
        last_ts: None,
    });
    let mut state = TranscriptState {
        root: ctx.root,
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
    let file_name = ctx.rel_path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    for flag in &path.flags {
        let (kind, what) = match *flag {
            "orphaned" => (AnomalyKind::OrphanedFile, "set aside as orphaned by the harness"),
            _ => (AnomalyKind::SupersededFile, "set aside as superseded by the harness"),
        };
        state.anomaly(sink, kind, Severity::Info, None, format!("transcript {file_name} was {what}; parsed anyway"), json!({ "file_name": file_name }));
    }
    if path.is_subagent {
        read_subagent_meta(&mut state, &ctx.abs_path, sink);
    }
    let file = File::open(&ctx.abs_path)?;
    let reader = JsonlReader::new(BufReader::new(file));
    for rec in reader {
        let rec = rec?;
        let prov = provenance(ctx.handle, &rec);
        if rec.oversized {
            state.anomaly(
                sink,
                AnomalyKind::OversizedRecord,
                Severity::Warning,
                Some(rec.offset),
                format!("record of {} bytes exceeds the size cap and was skipped", rec.length),
                json!({ "length": rec.length }),
            );
            continue;
        }
        match serde_json::from_slice::<Value>(&rec.bytes) {
            Ok(v) => state.record(v, prov, sink),
            Err(e) => {
                let (kind, severity, msg) = if rec.terminated {
                    (AnomalyKind::MalformedRecord, Severity::Error, format!("line {} is not valid JSON: {e}", rec.index))
                } else {
                    (AnomalyKind::TruncatedLine, Severity::Warning, format!("final line {} is truncated (no newline, not valid JSON): {e}", rec.index))
                };
                state.anomaly(sink, kind, severity, Some(rec.offset), msg, json!({ "length": rec.length, "terminated": rec.terminated }));
            }
        }
    }
    state.finish(sink);
    Ok(())
}

/// `agent-<id>.meta.json` next to a subagent transcript: title from `description`, claim on `toolUseId`.
fn read_subagent_meta(state: &mut TranscriptState<'_>, abs_path: &Path, sink: &mut dyn ParseSink) {
    let meta_path = abs_path.with_extension("meta.json");
    let Ok(bytes) = std::fs::read(&meta_path) else { return };
    let Ok(meta) = serde_json::from_slice::<Value>(&bytes) else {
        state.anomaly(sink, AnomalyKind::MalformedRecord, Severity::Warning, None, format!("subagent meta file {} is not valid JSON", meta_path.display()), json!({}));
        return;
    };
    let title = str_field(&meta, "description");
    if title.is_some() {
        sink.update_session(state.session, SessionUpdate { title, ..Default::default() });
    }
    if let Some(tool_use_id) = str_field(&meta, "toolUseId") {
        sink.identity_claim(
            state.session,
            IdentityClaimDraft { scheme: "claude:spawning_tool_use_id".to_string(), claimed_id: tool_use_id, source_file: state.handle, join_status: JoinStatus::Unmatched },
        );
    }
    let mut attrs = Map::new();
    attrs.insert("subagent_meta".to_string(), meta);
    // The meta file content is kept on the session through a meta message with inferred provenance.
    sink.message(
        state.session,
        MessageDraft {
            harness_record_type: "subagent-meta".to_string(),
            harness_uuid: None,
            parent_uuid: None,
            role: Role::Meta,
            timestamp: Timestamp::absent(),
            model: None,
            attributes: attrs,
            blocks: Vec::new(),
            provenance: Provenance {
                source_file: state.handle,
                byte_offset: 0,
                byte_length: 0,
                record_index: 0,
                content_sha256: sha256_hex(&bytes),
                parser_name: PARSER_NAME.to_string(),
                parser_version: PARSER_VERSION.to_string(),
                origin: ProvOrigin::Derived,
            },
        },
    );
}
