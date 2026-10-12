import { useQuery } from "@tanstack/react-query";
import { api } from "../../api/client";
import { HexView } from "../../components/HexView";
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
  const bytes = decodeBase64(q.data.bytes_b64);
  const complete = q.data.total_length <= WINDOW;
  // A binary record (a protobuf node from Cursor's chat store) is shown as hex, not as replacement characters.
  // A window cut inside a multi-byte character is still text, so only a complete record is judged.
  let text: string | null;
  try {
    text = new TextDecoder("utf-8", { fatal: complete }).decode(bytes);
  } catch {
    text = null;
  }
  let parsed: unknown;
  let ok = false;
  if (complete && text !== null) {
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
      {text === null ? (
        <>
          <p className="muted">The record is not UTF-8 text; showing its bytes.</p>
          <HexView bytes={bytes} baseOffset={0} />
        </>
      ) : ok ? (
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
