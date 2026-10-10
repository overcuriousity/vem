import { useEffect, useMemo, useState } from "react";
import type { MessageRow, ObservationRow, SessionRow, ToolCallRow } from "../api/types";
import { MessageView } from "./MessageView";
import type { Selection } from "./selection";

export type StreamItem = { kind: "message"; message: MessageRow } | { kind: "hidden"; count: number; key: string };
const HIDDEN_ROLES = new Set(["meta", "system"]);

/** Messages in order; with `meta` off, each run of meta/system records collapses into one counted divider. */
export function streamItems(messages: MessageRow[], meta: boolean): StreamItem[] {
  const out: StreamItem[] = [];
  for (const m of messages) {
    if (meta || !HIDDEN_ROLES.has(m.role)) {
      out.push({ kind: "message", message: m });
      continue;
    }
    const last = out[out.length - 1];
    if (last?.kind === "hidden") last.count += 1;
    else out.push({ kind: "hidden", count: 1, key: `hidden-${m.id}` });
  }
  return out;
}

export function Stream({ messages, toolCalls, observations, childSessions, meta, selection, onSelect, onShowMeta }: {
  messages: MessageRow[]; toolCalls: ToolCallRow[]; observations: ObservationRow[]; childSessions: SessionRow[]; meta: boolean;
  selection: Selection | null; onSelect: (s: Selection | null) => void; onShowMeta: () => void;
}) {
  const items = useMemo(() => streamItems(messages, meta), [messages, meta]);
  const toolCallsById = useMemo(() => new Map(toolCalls.map((t) => [t.id, t])), [toolCalls]);
  const visible = useMemo(() => items.flatMap((i) => (i.kind === "message" ? [i.message.id] : [])), [items]);
  const [focus, setFocus] = useState<number | null>(null);

  // A link with ?sel=message:N (from search) scrolls to and flashes that message once.
  useEffect(() => {
    if (selection?.type !== "message") return;
    const el = document.getElementById(`message-${selection.id}`);
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

  return (
    <div className="stream">
      {items.map((i) =>
        i.kind === "hidden" ? (
          <button key={i.key} type="button" className="meta-divider" onClick={onShowMeta}>
            {i.count} meta {i.count === 1 ? "record" : "records"} hidden
          </button>
        ) : (
          <MessageView key={i.message.id} message={i.message} toolCallsById={toolCallsById} observations={observations} childSessions={childSessions} selection={selection} focused={focus === i.message.id} onSelect={onSelect} />
        ),
      )}
      {items.length === 0 && <p className="muted">This session has no messages.</p>}
    </div>
  );
}
