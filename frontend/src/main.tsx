import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider, createBrowserRouter } from "react-router";
import { routes } from "./App";
import "./styles/tokens.css";
import "./styles/base.css";

const queryClient = new QueryClient({ defaultOptions: { queries: { staleTime: 60_000, retry: false, refetchOnWindowFocus: false } } });
const router = createBrowserRouter(routes);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>
  </StrictMode>,
);
