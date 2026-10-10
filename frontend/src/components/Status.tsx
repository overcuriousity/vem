import { ApiError } from "../api/client";
import "./components.css";

export function Loading() {
  return <p className="muted">Loading…</p>;
}

export function ErrorBox({ error }: { error: unknown }) {
  if (error instanceof ApiError && error.kind === "integrity_mismatch") {
    return (
      <div className="error-box" role="alert">
        <span className="integrity">Integrity failure:</span> {error.message}. The stored bytes no longer match the hash recorded at ingest.
      </div>
    );
  }
  const msg = error instanceof ApiError ? `${error.status} ${error.kind}: ${error.message}` : String(error);
  return <div className="error-box" role="alert">{msg}</div>;
}
