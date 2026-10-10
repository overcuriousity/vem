import type { MessageRow, ToolCallRow } from "../api/types";

export type SelectionType = "message" | "block" | "toolcall";
export interface Selection { type: SelectionType; id: number }

export function parseSelection(s: string | null): Selection | null {
  const m = s ? /^(message|block|toolcall):(\d+)$/.exec(s) : null;
  return m ? { type: m[1] as SelectionType, id: Number(m[2]) } : null;
}

export function formatSelection(s: Selection): string {
  return `${s.type}:${s.id}`;
}

/** The message a selection belongs to: itself, the block's message, or the message holding the tool call's `tool_use`. */
export function messageIdOf(sel: Selection, messages: MessageRow[], toolCalls: ToolCallRow[]): number | null {
  if (sel.type === "message") return messages.some((m) => m.id === sel.id) ? sel.id : null;
  const blockId = sel.type === "block" ? sel.id : toolCalls.find((t) => t.id === sel.id)?.tool_use_block_id;
  if (blockId === undefined) return null;
  return messages.find((m) => m.blocks.some((b) => b.id === blockId))?.id ?? null;
}
