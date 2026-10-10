import { useState } from "react";
import { Link, useSearchParams } from "react-router";
import { useQuery } from "@tanstack/react-query";
import { api } from "../api/client";
import type { AnomalyQuery } from "../api/types";
import { ErrorBox, Loading } from "../components/Status";
import { RawPeek } from "./RawPeek";

export function AnomalyTable() {
  const [params, setParams] = useSearchParams();
  const filter: AnomalyQuery = {};
  if (params.get("kind")) filter.kind = params.get("kind")!;
  if (params.get("severity")) filter.severity = params.get("severity")!;
  if (params.get("root")) filter.root = Number(params.get("root"));
  if (params.get("session")) filter.session = Number(params.get("session"));
  const all = useQuery({ queryKey: ["anomalies", {}], queryFn: () => api.anomalies({}) });
  const q = useQuery({ queryKey: ["anomalies", filter], queryFn: () => api.anomalies(filter) });
  const [peek, setPeek] = useState<number | null>(null);
  const set = (k: string, v: string) => {
    const next = new URLSearchParams(params);
    if (v) next.set(k, v);
    else next.delete(k);
    setParams(next, { replace: true });
  };
  const kinds = [...new Set((all.data ?? []).map((a) => a.kind))].sort();
  return (
    <section>
      <h2>Anomalies</h2>
      <div className="filters">
        <label>Severity
          <select aria-label="Severity" value={filter.severity ?? ""} onChange={(e) => set("severity", e.target.value)}>
            <option value="">any</option><option value="info">info</option><option value="warning">warning</option><option value="error">error</option>
          </select>
        </label>
        <label>Kind
          <select aria-label="Kind" value={filter.kind ?? ""} onChange={(e) => set("kind", e.target.value)}>
            <option value="">any</option>
            {kinds.map((k) => <option key={k} value={k}>{k}</option>)}
          </select>
        </label>
        {filter.session !== undefined && <button type="button" onClick={() => set("session", "")}>clear session filter ({filter.session})</button>}
      </div>
      {q.error ? <ErrorBox error={q.error} /> : !q.data ? <Loading /> : (
        <table className="grid" aria-label="Anomalies">
          <thead><tr><th>Severity</th><th>Kind</th><th>Message</th><th>Where</th><th /></tr></thead>
          <tbody>
            {q.data.map((a) => [
              <tr key={a.id}>
                <td><span className={`badge badge-${a.severity}`}>{a.severity}</span></td>
                <td className="mono">{a.kind}</td>
                <td>{a.message}</td>
                <td className="mono">
                  {a.source_file_id !== null && `file ${a.source_file_id}`}{a.byte_offset !== null && ` @${a.byte_offset}`}{" "}
                  {a.session_id !== null && <Link to={`/sessions/${a.session_id}`}>session {a.session_id}</Link>}
                </td>
                <td>{a.provenance_id !== null && <button type="button" onClick={() => setPeek(peek === a.id ? null : a.id)}>raw bytes</button>}</td>
              </tr>,
              peek === a.id && a.provenance_id !== null ? (
                <tr key={`${a.id}-raw`}><td colSpan={5}><RawPeek provenanceId={a.provenance_id} baseOffset={a.byte_offset ?? 0} /></td></tr>
              ) : null,
            ])}
          </tbody>
        </table>
      )}
      {q.data?.length === 0 && <p className="muted">No anomalies match.</p>}
    </section>
  );
}
