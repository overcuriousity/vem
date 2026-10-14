import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import type { BlockRow, MessageRow, ToolCallRow } from "../api/types";
import { renderWithProviders } from "../test/render";
import { runNames, runStatus, turnParts } from "./runs";
import type { Selection } from "./selection";
import { Stream } from "./Stream";

let nextBlock = 700;
const block = (kind: string, extra: Partial<BlockRow> = {}): BlockRow => ({ id: nextBlock++, ordinal: 0, kind, text: null, payload: null, tool_use_id: null, tool_call_id: null, ...extra });
const msg = (id: number, role: string, blocks: BlockRow[], extra: Partial<MessageRow> = {}): MessageRow => ({
  id, session_id: 9, ordinal: id, role, harness_record_type: role, harness_uuid: null, parent_uuid: null,
  timestamp: "2026-10-01T09:00:00.000Z", ts_origin: "stored", model: null, provenance_id: id * 10, origin: "stored", attributes: {}, blocks,
  turn_key: role === "assistant" ? "T" : null, content_class: role === "tool" ? "tool_result" : role === "user" ? "prompt" : role, ...extra,
});
const call = (id: number, use: BlockRow, extra: Partial<ToolCallRow> = {}): ToolCallRow => ({
  id, session_id: 9, name: "read_file", category: "file_read", input: { target_file: `f${id}.rs` }, result_text: null, result_payload: null,
  is_error: false, started_ts: null, ended_ts: null, ts_origin: "stored", tool_use_block_id: use.id, tool_result_block_id: null, ...extra,
});

/** One assistant turn of `n` id-less Cursor tool calls with no results, as after MIK-213. */
function cursorRun(n: number) {
  const uses = Array.from({ length: n }, (_, i) => block("tool_use", { tool_call_id: 100 + i }));
  const toolCalls = uses.map((u, i) => call(100 + i, u));
  const messages = [msg(1, "user", [block("text", { text: "Look around." })]), msg(2, "assistant", uses)];
  return { messages, toolCalls };
}

const stream = (messages: MessageRow[], toolCalls: ToolCallRow[], selection: Selection | null = null, runsOpen: { open: boolean } | null = null) => (
  <Stream messages={messages} toolCalls={toolCalls} observations={[]} childSessions={[]} meta={false} selection={selection} onSelect={() => {}} onShowMeta={() => {}} runsOpen={runsOpen} />
);
const runs = () => [...document.querySelectorAll<HTMLDetailsElement>("details.tool-run")];

test("a run is a maximal sequence of tool calls; text and a tool use without a call end it", () => {
  const u = [1, 2, 3, 4].map((i) => block("tool_use", { tool_call_id: i }));
  const byId = new Map(u.map((b, i) => [i + 1, call(i + 1, b)]));
  const orphan = block("tool_use", { tool_call_id: null });
  const parts = turnParts([u[0], u[1], block("text", { text: "so" }), u[2], orphan, u[3]], byId);
  expect(parts.map((p) => (p.kind === "run" ? `run:${p.calls.map((c) => c.id).join(",")}` : p.block.kind))).toEqual(["run:1,2", "text", "run:3", "tool_use", "run:4"]);
});

test("run summary: names by frequency and the worst state first", () => {
  const u = block("tool_use");
  const ok = (id: number, name: string) => call(id, u, { name, tool_result_block_id: 1 });
  expect(runNames([ok(1, "Grep"), ok(2, "Read"), ok(3, "Read"), ok(4, "Shell")])).toBe("Read ×2, Grep, Shell");
  expect(runStatus([ok(1, "Read"), ok(2, "Read")])).toEqual({ worst: "ok", text: "✓ all ok" });
  expect(runStatus([call(1, u), call(2, u)])).toEqual({ worst: "no result", text: "○ 2 of 2 no result" });
  expect(runStatus([ok(1, "Read"), call(2, u), call(3, u, { tool_result_block_id: 1, is_error: true })])).toEqual({ worst: "error", text: "✕ 1 error · ○ 1 no result · ✓ 1 ok" });
});

test("runs of up to 5 calls start open, longer ones collapsed", () => {
  const five = cursorRun(5);
  const { unmount } = renderWithProviders(stream(five.messages, five.toolCalls));
  expect(runs()).toHaveLength(1);
  expect(runs()[0].open).toBe(true);
  unmount();
  const six = cursorRun(6);
  renderWithProviders(stream(six.messages, six.toolCalls));
  expect(runs()[0].open).toBe(false);
  expect(within(runs()[0]).getByText("6 tool calls")).toBeInTheDocument();
  expect(within(runs()[0]).getByText("○ 6 of 6 no result")).toBeInTheDocument();
});

test("30+ id-less Cursor calls: one collapsed run, status told by glyph and text", () => {
  const { messages, toolCalls } = cursorRun(32);
  renderWithProviders(stream(messages, toolCalls));
  expect(runs()).toHaveLength(1);
  expect(runs()[0].open).toBe(false);
  expect(within(runs()[0]).getByText("read_file ×32")).toBeInTheDocument();
  const row = document.getElementById("toolcall-110")!;
  expect(row).toHaveClass("call-no-result");
  expect(within(row).getByText("no result")).toHaveClass("sr-only");
  expect(within(row).getByRole("button", { name: "read_file · f110.rs · no result", hidden: true })).toHaveAttribute("aria-expanded", "false");
});

test("?sel=toolcall:N inside a collapsed run opens the run and marks the row", () => {
  const { messages, toolCalls } = cursorRun(32);
  renderWithProviders(stream(messages, toolCalls, { type: "toolcall", id: 120 }));
  expect(runs()[0].open).toBe(true);
  expect(document.getElementById("toolcall-120")).toHaveClass("selected");
});

test("a result anchor inside a collapsed run opens the run and the row", () => {
  const uses = Array.from({ length: 8 }, (_, i) => block("tool_use", { tool_call_id: 200 + i }));
  const res = block("tool_result", { tool_call_id: 203, text: "contents of f203" });
  const toolCalls = uses.map((u, i) => call(200 + i, u, i === 3 ? { tool_result_block_id: res.id, result_text: "contents of f203" } : {}));
  const messages = [msg(1, "user", [block("text", { text: "Go." })]), msg(2, "assistant", uses), msg(3, "tool", [res])];
  renderWithProviders(stream(messages, toolCalls, { type: "message", id: 3 }));
  expect(runs()[0].open).toBe(true);
  // The result record is anchored in its row, and the row's body is open on the result.
  expect(document.getElementById("message-3")!.closest("li")).toBe(document.getElementById("toolcall-203"));
  expect(screen.getByText("contents of f203")).toBeInTheDocument();
});

function WithToggle({ messages, toolCalls }: { messages: MessageRow[]; toolCalls: ToolCallRow[] }) {
  const [runsOpen, setRunsOpen] = useState<{ open: boolean } | null>(null);
  return (
    <>
      <button type="button" onClick={() => setRunsOpen({ open: true })}>expand all</button>
      <button type="button" onClick={() => setRunsOpen({ open: false })}>collapse all</button>
      {stream(messages, toolCalls, null, runsOpen)}
    </>
  );
}

test("expand all / collapse all sets every run; a run can still be toggled by hand", async () => {
  const a = cursorRun(7);
  const more = Array.from({ length: 2 }, (_, i) => block("tool_use", { tool_call_id: 300 + i }));
  const messages = [...a.messages, msg(3, "user", [block("text", { text: "Again." })]), msg(4, "assistant", more, { turn_key: "U" })];
  const toolCalls = [...a.toolCalls, ...more.map((u, i) => call(300 + i, u))];
  renderWithProviders(<WithToggle messages={messages} toolCalls={toolCalls} />);
  expect(runs().map((r) => r.open)).toEqual([false, true]);
  await userEvent.click(screen.getByRole("button", { name: "expand all" }));
  expect(runs().map((r) => r.open)).toEqual([true, true]);
  await userEvent.click(screen.getByRole("button", { name: "collapse all" }));
  expect(runs().map((r) => r.open)).toEqual([false, false]);
  await userEvent.click(runs()[1].querySelector("summary")!);
  expect(runs()[1].open).toBe(true);
  // A second collapse-all after a hand toggle still applies.
  await userEvent.click(screen.getByRole("button", { name: "collapse all" }));
  expect(runs()[1].open).toBe(false);
});
