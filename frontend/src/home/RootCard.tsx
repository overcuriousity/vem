import { useState } from "react";
import type { RootOverview } from "../api/types";
import { InventoryTree } from "./InventoryTree";

export function RootCard({ overview }: { overview: RootOverview }) {
  const { root, stores, absent, ingest, identification } = overview;
  const [showTree, setShowTree] = useState(false);
  return (
    <section className="panel" aria-label={`Evidence root ${root.label}`}>
      <h2>{root.label}</h2>
      <p className="mono">{root.path}</p>
      <p>
        {root.harness} · host {root.host ?? "—"} · user {root.user ?? "—"} · os {root.os ?? "—"} · attached {root.attached_at}
      </p>
      <details>
        <summary>Identified by</summary>
        <ul>{identification.map((e) => <li key={e}>{e}</li>)}</ul>
      </details>
      <table className="grid">
        <thead><tr><th>Store</th><th>Path</th><th>Generation</th><th>Discovery</th><th>Status</th><th>Files</th></tr></thead>
        <tbody>
          {stores.map((s) => (
            <tr key={s.id}><td>{s.kind}</td><td className="mono">{s.rel_path}</td><td>{s.generation ?? "—"}</td><td>{s.discovery_method}</td><td>{s.status}</td><td>{s.file_count}</td></tr>
          ))}
        </tbody>
      </table>
      {absent.length > 0 && (
        <div className="callout">
          Not present in this root: <span className="mono">{absent.join(", ")}</span>. If these existed on the examined host, request them from the collector.
        </div>
      )}
      <h3>Ingest status</h3>
      <p>{Object.entries(ingest.counts).map(([k, v]) => `${k}: ${v}`).join(" · ") || "not ingested"}{ingest.last_ingest && <span className="muted"> · last ingest {ingest.last_ingest.ts}</span>}</p>
      {ingest.failed.length > 0 && (
        <ul>{ingest.failed.map((f) => <li key={f.id}><span className="mono">{f.rel_path}</span> <span className="muted">{f.parse_error}</span></li>)}</ul>
      )}
      <details onToggle={(e) => setShowTree((e.target as HTMLDetailsElement).open)}>
        <summary>Inventory (every file, with hashes)</summary>
        {showTree && <InventoryTree rootId={root.id} />}
      </details>
    </section>
  );
}
