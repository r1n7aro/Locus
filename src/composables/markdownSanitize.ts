import DOMPurify, { type Config } from "dompurify";

const MARKDOWN_SANITIZE_CONFIG: Config = {
  USE_PROFILES: { html: true },
  ALLOW_ARIA_ATTR: true,
  ALLOW_DATA_ATTR: true,
  FORBID_ATTR: ["style"],
  FORBID_TAGS: ["script", "style", "iframe", "object", "embed", "form"],
  ADD_ATTR: ["draggable"],
};

export function sanitizeRenderedMarkdownHtml(html: string): string {
  if (!html) return "";
  return DOMPurify.sanitize(html, MARKDOWN_SANITIZE_CONFIG);
}
