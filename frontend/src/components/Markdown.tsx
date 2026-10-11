import Md, { type Components } from "react-markdown";
import "./components.css";

interface MdNode { type: string; value?: string; children?: MdNode[] }

/** Raw HTML in evidence is shown as literal text: never rendered, never silently dropped. */
function htmlAsText() {
  const walk = (n: MdNode) => {
    if (n.type === "html") n.type = "text";
    n.children?.forEach(walk);
  };
  return walk;
}

// Links and images are not followed or fetched: a link shows its text and target, an image its alt text and source.
const components: Components = {
  a: ({ children, href }) => <span className="md-link">{children} <span className="muted mono">⟨{href}⟩</span></span>,
  img: ({ alt, src }) => <span className="md-link muted">[image {alt ? `“${alt}” ` : ""}<span className="mono">⟨{String(src ?? "")}⟩</span>]</span>,
};

/** Markdown for user and assistant text, sanitized: no raw HTML, no live links, no remote images. */
export function Markdown({ text }: { text: string }) {
  return (
    <div className="md">
      <Md remarkPlugins={[htmlAsText]} components={components} urlTransform={(url) => url}>{text}</Md>
    </div>
  );
}
