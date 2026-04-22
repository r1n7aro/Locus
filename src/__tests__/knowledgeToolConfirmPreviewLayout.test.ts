import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("KnowledgeToolConfirmPreview layout", () => {
  it("renders document previews as parallel before and after columns", () => {
    const preview = read("src/components/chat/KnowledgeToolConfirmPreview.vue");

    expect(preview).toContain("const documentPreviewPanels = computed<PreviewPanel[]>");
    expect(preview).toContain('t("chat.toolConfirm.knowledge.column.before")');
    expect(preview).toContain('t("chat.toolConfirm.knowledge.column.after")');
    expect(preview).toContain('class="preview-inline-grid"');
    expect(preview).toContain('class="preview-inline-panel"');
    expect(preview).toContain('class="preview-inline-label"');
    expect(preview).toContain('class="preview-excerpt preview-inline-code"');
    expect(preview).not.toContain("function buildDiffExcerpt");
    expect(preview).not.toContain('const prefix = line.kind === "add" ? "+" : line.kind === "delete" ? "-" : " ";');
  });

  it("uses natural-language diff summaries instead of git markers", () => {
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(zh).toContain('"chat.toolConfirm.knowledge.diffSummary": "新增 {0} 行 · 删除 {1} 行"');
    expect(zh).toContain('"chat.toolConfirm.knowledge.diffStats": "新增 {0} 行 · 删除 {1} 行 · {2} 处变更"');
    expect(en).toContain('"chat.toolConfirm.knowledge.diffSummary": "Added {0} lines · Removed {1} lines"');
    expect(en).toContain('"chat.toolConfirm.knowledge.diffStats": "Added {0} lines · Removed {1} lines · {2} hunks"');
  });
});
