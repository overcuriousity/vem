import { Link, useSearchParams } from "react-router";
import { useQuery } from "@tanstack/react-query";
import { api } from "../api/client";
import type { SessionQuery } from "../api/types";
import { ErrorBox, Loading } from "../components/Status";
import { Timestamp } from "../components/Timestamp";
import { nestSessions } from "../home/nest";
import "../home/home.css";

const TEXT_FILTERS = ["harness", "kind", "project", "from", "to"] as const;

export default function Sessions() {
  const [params, setParams] = useSearchParams();
  const q: SessionQuery = {};
  for (const k of TEXT_FILTERS) if (params.get(k)) q[k] = params.get(k)!;
  if (params.get("root")) q.root = Number(params.get("root"));
  if (params.get("has_children") === "1") q.has_children = true;
  if (params.get("has_anomalies") === "1") q.has_anomalies = true;
  const sessions = useQuery({ queryKey: ["sessions", q], queryFn: () => api.sessions(q) });
  const roots = useQuery({ queryKey: ["roots"], queryFn: api.roots });
  const set = (k: string, v: string) => {
    const next = new URLSearchParams(params);
    if (v) next.set(k, v);
    else next.delete(k);
    setParams(next, { replace: true });
  };
  return (
    <div>
      <h1>Sessions</h1>
      <div className="filters">
        <label>Harness<select aria-label="Harness" value={q.harness ?? ""} onChange={(e) => set("harness", e.target.value)}>
          <option value="">any</option><option value="claude_code">claude_code</option><option value="codex">codex</option><option value="cursor">cursor</option><option value="cursor_ide">cursor_ide</option>
        </select></label>
        <label>Root<select aria-label="Root" value={q.root ?? ""} onChange={(e) => set("root", e.target.value)}>
          <option value="">any</option>{roots.data?.map((r) => <option key={r.root.id} value={r.root.id}>{r.root.label}</option>)}
        </select></label>
        <label>Kind<select aria-label="Kind" value={q.kind ?? ""} onChange={(e) => set("kind", e.target.value)}>
          <option value="">any</option>{["primary", "subagent", "resumed", "forked", "sidecar_only"].map((k) => <option key={k} value={k}>{k}</option>)}
        </select></label>
        <label>Project contains<input aria-label="Project contains" defaultValue={q.project ?? ""} onBlur={(e) => set("project", e.target.value.trim())} /></label>
        <label>From (UTC)<input aria-label="From" type="date" value={q.from ?? ""} onChange={(e) => set("from", e.target.value)} /></label>
        <label>To (UTC)<input aria-label="To" type="date" value={q.to ?? ""} onChange={(e) => set("to", e.target.value)} /></label>
        <label><span><input type="checkbox" checked={!!q.has_children} onChange={(e) => set("has_children", e.target.checked ? "1" : "")} /> has subagents</span></label>
        <label><span><input type="checkbox" checked={!!q.has_anomalies} onChange={(e) => set("has_anomalies", e.target.checked ? "1" : "")} /> has anomalies</span></label>
      </div>
      {sessions.error ? <ErrorBox error={sessions.error} /> : !sessions.data ? <Loading /> : (
        <table className="grid">
          <thead><tr><th>Title</th><th>Harness</th><th>Kind</th><th>Project</th><th>Start</th><th>End</th><th>Msgs</th><th>Tools</th><th>Models</th><th>Anomalies</th></tr></thead>
          <tbody>
            {nestSessions(sessions.data).map(({ row, depth }) => (
              <tr key={row.id}>
                <td className={`depth-${Math.min(depth, 3)}`}>{depth > 0 && "↳ "}<Link to={`/sessions/${row.id}`}>{row.title ?? row.harness_session_id}</Link></td>
                <td>{row.harness}</td><td>{row.kind}</td><td className="mono">{row.project_path ?? "—"}</td>
                <td><Timestamp value={row.first_ts} origin={row.first_ts_origin} /></td>
                <td><Timestamp value={row.last_ts} origin={row.last_ts_origin} /></td>
                <td>{row.message_count}</td><td>{row.tool_call_count}</td><td>{row.models.join(", ")}</td><td>{row.anomaly_count}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {sessions.data?.length === 0 && <p className="muted">No sessions match these filters.</p>}
    </div>
  );
}
