//! Parser for Cursor's global `state.vscdb`, table `cursorDiskKV` (spec §6.3): `composerData:<c>` rows
//! become sessions, `bubbleId:<c>:<b>` rows become messages in the order of the composer's header.
//!
//! Rows are listed by key with their byte length and fetched one value at a time, so no table is read
//! whole and an oversized value is never fetched. Keys taken from evidence are bound parameters.
//! Integrity: a bubble row the header does not list is kept (after the listed ones, `unreferenced_record`),
//! a header entry without a row is `missing_transcript` (Info), and bubbles of a composer that has no
//! `composerData` row form a `sidecar_only` session (`missing_transcript`, Warning).

use super::sqlite::{has_table, open_for_parse, RECORD_CAP};
use super::tools::{categorize, derive_observations};
use super::{record_anomaly, FieldCensus};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OptionalExtension, ToSql};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use vem_core::adapter::{FileContext, ParseError, ParseOutcome};
use vem_core::model::*;
use vem_core::sink::ParseSink;

pub const PARSER_NAME: &str = "cursor-kv";
pub const PARSER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Key prefixes Cursor writes that v0.4.0 counts but does not parse.
const KNOWN_UNPARSED: &[&str] = &[
    "checkpointId",
    "codeBlockDiff",
    "messageRequestContext",
    "inlineDiffs",
    "codeBlockPartialInlineDiffFates",
];

/// Bubble fields this parser knows; any other gets one aggregated `unknown_field`.
const BUBBLE_FIELDS: &[&str] = &[
    "_v",
    "bubbleId",
    "type",
    "createdAt",
    "modelInfo",
    "text",
    "thinking",
    "toolFormerData",
    "images",
    "richText",
    "codeBlocks",
    "tokenCount",
    "isAgentic",
    "capabilityType",
    "context",
    "relevantFiles",
    "suggestedCodeBlocks",
    "attachedCodeChunks",
    "cursorRules",
    "lints",
    "serverBubbleId",
    "usageUuid",
];

/// Bubble fields that become blocks, so they are not repeated in the attributes.
const BLOCK_FIELDS: &[&str] = &["text", "thinking", "toolFormerData"];

const LIST_COMPOSERS: &str = "SELECT key, octet_length(value) FROM cursorDiskKV \
     WHERE key >= 'composerData:' AND key < 'composerData;' ORDER BY key";
const LIST_BUBBLES: &str = "SELECT key, octet_length(value) FROM cursorDiskKV \
     WHERE key >= 'bubbleId:' || ?1 || ':' AND key < 'bubbleId:' || ?1 || ';' ORDER BY key";
const ALL_BUBBLE_KEYS: &str =
    "SELECT key FROM cursorDiskKV WHERE key >= 'bubbleId:' AND key < 'bubbleId;' ORDER BY key";
const FETCH: &str = "SELECT value FROM cursorDiskKV WHERE key = ?1";
const PREFIX_CENSUS: &str = "SELECT substr(key, 1, instr(key, ':') - 1) AS p, count(*) \
     FROM cursorDiskKV GROUP BY p ORDER BY p";

fn sql(e: rusqlite::Error) -> ParseError {
    ParseError::Invalid(format!("sqlite: {e}"))
}

/// A timestamp stored as epoch milliseconds (number or digit string) or as RFC 3339.
pub(crate) fn stored_ts(v: Option<&Value>) -> Option<Timestamp> {
    match v? {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .and_then(Timestamp::stored_epoch_ms),
        Value::String(s) => Timestamp::stored(s)
            .or_else(|| s.parse::<i64>().ok().and_then(Timestamp::stored_epoch_ms)),
        _ => None,
    }
}

fn str_at<'v>(v: &'v Value, path: &[&str]) -> Option<&'v str> {
    path.iter().try_fold(v, |v, k| v.get(k))?.as_str()
}

/// How the bubbles of a composer are ordered.
enum Order {
    /// `fullConversationHeadersOnly[].bubbleId`, with the composer record that lists them.
    Header(Vec<String>, Provenance),
    /// `_v` < 3: the inline `conversation[]` objects are the bubbles, read from the composer's record.
    Inline(Vec<Value>, Provenance),
    /// The composer row is missing, oversized or unreadable: rows in key order, nothing is "unreferenced".
    Unknown,
}

/// Per-composer turn state: `vem_turn = <c>:<ordinal of the last user bubble>`.
struct Turn {
    composer: String,
    ordinal: u64,
    current: String,
    missing_ts: u64,
    first_missing: Option<Provenance>,
}

struct Kv<'a> {
    ctx: &'a FileContext<'a>,
    conn: &'a Connection,
    cap: u64,
    wal_suffix: &'static str,
    records: u64,
    fields: FieldCensus,
}

impl Kv<'_> {
    #[allow(clippy::too_many_arguments)]
    fn anomaly(
        &self,
        sink: &mut dyn ParseSink,
        session: Option<SessionHandle>,
        kind: AnomalyKind,
        severity: Severity,
        prov: Option<&Provenance>,
        message: String,
        details: Value,
    ) {
        sink.anomaly(record_anomaly(
            self.ctx.handle,
            session,
            kind,
            severity,
            prov,
            message,
            details,
        ));
    }

    fn list(&self, query: &str, params: &[&dyn ToSql]) -> Result<Vec<(String, u64)>, ParseError> {
        let mut stmt = self.conn.prepare(query).map_err(sql)?;
        let rows = stmt
            .query_map(params, |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?))
            })
            .map_err(sql)?;
        let mut out = Vec::new();
        for row in rows {
            let (k, len) = row.map_err(sql)?;
            out.push((k, len.unwrap_or(0).max(0) as u64));
        }
        Ok(out)
    }

    /// The bytes of `key` when its value is at most the cap; an oversized value is reported, not fetched.
    fn fetch(
        &mut self,
        key: &str,
        len: u64,
        session: Option<SessionHandle>,
        sink: &mut dyn ParseSink,
    ) -> Result<Option<Vec<u8>>, ParseError> {
        if len > self.cap {
            self.anomaly(
                sink,
                session,
                AnomalyKind::OversizedRecord,
                Severity::Warning,
                None,
                format!(
                    "value of {key} is {len} bytes, over the {} byte cap; skipped",
                    self.cap
                ),
                json!({ "key": key, "length": len }),
            );
            return Ok(None);
        }
        let bytes = self
            .conn
            .query_row(FETCH, [key], |r| {
                Ok(match r.get_ref(0)? {
                    ValueRef::Text(b) | ValueRef::Blob(b) => b.to_vec(),
                    ValueRef::Null => Vec::new(),
                    ValueRef::Integer(i) => i.to_string().into_bytes(),
                    ValueRef::Real(f) => f.to_string().into_bytes(),
                })
            })
            .optional()
            .map_err(sql)?;
        if bytes.is_some() {
            self.records += 1;
        }
        Ok(bytes)
    }

    fn provenance(&self, locator: String, bytes: &[u8], sink: &mut dyn ParseSink) -> Provenance {
        Provenance {
            source_file: self.ctx.handle,
            byte_offset: 0,
            byte_length: bytes.len() as u64,
            record_index: self.records.saturating_sub(1),
            content_sha256: sink.blob(bytes),
            parser_name: PARSER_NAME.into(),
            parser_version: PARSER_VERSION.into(),
            origin: ProvOrigin::Stored,
            locator: Some(format!("{locator}{}", self.wal_suffix)),
        }
    }

    /// The JSON object of a record, or `malformed_record` and `None`.
    fn object(
        &self,
        bytes: &[u8],
        what: &str,
        session: Option<SessionHandle>,
        prov: &Provenance,
        sink: &mut dyn ParseSink,
    ) -> Option<Map<String, Value>> {
        let err = match serde_json::from_slice::<Value>(bytes) {
            Ok(Value::Object(o)) => return Some(o),
            Ok(_) => "is JSON but not an object".to_string(),
            Err(e) => format!("is not JSON: {e}"),
        };
        self.anomaly(
            sink,
            session,
            AnomalyKind::MalformedRecord,
            Severity::Error,
            Some(prov),
            format!("{what} {err}; kept as its blob"),
            json!({ "length": bytes.len() }),
        );
        None
    }

    fn claim(&self, session: SessionHandle, c: &str, sink: &mut dyn ParseSink) {
        sink.identity_claim(
            session,
            IdentityClaimDraft {
                scheme: "cursor:composerId".into(),
                claimed_id: c.to_string(),
                source_file: self.ctx.handle,
                join_status: JoinStatus::Unmatched,
            },
        );
    }

    fn composer(
        &mut self,
        c: &str,
        key: &str,
        len: u64,
        sink: &mut dyn ParseSink,
    ) -> Result<(), ParseError> {
        let bare = |kind| SessionDraft {
            harness_session_id: c.to_string(),
            kind,
            parent_harness_session_id: None,
            title: None,
            project_path: None,
            git_branch: None,
            harness_version: None,
            first_ts: None,
            last_ts: None,
        };
        let Some(bytes) = self.fetch(key, len, None, sink)? else {
            let session = sink.session(bare(SessionKind::Primary));
            self.claim(session, c, sink);
            return self.bubbles(c, session, Order::Unknown, sink);
        };
        let prov = self.provenance(format!("cursorDiskKV:{key}"), &bytes, sink);
        let parsed = self.object(&bytes, &format!("composer {c}"), None, &prov, sink);
        drop(bytes);
        let Some(mut obj) = parsed else {
            let session = sink.session(bare(SessionKind::Primary));
            self.claim(session, c, sink);
            return self.bubbles(c, session, Order::Unknown, sink);
        };
        let v = Value::Object(obj.clone());
        let first_ts = stored_ts(obj.get("createdAt"));
        let session = sink.session(SessionDraft {
            title: str_at(&v, &["name"]).map(str::to_string),
            harness_version: obj.get("_v").filter(|x| !x.is_null()).map(|x| match x {
                Value::String(s) => format!("composer-v{s}"),
                other => format!("composer-v{other}"),
            }),
            first_ts: first_ts.clone(),
            last_ts: stored_ts(obj.get("lastUpdatedAt")),
            ..bare(SessionKind::Primary)
        });
        self.claim(session, c, sink);
        if let Some(m) = str_at(&v, &["modelConfig", "modelName"]) {
            sink.update_session(
                session,
                SessionUpdate {
                    model: Some(m.to_string()),
                    ..Default::default()
                },
            );
        }
        let header = obj.remove("fullConversationHeadersOnly");
        let inline = obj.remove("conversation");
        sink.message(
            session,
            MessageDraft {
                harness_record_type: "composerData".into(),
                harness_uuid: Some(c.to_string()),
                parent_uuid: None,
                role: Role::Meta,
                timestamp: first_ts.unwrap_or_else(Timestamp::absent),
                model: None,
                attributes: obj,
                blocks: Vec::new(),
                provenance: prov.clone(),
            },
        );
        let order = match (header, inline) {
            (Some(Value::Array(h)), _) => Order::Header(
                h.iter()
                    .filter_map(|e| {
                        e.get("bubbleId")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .collect(),
                prov,
            ),
            (_, Some(Value::Array(conv))) => Order::Inline(conv, prov),
            _ => Order::Unknown,
        };
        self.bubbles(c, session, order, sink)
    }

    fn bubbles(
        &mut self,
        c: &str,
        session: SessionHandle,
        order: Order,
        sink: &mut dyn ParseSink,
    ) -> Result<(), ParseError> {
        let mut rows: BTreeMap<String, (String, u64)> = BTreeMap::new();
        let prefix = format!("bubbleId:{c}:");
        for (key, len) in self.list(LIST_BUBBLES, &[&c])? {
            let b = key.strip_prefix(&prefix).unwrap_or(&key).to_string();
            rows.insert(b, (key, len));
        }
        let mut turn = Turn {
            composer: c.to_string(),
            ordinal: 0,
            current: format!("{c}:none"),
            missing_ts: 0,
            first_missing: None,
        };
        let known_order = !matches!(order, Order::Unknown);
        match order {
            Order::Header(ids, header_prov) => {
                let mut seen = HashSet::new();
                for b in ids {
                    if !seen.insert(b.clone()) {
                        continue;
                    }
                    match rows.remove(&b) {
                        Some((key, len)) => {
                            self.bubble_row(session, &b, &key, len, false, &mut turn, sink)?
                        }
                        None => self.anomaly(
                            sink,
                            Some(session),
                            AnomalyKind::MissingTranscript,
                            Severity::Info,
                            Some(&header_prov),
                            format!("composer {c} lists bubble {b}, which has no row"),
                            json!({ "composer": c, "bubble": b }),
                        ),
                    }
                }
            }
            Order::Inline(conv, cprov) => {
                for (i, v) in conv.into_iter().enumerate() {
                    let b = str_at(&v, &["bubbleId"]).unwrap_or_default().to_string();
                    rows.remove(&b);
                    let mut prov = cprov.clone();
                    prov.locator = Some(format!(
                        "cursorDiskKV:composerData:{c}#conversation[{i}]{}",
                        self.wal_suffix
                    ));
                    match v {
                        Value::Object(o) => {
                            self.bubble(session, &b, o, &prov, false, &mut turn, sink)
                        }
                        _ => self.anomaly(
                            sink,
                            Some(session),
                            AnomalyKind::MalformedRecord,
                            Severity::Error,
                            Some(&prov),
                            format!("conversation[{i}] of composer {c} is not an object"),
                            json!({ "composer": c, "index": i }),
                        ),
                    }
                }
            }
            Order::Unknown => {}
        }
        for (b, (key, len)) in rows {
            self.bubble_row(session, &b, &key, len, known_order, &mut turn, sink)?;
        }
        if turn.missing_ts > 0 {
            self.anomaly(
                sink,
                Some(session),
                AnomalyKind::MissingTimestamp,
                Severity::Warning,
                turn.first_missing.as_ref(),
                format!(
                    "{} bubble(s) of composer {c} have no usable createdAt",
                    turn.missing_ts
                ),
                json!({ "composer": c, "count": turn.missing_ts }),
            );
        }
        Ok(())
    }

    /// One `bubbleId:<c>:<b>` row. `unreferenced`: the composer's header does not list it.
    #[allow(clippy::too_many_arguments)]
    fn bubble_row(
        &mut self,
        session: SessionHandle,
        b: &str,
        key: &str,
        len: u64,
        unreferenced: bool,
        turn: &mut Turn,
        sink: &mut dyn ParseSink,
    ) -> Result<(), ParseError> {
        let Some(bytes) = self.fetch(key, len, Some(session), sink)? else {
            return Ok(());
        };
        let prov = self.provenance(format!("cursorDiskKV:{key}"), &bytes, sink);
        let c = turn.composer.clone();
        if unreferenced {
            self.anomaly(
                sink,
                Some(session),
                AnomalyKind::UnreferencedRecord,
                Severity::Info,
                Some(&prov),
                format!("bubble {b} has a row but composer {c} does not list it (deleted or edited conversation part); kept after the listed bubbles"),
                json!({ "composer": c, "bubble": b }),
            );
        }
        if let Some(obj) = self.object(&bytes, &format!("bubble {b}"), Some(session), &prov, sink) {
            self.bubble(session, b, obj, &prov, unreferenced, turn, sink);
        }
        Ok(())
    }

    /// One bubble object: a message, plus a `tool` message and a tool call for `toolFormerData`.
    /// An `unreferenced` bubble never starts a turn: it continues the last listed one.
    #[allow(clippy::too_many_arguments)]
    fn bubble(
        &mut self,
        session: SessionHandle,
        b: &str,
        obj: Map<String, Value>,
        prov: &Provenance,
        unreferenced: bool,
        turn: &mut Turn,
        sink: &mut dyn ParseSink,
    ) {
        let locator = prov.locator.clone().unwrap_or_default();
        for k in obj.keys().filter(|k| !BUBBLE_FIELDS.contains(&k.as_str())) {
            self.fields.note("bubble", k, &locator, prov);
        }
        let v = Value::Object(obj);
        let role = match v.get("type").and_then(Value::as_i64) {
            Some(1) => Role::User,
            Some(2) => Role::Assistant,
            _ => {
                self.anomaly(
                    sink,
                    Some(session),
                    AnomalyKind::UnknownRecordType,
                    Severity::Info,
                    Some(prov),
                    format!(
                        "bubble {b} has type {}; kept as meta",
                        v.get("type").unwrap_or(&Value::Null)
                    ),
                    json!({ "type": v.get("type") }),
                );
                Role::Meta
            }
        };
        let timestamp = stored_ts(v.get("createdAt")).unwrap_or_else(|| {
            turn.missing_ts += 1;
            turn.first_missing.get_or_insert_with(|| prov.clone());
            Timestamp::absent()
        });
        let mut blocks = Vec::new();
        if let Some(t) = v.get("thinking").filter(|t| !t.is_null()) {
            blocks.push(BlockDraft {
                kind: BlockKind::Thinking,
                text: str_at(t, &["text"]).map(str::to_string),
                payload: t.clone(),
                tool_use_id: None,
            });
        }
        if let Some(t) = str_at(&v, &["text"]).filter(|t| !t.is_empty()) {
            blocks.push(BlockDraft::text(t));
        }
        let tool = v.get("toolFormerData").filter(|t| t.is_object()).cloned();
        let tool_ordinal = blocks.len() as u32;
        let mut input = Value::Null;
        if let Some(tf) = &tool {
            input = match tf.get("rawArgs") {
                Some(Value::String(s)) => serde_json::from_str(s).unwrap_or_else(|e| {
                    self.anomaly(
                        sink,
                        Some(session),
                        AnomalyKind::MalformedRecord,
                        Severity::Info,
                        Some(prov),
                        format!("toolFormerData.rawArgs of bubble {b} is not JSON ({e}); kept raw"),
                        json!({ "bubble": b }),
                    );
                    json!({ "raw": s })
                }),
                Some(other) if !other.is_null() => other.clone(),
                _ => tf.get("params").cloned().unwrap_or(Value::Null),
            };
            blocks.push(BlockDraft {
                kind: BlockKind::ToolUse,
                text: None,
                payload: json!({
                    "name": tf.get("name"),
                    "rawArgs": tf.get("rawArgs"),
                    "params": tf.get("params"),
                    "status": tf.get("status"),
                }),
                tool_use_id: str_at(tf, &["toolCallId"]).map(str::to_string),
            });
        }
        if let Some(images) = v.get("images").and_then(Value::as_array) {
            for img in images {
                blocks.push(BlockDraft {
                    kind: BlockKind::Image,
                    text: None,
                    payload: img.clone(),
                    tool_use_id: None,
                });
            }
        }
        if role == Role::User && !unreferenced {
            turn.current = format!("{}:{}", turn.composer, turn.ordinal);
        }
        turn.ordinal += 1;
        let model = str_at(&v, &["modelInfo", "modelName"]).map(str::to_string);
        let obj = match v {
            Value::Object(o) => o,
            _ => Map::new(),
        };
        let mut attributes: Map<String, Value> = obj
            .into_iter()
            .filter(|(k, _)| !BLOCK_FIELDS.contains(&k.as_str()))
            .collect();
        if role == Role::Assistant {
            attributes.insert("vem_turn".into(), Value::String(turn.current.clone()));
        }
        let msg = sink.message(
            session,
            MessageDraft {
                harness_record_type: "bubble".into(),
                harness_uuid: Some(b.to_string()),
                parent_uuid: None,
                role,
                timestamp: timestamp.clone(),
                model,
                attributes,
                blocks,
                provenance: prov.clone(),
            },
        );
        let Some(tf) = tool else { return };
        let name = str_at(&tf, &["name"]).unwrap_or_default().to_string();
        let tool_use_id = str_at(&tf, &["toolCallId"]).map(str::to_string);
        let status = str_at(&tf, &["status"]).map(str::to_string);
        let result = tf.get("result").filter(|r| !r.is_null());
        let mut tool_result = None;
        let mut result_text = None;
        if result.is_some() || status.is_some() {
            let text = result.map(|r| match r {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            });
            let mut attributes = Map::new();
            attributes.insert("vem_turn".into(), Value::String(turn.current.clone()));
            let m = sink.message(
                session,
                MessageDraft {
                    harness_record_type: "bubble.toolFormerData.result".into(),
                    harness_uuid: Some(b.to_string()),
                    parent_uuid: None,
                    role: Role::Tool,
                    timestamp: timestamp.clone(),
                    model: None,
                    attributes,
                    blocks: vec![BlockDraft {
                        kind: BlockKind::ToolResult,
                        text: text.clone(),
                        payload: json!({ "status": status, "result": result }),
                        tool_use_id,
                    }],
                    provenance: prov.clone(),
                },
            );
            tool_result = Some(BlockRef {
                message: m,
                ordinal: 0,
            });
            result_text = text;
        }
        let result_ok = tool_result.map(|_| status.as_deref() == Some("completed"));
        let tc = sink.tool_call(
            session,
            ToolCallDraft {
                category: categorize(&name),
                name: name.clone(),
                input: input.clone(),
                tool_use: BlockRef {
                    message: msg,
                    ordinal: tool_ordinal,
                },
                tool_result,
                result_text,
                result_payload: None,
                is_error: status.as_deref() == Some("error"),
                started: timestamp.clone(),
                ended: if tool_result.is_some() {
                    timestamp.clone()
                } else {
                    Timestamp::absent()
                },
            },
        );
        for obs in derive_observations(&name, &input, result_ok, tc, &timestamp) {
            sink.observation(session, obs);
        }
    }

    /// Composers that have bubble rows but no `composerData` row, streamed over the keys only. Keys
    /// with no `<c>:<b>` shape are reported once.
    fn orphan_composers(
        &self,
        known: &BTreeSet<String>,
        sink: &mut dyn ParseSink,
    ) -> Result<BTreeSet<String>, ParseError> {
        let mut stmt = self.conn.prepare(ALL_BUBBLE_KEYS).map_err(sql)?;
        let mut rows = stmt.query([]).map_err(sql)?;
        let mut orphans = BTreeSet::new();
        let (mut bad, mut first_bad) = (0u64, None::<String>);
        while let Some(r) = rows.next().map_err(sql)? {
            let key: String = r.get(0).map_err(sql)?;
            match key["bubbleId:".len()..].split_once(':') {
                Some((c, _)) if !known.contains(c) => {
                    orphans.insert(c.to_string());
                }
                Some(_) => {}
                None => {
                    bad += 1;
                    first_bad.get_or_insert(key);
                }
            }
        }
        if bad > 0 {
            self.anomaly(
                sink,
                None,
                AnomalyKind::MalformedRecord,
                Severity::Warning,
                None,
                format!("{bad} bubbleId key(s) are not of the form bubbleId:<composer>:<bubble>; not parsed"),
                json!({ "count": bad, "first_key": first_bad }),
            );
        }
        Ok(orphans)
    }

    fn orphan(&mut self, c: &str, sink: &mut dyn ParseSink) -> Result<(), ParseError> {
        let session = sink.session(SessionDraft {
            harness_session_id: c.to_string(),
            kind: SessionKind::SidecarOnly,
            parent_harness_session_id: None,
            title: None,
            project_path: None,
            git_branch: None,
            harness_version: None,
            first_ts: None,
            last_ts: None,
        });
        self.claim(session, c, sink);
        self.anomaly(
            sink,
            Some(session),
            AnomalyKind::MissingTranscript,
            Severity::Warning,
            None,
            format!("bubbles of composer {c} exist but its composerData row does not; kept as a sidecar-only session"),
            json!({ "composer": c }),
        );
        self.bubbles(c, session, Order::Unknown, sink)
    }

    /// One `unknown_record_type` per key prefix this parser does not read.
    fn prefix_census(&self, sink: &mut dyn ParseSink) -> Result<(), ParseError> {
        let mut stmt = self.conn.prepare(PREFIX_CENSUS).map_err(sql)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get::<_, Option<String>>(0)?, r.get::<_, i64>(1)?))
            })
            .map_err(sql)?;
        for row in rows {
            let (p, count) = row.map_err(sql)?;
            let p = p.unwrap_or_default();
            if p == "composerData" || p == "bubbleId" {
                continue;
            }
            let known = KNOWN_UNPARSED.contains(&p.as_str());
            let mut details = json!({ "prefix": p, "count": count, "parsed": false });
            if !known {
                details["known"] = json!(false);
            }
            self.anomaly(
                sink,
                None,
                AnomalyKind::UnknownRecordType,
                Severity::Info,
                None,
                format!(
                    "{count} cursorDiskKV row(s) with key prefix {p:?} are inventoried, not parsed"
                ),
                details,
            );
        }
        Ok(())
    }
}

pub fn parse_state_vscdb(
    ctx: &FileContext<'_>,
    sink: &mut dyn ParseSink,
) -> Result<ParseOutcome, ParseError> {
    parse_state_vscdb_with_cap(ctx, sink, RECORD_CAP)
}

/// [`parse_state_vscdb`] with a different per-record cap (tests).
pub fn parse_state_vscdb_with_cap(
    ctx: &FileContext<'_>,
    sink: &mut dyn ParseSink,
    cap: u64,
) -> Result<ParseOutcome, ParseError> {
    let not_parsed = |sink: &mut dyn ParseSink, why: &str| {
        sink.anomaly(record_anomaly(
            ctx.handle,
            None,
            AnomalyKind::UnknownStoreGeneration,
            Severity::Info,
            None,
            format!("{}: {why}; inventoried, not parsed", ctx.rel_path.display()),
            json!({ "generation": ctx.store.generation }),
        ));
        Ok(ParseOutcome::NotParsed)
    };
    if ctx.store.generation.as_deref() == Some("itemtable-legacy") {
        return not_parsed(
            sink,
            "no cursorDiskKV table (store generation itemtable-legacy)",
        );
    }
    let view = open_for_parse(ctx, sink)?;
    if !has_table(&view.conn, "cursorDiskKV") {
        return not_parsed(sink, "cursorDiskKV is not an ordinary table");
    }
    let mut kv = Kv {
        ctx,
        conn: &view.conn,
        cap,
        wal_suffix: if view.wal_applied { " +wal" } else { "" },
        records: 0,
        fields: FieldCensus::default(),
    };
    let mut known = BTreeSet::new();
    for (key, len) in kv.list(LIST_COMPOSERS, &[])? {
        let c = key["composerData:".len()..].to_string();
        kv.composer(&c, &key, len, sink)?;
        known.insert(c);
    }
    for c in kv.orphan_composers(&known, sink)? {
        kv.orphan(&c, sink)?;
    }
    kv.prefix_census(sink)?;
    kv.fields.finish(ctx.handle, None, sink);
    Ok(ParseOutcome::Parsed {
        records: kv.records,
    })
}
