import type { DiffLine, DiffResult, Hunk } from "../api/types";
import "./components.css";

const header = (h: Hunk) => `@@ -${h.old_start},${h.old_lines} +${h.new_start},${h.new_lines} @@`;
const cls = (l: DiffLine | undefined) => (l?.tag === "insert" ? "diff-insert" : l?.tag === "delete" ? "diff-delete" : "");

/** Pairs each run of deletions with the following run of insertions for side-by-side display. */
function splitRows(lines: DiffLine[]): [DiffLine | undefined, DiffLine | undefined][] {
  const rows: [DiffLine | undefined, DiffLine | undefined][] = [];
  let i = 0;
  while (i < lines.length) {
    if (lines[i].tag === "equal") {
      rows.push([lines[i], lines[i]]);
      i++;
      continue;
    }
    const dels: DiffLine[] = [];
    const ins: DiffLine[] = [];
    while (i < lines.length && lines[i].tag === "delete") dels.push(lines[i++]);
    while (i < lines.length && lines[i].tag === "insert") ins.push(lines[i++]);
    for (let k = 0; k < Math.max(dels.length, ins.length); k++) rows.push([dels[k], ins[k]]);
  }
  return rows;
}

export function DiffView({ diff, mode }: { diff: DiffResult; mode: "unified" | "split" }) {
  if (diff.binary) return <p className="muted">Binary content: no line diff. Inspect the blobs as bytes.</p>;
  if (diff.hunks.length === 0) return <p className="muted">No differences.</p>;
  return (
    <table className="diff">
      <tbody>
        {diff.hunks.map((h, hi) => [
          <tr className="hunk" key={`h${hi}`}><td colSpan={4}>{header(h)}</td></tr>,
          ...(mode === "unified"
            ? h.lines.map((l, li) => (
                <tr className={cls(l)} key={`${hi}-${li}`}>
                  <td className="ln">{l.old_no ?? ""}</td>
                  <td className="ln">{l.new_no ?? ""}</td>
                  <td className="ln">{l.tag === "insert" ? "+" : l.tag === "delete" ? "-" : " "}</td>
                  <td>{l.text}</td>
                </tr>
              ))
            : splitRows(h.lines).map(([a, b], li) => (
                <tr key={`${hi}-${li}`}>
                  <td className="ln">{a?.old_no ?? ""}</td>
                  <td className={cls(a)}>{a?.text ?? ""}</td>
                  <td className="ln">{b?.new_no ?? ""}</td>
                  <td className={cls(b)}>{b?.text ?? ""}</td>
                </tr>
              ))),
        ])}
      </tbody>
    </table>
  );
}
