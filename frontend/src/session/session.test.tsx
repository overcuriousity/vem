import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { BlockRow, MessageRow, ToolCallRow, SessionDetail, SessionRow } from "../api/types";
import { chipSummary } from "./chip";
import { Stream, streamItems } from "./Stream";
import { renderWithProviders } from "../test/render";
import SessionView from "../pages/SessionView";
import { api } from "../api/client";

vi.mock("../api/client", () => ({
  api: {
    session: vi.fn(), messages: vi.fn(), toolCalls: vi.fn(), sessionObservations: vi.fn(),
    message: vi.fn(), raw: vi.fn(), provenance: vi.fn(), createAnnotation: vi.fn(), deleteAnnotation: vi.fn(),
  },
  ApiError: class extends Error {},
}));

let nextBlock = 100;
const block = (kind: string, extra: Partial<BlockRow> = {}): BlockRow => ({ id: nextBlock++, ordinal: 0, kind, text: null, payload: null, tool_use_id: null, tool_call_id: null, ...extra });
const msg = (id: number, role: string, blocks: BlockRow[]): MessageRow => ({
  id, session_id: 7, ordinal: id, role, harness_record_type: role, harness_uuid: null, parent_uuid: null,
  timestamp: "2026-09-30T10:00:00.000Z", ts_origin: "stored", model: null, provenance_id: id * 10, origin: "stored", attributes: {}, blocks,
});
const tool = (id: number, name: string, input: unknown, extra: Partial<ToolCallRow> = {}): ToolCallRow => ({
  id, session_id: 7, name, category: "shell", input, result_text: "README.md\n", result_payload: null, is_error: false,
  started_ts: null, ended_ts: null, ts_origin: "stored", tool_use_block_id: 0, tool_result_block_id: 1, ...extra,
});

test("chip summaries pick the key argument and status", () => {
  expect(chipSummary(tool(1, "Bash", { command: "ls -la", description: "List" }))).toEqual({ name: "Bash", arg: "ls -la", status: "ok" });
  expect(chipSummary(tool(2, "Edit", { file_path: "/p/notes.md" }, { is_error: true })).status).toBe("error");
  expect(chipSummary(tool(3, "Agent", { subagent_type: "Explore", description: "Find config files" }, { tool_result_block_id: null }))).toEqual({ name: "Agent", arg: "Find config files", status: "no result" });
  expect(chipSummary(tool(4, "Bash", { command: "a\n  b" })).arg).toBe("a b");
});

test("meta and system records are grouped into hidden dividers", () => {
  const ms = [msg(1, "user", []), msg(2, "meta", []), msg(3, "system", []), msg(4, "assistant", []), msg(5, "meta", [])];
  const items = streamItems(ms, false);
  expect(items.map((i) => (i.kind === "hidden" ? `hidden:${i.count}` : `m${i.message.id}`))).toEqual(["m1", "hidden:2", "m4", "hidden:1"]);
  expect(streamItems(ms, true)).toHaveLength(5);
});

test("stream: thinking collapsed, chips expand, big results truncated, divider shows meta", async () => {
  const useBlock = block("tool_use", { tool_call_id: 1, payload: { name: "Bash" } });
  const resultBlock = block("tool_result", { tool_call_id: 1, text: "x".repeat(70000) });
  const ms = [
    msg(1, "assistant", [block("thinking", { text: "List first." }), useBlock]),
    msg(2, "user", [resultBlock]),
    msg(3, "meta", [block("other", { payload: { type: "mode" } })]),
    msg(4, "meta", []),
  ];
  const tcs = [tool(1, "Bash", { command: "ls -la" }, { tool_use_block_id: useBlock.id, tool_result_block_id: resultBlock.id, result_text: "x".repeat(70000) })];
  const onShowMeta = vi.fn();
  renderWithProviders(<Stream messages={ms} toolCalls={tcs} observations={[]} childSessions={[]} meta={false} selection={null} onSelect={() => {}} onShowMeta={onShowMeta} />);
  const thinking = screen.getByText(/thinking \(11 chars\)/).closest("details")!;
  expect(thinking).not.toHaveAttribute("open");
  const chip = screen.getByRole("button", { name: /Bash · ls -la · ok/ });
  expect(chip).toHaveAttribute("aria-expanded", "false");
  await userEvent.click(chip);
  expect(screen.getByRole("button", { name: /show all/i })).toBeInTheDocument();
  expect(screen.getByText(/result of Bash/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "2 meta records hidden" }));
  expect(onShowMeta).toHaveBeenCalled();
});

const session: SessionRow = {
  id: 7, root_id: 1, store_id: 1, harness: "claude_code", harness_session_id: "s-7", kind: "primary", parent_session_id: null,
  title: "Add notes file", project_path: "/p", git_branch: "main", harness_version: "2.1", models: ["m"], first_ts: null,
  first_ts_origin: "absent", last_ts: null, last_ts_origin: "absent", message_count: 2, tool_call_count: 0, anomaly_count: 0, child_count: 1,
};

test("session view opens the drawer from the URL and adds annotations", async () => {
  const child = { ...session, id: 8, kind: "subagent", harness_session_id: "agent-1", title: "Explore", parent_session_id: 7 };
  const detail: SessionDetail = { session, ancestors: [], children: [child], claims: [] };
  const m2 = msg(2, "user", [block("text", { text: "hello" })]);
  vi.mocked(api.session).mockResolvedValue(detail);
  vi.mocked(api.messages).mockResolvedValue([msg(1, "assistant", [block("text", { text: "hi" })]), m2]);
  vi.mocked(api.toolCalls).mockResolvedValue([]);
  vi.mocked(api.sessionObservations).mockResolvedValue([]);
  vi.mocked(api.message).mockResolvedValue({ message: m2, tool_calls: [], observations: [], annotations: [] });
  vi.mocked(api.raw).mockResolvedValue({ total_length: 15, offset: 0, bytes_b64: btoa('{"type":"user"}'), verified: true });
  vi.mocked(api.createAnnotation).mockResolvedValue({ id: 1, target_type: "message", target_id: 2, kind: "note", value: "x", created_at: "t" });
  renderWithProviders(<SessionView />, { route: "/sessions/7?sel=message:2", path: "/sessions/:id" });
  expect(await screen.findByRole("heading", { name: "Add notes file" })).toBeInTheDocument();
  expect(screen.getByRole("link", { name: /Explore/ })).toHaveAttribute("href", "/sessions/8");
  const drawer = await screen.findByRole("complementary", { name: "Detail drawer" });
  expect(await within(drawer).findByText(/"type": "user"/)).toBeInTheDocument();
  await userEvent.click(within(drawer).getByRole("tab", { name: "Annotations" }));
  await userEvent.type(within(drawer).getByRole("textbox", { name: "Annotation text" }), "x");
  await userEvent.click(within(drawer).getByRole("button", { name: "Add" }));
  await waitFor(() => expect(api.createAnnotation).toHaveBeenCalledWith({ target_type: "message", target_id: 2, kind: "note", value: "x" }));
  await userEvent.click(within(drawer).getByRole("button", { name: "Close drawer" }));
  expect(screen.queryByRole("complementary", { name: "Detail drawer" })).not.toBeInTheDocument();
});

test("session view renders without a drawer when nothing is selected", async () => {
  vi.mocked(api.session).mockResolvedValue({ session, ancestors: [], children: [], claims: [] });
  vi.mocked(api.messages).mockResolvedValue([]);
  vi.mocked(api.toolCalls).mockResolvedValue([]);
  vi.mocked(api.sessionObservations).mockResolvedValue([]);
  renderWithProviders(<SessionView />, { route: "/sessions/7", path: "/sessions/:id" });
  expect(await screen.findByRole("heading", { name: "Add notes file" })).toBeInTheDocument();
  expect(screen.queryByRole("complementary")).not.toBeInTheDocument();
});
