import { useEffect, useMemo, useState } from "react";
import type { MessageRow, ObservationRow, SessionRow, ToolCallRow } from "../api/types";
import { MessageView } from "./MessageView";
import { TurnView } from "./TurnView";
import { UserMessage } from "./UserMessage";
import { itemMessageId, streamItems } from "./turns";
import type { Selection } from "./selection";

export { streamItems } from "./turns";

const USER_CLASSES = new Set(["prompt", "paste", "command", "command_output", "caveat", "injected", "tool_result"]);

export function Stream({ messages, toolCalls, observations, childSessions, meta, selection, onSelect, onShowMeta }: {
  messages: MessageRow[]; toolCalls: ToolCallRow[]; observations: ObservationRow[]; childSessions: SessionRow[]; meta: boolean;
  selection: Selection | null; onSelect: (s: Selection | null) => void; onShowMeta: () => void;
}) {
  const items = useMemo(() => streamItems(messages, meta), [messages, meta]);
  const toolCallsById = useMemo(() => new Map(toolCalls.map((t) => [t.id, t])), [toolCalls]);
  const visible = useMemo(() => items.flatMap((i) => { const id = itemMessageId(i); return id === null ? [] : [id]; }), [items]);
  const [focus, setFocus] = useState<number | null>(null);

  // A link with ?sel=message:N (from search) scrolls to and flashes that message once.
  useEffect(() => {
    if (selection?.type !== "message") return;
    const anchor = document.getElementById(`message-${selection.id}`);
    const el = anchor?.closest("article") ?? anchor;
    el?.scrollIntoView?.({ block: "center" });
    el?.classList.add("flash");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.target as HTMLElement | null)?.closest?.("input, textarea, select")) return;
      if (e.key === "j" || e.key === "k") {
        const idx = focus === null ? -1 : visible.indexOf(focus);
        const next = e.key === "j" ? Math.min(visible.length - 1, idx + 1) : Math.max(0, idx - 1);
        const id = visible[next];
        if (id !== undefined) {
          setFocus(id);
          document.getElementById(`message-${id}`)?.scrollIntoView?.({ block: "nearest" });
        }
      } else if (e.key === "Enter" && focus !== null) {
        onSelect({ type: "message", id: focus });
      } else if (e.key === "Escape") {
        onSelect(null);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [focus, visible, onSelect]);

  const card = { toolCallsById, observations, childSessions, selection, onSelect };
  return (
    <div className="stream">
      {items.map((i) => {
        if (i.kind === "hidden") {
          return (
            <button key={i.key} type="button" className="meta-divider" onClick={onShowMeta}>
              {i.count} meta {i.count === 1 ? "record" : "records"} hidden
            </button>
          );
        }
        const focused = focus === itemMessageId(i);
        if (i.kind === "turn") return <TurnView key={i.key} messages={i.messages} p={{ ...card, focused }} />;
        if (USER_CLASSES.has(i.message.content_class)) return <UserMessage key={i.key} message={i.message} p={{ ...card, focused }} />;
        return <MessageView key={i.key} message={i.message} {...card} focused={focused} />;
      })}
      {items.length === 0 && <p className="muted">This session has no messages.</p>}
    </div>
  );
}
