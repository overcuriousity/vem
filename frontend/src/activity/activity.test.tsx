import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ActivityRow } from "../api/types";
import { renderWithProviders } from "../test/render";
import { renderSnippet } from "./snippet";
import Activity from "../pages/Activity";
import Search from "../pages/Search";
import Export from "../pages/Export";
import { api } from "../api/client";
import { render } from "@testing-library/react";

vi.mock("../api/client", () => ({
  api: { activity: vi.fn(), diff: vi.fn(), blob: vi.fn(), search: vi.fn(), runExport: vi.fn(), exports: vi.fn(), roots: vi.fn(), exportUrl: (n: string) => `/api/exports/${n}` },
  ApiError: class extends Error {},
}));

const row = (id: number, kind: string, extra: Partial<ActivityRow> = {}): ActivityRow => ({
  id, session_id: 5, kind, path: null, command: null, before_blob: null, after_blob: null, timestamp: "2026-09-30T10:00:05.000Z",
  ts_origin: "stored", confidence: "high", details: {}, derived_from_tool_call_id: 1, derived_from_block_id: null, derived_from_provenance_id: null,
  message_id: 12, session_title: "Add notes file", harness_session_id: "s1", ...extra,
});

test("snippet markers become highlights", () => {
  render(<p>{renderSnippet("…create a [notes] [file] and…")}</p>);
  expect(screen.getAllByText(/notes|file/, { selector: "mark" })).toHaveLength(2);
});

test("file operations open a diff panel linked to the originating message", async () => {
  vi.mocked(api.activity).mockResolvedValue([row(3, "file_edited", { path: "/p/notes.md", before_blob: "a".repeat(64), after_blob: "b".repeat(64) })]);
  vi.mocked(api.diff).mockResolvedValue({ binary: false, hunks: [{ old_start: 1, old_lines: 1, new_start: 1, new_lines: 2, lines: [{ tag: "insert", old_no: null, new_no: 2, text: "- first" }] }] });
  renderWithProviders(<Activity />, { route: "/activity/files", path: "/activity/:tab" });
  await userEvent.click(await screen.findByRole("button", { name: "/p/notes.md" }));
  await waitFor(() => expect(api.diff).toHaveBeenCalledWith("a".repeat(64), "b".repeat(64)));
  const panel = await screen.findByRole("region", { name: "Diff" });
  expect(within(panel).getByRole("link", { name: /originating message/ })).toHaveAttribute("href", "/sessions/5?sel=message:12");
  expect(api.activity).toHaveBeenCalledWith("files", {});
});

test("secret candidates are masked in the indicators table", async () => {
  vi.mocked(api.activity).mockResolvedValue([row(4, "secret_candidate", { details: { rule: "github-token", match: "ghp_0123456789abcdef", field: "tool_input" } })]);
  renderWithProviders(<Activity />, { route: "/activity/indicators", path: "/activity/:tab" });
  expect(await screen.findByText("github-token")).toBeInTheDocument();
  expect(screen.queryByText("ghp_0123456789abcdef")).not.toBeInTheDocument();
});

test("search groups hits by session and links to the message", async () => {
  vi.mocked(api.search).mockResolvedValue([
    { block_id: 1, message_id: 12, session_id: 5, tool_call_id: null, snippet: "the [notes] file", session_title: "Add notes file", message_ordinal: 3 },
    { block_id: 2, message_id: 14, session_id: 5, tool_call_id: 9, snippet: "cat [notes].md", session_title: "Add notes file", message_ordinal: 5 },
  ]);
  renderWithProviders(<Search />, { route: "/search?q=notes", path: "/search" });
  expect(await screen.findByRole("heading", { name: /Add notes file/ })).toBeInTheDocument();
  expect(screen.getAllByRole("link", { name: /message 3|message 5/ }).map((a) => a.getAttribute("href"))).toEqual(["/sessions/5?sel=message:12", "/sessions/5?sel=message:14"]);
  expect(api.search).toHaveBeenCalledWith("notes");
});

test("export runs with the chosen format and scope and lists the result", async () => {
  vi.mocked(api.exports).mockResolvedValue([]);
  vi.mocked(api.roots).mockResolvedValue([]);
  vi.mocked(api.runExport).mockResolvedValue({ name: "20261009T100000Z-case.csv", format: "timesketch-csv", output: "/c/exports/x.csv", events: 33, sha256: "c".repeat(64) });
  renderWithProviders(<Export />, { route: "/export", path: "/export" });
  await userEvent.selectOptions(await screen.findByRole("combobox", { name: "Format" }), "timesketch-csv");
  await userEvent.click(screen.getByRole("button", { name: "Run export" }));
  await waitFor(() => expect(api.runExport).toHaveBeenCalledWith({ format: "timesketch-csv", scope: { kind: "case" } }));
  expect(await screen.findByText(/33 events/)).toBeInTheDocument();
  expect(screen.getByRole("link", { name: /download/i })).toHaveAttribute("href", "/api/exports/20261009T100000Z-case.csv");
});
