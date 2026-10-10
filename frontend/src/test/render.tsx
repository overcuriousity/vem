import type { ReactElement } from "react";
import { render } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createMemoryRouter, RouterProvider } from "react-router";

/** Renders `ui` at `route` inside a fresh query client and a memory router whose single route is `path`. */
export function renderWithProviders(ui: ReactElement, { route = "/", path = "/" }: { route?: string; path?: string } = {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const router = createMemoryRouter([{ path, element: ui }, { path: "*", element: <div>elsewhere</div> }], { initialEntries: [route] });
  return { client, router, ...render(<QueryClientProvider client={client}><RouterProvider router={router} /></QueryClientProvider>) };
}
