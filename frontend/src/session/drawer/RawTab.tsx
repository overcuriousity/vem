import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "../../api/client";
import { HexView } from "../../components/HexView";
import { ErrorBox, Loading } from "../../components/Status";
import { decodeBase64 } from "../../lib/base64";

const PAGE = 4096;

/** The record's bytes as hex + ASCII at their offsets in the evidence file, a page at a time. */
export function RawTab({ provenanceId }: { provenanceId: number }) {
  const [offset, setOffset] = useState(0);
  const [asText, setAsText] = useState(false);
  const prov = useQuery({ queryKey: ["provenance", provenanceId], queryFn: () => api.provenance(provenanceId) });
  const win = useQuery({ queryKey: ["raw", provenanceId, offset, PAGE], queryFn: () => api.raw(provenanceId, offset, PAGE) });
  const error = prov.error ?? win.error;
  if (error) return <ErrorBox error={error} />;
  if (!prov.data || !win.data) return <Loading />;
  const bytes = decodeBase64(win.data.bytes_b64);
  const total = win.data.total_length;
  return (
    <div>
      <p>
        bytes {win.data.offset}–{win.data.offset + bytes.length} of {total} · file offset {prov.data.byte_offset + win.data.offset}{" "}
        {win.data.verified && <span className="badge badge-stored">sha256 verified</span>}
      </p>
      <p>
        <button type="button" disabled={offset === 0} onClick={() => setOffset(Math.max(0, offset - PAGE))}>previous</button>{" "}
        <button type="button" disabled={offset + PAGE >= total} onClick={() => setOffset(offset + PAGE)}>next</button>{" "}
        <label><input type="checkbox" checked={asText} onChange={(e) => setAsText(e.target.checked)} /> as text (lossy UTF-8)</label>
      </p>
      {asText ? <pre className="json">{new TextDecoder("utf-8").decode(bytes)}</pre> : <HexView bytes={bytes} baseOffset={prov.data.byte_offset + win.data.offset} />}
    </div>
  );
}
