import type { ReactNode } from "react";

/** FTS snippets mark hits as `[hit]`; render those as `<mark>`. */
export function renderSnippet(snippet: string): ReactNode[] {
  return snippet.split(/(\[[^\]]*\])/).map((part, i) =>
    part.startsWith("[") && part.endsWith("]") ? <mark key={i}>{part.slice(1, -1)}</mark> : <span key={i}>{part}</span>,
  );
}
