import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { OriginBadge } from "./OriginBadge";
import { HexView } from "./HexView";
import { DiffView } from "./DiffView";
import { MaskedSecret } from "./MaskedSecret";
import { ErrorBox } from "./Status";
import { TruncatedText } from "./TruncatedText";
import { ApiError } from "../api/client";
import type { DiffResult } from "../api/types";

test("origin badges distinguish stored, inferred, absent and local clock", () => {
  const { container } = render(
    <>
      <OriginBadge origin="stored" />
      <OriginBadge origin="file_mtime" />
      <OriginBadge origin="neighbor_interpolated" />
      <OriginBadge origin="derived" />
      <OriginBadge origin="absent" />
      <OriginBadge origin="stored_local_clock" />
      <OriginBadge origin="something_new" />
    </>,
  );
  const classes = [...container.querySelectorAll(".badge")].map((b) => b.className);
  expect(classes).toEqual([
    "badge badge-stored", "badge badge-inferred", "badge badge-inferred", "badge badge-inferred",
    "badge badge-absent", "badge badge-local", "badge badge-inferred",
  ]);
  expect(screen.getByText("stored")).toHaveAttribute("title", expect.stringContaining("stored"));
});

test("hex view prints file-relative offsets, hex and ascii", () => {
  const bytes = new TextEncoder().encode('{"type":"user"}\nABCDEFGHIJKLMNOP');
  render(<HexView bytes={bytes} baseOffset={0x1f0} />);
  const rows = screen.getAllByTestId("hex-row");
  expect(rows).toHaveLength(2);
  expect(within(rows[0]).getByTestId("hex-offset")).toHaveTextContent("000001f0");
  expect(within(rows[1]).getByTestId("hex-offset")).toHaveTextContent("00000200");
  expect(within(rows[0]).getByTestId("hex-bytes")).toHaveTextContent("7b 22 74 79 70 65 22 3a 22 75 73 65 72 22 7d 0a"); // whitespace is normalized
  expect(within(rows[0]).getByTestId("hex-ascii")).toHaveTextContent('{"type":"user"}.');
});

const diff: DiffResult = {
  binary: false,
  hunks: [{
    old_start: 1, old_lines: 2, new_start: 1, new_lines: 2,
    lines: [
      { tag: "equal", old_no: 1, new_no: 1, text: "# Notes" },
      { tag: "delete", old_no: 2, new_no: null, text: "PASSWORD=" },
      { tag: "insert", old_no: null, new_no: 2, text: "PASSWORD=hunter2hunter2" },
    ],
  }],
};

test("diff view renders unified and split, and says when content is binary", () => {
  const { rerender, container } = render(<DiffView diff={diff} mode="unified" />);
  expect(screen.getByText("@@ -1,2 +1,2 @@")).toBeInTheDocument();
  expect(container.querySelectorAll(".diff-insert")).toHaveLength(1);
  expect(container.querySelectorAll(".diff-delete")).toHaveLength(1);
  rerender(<DiffView diff={diff} mode="split" />);
  const row = screen.getByText("PASSWORD=hunter2hunter2").closest("tr")!;
  expect(within(row).getByText("PASSWORD=")).toBeInTheDocument();
  rerender(<DiffView diff={{ binary: true, hunks: [] }} mode="unified" />);
  expect(screen.getByText(/binary content/i)).toBeInTheDocument();
  rerender(<DiffView diff={{ binary: false, hunks: [] }} mode="unified" />);
  expect(screen.getByText(/no differences/i)).toBeInTheDocument();
});

test("masked secrets reveal on request", async () => {
  render(<MaskedSecret value="ghp_0123456789abcdef" />);
  expect(screen.queryByText("ghp_0123456789abcdef")).not.toBeInTheDocument();
  expect(screen.getByText(/^ghp_•+$/)).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "reveal" }));
  expect(screen.getByText("ghp_0123456789abcdef")).toBeInTheDocument();
});

test("error box shows integrity failures prominently", () => {
  render(<ErrorBox error={new ApiError(409, "integrity_mismatch", "record bytes do not match")} />);
  expect(screen.getByRole("alert")).toHaveTextContent(/integrity failure/i);
  expect(screen.getByRole("alert")).toHaveTextContent("record bytes do not match");
});

test("truncated text shows a show-all button past the limit", async () => {
  render(<TruncatedText text={"x".repeat(100)} limit={10} />);
  expect(screen.getByText("x".repeat(10), { exact: false })).toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: /show all/i }));
  expect(screen.getByText("x".repeat(100))).toBeInTheDocument();
});
