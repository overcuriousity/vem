import type { MessageRow } from "../api/types";

/**
 * One card in the conversation: an assistant `turn` (every record of one API response), any other record
 * on its own, or a divider counting a run of hidden meta records.
 */
export type StreamItem =
  | { kind: "turn"; key: string; messages: MessageRow[] }
  | { kind: "message"; key: string; message: MessageRow }
  | { kind: "hidden"; key: string; count: number };

const HIDDEN_ROLES = new Set(["meta", "system"]);

/** A tool-result record whose every result belongs to a known call: it is drawn inside that call's chip. */
export function absorbed(m: MessageRow): boolean {
  return m.content_class === "tool_result" && m.blocks.length > 0 && m.blocks.every((b) => b.kind === "tool_result" && b.tool_call_id !== null);
}

/** The id of the record a card stands for (first record of a turn). */
export function itemMessageId(i: StreamItem): number | null {
  return i.kind === "turn" ? i.messages[0].id : i.kind === "message" ? i.message.id : null;
}

/**
 * Builds the cards. Records of one API response are merged into one turn even when tool results or hidden
 * meta records were written between them; with `meta` off, each run of meta/system records becomes one
 * divider. Grouping is presentation only: every record stays addressable.
 */
export function streamItems(messages: MessageRow[], meta: boolean): StreamItem[] {
  const out: StreamItem[] = [];
  let hidden = 0;
  let hiddenKey = "";
  const flush = () => {
    if (hidden > 0) out.push({ kind: "hidden", key: hiddenKey, count: hidden });
    hidden = 0;
  };
  for (const m of messages) {
    if (absorbed(m)) continue;
    if (!meta && HIDDEN_ROLES.has(m.role)) {
      if (hidden === 0) hiddenKey = `hidden-${m.id}`;
      hidden += 1;
      continue;
    }
    const last = out[out.length - 1];
    if (m.role === "assistant" && m.turn_key) {
      if (last?.kind === "turn" && last.messages[0].turn_key === m.turn_key) {
        last.messages.push(m);
        continue;
      }
      flush();
      out.push({ kind: "turn", key: `turn-${m.id}`, messages: [m] });
      continue;
    }
    flush();
    if (m.role === "assistant") out.push({ kind: "turn", key: `turn-${m.id}`, messages: [m] });
    else out.push({ kind: "message", key: `message-${m.id}`, message: m });
  }
  flush();
  return out;
}
