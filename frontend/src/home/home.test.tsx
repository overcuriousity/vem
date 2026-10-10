import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { RootOverview, SessionRow, SourceFileRow } from "../api/types";
import { buildTree } from "./tree";
import { nestSessions } from "./nest";
import { renderWithProviders } from "../test/render";
import CaseHome from "../pages/CaseHome";
import Sessions from "../pages/Sessions";
import { api } from "../api/client";

vi.mock("../api/client", () => ({
  api: { caseOverview: vi.fn(), roots: vi.fn(), rootFiles: vi.fn(), anomalies: vi.fn(), sessions: vi.fn(), audit: vi.fn(), raw: vi.fn() },
  ApiError: class extends Error {},
}));

const file = (id: number, rel_path: string, extra: Partial<SourceFileRow> = {}): SourceFileRow => ({
  id, root_id: 1, store_id: 1, rel_path, kind: "file", link_target: null, version: 1, size: 10, sha256: "ab".repeat(32),
  mtime: null, retained: true, parse_status: "parsed", parse_error: null, record_count: 1, anomaly_count: 0, ...extra,
});

test("buildTree nests files under directories, sorted", () => {
  const t = buildTree([file(1, "projects/b/x.jsonl"), file(2, "history.jsonl"), file(3, "projects/a/y.jsonl")]);
  expect(t.files.map((f) => f.rel_path)).toEqual(["history.jsonl"]);
  expect(t.dirs.map((d) => d.name)).toEqual(["projects"]);
  expect(t.dirs[0].dirs.map((d) => d.name)).toEqual(["a", "b"]);
  expect(t.dirs[0].dirs[1].files[0].rel_path).toBe("projects/b/x.jsonl");
});

const s = (id: number, parent: number | null, title: string): SessionRow => ({
  id, root_id: 1, store_id: 1, harness: "claude_code", harness_session_id: `h${id}`, kind: parent ? "subagent" : "primary",
  parent_session_id: parent, title, project_path: "/p", git_branch: null, harness_version: null, models: [], first_ts: null,
  first_ts_origin: "absent", last_ts: null, last_ts_origin: "absent", message_count: 1, tool_call_count: 0, anomaly_count: 0, child_count: 0,
});

test("nestSessions places children under their parent", () => {
  const rows = nestSessions([s(3, 1, "child"), s(1, null, "parent"), s(2, null, "other"), s(4, 99, "orphan child")]);
  expect(rows.map((r) => `${r.row.id}@${r.depth}`)).toEqual(["1@0", "3@1", "2@0", "4@0"]);
});

const overview: RootOverview = {
  root: { id: 1, path: "/evidence/.claude", label: "alice laptop", host: "h", user: "alice", os: "linux", harness: "claude_code", attached_at: "2026-10-09T10:00:00.000Z" },
  identification: ["history.jsonl carrying display and sessionId"],
  stores: [{ id: 1, root_id: 1, kind: "claude:projects", generation: "2.1.294", rel_path: "projects", discovery_method: "signature", status: "parsed", file_count: 6 }],
  absent: ["claude:todos"],
  ingest: { counts: { parsed: 6, failed: 1 }, failed: [{ id: 9, rel_path: "projects/x.jsonl", parse_error: "io error" }], last_ingest: null },
};

test("case home shows roots, absent stores, ingest status and filters anomalies via the URL", async () => {
  vi.mocked(api.caseOverview).mockResolvedValue({ info: { name: "Case 1", examiner: "e", created_at: "t", tool_version: "0.1.0" }, totals: { sessions: 5, messages: 23, tool_calls: 4, observations: 6, anomalies_info: 2, anomalies_warning: 1, anomalies_error: 1 } });
  vi.mocked(api.roots).mockResolvedValue([overview]);
  vi.mocked(api.anomalies).mockResolvedValue([
    { id: 1, root_id: 1, store_id: 1, source_file_id: 2, session_id: 5, kind: "missing_transcript", severity: "warning", byte_offset: 120, provenance_id: 7, message: "history references a deleted session", details: {} },
  ]);
  renderWithProviders(<CaseHome />);
  expect(await screen.findByText("alice laptop")).toBeInTheDocument();
  expect(screen.getByText(/claude:todos/)).toBeInTheDocument();
  expect(screen.getByText(/request/i)).toBeInTheDocument();
  expect(screen.getByText("projects/x.jsonl")).toBeInTheDocument();
  const table = await screen.findByRole("table", { name: "Anomalies" });
  expect(within(table).getByRole("link", { name: "session 5" })).toHaveAttribute("href", "/sessions/5");
  await userEvent.selectOptions(screen.getByRole("combobox", { name: "Severity" }), "error");
  await waitFor(() => expect(api.anomalies).toHaveBeenLastCalledWith({ severity: "error" }));
});

test("sessions page passes URL filters to the API and indents subagents", async () => {
  vi.mocked(api.sessions).mockResolvedValue([s(1, null, "parent"), s(3, 1, "child")]);
  vi.mocked(api.roots).mockResolvedValue([overview]);
  renderWithProviders(<Sessions />, { route: "/sessions?kind=primary&has_children=1", path: "/sessions" });
  expect(await screen.findByRole("link", { name: "parent" })).toHaveAttribute("href", "/sessions/1");
  expect(api.sessions).toHaveBeenCalledWith({ kind: "primary", has_children: true });
  expect(screen.getByRole("link", { name: "child" }).closest("td")).toHaveClass("depth-1");
});
