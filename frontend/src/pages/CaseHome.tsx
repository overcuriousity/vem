import { useQuery } from "@tanstack/react-query";
import { api } from "../api/client";
import { ErrorBox, Loading } from "../components/Status";
import { AnomalyTable } from "../home/AnomalyTable";
import { RootCard } from "../home/RootCard";
import "../home/home.css";

export default function CaseHome() {
  const overview = useQuery({ queryKey: ["case"], queryFn: api.caseOverview });
  const roots = useQuery({ queryKey: ["roots"], queryFn: api.roots });
  const error = overview.error ?? roots.error;
  if (error) return <ErrorBox error={error} />;
  if (!overview.data || !roots.data) return <Loading />;
  const t = overview.data.totals;
  return (
    <div>
      <h1>{overview.data.info.name}</h1>
      <p className="muted">created {overview.data.info.created_at} · vem {overview.data.info.tool_version}</p>
      <div className="totals">
        <div>{t.sessions} sessions</div><div>{t.messages} messages</div><div>{t.tool_calls} tool calls</div><div>{t.observations} observations</div>
        <div>anomalies: {t.anomalies_error} error · {t.anomalies_warning} warning · {t.anomalies_info} info</div>
      </div>
      {roots.data.length === 0 && <p>No evidence attached. Use <code>vem evidence add</code>, then <code>vem ingest</code>.</p>}
      {roots.data.map((r) => <RootCard key={r.root.id} overview={r} />)}
      <AnomalyTable />
    </div>
  );
}
