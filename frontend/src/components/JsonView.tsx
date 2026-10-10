import "./components.css";

export function JsonView({ value }: { value: unknown }) {
  const text = value === undefined ? "—" : JSON.stringify(value, null, 2);
  return <pre className="json">{text}</pre>;
}
