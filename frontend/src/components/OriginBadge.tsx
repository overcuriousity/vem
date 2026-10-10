export const ORIGIN_LABELS: Record<string, { cls: string; label: string; title: string }> = {
  stored: { cls: "badge-stored", label: "stored", title: "stored: the record itself carries this value" },
  stored_local_clock: { cls: "badge-local", label: "local clock", title: "stored_local_clock: carried by the record, in the examined machine's local time" },
  file_mtime: { cls: "badge-inferred", label: "file mtime", title: "file_mtime: inferred from the file's modification time" },
  neighbor_interpolated: { cls: "badge-inferred", label: "interpolated", title: "neighbor_interpolated: inferred from neighbouring records" },
  inferred: { cls: "badge-inferred", label: "inferred", title: "inferred: not carried by the evidence" },
  derived: { cls: "badge-inferred", label: "derived", title: "derived: rebuilt by vem from other evidence" },
  absent: { cls: "badge-absent", label: "absent", title: "absent: the evidence carries no value" },
};

/** Stored green, inferred/derived amber (dashed), absent grey, local clock blue. Unknown origins are never shown as stored. */
export function OriginBadge({ origin }: { origin: string }) {
  const o = ORIGIN_LABELS[origin] ?? { cls: "badge-inferred", label: origin, title: `${origin}: unrecognised origin` };
  return <span className={`badge ${o.cls}`} title={`origin ${o.title}`} data-origin={origin}>{o.label}</span>;
}
