import { useEffect, useState } from "react";
import { Link } from "react-router";
import type { ObservationRow, SessionRow, ToolCallRow } from "../api/types";
import { JsonView } from "../components/JsonView";
import { TruncatedText } from "../components/TruncatedText";
import { chipSummary } from "./chip";
import { GLYPH } from "./runs";
import type { Selection } from "./selection";

/** One tool call as one row of a tool run: status glyph | name | argument | actions, with the inline body below.
 * `resultIds`: the tool-result records drawn in this row; each gets a `message-N` anchor, and selecting one opens the row. */
export function ToolChip({ tc, resultIds, observations, childSessions, selection, onSelect }: {
  tc: ToolCallRow; resultIds: number[]; observations: ObservationRow[]; childSessions: SessionRow[]; selection: Selection | null; onSelect: (s: Selection) => void;
}) {
  const selected = selection?.type === "toolcall" && selection.id === tc.id;
  const resultSelected = selection?.type === "message" && resultIds.includes(selection.id);
  const [open, setOpen] = useState(resultSelected);
  useEffect(() => {
    if (resultSelected) setOpen(true);
  }, [resultSelected]);
  const { name, arg, status } = chipSummary(tc);
  const mine = observations.filter((o) => o.derived_from_tool_call_id === tc.id);
  const fileObs = mine.find((o) => o.kind === "file_written" || o.kind === "file_edited");
  const agentId = mine.find((o) => o.kind === "subagent_spawned")?.details.agent_id;
  const child = typeof agentId === "string" ? childSessions.find((c) => c.harness_session_id === `agent-${agentId}` || c.harness_session_id === agentId) : undefined;
  const cls = status.replace(" ", "-");
  return (
    <li id={`toolcall-${tc.id}`} className={`call call-${cls}${selected ? " selected" : ""}`}>
      {resultIds.map((id) => <span key={id} id={`message-${id}`} className="anchor" />)}
      <button type="button" className="call-line" aria-label={`${name}${arg ? ` · ${arg}` : ""} · ${status}`} aria-expanded={open} onClick={() => setOpen(!open)}>
        <span className="call-status"><span aria-hidden="true">{GLYPH[status]}</span><span className="sr-only">{status}</span></span>
        <span className="call-name" title={name}>{name}</span>
        <span className="call-arg" title={arg || undefined}>
          {status === "error" && <span className="call-error-tag">error · </span>}
          {arg}
        </span>
      </button>
      <span className="call-actions">
        <button type="button" className="chip-inspect" aria-label={`Inspect ${name} call`} onClick={() => onSelect({ type: "toolcall", id: tc.id })}>details</button>
        {fileObs && <Link className="chip-link" to={`/activity/files?obs=${fileObs.id}`}>diff</Link>}
        {child && <Link className="chip-link" to={`/sessions/${child.id}`}>subagent session</Link>}
      </span>
      {open && (
        <div className="call-body">
          <h4>Input <button type="button" className="raw-ref" aria-label={`Raw record of the ${name} call`} onClick={() => onSelect({ type: "block", id: tc.tool_use_block_id })}>raw</button></h4>
          <JsonView value={tc.input} />
          <h4>
            Result <span className={`badge badge-${status === "error" ? "error" : status === "ok" ? "stored" : "absent"}`}>{status}</span>{" "}
            {tc.tool_result_block_id !== null && (
              <button type="button" className="raw-ref" aria-label={`Raw record of the ${name} result`} onClick={() => onSelect({ type: "block", id: tc.tool_result_block_id! })}>raw</button>
            )}
          </h4>
          {tc.result_text === null ? <p className="muted">No result recorded.</p> : <TruncatedText text={tc.result_text} />}
        </div>
      )}
    </li>
  );
}
