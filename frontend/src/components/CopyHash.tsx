export function CopyHash({ sha, length = 12 }: { sha: string; length?: number }) { return <code title={sha}>{sha.slice(0, length)}</code>; }
