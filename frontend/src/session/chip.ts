import type { ToolCallRow } from "../api/types";

export type ChipStatus = "ok" | "error" | "no result";
const ARG_KEYS = ["command", "file_path", "notebook_path", "path", "pattern", "url", "query", "description", "subagent_type", "prompt"];

export function oneLine(s: string, max = 80): string {
  const t = s.replace(/\s+/g, " ").trim();
  return t.length > max ? `${t.slice(0, max - 1)}…` : t;
}

/** `Bash · git status · ok`: tool name, its most telling argument, and whether a result was recorded. */
export function chipSummary(tc: ToolCallRow): { name: string; arg: string; status: ChipStatus } {
  const input = tc.input && typeof tc.input === "object" ? (tc.input as Record<string, unknown>) : {};
  let arg = "";
  for (const k of ARG_KEYS) {
    const v = input[k];
    if (typeof v === "string" && v.trim()) {
      arg = oneLine(v);
      break;
    }
  }
  const status: ChipStatus = tc.tool_result_block_id === null ? "no result" : tc.is_error ? "error" : "ok";
  return { name: tc.name, arg, status };
}
