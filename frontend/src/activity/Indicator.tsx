import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "../api/client";
import type { ActivityRow } from "../api/types";
import { MaskedSecret } from "../components/MaskedSecret";
import { ErrorBox, Loading } from "../components/Status";
import { TruncatedText } from "../components/TruncatedText";

function BlobPreview({ sha, mime }: { sha: string; mime: string | null }) {
  const q = useQuery({ queryKey: ["blob", sha], queryFn: () => api.blob(sha) });
  if (q.error) return <ErrorBox error={q.error} />;
  if (!q.data) return <Loading />;
  if (mime?.startsWith("image/") && q.data.bytes_b64) return <div className="preview"><img alt="uploaded file" src={`data:${mime};base64,${q.data.bytes_b64}`} /></div>;
  if (q.data.text !== null) return <TruncatedText text={q.data.text} />;
  return <p className="muted">Binary content ({q.data.size} bytes{q.data.truncated ? ", truncated" : ""}).</p>;
}

function Preview({ sha, mime }: { sha: string; mime: string | null }) {
  const [open, setOpen] = useState(false);
  return open ? <BlobPreview sha={sha} mime={mime} /> : <button type="button" onClick={() => setOpen(true)}>preview</button>;
}

/** The kind-specific cell of an indicators row. */
export function Indicator({ row }: { row: ActivityRow }) {
  const d = row.details;
  switch (row.kind) {
    case "secret_candidate":
      return <span><code>{String(d.rule ?? "")}</code> in {String(d.field ?? "?")}: <MaskedSecret value={String(d.match ?? "")} /></span>;
    case "url_referenced":
      return <span className="mono">{row.path ?? String(d.query ?? "")}</span>;
    case "paste_detected": {
      const pastes = Array.isArray(d.pastes) ? (d.pastes as Record<string, unknown>[]) : [];
      const inline = (d.pastedContents ?? {}) as Record<string, { content?: string }>;
      return (
        <div>
          <div>{String(d.display ?? "")}</div>
          {pastes.map((p, i) => (
            <div key={i} className="muted">
              paste {String(p.id)}: {p.missing ? <span className="badge badge-warning">content missing</span>
                : p.inline ? <TruncatedText text={Object.values(inline).find((c) => c.content !== undefined)?.content ?? ""} limit={2000} />
                : typeof p.content_blob === "string" ? <Preview sha={p.content_blob} mime="text/plain" /> : null}
            </div>
          ))}
        </div>
      );
    }
    case "upload_detected":
      return (
        <span>
          <span className="mono">{row.path}</span> · {String(d.mime ?? "")} · {String(d.size ?? "")} bytes{" "}
          {typeof d.content_blob === "string" && <Preview sha={d.content_blob} mime={typeof d.mime === "string" ? d.mime : null} />}
        </span>
      );
    default:
      return <span className="mono">{row.path ?? row.command ?? ""}</span>;
  }
}
