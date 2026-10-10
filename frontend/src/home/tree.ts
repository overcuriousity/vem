import type { SourceFileRow } from "../api/types";

export interface TreeNode { name: string; path: string; dirs: TreeNode[]; files: SourceFileRow[] }

/** The inventory as a directory tree from `rel_path`s; directories and files sorted by name, versions in order. */
export function buildTree(files: SourceFileRow[]): TreeNode {
  const root: TreeNode = { name: "", path: "", dirs: [], files: [] };
  for (const f of files) {
    const parts = f.rel_path.split("/");
    let node = root;
    for (const part of parts.slice(0, -1)) {
      let next = node.dirs.find((d) => d.name === part);
      if (!next) {
        next = { name: part, path: node.path ? `${node.path}/${part}` : part, dirs: [], files: [] };
        node.dirs.push(next);
      }
      node = next;
    }
    node.files.push(f);
  }
  const sort = (n: TreeNode) => {
    n.dirs.sort((a, b) => a.name.localeCompare(b.name));
    n.files.sort((a, b) => a.rel_path.localeCompare(b.rel_path) || a.version - b.version);
    n.dirs.forEach(sort);
  };
  sort(root);
  return root;
}
