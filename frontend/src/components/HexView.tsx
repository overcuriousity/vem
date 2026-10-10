export function HexView({ bytes, baseOffset }: { bytes: Uint8Array; baseOffset: number }) { return <pre>{bytes.length} bytes at {baseOffset}</pre>; }
