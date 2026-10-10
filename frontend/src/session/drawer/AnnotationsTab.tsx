import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "../../api/client";
import type { AnnotationKind, AnnotationTarget, MessageDetail, NewAnnotation } from "../../api/types";
import { ErrorBox } from "../../components/Status";
import type { Selection } from "../selection";

export function AnnotationsTab({ detail, selection }: { detail: MessageDetail; selection: Selection }) {
  const qc = useQueryClient();
  const targets: { label: string; type: AnnotationTarget; id: number }[] = [{ label: "this message", type: "message", id: detail.message.id }];
  if (selection.type === "block") targets.push({ label: "this block", type: "block", id: selection.id });
  if (selection.type === "toolcall") targets.push({ label: "this tool call", type: "tool_call", id: selection.id });
  const [target, setTarget] = useState(targets.length - 1);
  const [kind, setKind] = useState<AnnotationKind>("note");
  const [value, setValue] = useState("");
  const refresh = () => qc.invalidateQueries({ queryKey: ["message", detail.message.id] });
  // Wrapped so the api functions receive exactly one argument (TanStack passes a context as the second).
  const create = useMutation({ mutationFn: (a: NewAnnotation) => api.createAnnotation(a), onSuccess: () => { setValue(""); void refresh(); } });
  const remove = useMutation({ mutationFn: (id: number) => api.deleteAnnotation(id), onSuccess: () => void refresh() });
  return (
    <div>
      {detail.annotations.length === 0 ? (
        <p className="muted">No annotations yet.</p>
      ) : (
        <ul className="annotations">
          {detail.annotations.map((a) => (
            <li key={a.id}>
              <span className="badge badge-info">{a.kind}</span> {a.value}{" "}
              <span className="muted">on {a.target_type} {a.target_id} · {a.created_at}</span>{" "}
              <button type="button" aria-label={`Delete annotation ${a.id}`} onClick={() => remove.mutate(a.id)}>delete</button>
            </li>
          ))}
        </ul>
      )}
      <form
        className="annotation-form"
        onSubmit={(e) => {
          e.preventDefault();
          const t = targets[target];
          create.mutate({ target_type: t.type, target_id: t.id, kind, value });
        }}
      >
        <select aria-label="Annotation target" value={target} onChange={(e) => setTarget(Number(e.target.value))}>
          {targets.map((t, i) => <option key={t.type} value={i}>{t.label}</option>)}
        </select>
        <select aria-label="Annotation kind" value={kind} onChange={(e) => setKind(e.target.value as AnnotationKind)}>
          <option value="note">note</option><option value="tag">tag</option><option value="bookmark">bookmark</option>
        </select>
        <textarea aria-label="Annotation text" value={value} onChange={(e) => setValue(e.target.value)} rows={3} />
        <button type="submit" disabled={!value.trim() || create.isPending}>Add</button>
      </form>
      {create.error && <ErrorBox error={create.error} />}
      {remove.error && <ErrorBox error={remove.error} />}
      <p className="muted">Annotations are examiner work product; every change is written to the case audit log.</p>
    </div>
  );
}
