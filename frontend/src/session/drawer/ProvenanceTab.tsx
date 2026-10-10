import { useQuery } from "@tanstack/react-query";
import { api } from "../../api/client";
import { CopyHash } from "../../components/CopyHash";
import { OriginBadge } from "../../components/OriginBadge";
import { ErrorBox, Loading } from "../../components/Status";

export function ProvenanceTab({ provenanceId }: { provenanceId: number }) {
  const q = useQuery({ queryKey: ["provenance", provenanceId], queryFn: () => api.provenance(provenanceId) });
  if (q.error) return <ErrorBox error={q.error} />;
  if (!q.data) return <Loading />;
  const p = q.data;
  return (
    <dl className="kv">
      <dt>Evidence file</dt><dd className="mono">{p.root_path}/{p.rel_path}{p.rel_path_encoded ? " (name percent-encoded)" : ""}</dd>
      <dt>File sha256</dt><dd><CopyHash sha={p.file_sha256} length={64} /></dd>
      <dt>Retained copy</dt><dd>{p.retained ? "yes" : "no (read from the evidence root)"}</dd>
      <dt>Byte offset</dt><dd className="mono">{p.byte_offset}</dd>
      <dt>Byte length</dt><dd className="mono">{p.byte_length}</dd>
      <dt>Record index</dt><dd className="mono">{p.record_index}</dd>
      <dt>Record sha256</dt><dd><CopyHash sha={p.content_sha256} length={64} /></dd>
      <dt>Parser</dt><dd className="mono">{p.parser_name} {p.parser_version}</dd>
      <dt>Origin</dt><dd><OriginBadge origin={p.origin} /></dd>
    </dl>
  );
}
