import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { BlockRow, MessageRow, ProvenanceRow, ToolCallRow } from "../api/types";
import { api } from "../api/client";
import { renderWithProviders } from "../test/render";
import Sessions from "../pages/Sessions";
import { ProvenanceTab } from "./drawer/ProvenanceTab";
import { RecordTab } from "./drawer/RecordTab";
import { Stream } from "./Stream";
import { streamItems } from "./turns";

vi.mock("../api/client", () => ({
  api: { provenance: vi.fn(), raw: vi.fn(), sessions: vi.fn(), roots: vi.fn() },
  ApiError: class extends Error {},
}));

// Shaped like `/api/sessions/{id}/messages` of a Cursor `cursorDiskKV` composer: a user bubble, then the
// assistant bubbles of that turn (`vem_turn`), with the tool result as its own `tool` record keyed to the turn.
let nextBlock = 900;
const block = (kind: string, extra: Partial<BlockRow> = {}): BlockRow => ({ id: nextBlock++, ordinal: 0, kind, text: null, payload: null, tool_use_id: null, tool_call_id: null, ...extra });
const T1 = "1c1c1c1c-0000-4000-8000-000000000001:0";
const msg = (id: number, role: string, blocks: BlockRow[], extra: Partial<MessageRow> = {}): MessageRow => ({
  id, session_id: 4, ordinal: id, role, harness_record_type: "bubble", harness_uuid: `b${id}`, parent_uuid: null,
  timestamp: "2026-01-01T00:00:01.000Z", ts_origin: "stored", model: null, provenance_id: id * 10, origin: "stored",
  attributes: role === "user" ? {} : { vem_turn: T1 }, blocks,
  turn_key: role === "user" ? null : T1, content_class: role === "tool" ? "tool_result" : role === "user" ? "prompt" : role, ...extra,
});

function cursorTurn() {
  const use = block("tool_use", { tool_use_id: "call_1", tool_call_id: 3 });
  const res = block("tool_result", { tool_use_id: "call_1", tool_call_id: 3, text: "fn main() {}" });
  const messages = [
    msg(1, "user", [block("text", { text: "Read main.rs" })]),
    msg(2, "assistant", [block("thinking", { text: "Look at the file." }), block("text", { text: "Reading it." })]),
    msg(3, "assistant", [use]),
    msg(4, "tool", [res], { harness_record_type: "bubble.toolFormerData.result" }),
    msg(5, "assistant", [block("text", { text: "It is empty." })]),
  ];
  const toolCalls: ToolCallRow[] = [{
    id: 3, session_id: 4, name: "read_file", category: "file_read", input: { target_file: "src/main.rs", should_read_entire_file: true },
    result_text: "fn main() {}", result_payload: null, is_error: false, started_ts: null, ended_ts: null, ts_origin: "stored",
    tool_use_block_id: use.id, tool_result_block_id: res.id,
  }];
  return { messages, toolCalls };
}

test("one_card_per_cursor_turn", () => {
  const items = streamItems(cursorTurn().messages, false);
  expect(items.map((i) => i.kind)).toEqual(["message", "turn"]);
  const t = items[1];
  expect(t.kind === "turn" && t.messages.map((m) => m.id)).toEqual([2, 3, 5]);
});

test("tool_result_inside_chip", async () => {
  const { messages, toolCalls } = cursorTurn();
  renderWithProviders(<Stream messages={messages} toolCalls={toolCalls} observations={[]} childSessions={[]} meta={false} selection={null} onSelect={() => {}} onShowMeta={() => {}} />);
  const card = document.getElementById("message-2")!;
  expect(document.getElementById("message-5")!.closest("article")).toBe(card);
  await userEvent.click(within(card).getByRole("button", { name: "read_file · src/main.rs · ok" }));
  expect(within(card).getByText("fn main() {}")).toBeInTheDocument();
});

const located: ProvenanceRow = {
  id: 11, source_file_id: 2, rel_path: "User/globalStorage/state.vscdb", rel_path_encoded: false, file_sha256: "a".repeat(64),
  root_path: "/evidence/ide", retained: true, byte_offset: 0, byte_length: 412, record_index: 0, content_sha256: "b".repeat(64),
  parser_name: "cursor-kv", parser_version: "0.4.0", origin: "stored", locator: "cursorDiskKV:bubbleId:c1:b1",
};

test("provenance_tab_shows_locator", async () => {
  vi.mocked(api.provenance).mockResolvedValue(located);
  renderWithProviders(<ProvenanceTab provenanceId={11} />);
  expect(await screen.findByText("cursorDiskKV:bubbleId:c1:b1")).toBeInTheDocument();
  expect(screen.getByText("Locator")).toBeInTheDocument();
  expect(screen.getByText("Content blob length")).toBeInTheDocument();
  expect(screen.queryByText("Byte offset")).not.toBeInTheDocument();
});

test("record_tab_hex_for_binary", async () => {
  // A protobuf node: field 1 (bytes, len 3) then a varint with high bits set; not valid UTF-8.
  const bytes = [0x0a, 0x03, 0xff, 0xfe, 0x00, 0x10, 0x96, 0x01];
  vi.mocked(api.raw).mockResolvedValue({ total_length: bytes.length, offset: 0, bytes_b64: btoa(String.fromCharCode(...bytes)), verified: true });
  renderWithProviders(<RecordTab provenanceId={12} />);
  expect(await screen.findByTestId("hex-row")).toBeInTheDocument();
  expect(screen.getByTestId("hex-bytes").textContent).toContain("0a 03 ff fe 00 10 96 01");
});

test("sessions_harness_options_match_model", async () => {
  vi.mocked(api.sessions).mockResolvedValue([]);
  vi.mocked(api.roots).mockResolvedValue([]);
  renderWithProviders(<Sessions />);
  const select = screen.getByRole("combobox", { name: "Harness" });
  expect([...select.querySelectorAll("option")].map((o) => o.value)).toEqual(["", "claude-code", "codex", "cursor", "cursor-ide"]);
  const kinds = [...screen.getByRole("combobox", { name: "Kind" }).querySelectorAll("option")].map((o) => o.value);
  expect(kinds).toContain("local_history");
});

// A Cursor agent-transcript user line: the adapter keeps the text verbatim and unwraps it into `vem_user_query`.
const WRAPPED = "<timestamp>Thursday, Oct 1, 2026, 9:30 AM (UTC+2)</timestamp>\n<user_query>\nlist the files\n</user_query>";
const renderUser = (m: MessageRow) =>
  renderWithProviders(<Stream messages={[m]} toolCalls={[]} observations={[]} childSessions={[]} meta={false} selection={null} onSelect={() => {}} onShowMeta={() => {}} />);

test("user_query_wrapper_rendered_as_query_with_embedded_timestamp", () => {
  renderUser(msg(1, "user", [block("text", { text: WRAPPED })], {
    timestamp: null, ts_origin: "absent",
    attributes: { vem_user_query: { text: "list the files", embedded_timestamp: "Thursday, Oct 1, 2026, 9:30 AM (UTC+2)", embedded_timestamp_utc: "2026-10-01T07:30:00.000Z" } },
  }));
  const card = document.getElementById("message-1")!;
  expect(card.textContent).not.toMatch(/<user_query>|<timestamp>/);
  expect(within(card).getByText("list the files")).toBeInTheDocument();
  const ts = within(card).getByText("2026-10-01T07:30:00.000Z");
  expect(ts).toHaveAttribute("title", expect.stringContaining("Thursday, Oct 1, 2026, 9:30 AM (UTC+2)"));
  expect(within(card).getByText("embedded")).toBeInTheDocument();
  // The record's own timestamp stays absent.
  expect(within(card).getByText("absent")).toBeInTheDocument();
});

test("user_query_unparsed_embedded_timestamp_kept_as_text", () => {
  renderUser(msg(1, "user", [block("text", { text: "<timestamp>later</timestamp><user_query>q</user_query>" })], {
    attributes: { vem_user_query: { text: "q", embedded_timestamp: "later" } },
  }));
  const card = document.getElementById("message-1")!;
  expect(within(card).getByText("later")).toBeInTheDocument();
  expect(within(card).getByText("embedded")).toBeInTheDocument();
});

test("user_text_without_unwrapped_query_is_verbatim", () => {
  renderUser(msg(1, "user", [block("text", { text: "<user_query>no close" })], { attributes: { vem_user_query: { text: 7 } } }));
  const card = document.getElementById("message-1")!;
  expect(card.textContent).toContain("<user_query>no close");
  expect(within(card).queryByText("embedded")).not.toBeInTheDocument();
});
