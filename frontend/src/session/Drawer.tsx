import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "../api/client";
import type { ClaimRow, MessageRow, ToolCallRow } from "../api/types";
import { ErrorBox, Loading } from "../components/Status";
import { formatSelection, messageIdOf, type Selection } from "./selection";
import { RecordTab } from "./drawer/RecordTab";
import { RawTab } from "./drawer/RawTab";
import { ProvenanceTab } from "./drawer/ProvenanceTab";
import { ObservationsTab } from "./drawer/ObservationsTab";
import { ClaimsTab } from "./drawer/ClaimsTab";
import { AnnotationsTab } from "./drawer/AnnotationsTab";

const TABS = ["Record", "Raw bytes", "Provenance", "Observations", "Claims", "Annotations"] as const;
type Tab = (typeof TABS)[number];

export function Drawer({ selection, messages, toolCalls, claims, onClose }: {
  selection: Selection; messages: MessageRow[]; toolCalls: ToolCallRow[]; claims: ClaimRow[]; onClose: () => void;
}) {
  const [tab, setTab] = useState<Tab>("Record");
  const mid = messageIdOf(selection, messages, toolCalls);
  const detail = useQuery({ queryKey: ["message", mid], queryFn: () => api.message(mid as number), enabled: mid !== null });
  const closeRef = useRef<HTMLButtonElement>(null);
  const opener = useRef<HTMLElement | null>(null);
  // Opening (or re-targeting) the drawer moves focus into it; remember what opened it. Without a focused opener
  // (a ?sel= link, Enter on a j/k focus), a tool call falls back to its details button.
  const key = formatSelection(selection);
  useEffect(() => {
    const active = document.activeElement;
    if (active instanceof HTMLElement && active !== document.body && !active.closest(".drawer")) opener.current = active;
    else if (!opener.current?.isConnected && selection.type === "toolcall") opener.current = document.querySelector<HTMLElement>(`#toolcall-${selection.id} .chip-inspect`);
    closeRef.current?.focus({ preventScroll: true });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  // Closing (× or Esc, both unmount the drawer) returns focus to the opener.
  useEffect(() => () => {
    if (opener.current?.isConnected) opener.current.focus({ preventScroll: true });
  }, []);
  const what = selection.type === "toolcall" ? "Tool call" : selection.type === "block" ? "Block" : "Message";
  return (
    <aside className="drawer" aria-label="Detail drawer">
      <header className="drawer-head">
        <strong>{what} {selection.id}</strong>
        <button ref={closeRef} type="button" aria-label="Close drawer" onClick={onClose}>×</button>
      </header>
      <div role="tablist" className="tabs">
        {TABS.map((t) => (
          <button key={t} type="button" role="tab" aria-selected={t === tab} onClick={() => setTab(t)}>{t}</button>
        ))}
      </div>
      <div role="tabpanel" className="drawer-body">
        {mid === null ? (
          <ErrorBox error="The selection is not part of this session." />
        ) : detail.error ? (
          <ErrorBox error={detail.error} />
        ) : !detail.data ? (
          <Loading />
        ) : tab === "Record" ? (
          <RecordTab provenanceId={detail.data.message.provenance_id} />
        ) : tab === "Raw bytes" ? (
          <RawTab provenanceId={detail.data.message.provenance_id} />
        ) : tab === "Provenance" ? (
          <ProvenanceTab provenanceId={detail.data.message.provenance_id} />
        ) : tab === "Observations" ? (
          <ObservationsTab observations={detail.data.observations} />
        ) : tab === "Claims" ? (
          <ClaimsTab claims={claims} />
        ) : (
          <AnnotationsTab detail={detail.data} selection={selection} />
        )}
      </div>
    </aside>
  );
}
