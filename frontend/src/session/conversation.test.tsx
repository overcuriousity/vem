import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { BlockRow, MessageRow, ToolCallRow } from "../api/types";
import { renderWithProviders } from "../test/render";
import { Stream } from "./Stream";
import { itemMessageId, streamItems } from "./turns";
import { commandLine, splitPastes } from "./UserMessage";

let nextBlock = 500;
const block = (kind: string, extra: Partial<BlockRow> = {}): BlockRow => ({ id: nextBlock++, ordinal: 0, kind, text: null, payload: null, tool_use_id: null, tool_call_id: null, ...extra });
const CLASS: Record<string, string> = { user: "prompt", tool: "tool_result" };
const msg = (id: number, role: string, blocks: BlockRow[], extra: Partial<MessageRow> = {}): MessageRow => ({
  id, session_id: 7, ordinal: id, role, harness_record_type: role === "tool" ? "user" : role, harness_uuid: null, parent_uuid: null,
  timestamp: "2026-10-01T09:00:00.000Z", ts_origin: "stored", model: null, provenance_id: id * 10, origin: "stored", attributes: {}, blocks,
  turn_key: null, content_class: CLASS[role] ?? role, ...extra,
});
const turn = (id: number, key: string, blocks: BlockRow[]) => msg(id, "assistant", blocks, { turn_key: key });
const call = (id: number, use: BlockRow, result: BlockRow | null, extra: Partial<ToolCallRow> = {}): ToolCallRow => ({
  id, session_id: 7, name: "Bash", category: "shell", input: { command: `cmd${id}` }, result_text: result?.text ?? null, result_payload: null,
  is_error: false, started_ts: null, ended_ts: null, ts_origin: "stored", tool_use_block_id: use.id, tool_result_block_id: result?.id ?? null, ...extra,
});

// The shape seen on Claude Code 2.1.29x: one API message streamed as several records, with its tool
// results (and harness bookkeeping) written between them.
function realisticSession() {
  const use1 = block("tool_use", { tool_call_id: 1 });
  const res1 = block("tool_result", { tool_call_id: 1, text: "test parse ... FAILED" });
  const use2 = block("tool_use", { tool_call_id: 2 });
  const res2 = block("tool_result", { tool_call_id: 2, text: "File does not exist." });
  const messages = [
    msg(1, "user", [block("text", { text: "<command-name>/clear</command-name>\n<command-message>clear</command-message>\n<command-args></command-args>" })], { content_class: "command" }),
    msg(2, "user", [block("text", { text: "Fix it.\n<pasted_content>\nerror: index 4\n</pasted_content>" })], { content_class: "paste" }),
    turn(3, "msg_A|req_A", [block("thinking", { text: "" })]),
    turn(4, "msg_A|req_A", [block("thinking", { text: "Run the tests first." })]),
    turn(5, "msg_A|req_A", [use1]),
    msg(6, "tool", [res1]),
    msg(7, "meta", []),
    turn(8, "msg_A|req_A", [use2]),
    msg(9, "tool", [res2]),
    msg(10, "user", [block("text", { text: "<system-reminder>\nThe user opened a file.\n</system-reminder>" })], { content_class: "injected" }),
    turn(11, "msg_B|req_B", [block("text", { text: "**Fixed.**\n\n- checks the bound\n\n```rust\nlet x = 1;\n```\n\n<script>alert(1)</script>\n\nSee [docs](https://example.invalid/) and ![logo](https://example.invalid/x.png)." })]),
    msg(12, "tool", [block("tool_result", { tool_use_id: "toolu_missing", text: "late output" })]),
    msg(13, "meta", []),
    msg(14, "meta", []),
  ];
  const toolCalls = [call(1, use1, res1), call(2, use2, res2, { is_error: true })];
  return { messages, toolCalls };
}

test("records of one API message become one turn across interleaved results and meta", () => {
  const { messages } = realisticSession();
  const items = streamItems(messages, false);
  expect(items.map((i) => (i.kind === "hidden" ? `hidden:${i.count}` : `${i.kind}:${itemMessageId(i)}`))).toEqual([
    "message:1", "message:2", "turn:3", "hidden:1", "message:10", "turn:11", "message:12", "hidden:2",
  ]);
  const t = items[2];
  expect(t.kind === "turn" && t.messages.map((m) => m.id)).toEqual([3, 4, 5, 8]);
  // With meta shown, a meta record is a card of its own and ends the turn.
  expect(streamItems(messages, true).filter((i) => i.kind === "turn")).toHaveLength(3);
});

test("a turn card: one redacted-thinking marker, results inside their chips, a raw button per block", async () => {
  const { messages, toolCalls } = realisticSession();
  const onSelect = vi.fn();
  renderWithProviders(<Stream messages={messages} toolCalls={toolCalls} observations={[]} childSessions={[]} meta={false} selection={null} onSelect={onSelect} onShowMeta={() => {}} />);
  const card = document.getElementById("message-3")!;
  expect(card.tagName).toBe("ARTICLE");
  expect(document.getElementById("message-8")!.closest("article")).toBe(card);
  expect(within(card).getAllByRole("button", { name: /thinking \(redacted\)/ })).toHaveLength(1);
  expect(within(card).queryByText(/thinking \(0 chars\)/)).not.toBeInTheDocument();
  expect(within(card).getByText(/thinking \(20 chars\)/)).toBeInTheDocument();
  expect(within(card).getByRole("button", { name: "Bash · cmd1 · ok" })).toBeInTheDocument();
  const failed = within(card).getByRole("button", { name: "Bash · cmd2 · error" });
  await userEvent.click(failed);
  expect(within(card).getByText("File does not exist.")).toBeInTheDocument();
  await userEvent.click(within(card).getByRole("button", { name: "Raw record of the Bash result" }));
  expect(onSelect).toHaveBeenLastCalledWith({ type: "block", id: toolCalls[1].tool_result_block_id });
  expect(screen.queryByText(/result of Bash/)).not.toBeInTheDocument();
  const thinkingBlock = messages[3].blocks[0];
  await userEvent.click(within(card).getByRole("button", { name: `Raw record of block ${thinkingBlock.id}` }));
  expect(onSelect).toHaveBeenLastCalledWith({ type: "block", id: thinkingBlock.id });
  // Two dividers: one for the meta record inside the turn, one for the trailing run of two.
  expect(screen.getAllByRole("button", { name: /meta records? hidden/ }).map((b) => b.textContent)).toEqual(["1 meta record hidden", "2 meta records hidden"]);
});

test("?sel=message:N on an absorbed tool result flashes the turn holding its chip and opens the chip", () => {
  const scroll = vi.fn();
  Element.prototype.scrollIntoView = scroll;
  const { messages, toolCalls } = realisticSession();
  for (const meta of [false, true]) {
    // Message 9 is the result of the call in message 8; meta record 7 sits between message 6's call and message 8.
    const { unmount } = renderWithProviders(<Stream messages={messages} toolCalls={toolCalls} observations={[]} childSessions={[]} meta={meta} selection={{ type: "message", id: 9 }} onSelect={() => {}} onShowMeta={() => {}} />);
    const anchor = document.getElementById("message-9")!;
    const card = anchor.closest("article")!;
    expect(card).toHaveClass("turn", "flash");
    expect(card.id).toBe(meta ? "message-8" : "message-3");
    expect(scroll.mock.contexts.at(-1)).toBe(card);
    expect(within(card).getByRole("button", { name: "Bash · cmd2 · error" })).toHaveAttribute("aria-expanded", "true");
    expect(within(card).getByText("File does not exist.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Bash · cmd1 · ok" })).toHaveAttribute("aria-expanded", "false");
    unmount();
  }
  delete (Element.prototype as Partial<Element>).scrollIntoView;
});

test("an absorbed result separated from its call by meta records is still anchored in the call's chip", () => {
  const use = block("tool_use", { tool_call_id: 30 });
  const res = block("tool_result", { tool_call_id: 30, text: "late result" });
  const messages = [turn(20, "msg_C|req_C", [use]), msg(21, "meta", []), msg(22, "system", []), msg(23, "tool", [res])];
  const toolCalls = [call(30, use, res)];
  for (const meta of [false, true]) {
    const { unmount } = renderWithProviders(<Stream messages={messages} toolCalls={toolCalls} observations={[]} childSessions={[]} meta={meta} selection={{ type: "message", id: 23 }} onSelect={() => {}} onShowMeta={() => {}} />);
    const card = document.getElementById("message-23")!.closest("article")!;
    expect(card.id).toBe("message-20");
    expect(card).toHaveClass("flash");
    expect(within(card).getByText("late result")).toBeInTheDocument();
    unmount();
  }
});

test("assistant text is markdown, sanitized: no raw HTML, no live links", () => {
  const { messages, toolCalls } = realisticSession();
  renderWithProviders(<Stream messages={messages} toolCalls={toolCalls} observations={[]} childSessions={[]} meta={false} selection={null} onSelect={() => {}} onShowMeta={() => {}} />);
  const card = document.getElementById("message-11")!;
  expect(within(card).getByText("Fixed.").tagName).toBe("STRONG");
  expect(within(card).getByRole("listitem")).toHaveTextContent("checks the bound");
  expect(within(card).getByText("let x = 1;").closest("pre")).not.toBeNull();
  expect(card.querySelector("script")).toBeNull();
  expect(card).toHaveTextContent("<script>alert(1)</script>");
  expect(card.querySelector("a")).toBeNull();
  expect(card).toHaveTextContent("docs ⟨https://example.invalid/⟩");
  expect(card.querySelector("img")).toBeNull();
  expect(card).toHaveTextContent("[image “logo” ⟨https://example.invalid/x.png⟩]");
});

test("harness-injected user records are drawn apart from prompts", async () => {
  const { messages, toolCalls } = realisticSession();
  renderWithProviders(<Stream messages={messages} toolCalls={toolCalls} observations={[]} childSessions={[]} meta={false} selection={null} onSelect={() => {}} onShowMeta={() => {}} />);
  const command = document.getElementById("message-1")!;
  expect(command).toHaveClass("msg-compact");
  expect(within(command).getByText("/clear")).toBeInTheDocument();
  const reminder = document.getElementById("message-10")!;
  expect(within(reminder).getByText("injected")).toBeInTheDocument();
  expect(reminder).toHaveTextContent("system reminder");
  expect(reminder.querySelector("details")).not.toHaveAttribute("open");
  const prompt = document.getElementById("message-2")!;
  expect(within(prompt).getByText("contains paste")).toBeInTheDocument();
  expect(prompt.querySelector("details.paste")).not.toHaveAttribute("open");
  expect(within(prompt).getByText("Fix it.")).toBeInTheDocument();
  const orphan = document.getElementById("message-12")!;
  expect(within(orphan).getByText("orphan result")).toBeInTheDocument();
  expect(orphan).toHaveTextContent("late output");
});

test("command lines and paste splitting", () => {
  expect(commandLine("<command-name>/model</command-name><command-args>opus</command-args>")).toBe("/model opus");
  expect(commandLine("<command-message>clear</command-message>")).toBe("/clear");
  expect(splitPastes("a <pasted_content>p</pasted_content> b")).toEqual([
    { paste: false, text: "a " }, { paste: true, text: "p" }, { paste: false, text: " b" },
  ]);
  expect(splitPastes("plain")).toEqual([{ paste: false, text: "plain" }]);
  // A mention of the tag is not a paste.
  expect(splitPastes("see `<pasted_content>` here")).toEqual([{ paste: false, text: "see `<pasted_content>` here" }]);
});
