import type { MouseEvent } from "react";
import type { MessageRow } from "../api/types";
import { Markdown } from "../components/Markdown";
import { Timestamp } from "../components/Timestamp";
import { TruncatedText } from "../components/TruncatedText";
import { RawButton, type CardProps } from "./TurnView";

export function textOf(m: MessageRow): string {
  return m.blocks.filter((b) => b.kind === "text").map((b) => b.text ?? "").join("\n");
}

function tag(text: string, name: string): string | null {
  const m = new RegExp(`<${name}>([\\s\\S]*?)</${name}>`).exec(text);
  return m ? m[1].trim() : null;
}

/** `/clear foo` from `<command-name>/clear</command-name>…<command-args>foo</command-args>`. */
export function commandLine(text: string): string {
  const name = tag(text, "command-name") ?? tag(text, "command-message") ?? "command";
  const args = tag(text, "command-args");
  return `${name.startsWith("/") ? name : `/${name}`}${args ? ` ${args}` : ""}`;
}

/** What an injected record is, from its markup: shown as the label of the collapsed card. */
export function injectedLabel(m: MessageRow, text: string): string {
  const t = text.trimStart();
  if (m.content_class === "caveat") return "local-command caveat";
  if (t.startsWith("<system-reminder>")) return "system reminder";
  if (m.attributes.isCompactSummary === true) return "compaction summary";
  if (t.startsWith("Base directory for this skill:") || typeof m.attributes.sourceToolUseID === "string") return "skill";
  return "harness text";
}

/** Prompt text split into its own words and `<pasted_content>` sections, which are drawn as collapsed pastes. */
export function splitPastes(text: string): { paste: boolean; text: string }[] {
  const out: { paste: boolean; text: string }[] = [];
  const re = /<pasted_content[^>]*>([\s\S]*?)(?:<\/pasted_content>|$)/g;
  let at = 0;
  for (let m = re.exec(text); m; m = re.exec(text)) {
    if (m.index > at) out.push({ paste: false, text: text.slice(at, m.index) });
    out.push({ paste: true, text: m[1].replace(/^\n/, "") });
    at = re.lastIndex;
    if (m[0].length === 0) break;
  }
  if (at < text.length) out.push({ paste: false, text: text.slice(at) });
  return out;
}

/** A user record, drawn by what it is: a prompt, a local command, its output, or text the harness injected. */
export function UserMessage({ message: m, p }: { message: MessageRow; p: CardProps }) {
  const text = textOf(m);
  const first = m.blocks[0];
  const raw = first ? <RawButton block={first} onSelect={p.onSelect} selection={p.selection} /> : null;
  const selected = p.selection?.type === "message" && p.selection.id === m.id;
  const base = ["msg", selected && "selected", p.focused && "focused", m.origin !== "stored" && "derived"];
  const select = (e: MouseEvent) => {
    if ((e.target as HTMLElement).closest("button, a, summary, details, input, textarea")) return;
    p.onSelect({ type: "message", id: m.id });
  };
  const ts = <Timestamp value={m.timestamp} origin={m.ts_origin} />;
  const props = { id: `message-${m.id}`, "data-message-id": m.id, onClick: select };

  switch (m.content_class) {
    case "command":
      return (
        <article {...props} className={[...base, "msg-compact", "msg-command"].filter(Boolean).join(" ")}>
          <span className="badge badge-info">command</span> <code>{commandLine(text)}</code> {ts} {raw}
        </article>
      );
    case "command_output": {
      const out = tag(text, "local-command-stdout") ?? tag(text, "local-command-stderr") ?? text;
      return (
        <article {...props} className={[...base, "msg-compact", "msg-injected"].filter(Boolean).join(" ")}>
          {out ? (
            <details><summary><span className="badge badge-info">command output</span> {ts} {raw}</summary><TruncatedText text={out} /></details>
          ) : (
            <span className="muted"><span className="badge badge-info">command output</span> (empty) {ts} {raw}</span>
          )}
        </article>
      );
    }
    case "caveat":
    case "injected":
      return (
        <article {...props} className={[...base, "msg-compact", "msg-injected"].filter(Boolean).join(" ")}>
          <details>
            <summary><span className="badge badge-warning">injected</span> {injectedLabel(m, text)} {ts} {raw}</summary>
            <TruncatedText text={text} />
          </details>
        </article>
      );
    default:
      return (
        <article {...props} className={[...base, m.role === "tool" ? "msg-tool" : "msg-user"].filter(Boolean).join(" ")}>
          <header className="msg-head">
            <span className="role">{m.role === "tool" ? "tool result" : "user"}</span>
            {ts}
            {m.content_class === "paste" && <span className="badge badge-info">contains paste</span>}
            {raw}
          </header>
          {splitPastes(text).map((part, i) =>
            part.paste ? (
              <details key={i} className="paste">
                <summary><span className="badge badge-info">paste</span> {part.text.length} chars</summary>
                <TruncatedText text={part.text} />
              </details>
            ) : (
              <Markdown key={i} text={part.text} />
            ),
          )}
          {m.blocks.filter((b) => b.kind !== "text").map((b) => (
            <div key={b.id} className="turn-block block-other">
              <RawButton block={b} onSelect={p.onSelect} selection={p.selection} /><span className="muted">{b.kind} block</span>
              {b.kind === "tool_result" && b.tool_call_id === null && <span className="badge badge-warning">orphan result</span>}
              {b.text && <TruncatedText text={b.text} />}
            </div>
          ))}
        </article>
      );
  }
}
