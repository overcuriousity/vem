export function Timestamp({ value, origin }: { value: string | null; origin: string }) { return <span>{value ?? "—"} {origin}</span>; }
