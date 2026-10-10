import { Link } from "react-router";
import type { SessionDetail, SessionRow } from "../api/types";

const label = (s: SessionRow) => `${s.title ?? s.harness_session_id} (${s.kind}, ${s.message_count} msgs)`;

export function SubagentTree({ detail }: { detail: SessionDetail }) {
  if (detail.ancestors.length === 0 && detail.children.length === 0) return null;
  return (
    <nav className="subagent-tree panel" aria-label="Subagent tree">
      <ol>
        {detail.ancestors.map((a) => <li key={a.id}><Link to={`/sessions/${a.id}`}>{label(a)}</Link></li>)}
        <li aria-current="page"><strong>{label(detail.session)}</strong>
          {detail.children.length > 0 && (
            <ul>{detail.children.map((c) => <li key={c.id}>↳ <Link to={`/sessions/${c.id}`}>{label(c)}</Link></li>)}</ul>
          )}
        </li>
      </ol>
    </nav>
  );
}
