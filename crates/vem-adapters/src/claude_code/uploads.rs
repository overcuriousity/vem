//! `uploads/<sessionId>/<name>`: files the user attached to a session (spec §6.1 → `upload_detected`).

use super::transcript::read_capped;
use serde_json::json;
use vem_core::adapter::{FileContext, ParseError, ParseOutcome};
use vem_core::jsonl::DEFAULT_MAX_LEN;
use vem_core::model::*;
use vem_core::sink::ParseSink;

pub const UPLOADS_PARSER_NAME: &str = "claude_code.uploads";
pub const UPLOADS_PARSER_VERSION: &str = "1";

/// MIME type from magic bytes, else from the extension, else `application/octet-stream`.
pub fn sniff_mime(bytes: &[u8], name: &str) -> &'static str {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return "image/png";
    }
    if bytes.starts_with(b"\xff\xd8\xff") {
        return "image/jpeg";
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return "image/gif";
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return "image/webp";
    }
    if bytes.starts_with(b"%PDF-") {
        return "application/pdf";
    }
    match name
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("pdf") => "application/pdf",
        Some("txt" | "log") => "text/plain",
        Some("md") => "text/markdown",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    }
}

/// One `upload_detected` observation per file, on the session named by the directory. An unknown session
/// gets a `sidecar_only` session and a `missing_transcript` anomaly; a transcript that is in the manifest
/// but not parsed yet fails this file so it is retried after the transcript.
pub fn parse_upload(
    ctx: &FileContext<'_>,
    sink: &mut dyn ParseSink,
) -> Result<ParseOutcome, ParseError> {
    let parts: Vec<String> = ctx
        .rel_path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    if parts.len() != 3 || parts[0] != "uploads" {
        return Ok(ParseOutcome::NotParsed);
    }
    let (session_id, name) = (&parts[1], &parts[2]);
    let Some(bytes) = read_capped(&ctx.abs_path, DEFAULT_MAX_LEN as u64)? else {
        sink.anomaly(AnomalyDraft {
            kind: AnomalyKind::OversizedRecord,
            severity: Severity::Warning,
            source_file: Some(ctx.handle),
            session: sink.find_session(session_id),
            byte_offset: Some(0),
            message: format!("upload {name} exceeds the size cap and was not retained as a blob"),
            details: json!({ "sessionId": session_id }),
            provenance: None,
        });
        return Ok(ParseOutcome::Parsed { records: 0 });
    };
    let sha = sink.blob(&bytes);
    let prov = Provenance {
        source_file: ctx.handle,
        byte_offset: 0,
        byte_length: bytes.len() as u64,
        record_index: 0,
        content_sha256: sha.clone(),
        parser_name: UPLOADS_PARSER_NAME.to_string(),
        parser_version: UPLOADS_PARSER_VERSION.to_string(),
        origin: ProvOrigin::Stored,
    };
    let session = match sink.find_session(session_id) {
        Some(h) => h,
        None => {
            if sink.find_unparsed_transcript(session_id).is_some() {
                return Err(ParseError::Invalid(format!(
                    "transcript of session {session_id} has not been parsed yet"
                )));
            }
            let h = sink.session(SessionDraft {
                harness_session_id: session_id.clone(),
                kind: SessionKind::SidecarOnly,
                parent_harness_session_id: None,
                title: None,
                project_path: None,
                git_branch: None,
                harness_version: None,
                first_ts: None,
                last_ts: None,
            });
            sink.anomaly(AnomalyDraft {
                kind: AnomalyKind::MissingTranscript, severity: Severity::Warning, source_file: Some(ctx.handle), session: Some(h), byte_offset: Some(0),
                message: format!("uploads/{session_id}/ holds files for session {session_id} but no transcript exists in this root"),
                details: json!({ "sessionId": session_id, "file": name }), provenance: Some(prov.clone()),
            });
            h
        }
    };
    sink.observation(session, ObservationDraft {
        kind: ObservationKind::UploadDetected,
        derived_from: Derivation::Record(prov),
        path: Some(name.clone()),
        command: None,
        before_blob: None,
        after_blob: None,
        timestamp: ctx.mtime.map(Timestamp::from_mtime).unwrap_or_else(Timestamp::absent),
        confidence: Confidence::High,
        details: json!({ "content_blob": sha, "size": bytes.len(), "mime": sniff_mime(&bytes, name), "sessionId": session_id }),
    });
    Ok(ParseOutcome::Parsed { records: 1 })
}
