import { shortHash } from "../lib/format";
import "./components.css";

export function CopyHash({ sha, length = 12 }: { sha: string; length?: number }) {
  return (
    <span className="copyhash">
      <code title={sha}>{shortHash(sha, length)}</code>
      <button type="button" aria-label="Copy hash" title="Copy full hash" onClick={() => void navigator.clipboard?.writeText(sha)}>copy</button>
    </span>
  );
}
