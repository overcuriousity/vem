//! Experimental parser for `chats/<workspace>/<agent>/store.db` (spec §6.3): a content-addressed blob
//! store (`blobs(id, data)`) with a hex-encoded JSON header in `meta(key, value)`.
//!
//! Blobs are visited depth-first from `latestRootBlobId` over the 32-byte references found in each blob
//! (`protowire::refs`) whose hex is a blob id; blobs no walk reaches follow in `rowid` order, each with
//! `unreferenced_record`. A blob that is a JSON object with `role` is a message. A tree node (only
//! references and varints) is counted, not emitted. Anything else is a `meta` message with the strings
//! the schemaless walk finds, marked `Derived` and `vem_confidence = low`.
//!
//! Blobs are listed by `rowid` and length and fetched one at a time (an oversized one never), twice: the
//! first pass orders and classifies them, the second emits, so the meta message can carry `tree_nodes`.

use super::kv::stored_ts;
use super::protowire::{extract_strings, is_ref_node, refs, MAX_DEPTH};
use super::sqlite::{has_table, open_for_parse, RECORD_CAP};
use super::tools::{categorize, derive_observations};
use super::{record_anomaly, FieldCensus};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use vem_core::adapter::{FileContext, ParseError, ParseOutcome};
use vem_core::model::*;
use vem_core::sink::ParseSink;

pub const PARSER_NAME: &str = "cursor-chat-store";
pub const PARSER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Message-blob fields this parser knows; any other gets one aggregated `unknown_field`.
const MESSAGE_FIELDS: &[&str] = &["role", "content", "id"];

const LIST_META: &str = "SELECT rowid, key, octet_length(value) FROM meta ORDER BY rowid";
const FETCH_META: &str = "SELECT value FROM meta WHERE rowid = ?1";
const LIST_BLOBS: &str = "SELECT rowid, id, octet_length(data) FROM blobs ORDER BY rowid";
const FETCH_BLOB: &str = "SELECT data FROM blobs WHERE rowid = ?1";

fn sql(e: rusqlite::Error) -> ParseError {
    ParseError::Invalid(format!("sqlite: {e}"))
}

fn bytes_of(v: ValueRef<'_>) -> Vec<u8> {
    match v {
        ValueRef::Text(b) | ValueRef::Blob(b) => b.to_vec(),
        ValueRef::Null => Vec::new(),
        ValueRef::Integer(i) => i.to_string().into_bytes(),
        ValueRef::Real(f) => f.to_string().into_bytes(),
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 || !s.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

struct BlobRow {
    rowid: i64,
    id: String,
    len: u64,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Pending,
    Oversized,
    TreeNode,
    Content,
}

/// Blob indices in emission order, and per blob its kind and whether no walk from the root reached it.
struct Walk {
    order: Vec<usize>,
    kinds: Vec<Kind>,
    unref: Vec<bool>,
}

/// The decoded `meta` header row: its key, raw value, and JSON object.
struct Meta {
    key: String,
    raw: Vec<u8>,
    obj: Map<String, Value>,
}

/// A tool call waiting for its result.
struct Pending {
    message: MessageHandle,
    ordinal: u32,
    name: String,
    input: Value,
}

struct Store<'a> {
    ctx: &'a FileContext<'a>,
    conn: &'a Connection,
    session: SessionHandle,
    session_id: String,
    cap: u64,
    wal_suffix: &'static str,
    records: u64,
    fields: FieldCensus,
    pending: HashMap<String, Pending>,
    order: Vec<String>,
    turn: String,
}

impl Store<'_> {
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

    fn provenance(
        &self,
        locator: String,
        bytes: &[u8],
        origin: ProvOrigin,
        sink: &mut dyn ParseSink,
    ) -> Provenance {
        Provenance {
            source_file: self.ctx.handle,
            byte_offset: 0,
            byte_length: bytes.len() as u64,
            record_index: self.records.saturating_sub(1),
            content_sha256: sink.blob(bytes),
            parser_name: PARSER_NAME.into(),
            parser_version: PARSER_VERSION.into(),
            origin,
            locator: Some(format!("{locator}{}", self.wal_suffix)),
        }
    }

    fn fetch_blob(&self, row: &BlobRow) -> Result<Option<Vec<u8>>, ParseError> {
        self.conn
            .query_row(FETCH_BLOB, [row.rowid], |r| Ok(bytes_of(r.get_ref(0)?)))
            .optional()
            .map_err(sql)
    }

    fn oversized(&self, row: &BlobRow, sink: &mut dyn ParseSink) {
        self.anomaly(
            sink,
            AnomalyKind::OversizedRecord,
            Severity::Warning,
            None,
            format!(
                "blob {} is {} bytes, over the {} byte cap; skipped",
                row.id, row.len, self.cap
            ),
            json!({ "blob": row.id, "length": row.len }),
        );
    }

    /// The emission order of `rows` (indices) and the kind of each: depth-first from `root` over
    /// references that name a blob, then every unvisited blob in `rowid` order (flagged in `unref`).
    fn order(
        &self,
        rows: &[BlobRow],
        root: Option<usize>,
        sink: &mut dyn ParseSink,
    ) -> Result<Walk, ParseError> {
        let by_id: HashMap<String, usize> = rows
            .iter()
            .enumerate()
            .map(|(i, r)| (r.id.to_ascii_lowercase(), i))
            .collect();
        let mut kinds = vec![Kind::Pending; rows.len()];
        let mut unref = vec![false; rows.len()];
        let mut order = Vec::with_capacity(rows.len());
        let mut visited = vec![false; rows.len()];
        let mut classify = |i: usize, sink: &mut dyn ParseSink| -> Result<Vec<usize>, ParseError> {
            if rows[i].len > self.cap {
                self.oversized(&rows[i], sink);
                kinds[i] = Kind::Oversized;
                return Ok(Vec::new());
            }
            let bytes = self.fetch_blob(&rows[i])?.unwrap_or_default();
            kinds[i] = if is_ref_node(&bytes) {
                Kind::TreeNode
            } else {
                Kind::Content
            };
            Ok(refs(&bytes)
                .iter()
                .filter_map(|r| by_id.get(&hex(r)).copied())
                .collect())
        };
        let mut stack: Vec<usize> = root.into_iter().collect();
        while let Some(i) = stack.pop() {
            if std::mem::replace(&mut visited[i], true) {
                continue;
            }
            order.push(i);
            let children = classify(i, sink)?;
            stack.extend(children.into_iter().rev().filter(|c| !visited[*c]));
        }
        for i in 0..rows.len() {
            if !visited[i] {
                unref[i] = root.is_some();
                order.push(i);
                classify(i, sink)?;
            }
        }
        Ok(Walk {
            order,
            kinds,
            unref,
        })
    }

    fn emit_call(
        &self,
        p: Pending,
        result: Option<(BlockRef, Option<String>)>,
        sink: &mut dyn ParseSink,
    ) {
        let (tool_result, result_text) = match result {
            Some((r, t)) => (Some(r), t),
            None => (None, None),
        };
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
                result_payload: None,
                is_error: false,
                started: Timestamp::absent(),
                ended: Timestamp::absent(),
            },
        );
        let result_ok = tool_result.map(|_| true);
        for obs in derive_observations(&p.name, &p.input, result_ok, handle, &Timestamp::absent()) {
            sink.observation(self.session, obs);
        }
    }

    /// Records tool calls of `blocks` (in `msg`) and pairs tool results with them by `toolCallId`.
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
                    let p = Pending {
                        message: msg,
                        ordinal: ordinal as u32,
                        name: b.payload["name"].as_str().unwrap_or_default().to_string(),
                        input: b.payload.get("input").cloned().unwrap_or(Value::Null),
                    };
                    // A repeated id: the earlier call is kept without a result.
                    if let Some(old) = self.pending.insert(id.clone(), p) {
                        self.emit_call(old, None, sink);
                    }
                    self.order.retain(|x| *x != id);
                    self.order.push(id);
                }
                BlockKind::ToolResult => match self.pending.remove(&id) {
                    Some(p) => {
                        self.order.retain(|x| *x != id);
                        let r = BlockRef {
                            message: msg,
                            ordinal: ordinal as u32,
                        };
                        self.emit_call(p, Some((r, b.text.clone())), sink);
                    }
                    None => self.anomaly(
                        sink,
                        AnomalyKind::UnpairedToolResult,
                        Severity::Warning,
                        Some(prov),
                        format!("tool-result {id} has no tool-call in this store"),
                        json!({ "tool_call_id": id }),
                    ),
                },
                _ => {}
            }
        }
    }

    /// The blocks of a message's `content` (a string, or an array of typed parts), split into the
    /// message's own blocks and its tool results.
    fn blocks(content: Option<Value>) -> (Vec<BlockDraft>, Vec<BlockDraft>) {
        let parts = match content {
            Some(Value::String(s)) => return (vec![BlockDraft::text(&s)], Vec::new()),
            Some(Value::Array(a)) => a,
            None | Some(Value::Null) => return (Vec::new(), Vec::new()),
            Some(other) => vec![other],
        };
        let (mut own, mut results) = (Vec::new(), Vec::new());
        for p in parts {
            let ty = p.get("type").and_then(Value::as_str).unwrap_or_default();
            let text = || p.get("text").and_then(Value::as_str).map(str::to_string);
            let call_id = || {
                p.get("toolCallId")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            };
            match ty {
                "text" => own.push(BlockDraft {
                    kind: BlockKind::Text,
                    text: text(),
                    payload: Value::Null,
                    tool_use_id: None,
                }),
                "reasoning" => own.push(BlockDraft {
                    kind: BlockKind::Thinking,
                    text: text(),
                    payload: Value::Null,
                    tool_use_id: None,
                }),
                "tool-call" => own.push(BlockDraft {
                    kind: BlockKind::ToolUse,
                    text: None,
                    payload: json!({ "name": p.get("toolName"), "input": p.get("args") }),
                    tool_use_id: call_id(),
                }),
                "tool-result" => results.push(BlockDraft {
                    kind: BlockKind::ToolResult,
                    text: p.get("result").filter(|r| !r.is_null()).map(|r| match r {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    }),
                    payload: p.clone(),
                    tool_use_id: call_id(),
                }),
                _ => own.push(BlockDraft {
                    kind: BlockKind::Other,
                    text: None,
                    payload: p.clone(),
                    tool_use_id: None,
                }),
            }
        }
        (own, results)
    }

    /// A JSON object with `role`: a message, plus a `tool` message for tool results of another role.
    fn json_message(
        &mut self,
        blob_id: &str,
        mut obj: Map<String, Value>,
        prov: &Provenance,
        sink: &mut dyn ParseSink,
    ) {
        let locator = prov.locator.clone().unwrap_or_default();
        for k in obj.keys().filter(|k| !MESSAGE_FIELDS.contains(&k.as_str())) {
            self.fields.note("message", k, &locator, prov);
        }
        let role_raw = obj.remove("role").unwrap_or(Value::Null);
        let role_str = role_raw.as_str().unwrap_or_default();
        let role = match role_str {
            "user" => Role::User,
            "assistant" => Role::Assistant,
            "system" => Role::System,
            "tool" => Role::Tool,
            _ => {
                self.anomaly(
                    sink,
                    AnomalyKind::UnknownRecordType,
                    Severity::Info,
                    Some(prov),
                    format!("blob {blob_id} has role {role_raw}; kept as meta"),
                    json!({ "role": role_raw }),
                );
                obj.insert("role".into(), role_raw.clone());
                Role::Meta
            }
        };
        let (mut blocks, results) = Self::blocks(obj.remove("content"));
        let results = if role == Role::Tool {
            blocks.extend(results);
            Vec::new()
        } else {
            results
        };
        // A user message made only of tool results continues the turn; any other user message starts one.
        let only_results =
            !blocks.is_empty() && blocks.iter().all(|b| b.kind == BlockKind::ToolResult);
        if role == Role::User && !only_results {
            self.turn = format!("{}:{}", self.session_id, prov.record_index);
        } else if role != Role::Meta {
            obj.insert("vem_turn".into(), Value::String(self.turn.clone()));
        }
        let msg = sink.message(
            self.session,
            MessageDraft {
                harness_record_type: "blob".into(),
                harness_uuid: Some(blob_id.to_string()),
                parent_uuid: None,
                role,
                timestamp: Timestamp::absent(),
                model: None,
                attributes: obj,
                blocks: blocks.clone(),
                provenance: prov.clone(),
            },
        );
        self.pair(msg, &blocks, prov, sink);
        if results.is_empty() {
            return;
        }
        let mut attributes = Map::new();
        attributes.insert("vem_turn".into(), Value::String(self.turn.clone()));
        let msg = sink.message(
            self.session,
            MessageDraft {
                harness_record_type: "blob.tool-result".into(),
                harness_uuid: Some(blob_id.to_string()),
                parent_uuid: None,
                role: Role::Tool,
                timestamp: Timestamp::absent(),
                model: None,
                attributes,
                blocks: results.clone(),
                provenance: prov.clone(),
            },
        );
        self.pair(msg, &results, prov, sink);
    }

    /// One content blob: a JSON message, other JSON as a `meta` record, or derived protobuf text.
    fn blob(&mut self, row: &BlobRow, bytes: &[u8], unref: bool, sink: &mut dyn ParseSink) {
        let locator = format!("blobs:{}", row.id);
        let json = serde_json::from_slice::<Value>(bytes).ok();
        let origin = if json.is_some() {
            ProvOrigin::Stored
        } else {
            ProvOrigin::Derived
        };
        let prov = self.provenance(locator, bytes, origin, sink);
        if unref {
            self.anomaly(
                sink,
                AnomalyKind::UnreferencedRecord,
                Severity::Info,
                Some(&prov),
                format!("blob {} is not reachable from latestRootBlobId (deleted or superseded part); kept after the reachable blobs", row.id),
                json!({ "blob": row.id }),
            );
        }
        match json {
            Some(Value::Object(o)) if o.contains_key("role") => {
                self.json_message(&row.id, o, &prov, sink)
            }
            Some(other) => {
                self.anomaly(
                    sink,
                    AnomalyKind::UnknownRecordType,
                    Severity::Info,
                    Some(&prov),
                    format!("blob {} is JSON without a role; kept as meta", row.id),
                    json!({ "blob": row.id }),
                );
                sink.message(
                    self.session,
                    MessageDraft {
                        harness_record_type: "json".into(),
                        harness_uuid: Some(row.id.clone()),
                        parent_uuid: None,
                        role: Role::Meta,
                        timestamp: Timestamp::absent(),
                        model: None,
                        attributes: Map::new(),
                        blocks: vec![BlockDraft {
                            kind: BlockKind::Other,
                            text: None,
                            payload: other,
                            tool_use_id: None,
                        }],
                        provenance: prov,
                    },
                );
            }
            None => {
                let strings = extract_strings(bytes, MAX_DEPTH);
                let block = if strings.is_empty() {
                    BlockDraft {
                        kind: BlockKind::Other,
                        text: None,
                        payload: json!({ "bytes": bytes.len() }),
                        tool_use_id: None,
                    }
                } else {
                    BlockDraft::text(&strings.join("\n"))
                };
                let mut attributes = Map::new();
                attributes.insert("vem_confidence".into(), json!("low"));
                attributes.insert("harness_record_type".into(), json!("protobuf"));
                sink.message(
                    self.session,
                    MessageDraft {
                        harness_record_type: "protobuf".into(),
                        harness_uuid: Some(row.id.clone()),
                        parent_uuid: None,
                        role: Role::Meta,
                        timestamp: Timestamp::absent(),
                        model: None,
                        attributes,
                        blocks: vec![block],
                        provenance: prov,
                    },
                );
            }
        }
    }
}

/// The decoded `meta` header: the first row that is hex of a JSON object, with its provenance bytes.
/// Every row that is not is reported as `malformed_record`.
fn read_meta(
    ctx: &FileContext<'_>,
    conn: &Connection,
    cap: u64,
    sink: &mut dyn ParseSink,
) -> Result<Option<Meta>, ParseError> {
    let mut stmt = conn.prepare(LIST_META).map_err(sql)?;
    let keys = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                String::from_utf8_lossy(&bytes_of(r.get_ref(1)?)).into_owned(),
                r.get::<_, Option<i64>>(2)?.unwrap_or(0).max(0) as u64,
            ))
        })
        .map_err(sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql)?;
    let mut found = None;
    for (rowid, key, len) in keys {
        let file_anomaly = |sink: &mut dyn ParseSink, kind, severity, message: String| {
            sink.anomaly(record_anomaly(
                ctx.handle,
                None,
                kind,
                severity,
                None,
                message,
                json!({ "key": key, "length": len }),
            ))
        };
        if len > cap {
            file_anomaly(
                sink,
                AnomalyKind::OversizedRecord,
                Severity::Warning,
                format!("meta row {key} is {len} bytes, over the {cap} byte cap; skipped"),
            );
            continue;
        }
        let Some(raw) = conn
            .query_row(FETCH_META, [rowid], |r| Ok(bytes_of(r.get_ref(0)?)))
            .optional()
            .map_err(sql)?
        else {
            continue;
        };
        let decoded = std::str::from_utf8(&raw).ok().and_then(unhex);
        let err = match decoded.as_deref().map(serde_json::from_slice::<Value>) {
            Some(Ok(Value::Object(o))) => {
                if found.is_none() {
                    found = Some(Meta {
                        key: key.clone(),
                        raw,
                        obj: o,
                    });
                }
                continue;
            }
            Some(Ok(_)) => "is hex of JSON that is not an object",
            Some(Err(_)) => "is hex of something that is not JSON",
            None => "is not hex",
        };
        file_anomaly(
            sink,
            AnomalyKind::MalformedRecord,
            Severity::Error,
            format!("meta row {key} {err}; the session falls back to the directory name"),
        );
    }
    Ok(found)
}

pub fn parse_store_db(
    ctx: &FileContext<'_>,
    sink: &mut dyn ParseSink,
) -> Result<ParseOutcome, ParseError> {
    parse_store_db_with_cap(ctx, sink, RECORD_CAP)
}

/// [`parse_store_db`] with a different per-record cap (tests).
pub fn parse_store_db_with_cap(
    ctx: &FileContext<'_>,
    sink: &mut dyn ParseSink,
    cap: u64,
) -> Result<ParseOutcome, ParseError> {
    let view = open_for_parse(ctx, sink)?;
    for table in ["meta", "blobs"] {
        if !has_table(&view.conn, table) {
            sink.anomaly(record_anomaly(
                ctx.handle,
                None,
                AnomalyKind::UnknownStoreGeneration,
                Severity::Info,
                None,
                format!(
                    "{}: {table} is not an ordinary table; inventoried, not parsed",
                    ctx.rel_path.display()
                ),
                json!({ "generation": ctx.store.generation, "missing": table }),
            ));
            return Ok(ParseOutcome::NotParsed);
        }
    }
    let conn = &view.conn;
    let meta = read_meta(ctx, conn, cap, sink)?;
    let dir = ctx
        .rel_path
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let field = |k: &str| {
        meta.as_ref()
            .and_then(|m| m.obj.get(k))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let agent = field("agentId");
    let session_id = agent.clone().unwrap_or_else(|| dir.clone());
    let session = sink.session(SessionDraft {
        harness_session_id: session_id.clone(),
        kind: SessionKind::Primary,
        parent_harness_session_id: None,
        title: field("name"),
        project_path: None,
        git_branch: None,
        harness_version: None,
        first_ts: stored_ts(meta.as_ref().and_then(|m| m.obj.get("createdAt"))),
        last_ts: ctx.mtime.map(Timestamp::from_mtime),
    });
    if let Some(model) = field("lastUsedModel") {
        sink.update_session(
            session,
            SessionUpdate {
                model: Some(model),
                ..Default::default()
            },
        );
    }
    let claim = |scheme: &str, id: &str, sink: &mut dyn ParseSink| {
        sink.identity_claim(
            session,
            IdentityClaimDraft {
                scheme: scheme.into(),
                claimed_id: id.to_string(),
                source_file: ctx.handle,
                join_status: JoinStatus::Unmatched,
            },
        )
    };
    if let Some(a) = &agent {
        claim("cursor:composerId", a, sink);
    }
    if agent.as_deref() != Some(dir.as_str()) && !dir.is_empty() {
        claim("cursor:store-session", &dir, sink);
    }

    let mut st = Store {
        ctx,
        conn,
        session,
        session_id,
        cap,
        wal_suffix: if view.wal_applied { " +wal" } else { "" },
        records: 0,
        fields: FieldCensus::default(),
        pending: HashMap::new(),
        order: Vec::new(),
        turn: String::new(),
    };
    st.turn = format!("{}:none", st.session_id);
    let rows: Vec<BlobRow> = {
        let mut stmt = conn.prepare(LIST_BLOBS).map_err(sql)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(BlobRow {
                    rowid: r.get(0)?,
                    id: String::from_utf8_lossy(&bytes_of(r.get_ref(1)?)).into_owned(),
                    len: r.get::<_, Option<i64>>(2)?.unwrap_or(0).max(0) as u64,
                })
            })
            .map_err(sql)?;
        rows.collect::<Result<_, _>>().map_err(sql)?
    };
    let root_id = field("latestRootBlobId");
    let root = root_id
        .as_deref()
        .and_then(|r| rows.iter().position(|b| b.id.eq_ignore_ascii_case(r)));
    if let (Some(r), None) = (&root_id, root) {
        st.anomaly(
            sink,
            AnomalyKind::MissingTranscript,
            Severity::Info,
            None,
            format!("latestRootBlobId {r} names no blob; blobs follow in rowid order"),
            json!({ "latestRootBlobId": r }),
        );
    }
    let Walk {
        order,
        kinds,
        unref,
    } = st.order(&rows, root, sink)?;

    if let Some(Meta { key, raw, mut obj }) = meta {
        st.records += 1;
        let tree_nodes = kinds.iter().filter(|k| **k == Kind::TreeNode).count();
        obj.insert("tree_nodes".into(), json!(tree_nodes));
        let prov = st.provenance(format!("meta:{key}"), &raw, ProvOrigin::Stored, sink);
        sink.message(
            session,
            MessageDraft {
                harness_record_type: "meta".into(),
                harness_uuid: Some(key),
                parent_uuid: None,
                role: Role::Meta,
                timestamp: stored_ts(obj.get("createdAt")).unwrap_or_else(Timestamp::absent),
                model: None,
                attributes: obj,
                blocks: Vec::new(),
                provenance: prov,
            },
        );
    }
    for i in order {
        let row = &rows[i];
        if kinds[i] == Kind::Oversized {
            continue;
        }
        if kinds[i] == Kind::TreeNode && !unref[i] {
            st.records += 1;
            continue;
        }
        let Some(bytes) = st.fetch_blob(row)? else {
            continue;
        };
        st.records += 1;
        if kinds[i] == Kind::TreeNode {
            // Unreferenced tree node: no message, but the anomaly cites its bytes.
            let prov = st.provenance(
                format!("blobs:{}", row.id),
                &bytes,
                ProvOrigin::Stored,
                sink,
            );
            st.anomaly(
                sink,
                AnomalyKind::UnreferencedRecord,
                Severity::Info,
                Some(&prov),
                format!(
                    "tree node {} is not reachable from latestRootBlobId",
                    row.id
                ),
                json!({ "blob": row.id }),
            );
            continue;
        }
        st.blob(row, &bytes, unref[i], sink);
    }
    for id in std::mem::take(&mut st.order) {
        if let Some(p) = st.pending.remove(&id) {
            st.emit_call(p, None, sink);
        }
    }
    st.fields.finish(ctx.handle, Some(session), sink);
    Ok(ParseOutcome::Parsed {
        records: st.records,
    })
}
