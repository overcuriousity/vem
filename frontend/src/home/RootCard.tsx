import { useState } from "react";
import type { IngestStatus, RootOverview } from "../api/types";
import { InventoryTree } from "./InventoryTree";

const STATUS_LABEL: Record<string, string> = {
  parsed: "parsed", unparsed: "not parsed (no parser for them)", inventoried: "inventoried only", failed: "failed", superseded: "superseded",
};

/** `files: 1088, 68 in the stores above and 1020 elsewhere in the root (57 parsed · …) · records parsed: 5000`: every count says what it counts. */
export function ingestLine(ingest: IngestStatus): string {
  if (ingest.files === 0) return "not ingested";
  const order = Object.keys(STATUS_LABEL);
  const rank = (k: string) => (order.includes(k) ? order.indexOf(k) : order.length);
  const parts = Object.entries(ingest.counts).sort(([a], [b]) => rank(a) - rank(b)).map(([k, v]) => `${v} ${STATUS_LABEL[k] ?? k}`);
  const elsewhere = ingest.files - ingest.files_in_stores;
  const where = elsewhere > 0 ? `, ${ingest.files_in_stores} in the stores above and ${elsewhere} elsewhere in the root` : "";
  return `files: ${ingest.files}${where} (${parts.join(" · ")}) · records parsed: ${ingest.records}`;
}

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
      <p>{ingestLine(ingest)}{ingest.last_ingest && <span className="muted"> · last ingest {ingest.last_ingest.ts}</span>}</p>
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
