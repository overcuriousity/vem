//! VS Code/Cursor local history: `User/History/<d>/entries.json` names one edited file (`resource`) and
//! its snapshots (`entries[].id`, files next to it). Each directory is one `local_history` session; each
//! entry is a `meta` record with the snapshot as an attachment and a `file_written` observation from the
//! previous snapshot to this one. History is not joined to composer sessions (no guessed joins).

use super::workspace::file_uri_path;
use super::{record_anomaly, RECORD_CAP};
use crate::claude_code::transcript::{read_capped, root_file_anomaly, unsafe_component};
use serde_json::value::RawValue;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::Path;
use vem_core::adapter::{FileContext, ParseError, ParseOutcome};
use vem_core::hash::sha256_hex;
use vem_core::model::*;
use vem_core::sink::ParseSink;

pub const PARSER_NAME: &str = "cursor-history";
pub const PARSER_VERSION: &str = env!("CARGO_PKG_VERSION");

const ENTRY_KEYS: &[&str] = &["id", "timestamp", "source"];

fn invalid(e: impl std::fmt::Display) -> ParseError {
    ParseError::Invalid(format!("entries.json: {e}"))
}

/// Byte offset of `inner` inside `outer`; `inner` is a sub-slice of it (borrowed `RawValue`).
fn offset_in(outer: &str, inner: &str) -> u64 {
    (inner.as_ptr() as usize).saturating_sub(outer.as_ptr() as usize) as u64
}

pub fn parse_entries(
    ctx: &FileContext<'_>,
    sink: &mut dyn ParseSink,
) -> Result<ParseOutcome, ParseError> {
    if ctx.rel_path.file_name() != Some("entries.json".as_ref()) {
        return Ok(ParseOutcome::NotParsed); // snapshot files are retained, read through entries.json
    }
    let dir = ctx.rel_path.parent().unwrap_or(Path::new(""));
    let d = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let bytes = read_capped(&ctx.abs_path, RECORD_CAP)?
        .ok_or_else(|| invalid("larger than the 64 MiB cap"))?;
    let text = std::str::from_utf8(&bytes).map_err(invalid)?;
    let top: HashMap<String, &RawValue> = serde_json::from_str(text).map_err(invalid)?;
    let resource = top
        .get("resource")
        .and_then(|r| serde_json::from_str::<String>(r.get()).ok())
        .map(|r| file_uri_path(&r));
    let entries: Vec<&RawValue> = match top.get("entries") {
        Some(raw) => serde_json::from_str(raw.get()).map_err(invalid)?,
        None => Vec::new(),
    };
    // A non-object entry is `None`: reported as malformed and skipped.
    let parsed: Vec<(Option<Map<String, Value>>, Provenance)> = entries
        .iter()
        .enumerate()
        .map(|(i, raw)| {
            let span = raw.get();
            let obj = match serde_json::from_str::<Value>(span) {
                Ok(Value::Object(o)) => Some(o),
                _ => None,
            };
            let prov = Provenance {
                source_file: ctx.handle,
                byte_offset: offset_in(text, span),
                byte_length: span.len() as u64,
                record_index: i as u64,
                content_sha256: sha256_hex(span.as_bytes()),
                parser_name: PARSER_NAME.into(),
                parser_version: PARSER_VERSION.into(),
                origin: ProvOrigin::Stored,
                locator: None,
            };
            (obj, prov)
        })
        .collect();
    let ts = |o: &Map<String, Value>| {
        o.get("timestamp")
            .and_then(Value::as_i64)
            .and_then(Timestamp::stored_epoch_ms)
    };
    let session = sink.session(SessionDraft {
        harness_session_id: format!("history:{d}"),
        kind: SessionKind::LocalHistory,
        parent_harness_session_id: None,
        title: resource.clone(),
        project_path: None,
        git_branch: None,
        harness_version: None,
        first_ts: parsed.iter().find_map(|(o, _)| o.as_ref()).and_then(ts),
        last_ts: parsed
            .iter()
            .rev()
            .find_map(|(o, _)| o.as_ref())
            .and_then(ts),
    });
    let anomaly =
        |sink: &mut dyn ParseSink, kind, severity, prov: &Provenance, msg: String, details| {
            sink.anomaly(record_anomaly(
                ctx.handle,
                Some(session),
                kind,
                severity,
                Some(prov),
                msg,
                details,
            ));
        };
    let mut before: Option<String> = None;
    for (obj, prov) in &parsed {
        let Some(obj) = obj else {
            anomaly(
                sink,
                AnomalyKind::MalformedRecord,
                Severity::Error,
                prov,
                format!(
                    "local-history entry {} of {d} is not an object; skipped",
                    prov.record_index
                ),
                json!({ "history": d, "index": prov.record_index }),
            );
            continue;
        };
        let id = obj.get("id").and_then(Value::as_str).unwrap_or_default();
        let source = obj.get("source").cloned().unwrap_or(Value::Null);
        let timestamp = ts(obj).unwrap_or_else(Timestamp::absent);
        let after = match unsafe_component(id) {
            Some(why) => {
                anomaly(
                    sink,
                    AnomalyKind::SuspiciousPath,
                    Severity::Warning,
                    prov,
                    format!("local-history entry id {id:?} {why}; its snapshot was not read"),
                    json!({ "history": d, "id": id }),
                );
                None
            }
            None => match sink.read_root_file(&dir.join(id), RECORD_CAP) {
                Ok(Some(b)) => Some(sink.blob(&b)),
                Ok(None) => {
                    anomaly(
                        sink,
                        AnomalyKind::MissingTranscript,
                        Severity::Info,
                        prov,
                        format!("local-history snapshot {d}/{id} is listed but was not collected"),
                        json!({ "history": d, "id": id }),
                    );
                    None
                }
                Err(e) => {
                    let (kind, severity, msg) = root_file_anomaly(&e);
                    anomaly(
                        sink,
                        kind,
                        severity,
                        prov,
                        format!("local-history snapshot {d}/{id}: {msg}"),
                        json!({ "history": d, "id": id }),
                    );
                    None
                }
            },
        };
        let attributes: Map<String, Value> = obj
            .iter()
            .filter(|(k, _)| !ENTRY_KEYS.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        sink.message(
            session,
            MessageDraft {
                harness_record_type: "local-history-entry".into(),
                harness_uuid: None,
                parent_uuid: None,
                role: Role::Meta,
                timestamp: timestamp.clone(),
                model: None,
                attributes,
                blocks: vec![BlockDraft {
                    kind: BlockKind::Attachment,
                    text: None,
                    payload: json!({ "blob": after, "id": id, "source": source }),
                    tool_use_id: None,
                }],
                provenance: prov.clone(),
            },
        );
        sink.observation(
            session,
            ObservationDraft {
                kind: ObservationKind::FileWritten,
                derived_from: Derivation::Record(prov.clone()),
                path: resource.clone(),
                command: None,
                before_blob: before.clone(),
                after_blob: after.clone(),
                timestamp,
                confidence: Confidence::High,
                details: json!({ "id": id, "source": source }),
            },
        );
        before = after;
    }
    Ok(ParseOutcome::Parsed {
        records: parsed.len() as u64,
    })
}
