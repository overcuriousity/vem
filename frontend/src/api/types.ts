// The API contract. Rust structs in vem-case::query / vem-case::annotations / vem-case::diff /
// vem-case::export and vem-web::routes serialize to exactly these shapes.

export type Origin = string; // stored | stored_local_clock | file_mtime | neighbor_interpolated | absent | derived | inferred

export interface CaseInfo { name: string; examiner: string | null; created_at: string; tool_version: string }
export interface CaseTotals {
  sessions: number; messages: number; tool_calls: number; observations: number;
  anomalies_info: number; anomalies_warning: number; anomalies_error: number;
}
export interface CaseOverview { info: CaseInfo; totals: CaseTotals }

export interface RootRow { id: number; path: string; label: string; host: string | null; user: string | null; os: string | null; harness: string; attached_at: string }
export interface StoreRow { id: number; root_id: number; kind: string; generation: string | null; rel_path: string; discovery_method: string; status: string; file_count: number }
export interface AuditRow { id: number; ts: string; action: string; target: string | null; details: unknown }
export interface FailedFile { id: number; rel_path: string; parse_error: string | null }
export interface IngestStatus { counts: Record<string, number>; failed: FailedFile[]; last_ingest: AuditRow | null }
export interface RootOverview { root: RootRow; identification: string[]; stores: StoreRow[]; absent: string[]; ingest: IngestStatus }

export interface SourceFileRow {
  id: number; root_id: number; store_id: number | null; rel_path: string; kind: string; link_target: string | null;
  version: number; size: number; sha256: string; mtime: string | null; retained: boolean; parse_status: string;
  parse_error: string | null; record_count: number; anomaly_count: number;
}

export interface AnomalyRow {
  id: number; root_id: number; store_id: number | null; source_file_id: number | null; session_id: number | null;
  kind: string; severity: "info" | "warning" | "error"; byte_offset: number | null; provenance_id: number | null;
  message: string; details: unknown;
}

export interface SessionRow {
  id: number; root_id: number; store_id: number; harness: string; harness_session_id: string; kind: string;
  parent_session_id: number | null; title: string | null; project_path: string | null; git_branch: string | null;
  harness_version: string | null; models: string[]; first_ts: string | null; first_ts_origin: Origin;
  last_ts: string | null; last_ts_origin: Origin; message_count: number; tool_call_count: number;
  anomaly_count: number; child_count: number;
}
export interface ClaimRow { id: number; session_id: number; scheme: string; claimed_id: string; source_file_id: number; join_status: string; matched_session_id: number | null }
export interface SessionDetail { session: SessionRow; ancestors: SessionRow[]; children: SessionRow[]; claims: ClaimRow[] }

export interface BlockRow { id: number; ordinal: number; kind: string; text: string | null; payload: unknown; tool_use_id: string | null; tool_call_id: number | null }
export interface MessageRow {
  id: number; session_id: number; ordinal: number; role: string; harness_record_type: string;
  harness_uuid: string | null; parent_uuid: string | null; timestamp: string | null; ts_origin: Origin;
  model: string | null; provenance_id: number; origin: Origin; attributes: Record<string, unknown>; blocks: BlockRow[];
}
export interface ToolCallRow {
  id: number; session_id: number; name: string; category: string; input: unknown; result_text: string | null;
  result_payload: unknown | null; is_error: boolean; started_ts: string | null; ended_ts: string | null; ts_origin: Origin;
  tool_use_block_id: number; tool_result_block_id: number | null;
}
export interface ObservationRow {
  id: number; session_id: number; kind: string; path: string | null; command: string | null;
  before_blob: string | null; after_blob: string | null; timestamp: string | null; ts_origin: Origin;
  confidence: "high" | "medium" | "low"; details: Record<string, unknown>;
  derived_from_tool_call_id: number | null; derived_from_block_id: number | null; derived_from_provenance_id: number | null;
}

export type AnnotationTarget = "session" | "message" | "block" | "tool_call" | "observation" | "anomaly" | "source_file";
export type AnnotationKind = "tag" | "bookmark" | "note";
export interface Annotation { id: number; target_type: AnnotationTarget; target_id: number; kind: AnnotationKind; value: string; created_at: string }
export interface NewAnnotation { target_type: AnnotationTarget; target_id: number; kind: AnnotationKind; value: string }

export interface MessageDetail { message: MessageRow; tool_calls: ToolCallRow[]; observations: ObservationRow[]; annotations: Annotation[] }

export interface ProvenanceRow {
  id: number; source_file_id: number; rel_path: string; rel_path_encoded: boolean; file_sha256: string; root_path: string;
  retained: boolean; byte_offset: number; byte_length: number; record_index: number; content_sha256: string;
  parser_name: string; parser_version: string; origin: Origin;
}
export interface RawWindow { total_length: number; offset: number; bytes_b64: string; verified: boolean }

export type ActivityTab = "commands" | "files" | "indicators";
export interface ActivityRow extends ObservationRow { message_id: number | null; session_title: string | null; harness_session_id: string }

export interface BlobContent { sha256: string; size: number; binary: boolean; truncated: boolean; text: string | null; bytes_b64: string | null }

export interface DiffLine { tag: "equal" | "insert" | "delete"; old_no: number | null; new_no: number | null; text: string }
export interface Hunk { old_start: number; old_lines: number; new_start: number; new_lines: number; lines: DiffLine[] }
export interface DiffResult { binary: boolean; hunks: Hunk[] }

export interface SearchHit { block_id: number; message_id: number; session_id: number; tool_call_id: number | null; snippet: string; session_title: string | null; message_ordinal: number }

export type ExportFormat = "timesketch-jsonl" | "timesketch-csv" | "vestigo-parquet";
export type ExportScope = { kind: "case" } | { kind: "root"; id: number } | { kind: "session"; id: number };
export interface ExportRequest { format: ExportFormat; scope: ExportScope }
export interface ExportReport { name: string; format: string; output: string; events: number; sha256: string }
export interface ExportEntry { name: string; format: string; events: number; sha256: string; ts: string; exists: boolean }

export interface SessionQuery { harness?: string; root?: number; kind?: string; project?: string; from?: string; to?: string; has_children?: boolean; has_anomalies?: boolean }
export interface AnomalyQuery { root?: number; kind?: string; severity?: string; session?: number }
export interface ActivityQuery { root?: number; session?: number; kind?: string }
