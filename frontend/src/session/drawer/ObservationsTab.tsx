import { Link } from "react-router";
import type { ObservationRow } from "../../api/types";
import { Timestamp } from "../../components/Timestamp";

export function activityTabOf(kind: string): string | null {
  if (kind === "command_executed") return "commands";
  if (kind.startsWith("file_")) return "files";
  if (["url_referenced", "secret_candidate", "paste_detected", "upload_detected"].includes(kind)) return "indicators";
  return null;
}

export function ObservationsTab({ observations }: { observations: ObservationRow[] }) {
  if (observations.length === 0) return <p className="muted">No observations were derived from this record.</p>;
  return (
    <table className="grid">
      <thead><tr><th>Kind</th><th>Subject</th><th>Confidence</th><th>Time</th><th /></tr></thead>
      <tbody>
        {observations.map((o) => {
          const tab = activityTabOf(o.kind);
          return (
            <tr key={o.id} className="derived">
              <td>{o.kind}</td>
              <td className="mono">{o.command ?? o.path ?? (o.kind === "secret_candidate" ? String(o.details.rule ?? "") : "")}</td>
              <td>{o.confidence}</td>
              <td><Timestamp value={o.timestamp} origin={o.ts_origin} /></td>
              <td>{tab && <Link to={`/activity/${tab}?obs=${o.id}`}>open</Link>}</td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}
