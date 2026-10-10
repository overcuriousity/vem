//! One function per subcommand. Each prints text or, with `--json`, the report as JSON.

use crate::ExportFormat;
use serde::Serialize;
use std::path::Path;
use vem_case::evidence::{attach, AttachOptions};
use vem_case::export::{events, Scope};
use vem_case::query;
use vem_case::{Case, CaseError};
use vem_core::model::Harness;

fn emit<T: Serialize>(
    json: bool,
    value: &T,
    text: impl FnOnce(&T) -> String,
) -> Result<(), CaseError> {
    if json {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        println!("{}", text(value));
    }
    Ok(())
}

pub fn case_new(
    dir: &Path,
    name: &str,
    examiner: Option<&str>,
    json: bool,
) -> Result<(), CaseError> {
    let case = Case::create(dir, name, examiner)?;
    let info = case.info()?;
    emit(json, &info, |i| {
        format!(
            "created case {:?} at {} (examiner: {}, vem {})",
            i.name,
            dir.display(),
            i.examiner.as_deref().unwrap_or("-"),
            i.tool_version
        )
    })
}

#[allow(clippy::too_many_arguments)]
pub fn evidence_add(
    case_dir: &Path,
    path: &Path,
    label: String,
    host: Option<String>,
    user: Option<String>,
    os: Option<String>,
    harness: Option<Harness>,
    retain: bool,
    json: bool,
) -> Result<(), CaseError> {
    let mut case = Case::open(case_dir)?;
    let report = attach(
        &mut case,
        path,
        AttachOptions {
            label,
            host,
            user,
            os,
            harness,
            retain,
        },
    )?;
    emit(json, &report, |r| {
        let mut s = format!("attached root {} as {} ({} files, {} bytes, {} unclaimed, retained: {})\n  evidence: {}\n", r.root_id, r.harness, r.file_count, r.total_bytes, r.unclaimed_files, retain, r.evidence.join("; "));
        for st in &r.stores {
            s.push_str(&format!(
                "  store {} {} ({} files){}\n",
                st.id,
                st.kind,
                st.file_count,
                st.generation
                    .as_ref()
                    .map(|g| format!(", generation {g}"))
                    .unwrap_or_default()
            ));
        }
        if !r.absent.is_empty() {
            s.push_str(&format!("  absent: {}\n", r.absent.join(", ")));
        }
        if !r.symlinks.is_empty() {
            s.push_str(&format!(
                "  symbolic links (recorded, not followed): {}\n",
                r.symlinks.join(", ")
            ));
        }
        if !r.unreadable.is_empty() {
            s.push_str(&format!("  unreadable: {}\n", r.unreadable.join(", ")));
        }
        s
    })
}

pub fn evidence_list(case_dir: &Path, json: bool) -> Result<(), CaseError> {
    let case = Case::open(case_dir)?;
    let roots = query::roots(&case)?;
    emit(json, &roots, |rs| {
        rs.iter()
            .map(|r| {
                format!(
                    "{}  {}  {}  {}  host={} user={} os={}",
                    r.id,
                    r.harness,
                    r.label,
                    r.path,
                    r.host.as_deref().unwrap_or("-"),
                    r.user.as_deref().unwrap_or("-"),
                    r.os.as_deref().unwrap_or("-")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    })
}

pub fn ingest(case_dir: &Path, root: Option<i64>, json: bool) -> Result<(), CaseError> {
    let mut case = Case::open(case_dir)?;
    let report = vem_case::ingest::ingest(&mut case, root)?;
    emit(json, &report, |r| {
        format!(
            "ingest done\n  files_parsed: {}\n  files_inventoried: {}\n  files_failed: {}\n  files_skipped: {}\n  files_drifted: {}\n  sessions: {}\n  messages: {}\n  tool_calls: {}\n  observations: {}\n  anomalies: {}",
            r.files_parsed, r.files_inventoried, r.files_failed, r.files_skipped, r.files_drifted, r.sessions, r.messages, r.tool_calls, r.observations, r.anomalies
        )
    })
}

pub fn verify(case_dir: &Path, json: bool) -> Result<(), CaseError> {
    let mut case = Case::open(case_dir)?;
    let report = vem_case::verify::verify(&mut case)?;
    emit(json, &report, |r| {
        format!(
            "verify done\n  files_checked: {}\n  drifted: {}{}\n  missing: {}{}\n  roots_unavailable: {}\n  blobs_checked: {}\n  blob_errors: {}",
            r.files_checked,
            r.drifted.len(),
            if r.drifted.is_empty() { String::new() } else { format!(" ({})", r.drifted.join(", ")) },
            r.missing.len(),
            if r.missing.is_empty() { String::new() } else { format!(" ({})", r.missing.join(", ")) },
            r.roots_unavailable.len(),
            r.blobs_checked,
            r.blob_errors.len()
        )
    })
}

#[derive(Serialize)]
struct Inventory {
    case: vem_case::CaseInfo,
    roots: Vec<RootInventory>,
    anomalies: Vec<query::AnomalyRow>,
    audit_log: Vec<AuditRow>,
}

#[derive(Serialize)]
struct RootInventory {
    root: query::RootRow,
    stores: Vec<query::StoreRow>,
    absent: Vec<String>,
    files: Vec<query::SourceFileRow>,
}

#[derive(Serialize)]
struct AuditRow {
    id: i64,
    ts: String,
    action: String,
    target: Option<String>,
    details: serde_json::Value,
}

fn audit_rows(case: &Case) -> Result<Vec<AuditRow>, CaseError> {
    let mut stmt = case
        .conn
        .prepare("SELECT id, ts, action, target, details FROM audit_log ORDER BY id")?;
    let rows = stmt.query_map([], |r| {
        let d: String = r.get(4)?;
        Ok(AuditRow {
            id: r.get(0)?,
            ts: r.get(1)?,
            action: r.get(2)?,
            target: r.get(3)?,
            details: serde_json::from_str(&d).unwrap_or(serde_json::Value::Null),
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn inventory(case_dir: &Path, json: bool) -> Result<(), CaseError> {
    let case = Case::open(case_dir)?;
    let mut roots = Vec::new();
    for root in query::roots(&case)? {
        roots.push(RootInventory {
            stores: query::stores(&case, root.id)?,
            absent: query::absent_stores(&case, root.id)?,
            files: query::source_files(&case, root.id)?,
            root,
        });
    }
    let inv = Inventory {
        case: case.info()?,
        roots,
        anomalies: query::anomalies(&case, &query::AnomalyFilter::default())?,
        audit_log: audit_rows(&case)?,
    };
    emit(json, &inv, |inv| {
        let mut s = format!("case {:?} (vem {})\n", inv.case.name, inv.case.tool_version);
        for r in &inv.roots {
            s.push_str(&format!(
                "root {} {} {} ({})\n",
                r.root.id, r.root.harness, r.root.label, r.root.path
            ));
            for st in &r.stores {
                s.push_str(&format!(
                    "  store {} {} {} [{}] files={}\n",
                    st.id, st.kind, st.rel_path, st.status, st.file_count
                ));
            }
            if !r.absent.is_empty() {
                s.push_str(&format!("  absent stores: {}\n", r.absent.join(", ")));
            }
            for f in &r.files {
                let link = f
                    .link_target
                    .as_ref()
                    .map(|t| format!(" symlink -> {t} (not followed)"))
                    .unwrap_or_default();
                let version = if f.version > 1 {
                    format!(" v{}", f.version)
                } else {
                    String::new()
                };
                s.push_str(&format!(
                    "  file {} {}{}{} {} bytes sha256={} {}{}\n",
                    f.id,
                    f.rel_path,
                    version,
                    link,
                    f.size,
                    &f.sha256[..12],
                    f.parse_status,
                    f.parse_error
                        .as_ref()
                        .map(|e| format!(" ({e})"))
                        .unwrap_or_default()
                ));
            }
        }
        s.push_str(&format!("anomalies ({}):\n", inv.anomalies.len()));
        for a in &inv.anomalies {
            s.push_str(&format!(
                "  [{}] {} {}{}\n",
                a.severity,
                a.kind,
                a.message,
                a.byte_offset.map(|o| format!(" @{o}")).unwrap_or_default()
            ));
        }
        s.push_str(&format!("audit log ({} entries)\n", inv.audit_log.len()));
        s
    })
}

pub fn sessions(
    case_dir: &Path,
    root: Option<i64>,
    kind: Option<String>,
    json: bool,
) -> Result<(), CaseError> {
    let case = Case::open(case_dir)?;
    let rows = query::sessions(
        &case,
        &query::SessionFilter {
            root_id: root,
            kind,
            ..Default::default()
        },
    )?;
    emit(json, &rows, |rows| {
        rows.iter()
            .map(|s| {
                format!(
                    "{}  {}  {}  {}  {}..{}  msgs={} tools={} anomalies={} children={}  {}",
                    s.id,
                    s.harness,
                    s.kind,
                    s.harness_session_id,
                    s.first_ts.as_deref().unwrap_or("-"),
                    s.last_ts.as_deref().unwrap_or("-"),
                    s.message_count,
                    s.tool_call_count,
                    s.anomaly_count,
                    s.child_count,
                    s.title.as_deref().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    })
}

#[derive(Serialize)]
struct ExportReport {
    format: String,
    output: String,
    events: usize,
    sha256: String,
}

pub fn export(
    case_dir: &Path,
    format: ExportFormat,
    root: Option<i64>,
    session: Option<i64>,
    output: &Path,
    json: bool,
) -> Result<(), CaseError> {
    let case = Case::open(case_dir)?;
    let scope = match (root, session) {
        (_, Some(s)) => Scope::Session(s),
        (Some(r), None) => Scope::Root(r),
        (None, None) => Scope::Case,
    };
    vem_case::export::check_scope(&case, &scope)?;
    vem_case::export::check_output_path(&case, output)?;
    let ev = events(&case, &scope)?;
    if ev.is_empty() {
        return Err(CaseError::Export(
            "nothing to export in this scope".to_string(),
        ));
    }
    let format_name = match format {
        ExportFormat::TimesketchJsonl => {
            let f = std::fs::File::create(output)?;
            vem_case::export::timesketch::write_jsonl(&ev, std::io::BufWriter::new(f))?;
            "timesketch-jsonl"
        }
        ExportFormat::TimesketchCsv => {
            let f = std::fs::File::create(output)?;
            vem_case::export::timesketch::write_csv(&ev, std::io::BufWriter::new(f))?;
            "timesketch-csv"
        }
        ExportFormat::VestigoParquet => {
            vem_case::export::parquet::write_parquet(&ev, output)?;
            "vestigo-parquet"
        }
    };
    let (sha256, _) = vem_core::hash::sha256_file(output)?;
    let report = ExportReport {
        format: format_name.to_string(),
        output: output.display().to_string(),
        events: ev.len(),
        sha256: sha256.clone(),
    };
    case.audit(
        "export",
        Some(&report.output),
        serde_json::to_value(&report)?,
    )?;
    emit(json, &report, |r| {
        format!(
            "exported {} events as {} to {}\n  sha256: {}",
            r.events, r.format, r.output, r.sha256
        )
    })
}
