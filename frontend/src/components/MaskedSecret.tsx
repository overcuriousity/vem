import { useState } from "react";
import "./components.css";

/** Shows the first 4 characters and masks the rest until the examiner chooses to reveal it. */
export function MaskedSecret({ value }: { value: string }) {
  const [shown, setShown] = useState(false);
  const masked = value.slice(0, 4) + "•".repeat(Math.min(12, Math.max(1, value.length - 4)));
  return (
    <span className="masked">
      <code>{shown ? value : masked}</code>
      <button type="button" aria-pressed={shown} onClick={() => setShown(!shown)}>{shown ? "hide" : "reveal"}</button>
    </span>
  );
}
