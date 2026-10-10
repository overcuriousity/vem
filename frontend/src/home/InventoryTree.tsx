import { useQuery } from "@tanstack/react-query";
import { api } from "../api/client";
import type { SourceFileRow } from "../api/types";
import { CopyHash } from "../components/CopyHash";
import { ErrorBox, Loading } from "../components/Status";
import { formatBytes } from "../lib/format";
import { buildTree, type TreeNode } from "./tree";

function FileLine({ f }: { f: SourceFileRow }) {
  const name = f.rel_path.split("/").pop();
  return (
    <div className="file">
      <span>{name}</span>
      {f.version > 1 && <span className="badge badge-warning">v{f.version}</span>}
      {f.kind === "symlink" ? (
        <span className="badge badge-warning">symlink → {f.link_target} (not followed)</span>
      ) : (
        <>
          <span className="muted">{formatBytes(f.size)}</span>
          <CopyHash sha={f.sha256} />
        </>
      )}
      <span className={`badge ${f.parse_status === "failed" ? "badge-error" : "badge-info"}`}>{f.parse_status}</span>
      <span className="muted">{f.retained ? "retained" : "live only"}</span>
      {f.parse_error && <span className="muted">{f.parse_error}</span>}
    </div>
  );
}

function Dir({ node, open }: { node: TreeNode; open: boolean }) {
  return (
    <details open={open}>
      <summary>{node.name}/</summary>
      {node.dirs.map((d) => <Dir key={d.path} node={d} open={false} />)}
      {node.files.map((f) => <FileLine key={f.id} f={f} />)}
    </details>
  );
}

export function InventoryTree({ rootId }: { rootId: number }) {
  const q = useQuery({ queryKey: ["rootFiles", rootId], queryFn: () => api.rootFiles(rootId) });
  if (q.error) return <ErrorBox error={q.error} />;
  if (!q.data) return <Loading />;
  const tree = buildTree(q.data);
  return (
    <div className="tree">
      {tree.dirs.map((d) => <Dir key={d.path} node={d} open />)}
      {tree.files.map((f) => <FileLine key={f.id} f={f} />)}
    </div>
  );
}
