import { Link, useSearchParams } from "react-router";
import { useQuery } from "@tanstack/react-query";
import { api } from "../api/client";
import type { SearchHit } from "../api/types";
import { ErrorBox, Loading } from "../components/Status";
import { renderSnippet } from "../activity/snippet";
import "../activity/activity.css";

export default function Search() {
  const [params, setParams] = useSearchParams();
  const q = params.get("q") ?? "";
  const hits = useQuery({ queryKey: ["search", q], queryFn: () => api.search(q), enabled: q.trim().length > 0 });
  const groups = new Map<number, SearchHit[]>();
  for (const h of hits.data ?? []) groups.set(h.session_id, [...(groups.get(h.session_id) ?? []), h]);
  return (
    <div>
      <h1>Search</h1>
      <form onSubmit={(e) => { e.preventDefault(); const v = new FormData(e.currentTarget).get("q"); setParams(typeof v === "string" && v.trim() ? { q: v.trim() } : {}); }}>
        <input name="q" type="search" defaultValue={q} aria-label="Search query" /> <button type="submit">Search</button>
      </form>
      <p className="muted">Matches the words as one phrase across message text, tool names, tool inputs and tool results.</p>
      {!q.trim() ? null : hits.error ? <ErrorBox error={hits.error} /> : !hits.data ? <Loading /> : (
        <div className="hits">
          <p>{hits.data.length} hits in {groups.size} sessions</p>
          {[...groups.entries()].map(([sid, hs]) => (
            <section key={sid}>
              <h2><Link to={`/sessions/${sid}`}>{hs[0].session_title ?? `session ${sid}`}</Link></h2>
              <ul>
                {hs.map((h) => (
                  <li key={`${h.block_id}-${h.tool_call_id ?? "b"}`}>
                    <Link to={`/sessions/${sid}?sel=message:${h.message_id}`}>message {h.message_ordinal}</Link>
                    {h.tool_call_id !== null && <span className="badge badge-info">tool call</span>} {renderSnippet(h.snippet)}
                  </li>
                ))}
              </ul>
            </section>
          ))}
        </div>
      )}
    </div>
  );
}
