import type * as T from "./types";

export class ApiError extends Error {
  constructor(public status: number, public kind: string, message: string) {
    super(message);
  }
}

async function request<R>(method: string, path: string, body?: unknown): Promise<R> {
  // Every non-GET carries JSON content type: the server's write guard requires it.
  const headers: Record<string, string> = method === "GET" ? {} : { "Content-Type": "application/json" };
  const res = await fetch(path, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) });
  if (res.status === 204) return undefined as R;
  const text = await res.text();
  let data: unknown = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    data = null;
  }
  if (!res.ok) {
    const d = (data ?? {}) as { kind?: string; error?: string };
    throw new ApiError(res.status, d.kind ?? "http", d.error ?? res.statusText);
  }
  return data as R;
}

type Params = Record<string, string | number | boolean | null | undefined>;
export function qs(params: Params): string {
  const p = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (v === undefined || v === null || v === "" || v === false) continue;
    p.set(k, String(v));
  }
  const s = p.toString();
  return s ? `?${s}` : "";
}

export const api = {
  caseOverview: () => request<T.CaseOverview>("GET", "/api/case"),
  roots: () => request<T.RootOverview[]>("GET", "/api/roots"),
  rootFiles: (id: number) => request<T.SourceFileRow[]>("GET", `/api/roots/${id}/files`),
  anomalies: (f: T.AnomalyQuery = {}) => request<T.AnomalyRow[]>("GET", `/api/anomalies${qs({ ...f })}`),
  audit: () => request<T.AuditRow[]>("GET", "/api/audit"),
  sessions: (f: T.SessionQuery = {}) => request<T.SessionRow[]>("GET", `/api/sessions${qs({ ...f })}`),
  session: (id: number) => request<T.SessionDetail>("GET", `/api/sessions/${id}`),
  messages: (id: number, meta: boolean) => request<T.MessageRow[]>("GET", `/api/sessions/${id}/messages${qs({ meta })}`),
  toolCalls: (id: number) => request<T.ToolCallRow[]>("GET", `/api/sessions/${id}/tool-calls`),
  sessionObservations: (id: number) => request<T.ObservationRow[]>("GET", `/api/sessions/${id}/observations`),
  message: (id: number) => request<T.MessageDetail>("GET", `/api/messages/${id}`),
  provenance: (id: number) => request<T.ProvenanceRow>("GET", `/api/provenance/${id}`),
  raw: (id: number, offset = 0, len = 65536) => request<T.RawWindow>("GET", `/api/provenance/${id}/raw${qs({ offset, len })}`),
  activity: (tab: T.ActivityTab, f: T.ActivityQuery = {}) => request<T.ActivityRow[]>("GET", `/api/activity/${tab}${qs({ ...f })}`),
  blob: (sha: string) => request<T.BlobContent>("GET", `/api/blobs/${sha}`),
  diff: (before: string | null, after: string | null) => request<T.DiffResult>("GET", `/api/diff${qs({ before, after })}`),
  search: (q: string, limit = 200) => request<T.SearchHit[]>("GET", `/api/search${qs({ q, limit })}`),
  annotations: (target_type?: T.AnnotationTarget, target_id?: number) => request<T.Annotation[]>("GET", `/api/annotations${qs({ target_type, target_id })}`),
  createAnnotation: (a: T.NewAnnotation) => request<T.Annotation>("POST", "/api/annotations", a),
  deleteAnnotation: (id: number) => request<void>("DELETE", `/api/annotations/${id}`),
  runExport: (r: T.ExportRequest) => request<T.ExportReport>("POST", "/api/exports", r),
  exports: () => request<T.ExportEntry[]>("GET", "/api/exports"),
  exportUrl: (name: string) => `/api/exports/${encodeURIComponent(name)}`,
};
