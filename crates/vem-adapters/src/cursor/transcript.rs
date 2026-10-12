//! Parser for Cursor agent transcripts: `projects/<slug>/agent-transcripts/<id>/<id>.jsonl`,
//! `…/agent-transcripts/<id>.jsonl` and `…/<id>/subagents/<sid>.jsonl` (spec §6.3). Lines are
//! `{role, message: {content}}` with Anthropic-style content and no timestamps.

use super::tools::{categorize, derive_observations};
use super::{record_anomaly, FieldCensus, RECORD_CAP};
use crate::claude_code::transcript::{
    blocks_from_content, decode_record, provenance, root_file_anomaly, str_field, unsafe_component,
};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use vem_core::adapter::{FileContext, ParseError, ParseOutcome};
use vem_core::jsonl::JsonlReader;
use vem_core::model::*;
use vem_core::sink::ParseSink;

pub const PARSER_NAME: &str = "cursor-transcript";
pub const PARSER_VERSION: &str = env!("CARGO_PKG_VERSION");

const LINE_KEYS: &[&str] = &["role", "message"];
const MESSAGE_KEYS: &[&str] = &["content", "role", "id", "model"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTranscriptPath {
    pub session_id: String,
    pub parent_session_id: Option<String>,
    /// The `projects/<slug>` directory name, as given (lossy; not decoded).
    pub slug: Option<String>,
    /// `agent-transcripts/<id>/agent-tools`, where tool-output sidecars named `<tool_use_id>.txt` live.
    pub agent_tools: PathBuf,
}

/// Classifies a `.jsonl` file under `agent-transcripts/`; `None` for anything else (legacy `.txt`,
/// `agent-tools/*.txt`).
pub fn classify_path(rel: &Path) -> Option<AgentTranscriptPath> {
    let comps: Vec<&str> = rel
        .components()
        .map(|c| c.as_os_str().to_str())
        .collect::<Option<_>>()?;
    let at = comps.iter().rposition(|c| *c == "agent-transcripts")?;
    let base = comps[..=at].iter().collect::<PathBuf>();
    let slug = at.checked_sub(1).map(|i| comps[i].to_string());
    let stem = |name: &str| name.strip_suffix(".jsonl").map(str::to_string);
    let (session_id, parent, dir) = match &comps[at + 1..] {
        [file] => {
            let id = stem(file)?;
            (id.clone(), None, id)
        }
        [dir, file] if stem(file).as_deref() == Some(*dir) => {
            (dir.to_string(), None, dir.to_string())
        }
        [dir, "subagents", file] => (stem(file)?, Some(dir.to_string()), dir.to_string()),
        _ => return None,
    };
    Some(AgentTranscriptPath {
        session_id,
        parent_session_id: parent,
        slug,
        agent_tools: base.join(dir).join("agent-tools"),
    })
}

struct Pending {
    message: MessageHandle,
    ordinal: u32,
    name: String,
    input: Value,
    result_payload: Option<Value>,
}

/// Content blocks this parser keeps but cannot interpret, by `(type, reason)`: count, first locator,
/// first record. A block of an unknown type is kept on its message as `other`, and a `tool_use` or
/// `tool_result` without an id cannot be paired; either way what it carries (a tool call in an
/// unexpected shape, say) is not parsed, so each gets one `unknown_record_type` per file.
#[derive(Default)]
struct BlockCensus {
    seen: BTreeMap<(String, &'static str), (u64, String, Provenance)>,
}

impl BlockCensus {
    fn note(&mut self, blocks: &[BlockDraft], prov: &Provenance) {
        for (ordinal, b) in blocks.iter().enumerate() {
            let reason = match b.kind {
                BlockKind::Other => "unknown_type",
                BlockKind::ToolUse | BlockKind::ToolResult if b.tool_use_id.is_none() => {
                    "missing_id"
                }
                _ => continue,
            };
            let block_type = b
                .payload
                .get("type")
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty())
                .unwrap_or("(no type)");
            let locator = format!("line {} block {ordinal}", prov.record_index);
            self.seen
                .entry((block_type.to_string(), reason))
                .or_insert_with(|| (0, locator, prov.clone()))
                .0 += 1;
        }
    }

    fn finish(&mut self, file: SourceFileHandle, session: SessionHandle, sink: &mut dyn ParseSink) {
        for ((block_type, reason), (count, first, prov)) in std::mem::take(&mut self.seen) {
            let what = match reason {
                "missing_id" => format!("{block_type:?} without an id were kept and not paired"),
                _ => format!("of unknown type {block_type:?} were kept as `other` and not parsed"),
            };
            sink.anomaly(record_anomaly(
                file,
                Some(session),
                AnomalyKind::UnknownRecordType,
                Severity::Warning,
                Some(&prov),
                format!("{count} content block(s) {what}"),
                json!({ "record_kind": "content_block", "block_type": block_type, "reason": reason, "count": count, "first_locator": first }),
            ));
        }
    }
}

struct State<'a> {
    ctx: &'a FileContext<'a>,
    path: AgentTranscriptPath,
    session: SessionHandle,
    census: FieldCensus,
    blocks: BlockCensus,
    pending: HashMap<String, Pending>,
    /// Calls in the order their `tool_use` was seen, for a stable flush.
    order: Vec<String>,
    turn: String,
}

impl State<'_> {
    fn anomaly(
        &self,
        sink: &mut dyn ParseSink,
        kind: AnomalyKind,
        severity: Severity,
        prov: Option<&Provenance>,
        message: String,
        details: Value,
    ) {
        sink.anomaly(record_anomaly(
            self.ctx.handle,
            Some(self.session),
            kind,
            severity,
            prov,
            message,
            details,
        ));
    }

    /// The sidecar `agent-tools/<id>.txt` as a blob reference, if the manifest has one. An id that is not
    /// one plain file name is reported and never used as a path.
    fn sidecar(&self, id: &str, prov: &Provenance, sink: &mut dyn ParseSink) -> Option<Value> {
        if let Some(why) = unsafe_component(id) {
            self.anomaly(
                sink,
                AnomalyKind::SuspiciousPath,
                Severity::Warning,
                Some(prov),
                format!("tool_use id {id:?} {why}; its agent-tools sidecar was not looked up"),
                json!({ "tool_use_id": id }),
            );
            return None;
        }
        let rel = self.path.agent_tools.join(format!("{id}.txt"));
        match sink.read_root_file(&rel, RECORD_CAP) {
            Ok(Some(bytes)) => Some(json!({ "sidecar_blob": sink.blob(&bytes) })),
            Ok(None) => None,
            Err(e) => {
                let (kind, severity, msg) = root_file_anomaly(&e);
                self.anomaly(
                    sink,
                    kind,
                    severity,
                    Some(prov),
                    format!("agent-tools sidecar {}: {msg}", rel.display()),
                    json!({ "tool_use_id": id, "path": rel.to_string_lossy() }),
                );
                None
            }
        }
    }

    fn emit_call(
        &self,
        p: Pending,
        result: Option<(BlockRef, Option<String>, bool)>,
        sink: &mut dyn ParseSink,
    ) {
        let (tool_result, result_text, is_error) = match result {
            Some((r, t, e)) => (Some(r), t, e),
            None => (None, None, false),
        };
        let result_ok = tool_result.map(|_| !is_error);
        let handle = sink.tool_call(
            self.session,
            ToolCallDraft {
                category: categorize(&p.name),
                name: p.name.clone(),
                input: p.input.clone(),
                tool_use: BlockRef {
                    message: p.message,
                    ordinal: p.ordinal,
                },
                tool_result,
                result_text,
                result_payload: p.result_payload,
                is_error,
                started: Timestamp::absent(),
                ended: Timestamp::absent(),
            },
        );
        for obs in derive_observations(&p.name, &p.input, result_ok, handle, &Timestamp::absent()) {
            sink.observation(self.session, obs);
        }
    }

    fn pair(
        &mut self,
        msg: MessageHandle,
        blocks: &[BlockDraft],
        prov: &Provenance,
        sink: &mut dyn ParseSink,
    ) {
        for (ordinal, b) in blocks.iter().enumerate() {
            let Some(id) = b.tool_use_id.clone() else {
                continue;
            };
            match b.kind {
                BlockKind::ToolUse => {
                    let result_payload = self.sidecar(&id, prov, sink);
                    let p = Pending {
                        message: msg,
                        ordinal: ordinal as u32,
                        name: str_field(&b.payload, "name").unwrap_or_default(),
                        input: b.payload.get("input").cloned().unwrap_or(Value::Null),
                        result_payload,
                    };
                    // A repeated id: the earlier use is kept as a result-less call.
                    if let Some(old) = self.pending.insert(id.clone(), p) {
                        self.emit_call(old, None, sink);
                    }
                    self.order.retain(|x| *x != id);
                    self.order.push(id);
                }
                BlockKind::ToolResult => match self.pending.remove(&id) {
                    Some(p) => {
                        self.order.retain(|x| *x != id);
                        let is_error =
                            b.payload.get("is_error").and_then(Value::as_bool) == Some(true);
                        let r = BlockRef {
                            message: msg,
                            ordinal: ordinal as u32,
                        };
                        self.emit_call(p, Some((r, b.text.clone(), is_error)), sink);
                    }
                    None => self.anomaly(
                        sink,
                        AnomalyKind::UnpairedToolResult,
                        Severity::Warning,
                        Some(prov),
                        format!("tool_result {id} has no tool_use in this file"),
                        json!({ "tool_use_id": id }),
                    ),
                },
                _ => {}
            }
        }
    }

    fn line(&mut self, v: Value, prov: Provenance, sink: &mut dyn ParseSink) {
        let Value::Object(mut obj) = v else {
            self.anomaly(
                sink,
                AnomalyKind::MalformedRecord,
                Severity::Error,
                Some(&prov),
                format!("line {} is JSON but not an object", prov.record_index),
                json!({}),
            );
            return;
        };
        let locator = format!("line {}", prov.record_index);
        for k in obj.keys().filter(|k| !LINE_KEYS.contains(&k.as_str())) {
            self.census.note("line", k, &locator, &prov);
        }
        let message = obj.remove("message").unwrap_or(Value::Null);
        let role_raw = obj.remove("role");
        let mut attributes: Map<String, Value> = obj;
        let mut content = None;
        let mut model = None;
        if let Value::Object(mut m) = message {
            for k in m.keys().filter(|k| !MESSAGE_KEYS.contains(&k.as_str())) {
                self.census.note("message", k, &locator, &prov);
            }
            content = m.remove("content");
            model = m.get("model").and_then(Value::as_str).map(str::to_string);
            if !m.is_empty() {
                attributes.insert("message".into(), Value::Object(m));
            }
        } else if !message.is_null() {
            attributes.insert("message".into(), message);
        }
        let blocks = blocks_from_content(content.as_ref());
        self.blocks.note(&blocks, &prov);
        let role_str = role_raw
            .as_ref()
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let role = match role_str.as_str() {
            "user" => Role::User,
            "assistant" => Role::Assistant,
            _ => {
                self.anomaly(
                    sink,
                    AnomalyKind::UnknownRecordType,
                    Severity::Info,
                    Some(&prov),
                    format!(
                        "line {} has role {role_str:?}; kept as meta",
                        prov.record_index
                    ),
                    json!({ "role": role_raw }),
                );
                attributes.insert("role".into(), role_raw.unwrap_or(Value::Null));
                Role::Meta
            }
        };
        // A user line made only of tool results continues the turn; any other user line starts one.
        let only_results =
            !blocks.is_empty() && blocks.iter().all(|b| b.kind == BlockKind::ToolResult);
        if role == Role::User && !only_results {
            self.turn = format!("{}:{}", self.path.session_id, prov.record_index);
        } else if role != Role::Meta {
            attributes.insert("vem_turn".into(), Value::String(self.turn.clone()));
        }
        let msg = sink.message(
            self.session,
            MessageDraft {
                harness_record_type: if role_str.is_empty() {
                    "line".into()
                } else {
                    role_str
                },
                harness_uuid: None,
                parent_uuid: None,
                role,
                timestamp: Timestamp::absent(),
                model,
                attributes,
                blocks: blocks.clone(),
                provenance: prov.clone(),
            },
        );
        self.pair(msg, &blocks, &prov, sink);
    }
}

pub fn parse_transcript(
    ctx: &FileContext<'_>,
    sink: &mut dyn ParseSink,
) -> Result<ParseOutcome, ParseError> {
    let Some(path) = classify_path(ctx.rel_path) else {
        return Ok(ParseOutcome::NotParsed);
    };
    let file = File::open(&ctx.abs_path)?;
    let session = sink.session(SessionDraft {
        harness_session_id: path.session_id.clone(),
        kind: if path.parent_session_id.is_some() {
            SessionKind::Subagent
        } else {
            SessionKind::Primary
        },
        parent_harness_session_id: path.parent_session_id.clone(),
        title: None,
        project_path: path.slug.clone(),
        git_branch: None,
        harness_version: None,
        // The format has no timestamps: absence is recorded, the file mtime bounds the session.
        first_ts: Some(Timestamp::absent()),
        last_ts: Some(
            ctx.mtime
                .map(Timestamp::from_mtime)
                .unwrap_or_else(Timestamp::absent),
        ),
    });
    sink.identity_claim(
        session,
        IdentityClaimDraft {
            scheme: "cursor:composerId".into(),
            claimed_id: path.session_id.clone(),
            source_file: ctx.handle,
            join_status: JoinStatus::Unmatched,
        },
    );
    let mut st = State {
        ctx,
        turn: format!("{}:none", path.session_id),
        path,
        session,
        census: FieldCensus::default(),
        blocks: BlockCensus::default(),
        pending: HashMap::new(),
        order: Vec::new(),
    };
    let mut records = 0u64;
    for rec in JsonlReader::new(BufReader::new(file)) {
        let rec = rec?;
        records += 1;
        if rec.oversized {
            st.anomaly(
                sink,
                AnomalyKind::OversizedRecord,
                Severity::Warning,
                None,
                format!(
                    "line {} of {} bytes exceeds the size cap and was skipped",
                    rec.index, rec.length
                ),
                json!({ "length": rec.length, "byte_offset": rec.offset }),
            );
            continue;
        }
        let mut prov = provenance(ctx.handle, &rec);
        prov.parser_name = PARSER_NAME.into();
        prov.parser_version = PARSER_VERSION.into();
        let (text, lossy) = decode_record(&rec.bytes);
        if lossy {
            st.anomaly(
                sink,
                AnomalyKind::InvalidUtf8,
                Severity::Info,
                Some(&prov),
                format!("line {} is not valid UTF-8; decoded lossily", rec.index),
                json!({ "length": rec.length }),
            );
        }
        match serde_json::from_str::<Value>(&text) {
            Ok(v) => st.line(v, prov, sink),
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
                st.anomaly(
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
    for id in std::mem::take(&mut st.order) {
        if let Some(p) = st.pending.remove(&id) {
            st.emit_call(p, None, sink);
        }
    }
    st.census.finish(ctx.handle, Some(session), sink);
    st.blocks.finish(ctx.handle, session, sink);
    Ok(ParseOutcome::Parsed { records })
}
