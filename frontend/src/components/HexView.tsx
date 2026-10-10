import "./components.css";

const hex2 = (b: number) => b.toString(16).padStart(2, "0");

/** 16 bytes per row: file-relative offset, hex (gap after 8), ASCII with `.` for non-printables. */
export function HexView({ bytes, baseOffset }: { bytes: Uint8Array; baseOffset: number }) {
  const rows = [];
  for (let i = 0; i < bytes.length; i += 16) {
    const chunk = bytes.subarray(i, i + 16);
    const hex = [...chunk].map(hex2);
    const hexText = [hex.slice(0, 8).join(" "), hex.slice(8).join(" ")].filter(Boolean).join("  ");
    const ascii = [...chunk].map((b) => (b >= 0x20 && b < 0x7f ? String.fromCharCode(b) : ".")).join("");
    rows.push(
      <div className="hex-row" data-testid="hex-row" key={i}>
        <span className="hex-offset" data-testid="hex-offset">{(baseOffset + i).toString(16).padStart(8, "0")}</span>
        <span className="hex-bytes" data-testid="hex-bytes">{hexText.padEnd(48, " ")}</span>
        <span className="hex-ascii" data-testid="hex-ascii">{ascii}</span>
      </div>,
    );
  }
  return <div className="hexview">{rows}</div>;
}
