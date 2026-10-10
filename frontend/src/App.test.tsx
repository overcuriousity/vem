import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createMemoryRouter, RouterProvider } from "react-router";
import { routes } from "./App";

vi.mock("./api/client", () => ({
  api: new Proxy({}, { get: () => () => new Promise(() => {}) }),
  qs: () => "",
}));

test("shell shows every navigation link", () => {
  const router = createMemoryRouter(routes, { initialEntries: ["/audit"] });
  render(<QueryClientProvider client={new QueryClient()}><RouterProvider router={router} /></QueryClientProvider>);
  for (const name of ["Home", "Sessions", "Activity", "Search", "Export", "Audit"]) {
    expect(screen.getByRole("link", { name })).toBeInTheDocument();
  }
  expect(screen.getByRole("searchbox", { name: "Search case" })).toBeInTheDocument();
});
