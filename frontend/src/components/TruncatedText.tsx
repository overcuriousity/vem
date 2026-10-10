import { useState } from "react";
import { formatBytes } from "../lib/format";
import "./components.css";

/** Renders at most `limit` characters until "show all" is pressed, so multi-MB tool output stays responsive. */
export function TruncatedText({ text, limit = 65536 }: { text: string; limit?: number }) {
  const [all, setAll] = useState(false);
  if (all || text.length <= limit) return <pre className="json">{text}</pre>;
  return (
    <div className="truncated">
      <pre className="json">{text.slice(0, limit)}…</pre>
      <button type="button" onClick={() => setAll(true)}>Show all ({formatBytes(text.length)})</button>
    </div>
  );
}
