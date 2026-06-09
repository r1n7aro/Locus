import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("Markdown table styles", () => {
  it("sanitizes rendered markdown before v-html receives plugin descriptions", () => {
    const renderer = read("src/components/MarkdownRenderer.vue");
    const sanitizer = read("src/composables/markdownSanitize.ts");

    expect(sanitizer).toContain('import DOMPurify, { type Config } from "dompurify";');
    expect(sanitizer).toContain("FORBID_ATTR: [\"style\"]");
    expect(sanitizer).toContain("FORBID_TAGS: [\"script\", \"style\", \"iframe\", \"object\", \"embed\", \"form\"]");
    expect(sanitizer).toContain("ALLOW_DATA_ATTR: true");
    expect(sanitizer).toContain("ADD_ATTR: [\"draggable\"]");
    expect(renderer).toContain("import { sanitizeRenderedMarkdownHtml } from \"../composables/markdownSanitize\";");
    expect(renderer).toContain("return sanitizeRenderedMarkdownHtml(html);");
    expect(renderer).toContain("return sanitizeRenderedMarkdownHtml(escapeHtml(props.content));");
    expect(renderer).toContain('v-html="renderedHtml"');
  });

  it("wraps rendered tables and constrains cell styling in MarkdownRenderer", () => {
    const source = read("src/components/MarkdownRenderer.vue");

    expect(source).toContain('wrapMarkdownTables(html)');
    expect(source).toMatch(/\.markdown-body \.md-table-wrap\s*\{[\s\S]*width:\s*fit-content;[\s\S]*overflow-x:\s*auto;/);
    expect(source).toMatch(/\.markdown-body table\s*\{[\s\S]*width:\s*max-content;[\s\S]*min-width:\s*100%;/);
    expect(source).toMatch(/\.markdown-body th,\s*[\s\S]*\.markdown-body td\s*\{[\s\S]*overflow-wrap:\s*anywhere;[\s\S]*border-right:[\s\S]*!important;[\s\S]*border-bottom:[\s\S]*!important;/);
    expect(source).toMatch(/tbody tr:nth-child\(even\) td\s*\{[\s\S]*!important;/);
  });

  it("supports optional search-term highlights in MarkdownRenderer", () => {
    const source = read("src/components/MarkdownRenderer.vue");

    expect(source).toContain("highlightTerms?: string[];");
    expect(source).toContain("function normalizeHighlightTerms(terms?: string[]): string[] {");
    expect(source).toContain('mark.className = "markdown-search-mark";');
    expect(source).toContain("html = highlightHtml(html, highlightTerms);");
    expect(source).toMatch(/\.markdown-body mark\.markdown-search-mark\s*\{[\s\S]*border-radius:\s*4px;[\s\S]*background:[\s\S]*box-shadow:/);
    expect(source).toMatch(/\.markdown-body mark\.markdown-search-mark-target\s*\{[\s\S]*background:[\s\S]*box-shadow:/);
  });

  it("keeps editor table cells readable under the same theme constraints", () => {
    const source = read("src/components/ui/BaseMarkdownEditor.vue");

    expect(source).toMatch(/\.base-markdown-editor :deep\(\.vditor-reset table\)\s*\{[\s\S]*width:\s*max-content;[\s\S]*min-width:\s*100%;/);
    expect(source).toMatch(/\.base-markdown-editor :deep\(\.vditor-reset th\),\s*[\s\S]*\.base-markdown-editor :deep\(\.vditor-reset td\)\s*\{[\s\S]*overflow-wrap:\s*anywhere;[\s\S]*border-right:[\s\S]*!important;[\s\S]*border-bottom:[\s\S]*!important;/);
    expect(source).toMatch(/tbody tr:nth-child\(even\) td\)\s*\{[\s\S]*!important;/);
  });
});
