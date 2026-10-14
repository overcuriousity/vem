import { useEffect, useState } from "react";
import type { ToolCallRow } from "../api/types";
import { RUN_OPEN_MAX, runHoldsSelection, runNames, runStatus } from "./runs";
import { ToolChip } from "./ToolChip";
import type { CardProps } from "./TurnView";

/** Consecutive tool calls of one turn as one recessed band; a run of more than `RUN_OPEN_MAX` calls starts collapsed.
 * A selection inside the run (the call, or one of its result records) opens it; the header's expand/collapse-all
 * sets every run. The rows stay in the DOM while collapsed, so their `message-N` anchors resolve. */
export function ToolRun({ calls, p }: { calls: ToolCallRow[]; p: CardProps }) {
  const holds = runHoldsSelection(calls, p.resultsByCall, p.selection);
  const [open, setOpen] = useState(holds || (p.runsOpen ? p.runsOpen.open : calls.length <= RUN_OPEN_MAX));
  const selKey = holds && p.selection ? `${p.selection.type}:${p.selection.id}` : null;
  useEffect(() => {
    if (!selKey || !p.selection) return;
    setOpen(true);
    if (p.selection.type === "toolcall") document.getElementById(`toolcall-${p.selection.id}`)?.scrollIntoView?.({ block: "nearest" });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selKey]);
  useEffect(() => {
    if (p.runsOpen) setOpen(p.runsOpen.open || holds);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [p.runsOpen]);
  const names = runNames(calls);
  const status = runStatus(calls);
  return (
    <details className={`tool-run run-${status.worst.replace(" ", "-")}`} open={open} onToggle={(e) => setOpen(e.currentTarget.open)}>
      <summary>
        <span className="tool-run-label">{calls.length} {calls.length === 1 ? "tool call" : "tool calls"}</span>
        <span className="tool-run-names" title={names}>{names}</span>
        <span className="tool-run-status">{status.text}</span>
      </summary>
      <ol>
        {calls.map((tc) => (
          <ToolChip key={tc.id} tc={tc} resultIds={p.resultsByCall.get(tc.id) ?? []} selection={p.selection} observations={p.observations} childSessions={p.childSessions} onSelect={p.onSelect} />
        ))}
      </ol>
    </details>
  );
}
