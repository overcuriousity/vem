import { useParams, useSearchParams } from "react-router";
import { useQuery } from "@tanstack/react-query";
import { api } from "../api/client";
import { ErrorBox, Loading } from "../components/Status";
import { Drawer } from "../session/Drawer";
import { SessionHeader } from "../session/SessionHeader";
import { Stream } from "../session/Stream";
import { SubagentTree } from "../session/SubagentTree";
import { formatSelection, parseSelection, type Selection } from "../session/selection";
import "../session/session.css";

export default function SessionView() {
  const id = Number(useParams().id);
  const [params, setParams] = useSearchParams();
  const meta = params.get("meta") === "1";
  const sel = parseSelection(params.get("sel"));
  const detail = useQuery({ queryKey: ["session", id], queryFn: () => api.session(id), enabled: Number.isFinite(id) });
  const messages = useQuery({ queryKey: ["messages", id], queryFn: () => api.messages(id, true), enabled: Number.isFinite(id) });
  const toolCalls = useQuery({ queryKey: ["toolCalls", id], queryFn: () => api.toolCalls(id), enabled: Number.isFinite(id) });
  const observations = useQuery({ queryKey: ["sessionObservations", id], queryFn: () => api.sessionObservations(id), enabled: Number.isFinite(id) });
  const update = (patch: Record<string, string | null>) => {
    const next = new URLSearchParams(params);
    for (const [k, v] of Object.entries(patch)) {
      if (v === null) next.delete(k);
      else next.set(k, v);
    }
    setParams(next, { replace: true });
  };
  if (!Number.isFinite(id)) return <ErrorBox error="Invalid session id." />;
  const error = detail.error ?? messages.error ?? toolCalls.error ?? observations.error;
  if (error) return <ErrorBox error={error} />;
  if (!detail.data || !messages.data || !toolCalls.data || !observations.data) return <Loading />;
  const select = (s: Selection | null) => update({ sel: s ? formatSelection(s) : null });
  return (
    <div className={`session-layout${sel ? " with-drawer" : ""}`}>
      <div className="session-main">
        <SessionHeader detail={detail.data} meta={meta} onMeta={(on) => update({ meta: on ? "1" : null })} />
        <SubagentTree detail={detail.data} />
        <Stream
          messages={messages.data} toolCalls={toolCalls.data} observations={observations.data} childSessions={detail.data.children}
          meta={meta} selection={sel} onSelect={select} onShowMeta={() => update({ meta: "1" })}
        />
      </div>
      {sel && <Drawer selection={sel} messages={messages.data} toolCalls={toolCalls.data} claims={detail.data.claims} onClose={() => select(null)} />}
    </div>
  );
}
