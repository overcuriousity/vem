import { useState } from "react";
import { Link } from "react-router";
import { useQuery } from "@tanstack/react-query";
import { api } from "../api/client";
import type { ActivityRow } from "../api/types";
import { CopyHash } from "../components/CopyHash";
import { DiffView } from "../components/DiffView";
import { ErrorBox, Loading } from "../components/Status";

export function messageHref(r: ActivityRow): string {
  return r.message_id !== null ? `/sessions/${r.session_id}?sel=message:${r.message_id}` : `/sessions/${r.session_id}`;
}

export function DiffPanel({ row }: { row: ActivityRow }) {
  const [mode, setMode] = useState<"unified" | "split">("unified");
  const has = row.before_blob !== null || row.after_blob !== null;
  const q = useQuery({ queryKey: ["diff", row.before_blob, row.after_blob], queryFn: () => api.diff(row.before_blob, row.after_blob), enabled: has });
  return (
    <section className="panel" role="region" aria-label="Diff">
      <h2 className="mono">{row.path ?? "(no path)"}</h2>
      <p>
        {row.kind} · confidence {row.confidence} · <Link to={messageHref(row)}>originating message</Link>
      </p>
      <p className="muted">
        before {row.before_blob ? <CopyHash sha={row.before_blob} /> : "— (none recorded)"} · after {row.after_blob ? <CopyHash sha={row.after_blob} /> : "— (none recorded)"}
      </p>
      {!has ? (
        <p className="muted">No before or after content was recorded for this operation.</p>
      ) : (
        <>
          <p>
            <label><input type="radio" checked={mode === "unified"} onChange={() => setMode("unified")} /> unified</label>{" "}
            <label><input type="radio" checked={mode === "split"} onChange={() => setMode("split")} /> side by side</label>
          </p>
          {q.error ? <ErrorBox error={q.error} /> : !q.data ? <Loading /> : <DiffView diff={q.data} mode={mode} />}
        </>
      )}
    </section>
  );
}
