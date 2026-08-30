// @vitest-environment jsdom
import { describe, expect, it } from "vitest";

import { prepareMarkdownCitations } from "../composables/markdownCitations";
import { markdownEngine } from "../composables/markdownEngine";
import { sanitizeRenderedMarkdownHtml } from "../composables/markdownSanitize";
import type { Citation } from "../types";

function urlCitation(overrides: Partial<Citation> = {}): Citation {
  return {
    id: "citation-1",
    kind: "url",
    url: "https://example.com/source",
    title: "Example source",
    ...overrides,
  };
}

describe("markdown citations", () => {
  it("consumes unresolved private citation markers during streaming", () => {
    const source = "结论\uE200cite\uE202turn5view0\uE201继续";
    expect(prepareMarkdownCitations(source)).toBe("结论继续");
  });

  it("replaces a referenced private marker with a safe clickable citation", () => {
    const marker = "\uE200cite\uE202turn5view0\uE201";
    const source = `结论${marker}`;
    const rendered = prepareMarkdownCitations(source, [urlCitation({
      startIndex: 2,
      endIndex: 2 + marker.length,
      referenceIds: ["turn5view0"],
    })]);

    expect(rendered).not.toContain("\uE200cite");
    expect(rendered).toContain('class="md-citation"');
    expect(rendered).toContain('href="https://example.com/source"');
    expect(rendered).toContain("<sup>[1]</sup>");
  });

  it("inserts public API citations at UTF-16 text offsets", () => {
    const source = "😀结论";
    const rendered = prepareMarkdownCitations(source, [urlCitation({
      startIndex: 2,
      endIndex: 4,
    })]);

    expect(rendered.indexOf("md-citation")).toBeGreaterThan(rendered.indexOf("结论"));
  });

  it("renders unsafe citation URLs as non-clickable source references", () => {
    const rendered = prepareMarkdownCitations("结果", [urlCitation({
      endIndex: 2,
      url: "javascript:alert(1)",
    })]);

    expect(rendered).toContain('<span class="md-citation"');
    expect(rendered).not.toContain("javascript:");
  });

  it("renders file citations as titled non-clickable references", () => {
    const rendered = prepareMarkdownCitations("文件", [{
      id: "citation-file",
      kind: "file",
      startIndex: 2,
      endIndex: 2,
      fileId: "file-1",
      filename: "design.pdf",
    }]);

    expect(rendered).toContain('title="design.pdf"');
    expect(rendered).toContain("<sup>[1]</sup>");
    expect(rendered).not.toContain("<a ");
  });

  it("survives the complete Markdown and sanitization pipeline", () => {
    const prepared = prepareMarkdownCitations("结论", [urlCitation({ endIndex: 2 })]);
    const html = sanitizeRenderedMarkdownHtml(markdownEngine.parse(prepared) as string);

    expect(html).toContain('class="md-citation"');
    expect(html).toContain('href="https://example.com/source"');
    expect(html).toContain('data-md-citation-id="citation-1"');
  });
});
