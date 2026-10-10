export function Loading() { return <p className="muted">Loading…</p>; }
export function ErrorBox({ error }: { error: unknown }) { return <div className="error-box">{String(error)}</div>; }
