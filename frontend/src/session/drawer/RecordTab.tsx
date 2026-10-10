import { useQuery } from "@tanstack/react-query";
import { api } from "../../api/client";
import { JsonView } from "../../components/JsonView";
import { ErrorBox, Loading } from "../../components/Status";
import { TruncatedText } from "../../components/TruncatedText";
import { decodeBase64 } from "../../lib/base64";
import { formatBytes } from "../../lib/format";

const WINDOW = 65536;

/** The record parsed from its verified raw bytes (not from the database copy). */
export function RecordTab({ provenanceId }: { provenanceId: number }) {
  const q = useQuery({ queryKey: ["raw", provenanceId, 0, WINDOW], queryFn: () => api.raw(provenanceId, 0, WINDOW) });
  if (q.error) return <ErrorBox error={q.error} />;
  if (!q.data) return <Loading />;
  const text = new TextDecoder("utf-8").decode(decodeBase64(q.data.bytes_b64));
  const complete = q.data.total_length <= WINDOW;
  let parsed: unknown;
  let ok = false;
  if (complete) {
    try {
      parsed = JSON.parse(text);
      ok = true;
    } catch {
      ok = false;
    }
  }
  return (
    <div>
      <p>
        {q.data.verified && <span className="badge badge-stored">sha256 verified</span>} {formatBytes(q.data.total_length)}
      </p>
      {ok ? (
        <JsonView value={parsed} />
      ) : (
        <>
          <p className="muted">{complete ? "The record is not valid JSON; showing its text." : `The record is ${formatBytes(q.data.total_length)}; showing the first ${formatBytes(WINDOW)} as text.`}</p>
          <TruncatedText text={text} />
        </>
      )}
    </div>
  );
}
