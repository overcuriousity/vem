import type { SessionRow } from "../api/types";

/** Each listed session followed by its listed descendants (depth-first); sessions whose parent is not listed stay at depth 0 in list order. */
export function nestSessions(rows: SessionRow[]): { row: SessionRow; depth: number }[] {
  const ids = new Set(rows.map((r) => r.id));
  const kids = new Map<number, SessionRow[]>();
  for (const r of rows) {
    if (r.parent_session_id !== null && ids.has(r.parent_session_id)) {
      kids.set(r.parent_session_id, [...(kids.get(r.parent_session_id) ?? []), r]);
    }
  }
  const out: { row: SessionRow; depth: number }[] = [];
  const seen = new Set<number>();
  const visit = (r: SessionRow, depth: number) => {
    if (seen.has(r.id)) return;
    seen.add(r.id);
    out.push({ row: r, depth });
    for (const k of kids.get(r.id) ?? []) visit(k, depth + 1);
  };
  for (const r of rows) {
    if (r.parent_session_id === null || !ids.has(r.parent_session_id)) visit(r, 0);
  }
  for (const r of rows) visit(r, 0); // cycles: never lose a row
  return out;
}
