import { useQuery } from "@tanstack/react-query";
import { api } from "../api/client";
import { JsonView } from "../components/JsonView";
import { ErrorBox, Loading } from "../components/Status";

export default function Audit() {
  const q = useQuery({ queryKey: ["audit"], queryFn: api.audit });
  if (q.error) return <ErrorBox error={q.error} />;
  if (!q.data) return <Loading />;
  return (
    <div>
      <h1>Audit log</h1>
      <p className="muted">Append-only record of case creation, evidence attachment, ingest, verify, annotations and exports.</p>
      <table className="grid">
        <thead><tr><th>#</th><th>Time (UTC)</th><th>Action</th><th>Target</th><th>Details</th></tr></thead>
        <tbody>
          {q.data.map((a) => (
            <tr key={a.id}>
              <td>{a.id}</td><td className="mono">{a.ts}</td><td>{a.action}</td><td className="mono">{a.target ?? "—"}</td>
              <td><details><summary>details</summary><JsonView value={a.details} /></details></td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
