import { useQuery } from "@tanstack/react-query";
import { api } from "../api/client";
import { HexView } from "../components/HexView";
import { ErrorBox, Loading } from "../components/Status";
import { decodeBase64 } from "../lib/base64";

/** The first 1 KiB of a record's verified bytes. `baseOffset` is the record's offset in its file. */
export function RawPeek({ provenanceId, baseOffset }: { provenanceId: number; baseOffset: number }) {
  const q = useQuery({ queryKey: ["raw", provenanceId, 0, 1024], queryFn: () => api.raw(provenanceId, 0, 1024) });
  if (q.error) return <ErrorBox error={q.error} />;
  if (!q.data) return <Loading />;
  return <HexView bytes={decodeBase64(q.data.bytes_b64)} baseOffset={baseOffset} />;
}
