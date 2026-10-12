//! Sidecar stores of a `.claude` directory: `history.jsonl` (prompt history with pasted content).

use super::transcript::{decode_record, root_file_anomaly, unsafe_component};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use vem_core::adapter::{FileContext, ParseError, ParseOutcome};
use vem_core::hash::sha256_hex;
use vem_core::jsonl::{JsonlReader, DEFAULT_MAX_LEN};
use vem_core::model::*;
use vem_core::sink::ParseSink;

pub const HISTORY_PARSER_NAME: &str = "claude_code.history";
pub const HISTORY_PARSER_VERSION: &str = "1";

fn epoch_ms(v: Option<&Value>) -> Timestamp {
    match v {
        Some(Value::Number(n)) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .and_then(Timestamp::stored_epoch_ms),
        Some(Value::String(s)) => s
            .parse::<i64>()
            .ok()
            .and_then(Timestamp::stored_epoch_ms)
            .or_else(|| Timestamp::stored(s)),
        _ => None,
    }
    .unwrap_or_else(Timestamp::absent)
}

/// Each `history.jsonl` line is one submitted prompt: `display`, `pastedContents`, `timestamp` (ms), `project`, `sessionId`.
/// Pasted content becomes a `paste_detected` observation; a paste stored by `contentHash` is read from
/// `paste-cache/<hash>.txt` through the manifest and retained as a blob. A session id whose transcript is in the manifest
/// but not parsed yet fails the file, so it is retried after the transcript. A session id with no transcript is deletion evidence:
/// it gets a `sidecar_only` session holding one `user` message per history line (so no prompt or paste is
/// lost), and one `missing_transcript` anomaly linked to that session.
pub fn parse_history(
    ctx: &FileContext<'_>,
    sink: &mut dyn ParseSink,
) -> Result<ParseOutcome, ParseError> {
    let file = File::open(&ctx.abs_path)?;
    let mut sidecar: HashMap<String, SessionHandle> = HashMap::new();
    let mut records = 0u64;
    for rec in JsonlReader::new(BufReader::new(file)) {
        let rec = rec?;
        records += 1;
        if rec.oversized {
            sink.anomaly(AnomalyDraft {
                kind: AnomalyKind::OversizedRecord,
                severity: Severity::Warning,
                source_file: Some(ctx.handle),
                session: None,
                byte_offset: Some(rec.offset),
                message: format!(
                    "history.jsonl line {} of {} bytes exceeds the size cap and was skipped",
                    rec.index, rec.length
                ),
                details: json!({ "length": rec.length }),
                provenance: None,
            });
            continue;
        }
        let prov = Provenance {
            source_file: ctx.handle,
            byte_offset: rec.offset,
            byte_length: rec.length,
            record_index: rec.index,
            content_sha256: sha256_hex(&rec.bytes),
            parser_name: HISTORY_PARSER_NAME.to_string(),
            parser_version: HISTORY_PARSER_VERSION.to_string(),
            origin: ProvOrigin::Stored,
        };
        let anomaly = |sink: &mut dyn ParseSink,
                       kind,
                       severity,
                       session: Option<SessionHandle>,
                       message: String,
                       details: Value| {
            sink.anomaly(AnomalyDraft {
                kind,
                severity,
                source_file: Some(ctx.handle),
                session,
                byte_offset: Some(rec.offset),
                message,
                details,
                provenance: Some(prov.clone()),
            })
        };
        let (text, lossy) = decode_record(&rec.bytes);
        if lossy {
            anomaly(
                sink,
                AnomalyKind::InvalidUtf8,
                Severity::Info,
                None,
                format!(
                    "history.jsonl line {} is not valid UTF-8; decoded lossily",
                    rec.index
                ),
                json!({ "length": rec.length }),
            );
        }
        let v: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                let (kind, severity) = if rec.terminated {
                    (AnomalyKind::MalformedRecord, Severity::Error)
                } else {
                    (AnomalyKind::TruncatedLine, Severity::Warning)
                };
                anomaly(
                    sink,
                    kind,
                    severity,
                    None,
                    format!("history.jsonl line {} is not valid JSON: {e}", rec.index),
                    json!({}),
                );
                continue;
            }
        };
        let session_id = v
            .get("sessionId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let timestamp = epoch_ms(v.get("timestamp"));
        let pasted = v
            .get("pastedContents")
            .and_then(Value::as_object)
            .map(|m| !m.is_empty())
            .unwrap_or(false);
        if session_id.is_empty() {
            anomaly(
                sink,
                AnomalyKind::MalformedRecord,
                Severity::Info,
                None,
                format!(
                    "history.jsonl line {} has no sessionId; kept here",
                    rec.index
                ),
                json!({ "record": v }),
            );
            continue;
        }
        let session = match sidecar.get(&session_id).copied() {
            Some(h) => {
                history_message(sink, h, &v, &timestamp, &prov);
                h
            }
            None => match sink.find_session(&session_id) {
                Some(h) => h,
                None => {
                    if sink.find_unparsed_transcript(&session_id).is_some() {
                        // The transcript is in the manifest but has not been parsed (it failed, or comes later);
                        // it is not deletion evidence. Fail so history.jsonl is retried after the transcript.
                        return Err(ParseError::Invalid(format!(
                            "transcript of session {session_id} has not been parsed yet"
                        )));
                    }
                    let h = sink.session(SessionDraft {
                        harness_session_id: session_id.clone(),
                        kind: SessionKind::SidecarOnly,
                        parent_harness_session_id: None,
                        title: None,
                        project_path: v.get("project").and_then(Value::as_str).map(str::to_string),
                        git_branch: None,
                        harness_version: None,
                        first_ts: None,
                        last_ts: None,
                    });
                    sidecar.insert(session_id.clone(), h);
                    anomaly(
                        sink,
                        AnomalyKind::MissingTranscript,
                        Severity::Warning,
                        Some(h),
                        format!("history.jsonl references session {session_id} but no transcript exists in this root"),
                        json!({
                            "sessionId": session_id,
                            "project": v.get("project").cloned().unwrap_or(Value::Null),
                            "display": v.get("display").cloned().unwrap_or(Value::Null),
                            "timestamp": timestamp.value,
                        }),
                    );
                    history_message(sink, h, &v, &timestamp, &prov);
                    h
                }
            },
        };
        if pasted {
            let mut pastes = Vec::new();
            for (key, entry) in v
                .get("pastedContents")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
            {
                let id = entry.get("id").cloned().unwrap_or_else(|| json!(key));
                if let Some(content) = entry.get("content").and_then(Value::as_str) {
                    pastes.push(json!({ "id": id, "inline": true, "size": content.len(), "missing": false }));
                    continue;
                }
                let Some(hash) = entry.get("contentHash").and_then(Value::as_str) else {
                    pastes.push(json!({ "id": id, "inline": false, "missing": true }));
                    continue;
                };
                if let Some(reason) = unsafe_component(hash) {
                    anomaly(
                        sink,
                        AnomalyKind::SuspiciousPath,
                        Severity::Warning,
                        Some(session),
                        format!("paste contentHash {hash:?} {reason}; paste-cache not read"),
                        json!({ "contentHash": hash }),
                    );
                    pastes.push(
                        json!({ "id": id, "content_hash": hash, "inline": false, "missing": true }),
                    );
                    continue;
                }
                let rel = Path::new("paste-cache").join(format!("{hash}.txt"));
                match sink.read_root_file(&rel, DEFAULT_MAX_LEN as u64) {
                    Ok(Some(bytes)) => {
                        let blob = sink.blob(&bytes);
                        pastes.push(json!({ "id": id, "content_hash": hash, "content_blob": blob, "size": bytes.len(), "inline": false, "missing": false }));
                    }
                    Ok(None) => pastes.push(
                        json!({ "id": id, "content_hash": hash, "inline": false, "missing": true }),
                    ),
                    Err(e) => {
                        let (kind, severity, reason) = root_file_anomaly(&e);
                        anomaly(
                            sink,
                            kind,
                            severity,
                            Some(session),
                            format!("paste-cache/{hash}.txt was not read: {reason}"),
                            json!({ "contentHash": hash }),
                        );
                        pastes.push(json!({ "id": id, "content_hash": hash, "inline": false, "missing": true }));
                    }
                }
            }
            sink.observation(
                session,
                ObservationDraft {
                    kind: ObservationKind::PasteDetected,
                    derived_from: Derivation::Record(prov),
                    path: None,
                    command: None,
                    before_blob: None,
                    after_blob: None,
                    timestamp,
                    confidence: Confidence::High,
                    details: json!({
                        "display": v.get("display").cloned().unwrap_or(Value::Null),
                        "pastedContents": v.get("pastedContents").cloned().unwrap_or(Value::Null),
                        "project": v.get("project").cloned().unwrap_or(Value::Null),
                        "pastes": pastes,
                    }),
                },
            );
        }
    }
    Ok(ParseOutcome::Parsed { records })
}

/// One prompt of a `sidecar_only` session. Provenance is the history line; origin is `derived` because
/// the conversation record itself is gone and this message is rebuilt from the prompt history.
fn history_message(
    sink: &mut dyn ParseSink,
    session: SessionHandle,
    v: &Value,
    timestamp: &Timestamp,
    prov: &Provenance,
) {
    let attributes = v.as_object().cloned().unwrap_or_default();
    let blocks = v
        .get("display")
        .and_then(Value::as_str)
        .map(|d| vec![BlockDraft::text(d)])
        .unwrap_or_default();
    sink.message(
        session,
        MessageDraft {
            harness_record_type: "history".to_string(),
            harness_uuid: None,
            parent_uuid: None,
            role: Role::User,
            timestamp: timestamp.clone(),
            model: None,
            attributes,
            blocks,
            provenance: Provenance {
                origin: ProvOrigin::Derived,
                ..prov.clone()
            },
        },
    );
}
