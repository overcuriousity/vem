import { useState } from "react";
import { Link } from "react-router";
import type { ObservationRow, SessionRow, ToolCallRow } from "../api/types";
import { JsonView } from "../components/JsonView";
import { TruncatedText } from "../components/TruncatedText";
import { chipSummary } from "./chip";
import type { Selection } from "./selection";

export function ToolChip({ tc, observations, childSessions, selected, onSelect }: {
  tc: ToolCallRow; observations: ObservationRow[]; childSessions: SessionRow[]; selected: boolean; onSelect: (s: Selection) => void;
}) {
  const [open, setOpen] = useState(false);
  const { name, arg, status } = chipSummary(tc);
  const mine = observations.filter((o) => o.derived_from_tool_call_id === tc.id);
  const fileObs = mine.find((o) => o.kind === "file_written" || o.kind === "file_edited");
  const agentId = mine.find((o) => o.kind === "subagent_spawned")?.details.agent_id;
  const child = typeof agentId === "string" ? childSessions.find((c) => c.harness_session_id === `agent-${agentId}` || c.harness_session_id === agentId) : undefined;
  return (
    <div className={`chip chip-${status.replace(" ", "-")}${selected ? " selected" : ""}`}>
      <button type="button" className="chip-line" aria-expanded={open} onClick={() => setOpen(!open)}>
        {`${name}${arg ? ` · ${arg}` : ""} · ${status}`}
      </button>
      <button type="button" className="chip-inspect" aria-label={`Inspect ${name} call`} onClick={() => onSelect({ type: "toolcall", id: tc.id })}>details</button>
      {fileObs && <Link className="chip-link" to={`/activity/files?obs=${fileObs.id}`}>diff</Link>}
      {child && <Link className="chip-link" to={`/sessions/${child.id}`}>subagent session</Link>}
      {open && (
        <div className="chip-body">
          <h4>Input</h4>
          <JsonView value={tc.input} />
          <h4>Result</h4>
          {tc.result_text === null ? <p className="muted">No result recorded.</p> : <TruncatedText text={tc.result_text} />}
        </div>
      )}
    </div>
  );
}
