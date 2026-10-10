import { Link } from "react-router";
import type { ClaimRow } from "../../api/types";

export function ClaimsTab({ claims }: { claims: ClaimRow[] }) {
  if (claims.length === 0) return <p className="muted">No identity claims recorded for this session.</p>;
  return (
    <table className="grid">
      <thead><tr><th>Scheme</th><th>Claimed id</th><th>Join</th><th>Matched session</th></tr></thead>
      <tbody>
        {claims.map((c) => (
          <tr key={c.id}>
            <td>{c.scheme}</td><td className="mono">{c.claimed_id}</td><td>{c.join_status}</td>
            <td>{c.matched_session_id !== null ? <Link to={`/sessions/${c.matched_session_id}`}>{c.matched_session_id}</Link> : "—"}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
