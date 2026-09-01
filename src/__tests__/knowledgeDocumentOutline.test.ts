import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { extractKnowledgeDocumentOutline } from "../components/knowledge/knowledgeDocumentOutline";

describe("knowledge document outline", () => {
  it("extracts ATX and setext headings in document order", () => {
    const outline = extractKnowledgeDocumentOutline([
      "# Overview",
      "",
      "## [Setup](./setup.md)",
      "",
      "Details",
      "-------",
      "",
      "### `Runtime` **rules** ###",
    ].join("\n"));

    expect(outline.map(({ level, text }) => ({ level, text }))).toEqual([
      { level: 1, text: "Overview" },
      { level: 2, text: "Setup" },
      { level: 2, text: "Details" },
      { level: 3, text: "Runtime rules" },
    ]);
    expect(outline.map((item) => item.from)).toEqual(
      [...outline.map((item) => item.from)].sort((left, right) => left - right),
    );
  });

  it("ignores heading-like text inside fenced code blocks", () => {
    const outline = extractKnowledgeDocumentOutline([
      "## Visible",
      "",
      "```md",
      "# Hidden",
      "```",
      "",
      "### Visible too",
    ].join("\n"));

    expect(outline.map((item) => item.text)).toEqual(["Visible", "Visible too"]);
  });

  it("uses a responsive, clickable rail beside the continuous document", () => {
    const preview = readFileSync(
      resolve(process.cwd(), "src/components/knowledge/KnowledgePreview.vue"),
      "utf8",
    );

    expect(preview).toContain('class="document-workspace"');
    expect(preview).toContain('class="document-outline"');
    expect(preview).toContain('@click="scrollToDocumentOutlineItem(item)"');
    expect(preview).toContain('@scroll.passive="scheduleDocumentOutlineActiveUpdate"');
    expect(preview).toMatch(/@container knowledge-document \(min-width: 1120px\)/);
    expect(preview).toMatch(/\.document-workspace\.has-outline \.document-outline\s*\{[\s\S]*position:\s*sticky;[\s\S]*top:\s*24px;/);
    expect(preview).toMatch(/\.document-outline-item\s*\{[\s\S]*border:\s*0;[\s\S]*background:\s*transparent;/);
  });
});
