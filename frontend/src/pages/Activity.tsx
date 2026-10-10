import { Link, NavLink, useParams, useSearchParams } from "react-router";
import { useQuery } from "@tanstack/react-query";
import { api } from "../api/client";
import type { ActivityQuery, ActivityTab } from "../api/types";
import { ErrorBox, Loading } from "../components/Status";
import { Timestamp } from "../components/Timestamp";
import { DiffPanel, messageHref } from "../activity/DiffPanel";
import { Indicator } from "../activity/Indicator";
import "../activity/activity.css";

const TABS: ActivityTab[] = ["commands", "files", "indicators"];
const INDICATOR_KINDS = ["url_referenced", "secret_candidate", "paste_detected", "upload_detected"];

export default function Activity() {
  const tab = (useParams().tab ?? "commands") as ActivityTab;
  const [params, setParams] = useSearchParams();
  const f: ActivityQuery = {};
  if (params.get("root")) f.root = Number(params.get("root"));
  if (params.get("session")) f.session = Number(params.get("session"));
  if (tab === "indicators" && params.get("kind")) f.kind = params.get("kind")!;
  const selected = params.get("obs") ? Number(params.get("obs")) : null;
  const q = useQuery({ queryKey: ["activity", tab, f], queryFn: () => api.activity(tab, f), enabled: TABS.includes(tab) });
  const select = (id: number) => {
    const next = new URLSearchParams(params);
    next.set("obs", String(id));
    setParams(next, { replace: true });
  };
  const setKind = (k: string) => {
    const next = new URLSearchParams(params);
    if (k) next.set("kind", k);
    else next.delete("kind");
    setParams(next, { replace: true });
  };
  if (!TABS.includes(tab)) return <ErrorBox error={`Unknown activity view ${tab}`} />;
  const row = q.data?.find((r) => r.id === selected);
  return (
    <div>
      <h1>Activity</h1>
      <nav className="tabs-nav">{TABS.map((t) => <NavLink key={t} to={`/activity/${t}`}>{t}</NavLink>)}</nav>
      {tab === "indicators" && (
        <label>Kind{" "}
          <select aria-label="Indicator kind" value={f.kind ?? ""} onChange={(e) => setKind(e.target.value)}>
            <option value="">all</option>
            {INDICATOR_KINDS.map((k) => <option key={k} value={k}>{k}</option>)}
          </select>
        </label>
      )}
      <div className={`activity-layout${tab === "files" && row ? " with-panel" : ""}`}>
        <div>
          {q.error ? <ErrorBox error={q.error} /> : !q.data ? <Loading /> : (
            <table className="grid">
              <thead><tr><th>Time</th><th>Session</th>{tab === "indicators" && <th>Kind</th>}<th>{tab === "commands" ? "Command" : tab === "files" ? "Path" : "Indicator"}</th><th>Confidence</th><th /></tr></thead>
              <tbody>
                {q.data.map((r) => (
                  <tr key={r.id} className={`derived${r.id === selected ? " selected" : ""}`}>
                    <td><Timestamp value={r.timestamp} origin={r.ts_origin} /></td>
                    <td><Link to={`/sessions/${r.session_id}`}>{r.session_title ?? r.harness_session_id}</Link></td>
                    {tab === "indicators" && <td>{r.kind}</td>}
                    <td>
                      {tab === "files" ? <button type="button" className="linklike mono" onClick={() => select(r.id)}>{r.path ?? "(no path)"}</button>
                        : tab === "commands" ? <code>{r.command}</code>
                        : <Indicator row={r} />}
                      {tab === "files" && <span className="muted"> {r.kind}{typeof r.details.source === "string" ? ` · ${r.details.source}` : ""}</span>}
                    </td>
                    <td>{r.confidence}</td>
                    <td><Link to={messageHref(r)}>message</Link></td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          {q.data?.length === 0 && <p className="muted">Nothing recorded.</p>}
        </div>
        {tab === "files" && row && <DiffPanel row={row} />}
      </div>
    </div>
  );
}
