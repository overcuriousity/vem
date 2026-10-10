import type { DiffResult } from "../api/types";
export function DiffView({ diff, mode }: { diff: DiffResult; mode: "unified" | "split" }) { return <pre>{diff.hunks.length} hunks ({mode})</pre>; }
