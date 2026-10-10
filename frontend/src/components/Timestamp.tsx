import { OriginBadge } from "./OriginBadge";

export function Timestamp({ value, origin }: { value: string | null; origin: string }) {
  return (
    <span className="ts">
      <time className="mono" dateTime={value ?? undefined}>{value ?? "—"}</time> <OriginBadge origin={origin} />
    </span>
  );
}
