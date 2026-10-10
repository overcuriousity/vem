import type { BlockRow, MessageRow, ObservationRow, SessionRow, ToolCallRow } from "../api/types";
import { JsonView } from "../components/JsonView";
import { OriginBadge } from "../components/OriginBadge";
import { Timestamp } from "../components/Timestamp";
import { TruncatedText } from "../components/TruncatedText";
import { chipSummary } from "./chip";
import { ToolChip } from "./ToolChip";
import type { Selection } from "./selection";

interface Props {
  message: MessageRow; toolCallsById: Map<number, ToolCallRow>; observations: ObservationRow[]; childSessions: SessionRow[];
  selection: Selection | null; focused: boolean; onSelect: (s: Selection) => void;
}

function BlockView({ block, p }: { block: BlockRow; p: Props }) {
  const tc = block.tool_call_id !== null ? p.toolCallsById.get(block.tool_call_id) : undefined;
  switch (block.kind) {
    case "text":
      return <div className="block-text"><TruncatedText text={block.text ?? ""} /></div>;
    case "thinking":
      return (
        <details className="block-thinking">
          <summary>thinking ({(block.text ?? "").length} chars)</summary>
          <TruncatedText text={block.text ?? ""} />
        </details>
      );
    case "tool_use":
      return tc ? (
        <ToolChip tc={tc} observations={p.observations} childSessions={p.childSessions} selected={p.selection?.type === "toolcall" && p.selection.id === tc.id} onSelect={p.onSelect} />
      ) : (
        <div className="block-other"><span className="badge badge-warning">tool use without a tool call</span><JsonView value={block.payload} /></div>
      );
    case "tool_result":
      return tc ? (
        <button type="button" className="result-ref" onClick={() => p.onSelect({ type: "block", id: block.id })}>
          ↳ result of {tc.name} · {chipSummary(tc).status}
        </button>
      ) : (
        <div className="block-other"><span className="badge badge-warning">unpaired tool result</span><TruncatedText text={block.text ?? ""} /></div>
      );
    default:
      // Images, attachments and unknown kinds: shown raw, never dropped.
      return (
        <div className="block-other">
          <span className="muted">{block.kind} block</span>
          {block.text && <TruncatedText text={block.text} />}
          <JsonView value={block.payload} />
        </div>
      );
  }
}

export function MessageView(p: Props) {
  const m = p.message;
  const compact = m.blocks.length > 0 && m.blocks.every((b) => b.kind === "tool_result" && b.tool_call_id !== null);
  const derived = m.origin !== "stored";
  const selected = p.selection?.type === "message" && p.selection.id === m.id;
  const classes = ["msg", `msg-${m.role}`, derived && "derived", compact && "msg-compact", selected && "selected", p.focused && "focused"].filter(Boolean).join(" ");
  return (
    <article
      id={`message-${m.id}`}
      className={classes}
      data-message-id={m.id}
      onClick={(e) => {
        if ((e.target as HTMLElement).closest("button, a, summary, details, input, textarea")) return;
        p.onSelect({ type: "message", id: m.id });
      }}
    >
      {!compact && (
        <header className="msg-head">
          <span className="role">{m.role}</span>
          <Timestamp value={m.timestamp} origin={m.ts_origin} />
          {m.model && <span className="muted">{m.model}</span>}
          {derived && <OriginBadge origin={m.origin} />}
          <span className="muted mono">{m.harness_record_type}</span>
        </header>
      )}
      {m.blocks.map((b) => <BlockView key={b.id} block={b} p={p} />)}
    </article>
  );
}
