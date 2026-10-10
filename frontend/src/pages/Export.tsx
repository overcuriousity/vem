import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../api/client";
import type { ExportFormat, ExportRequest, ExportScope } from "../api/types";
import { CopyHash } from "../components/CopyHash";
import { ErrorBox, Loading } from "../components/Status";

const FORMATS: ExportFormat[] = ["timesketch-jsonl", "timesketch-csv", "vestigo-parquet"];

export default function Export() {
  const qc = useQueryClient();
  const [format, setFormat] = useState<ExportFormat>("timesketch-jsonl");
  const [scopeKind, setScopeKind] = useState<"case" | "root" | "session">("case");
  const [scopeId, setScopeId] = useState("");
  const roots = useQuery({ queryKey: ["roots"], queryFn: api.roots });
  const list = useQuery({ queryKey: ["exports"], queryFn: api.exports });
  const run = useMutation({ mutationFn: (r: ExportRequest) => api.runExport(r), onSuccess: () => void qc.invalidateQueries({ queryKey: ["exports"] }) });
  const scope: ExportScope | null = scopeKind === "case" ? { kind: "case" } : scopeId ? { kind: scopeKind, id: Number(scopeId) } : null;
  return (
    <div>
      <h1>Export</h1>
      <form
        className="filters"
        onSubmit={(e) => {
          e.preventDefault();
          if (scope) run.mutate({ format, scope });
        }}
      >
        <label>Format
          <select aria-label="Format" value={format} onChange={(e) => setFormat(e.target.value as ExportFormat)}>
            {FORMATS.map((f) => <option key={f} value={f}>{f}</option>)}
          </select>
        </label>
        <label>Scope
          <select aria-label="Scope" value={scopeKind} onChange={(e) => { setScopeKind(e.target.value as "case" | "root" | "session"); setScopeId(""); }}>
            <option value="case">whole case</option><option value="root">one evidence root</option><option value="session">one session</option>
          </select>
        </label>
        {scopeKind === "root" && (
          <label>Root
            <select aria-label="Root" value={scopeId} onChange={(e) => setScopeId(e.target.value)}>
              <option value="">choose…</option>
              {roots.data?.map((r) => <option key={r.root.id} value={r.root.id}>{r.root.label}</option>)}
            </select>
          </label>
        )}
        {scopeKind === "session" && (
          <label>Session id<input aria-label="Session id" inputMode="numeric" value={scopeId} onChange={(e) => setScopeId(e.target.value.replace(/\D/g, ""))} /></label>
        )}
        <button type="submit" disabled={!scope || run.isPending}>Run export</button>
      </form>
      <p className="muted">Exports are written to the case's exports/ directory, hashed (SHA-256) and recorded in the audit log.</p>
      {run.error && <ErrorBox error={run.error} />}
      {run.data && (
        <p className="panel">
          Exported {run.data.events} events as {run.data.format}: <a href={api.exportUrl(run.data.name)} download>download {run.data.name}</a> · sha256 <CopyHash sha={run.data.sha256} length={64} />
        </p>
      )}
      <h2>Earlier exports</h2>
      {list.error ? <ErrorBox error={list.error} /> : !list.data ? <Loading /> : list.data.length === 0 ? <p className="muted">None yet.</p> : (
        <table className="grid">
          <thead><tr><th>Time (UTC)</th><th>File</th><th>Format</th><th>Events</th><th>SHA-256</th></tr></thead>
          <tbody>
            {list.data.map((e) => (
              <tr key={e.name}>
                <td className="mono">{e.ts}</td>
                <td>{e.exists ? <a href={api.exportUrl(e.name)} download>{e.name}</a> : <span>{e.name} <span className="badge badge-warning">file missing</span></span>}</td>
                <td>{e.format}</td><td>{e.events}</td><td><CopyHash sha={e.sha256} /></td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
