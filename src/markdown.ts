import { Marked } from "marked";
import DOMPurify from "dompurify";

const escapeHtml = (text: string) => text.replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);
const markdown = new Marked({
  gfm: true,
  breaks: true,
  renderer: {
    // A translated HTML snippet is source text, never UI. Images are labels,
    // not remote fetches; selected text must not cause network side effects.
    html: ({ text }) => escapeHtml(text),
    image: ({ text }) => escapeHtml(text),
  },
});

/** Render only passive document formatting in both result surfaces. Copy and
 * backfill use the original Markdown string, never text extracted from this DOM.
 * No active HTML, links, remote images, styles, IDs or event attributes survive. */
export function renderTranslation(node: HTMLElement, text: string, formatted: boolean) {
  node.classList.toggle("markdown", formatted);
  node.classList.toggle("empty", !text);
  if (!formatted) { node.textContent = text; return; }
  const html = markdown.parse(text, { async: false });
  node.innerHTML = DOMPurify.sanitize(html, {
    ALLOWED_TAGS: ["p", "br", "strong", "em", "del", "pre", "code", "h1", "h2", "h3", "h4", "h5", "h6", "ul", "ol", "li", "blockquote", "hr", "table", "thead", "tbody", "tr", "th", "td", "a"],
    ALLOWED_ATTR: ["start"],
    ALLOW_DATA_ATTR: false,
    ALLOW_ARIA_ATTR: false,
  });
}
