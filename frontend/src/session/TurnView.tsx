import type { BlockRow, MessageRow, ObservationRow, SessionRow, ToolCallRow } from "../api/types";
import { JsonView } from "../components/JsonView";
import { Markdown } from "../components/Markdown";
import { OriginBadge } from "../components/OriginBadge";
import { Timestamp } from "../components/Timestamp";
import { TruncatedText } from "../components/TruncatedText";
import { turnParts } from "./runs";
import { ToolRun } from "./ToolRun";
import type { Selection } from "./selection";

export interface CardProps {
  toolCallsById: Map<number, ToolCallRow>; resultsByCall: Map<number, number[]>; observations: ObservationRow[]; childSessions: SessionRow[];
  selection: Selection | null; focused: boolean; onSelect: (s: Selection) => void;
  /** The last expand/collapse-all from the session header; a new object each click, so every run follows it. */
  runsOpen: { open: boolean } | null;
}

/** A small button that opens the record behind one block in the drawer (raw bytes, provenance). */
export function RawButton({ block, onSelect, selection }: { block: BlockRow; onSelect: (s: Selection) => void; selection: Selection | null }) {
  const on = selection?.type === "block" && selection.id === block.id;
  return (
    <button type="button" className={`raw-ref${on ? " on" : ""}`} aria-label={`Raw record of block ${block.id}`} title="Open the stored record of this block" onClick={() => onSelect({ type: "block", id: block.id })}>
      raw
    </button>
  );
}

function isRedactedThinking(b: BlockRow): boolean {
  return b.kind === "thinking" && !(b.text ?? "").trim();
}

function TurnBlock({ block, p }: { block: BlockRow; p: CardProps }) {
  const raw = <RawButton block={block} onSelect={p.onSelect} selection={p.selection} />;
  switch (block.kind) {
    case "text":
      return <div className="turn-block block-text">{raw}<Markdown text={block.text ?? ""} /></div>;
    case "thinking":
      return (
        <details className="turn-block block-thinking">
          <summary>thinking ({(block.text ?? "").length} chars) {raw}</summary>
          <TruncatedText text={block.text ?? ""} />
        </details>
      );
    case "tool_use":
      // Tool uses with a tool call are drawn in a ToolRun; this is one without.
      return <div className="turn-block block-other">{raw}<span className="badge badge-warning">tool use without a tool call</span><JsonView value={block.payload} /></div>;
    default:
      return (
        <div className="turn-block block-other">
          {raw}<span className="muted">{block.kind} block</span>
          {block.text && <TruncatedText text={block.text} />}
          <JsonView value={block.payload} />
        </div>
      );
  }
}

/** Every record of one API response as one card; blocks keep their original order. */
export function TurnView({ messages, p }: { messages: MessageRow[]; p: CardProps }) {
  const first = messages[0];
  const ids = new Set(messages.map((m) => m.id));
  const blocks = messages.flatMap((m) => m.blocks);
  const redacted = blocks.filter(isRedactedThinking);
  const shown = blocks.filter((b) => !isRedactedThinking(b));
  const derived = messages.some((m) => m.origin !== "stored");
  const selected = p.selection?.type === "message" && ids.has(p.selection.id);
  const classes = ["msg", "msg-assistant", "turn", derived && "derived", selected && "selected", p.focused && "focused"].filter(Boolean).join(" ");
  return (
    <article
      id={`message-${first.id}`}
      className={classes}
      data-message-id={first.id}
      onClick={(e) => {
        if ((e.target as HTMLElement).closest("button, a, summary, details, input, textarea")) return;
        p.onSelect({ type: "message", id: first.id });
      }}
    >
      {/* Anchors for search links and ?sel=message:N on every record of the turn. */}
      {messages.slice(1).map((m) => <span key={m.id} id={`message-${m.id}`} className="anchor" />)}
      <header className="msg-head">
        <span className="role">assistant</span>
        <Timestamp value={first.timestamp} origin={first.ts_origin} />
        {first.model && <span className="muted">{first.model}</span>}
        {derived && <OriginBadge origin={messages.find((m) => m.origin !== "stored")!.origin} />}
        {messages.length > 1 && <span className="muted" title="Records of one API response">{messages.length} records</span>}
        {redacted.length > 0 && (
          <button type="button" className="thinking-redacted" title="Thinking blocks with no stored text (signature only)" onClick={() => p.onSelect({ type: "block", id: redacted[0].id })}>
            thinking (redacted){redacted.length > 1 ? ` ×${redacted.length}` : ""}
          </button>
        )}
      </header>
      {turnParts(shown, p.toolCallsById).map((part) =>
        part.kind === "run" ? <ToolRun key={`run-${part.key}`} calls={part.calls} p={p} /> : <TurnBlock key={part.block.id} block={part.block} p={p} />,
      )}
    </article>
  );
}
