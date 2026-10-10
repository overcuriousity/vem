import { Navigate, NavLink, Outlet, useNavigate, type RouteObject } from "react-router";
import { useQuery } from "@tanstack/react-query";
import { api } from "./api/client";
import CaseHome from "./pages/CaseHome";
import Sessions from "./pages/Sessions";
import SessionView from "./pages/SessionView";
import Activity from "./pages/Activity";
import Search from "./pages/Search";
import Export from "./pages/Export";
import Audit from "./pages/Audit";
import NotFound from "./pages/NotFound";

const LINKS: [string, string][] = [
  ["/", "Home"], ["/sessions", "Sessions"], ["/activity/commands", "Activity"], ["/search", "Search"], ["/export", "Export"], ["/audit", "Audit"],
];

export function Shell() {
  const navigate = useNavigate();
  const { data } = useQuery({ queryKey: ["case"], queryFn: api.caseOverview });
  return (
    <div className="shell">
      <header className="topbar">
        <span className="brand">vem</span>
        <span>{data?.info.name ?? "…"}</span>
        {data?.info.examiner && <span className="muted">examiner: {data.info.examiner}</span>}
        <nav>
          {LINKS.map(([to, label]) => (
            <NavLink key={to} to={to} end={to === "/"}>{label}</NavLink>
          ))}
        </nav>
        <form
          role="search"
          onSubmit={(e) => {
            e.preventDefault();
            const q = new FormData(e.currentTarget).get("q");
            if (typeof q === "string" && q.trim()) navigate(`/search?q=${encodeURIComponent(q.trim())}`);
          }}
        >
          <input name="q" type="search" placeholder="Search case…" aria-label="Search case" />
        </form>
      </header>
      <main className="content">
        <Outlet />
      </main>
    </div>
  );
}

export const routes: RouteObject[] = [
  {
    path: "/",
    element: <Shell />,
    children: [
      { index: true, element: <CaseHome /> },
      { path: "sessions", element: <Sessions /> },
      { path: "sessions/:id", element: <SessionView /> },
      { path: "activity", element: <Navigate to="/activity/commands" replace /> },
      { path: "activity/:tab", element: <Activity /> },
      { path: "search", element: <Search /> },
      { path: "export", element: <Export /> },
      { path: "audit", element: <Audit /> },
      { path: "*", element: <NotFound /> },
    ],
  },
];
