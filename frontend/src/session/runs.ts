import type { BlockRow, ToolCallRow } from "../api/types";
import { chipSummary, type ChipStatus } from "./chip";
import type { Selection } from "./selection";

/** Runs with more calls than this start collapsed. */
export const RUN_OPEN_MAX = 5;

export type TurnPart = { kind: "block"; block: BlockRow } | { kind: "run"; key: number; calls: ToolCallRow[] };

/** The blocks of a turn, with each maximal sequence of consecutive tool calls gathered into one run. A tool_use
 * block without a tool call row is drawn on its own and ends the run. */
export function turnParts(blocks: BlockRow[], toolCallsById: Map<number, ToolCallRow>): TurnPart[] {
  const out: TurnPart[] = [];
  for (const b of blocks) {
    const tc = b.kind === "tool_use" && b.tool_call_id !== null ? toolCallsById.get(b.tool_call_id) : undefined;
    const last = out[out.length - 1];
    if (!tc) out.push({ kind: "block", block: b });
    else if (last?.kind === "run") last.calls.push(tc);
    else out.push({ kind: "run", key: b.id, calls: [tc] });
  }
  return out;
}

export const GLYPH: Record<ChipStatus, string> = { ok: "✓", error: "✕", "no result": "○" };

/** `Read ×12, Shell ×9, Grep`: tool names by frequency, ties in order of first use. */
export function runNames(calls: ToolCallRow[]): string {
  const counts = new Map<string, number>();
  for (const c of calls) counts.set(c.name, (counts.get(c.name) ?? 0) + 1);
  return [...counts].sort((a, b) => b[1] - a[1]).map(([n, k]) => (k > 1 ? `${n} ×${k}` : n)).join(", ");
}

/** `✓ all ok`, `○ 10 of 10 no result`, `✕ 2 errors · ✓ 8 ok`; the leading glyph is the worst state present. */
export function runStatus(calls: ToolCallRow[]): { worst: ChipStatus; text: string } {
  const n: Record<ChipStatus, number> = { ok: 0, error: 0, "no result": 0 };
  for (const c of calls) n[chipSummary(c).status]++;
  const worst: ChipStatus = n.error ? "error" : n["no result"] ? "no result" : "ok";
  if (n.ok === calls.length) return { worst, text: `${GLYPH.ok} all ok` };
  if (n["no result"] === calls.length) return { worst, text: `${GLYPH["no result"]} ${calls.length} of ${calls.length} no result` };
  const parts = [
    n.error && `${GLYPH.error} ${n.error} ${n.error === 1 ? "error" : "errors"}`,
    n["no result"] && `${GLYPH["no result"]} ${n["no result"]} no result`,
    n.ok && `${GLYPH.ok} ${n.ok} ok`,
  ];
  return { worst, text: parts.filter(Boolean).join(" · ") };
}

/** Whether the selection points into this run: one of its calls, or a tool-result record drawn in one of them. */
export function runHoldsSelection(calls: ToolCallRow[], resultsByCall: Map<number, number[]>, sel: Selection | null): boolean {
  if (sel?.type === "toolcall") return calls.some((c) => c.id === sel.id);
  if (sel?.type === "message") return calls.some((c) => (resultsByCall.get(c.id) ?? []).includes(sel.id));
  return false;
}
