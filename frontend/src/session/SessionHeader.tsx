import { Link } from "react-router";
import type { SessionDetail } from "../api/types";
import { Timestamp } from "../components/Timestamp";

export function SessionHeader({ detail, meta, onMeta }: { detail: SessionDetail; meta: boolean; onMeta: (on: boolean) => void }) {
  const s = detail.session;
  return (
    <header className="session-header panel">
      <h1>{s.title ?? s.harness_session_id}</h1>
      <dl className="kv">
        <dt>Session</dt><dd className="mono">{s.harness_session_id} ({s.kind}, {s.harness})</dd>
        <dt>Project</dt><dd className="mono">{s.project_path ?? "—"}{s.git_branch ? ` @ ${s.git_branch}` : ""}</dd>
        <dt>Harness version</dt><dd>{s.harness_version ?? "—"}</dd>
        <dt>Models</dt><dd>{s.models.join(", ") || "—"}</dd>
        <dt>Start</dt><dd><Timestamp value={s.first_ts} origin={s.first_ts_origin} /></dd>
        <dt>End</dt><dd><Timestamp value={s.last_ts} origin={s.last_ts_origin} /></dd>
        <dt>Counts</dt><dd>{s.message_count} messages · {s.tool_call_count} tool calls · {s.anomaly_count > 0 ? <Link to={`/?session=${s.id}`}>{s.anomaly_count} anomalies</Link> : "0 anomalies"}</dd>
      </dl>
      <label><input type="checkbox" checked={meta} onChange={(e) => onMeta(e.target.checked)} /> show meta and system records</label>
    </header>
  );
}
