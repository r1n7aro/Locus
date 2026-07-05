import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("Knowledge injection preview", () => {
  it("adds injection preview as a dedicated special page in KnowledgeView", () => {
    const view = read("src/components/KnowledgeView.vue");

    expect(view).toContain('const specialPage = ref<null | "retrieval" | "injection">(null)');
    expect(view).toContain("openInjectionPreview");
    expect(view).toContain('knowledge.injectionPreview.entry');
    expect(view).toContain("<KnowledgeInjectionPreviewPanel");
  });

  it("filters injected items down to knowledge-related runtime blocks", () => {
    const panel = read("src/components/knowledge/KnowledgeInjectionPreviewPanel.vue");

    expect(panel).toContain("listAgentInjectedItems");
    expect(panel).toContain('item.id === "knowledge_context" || item.id.startsWith("knowledge_rule::")');
    expect(panel).toContain("function splitKnowledgeItem(item: InjectedPromptItem): InjectedPromptItem[]");
    expect(panel).toContain('line.match(/^###\\s+(.+)$/)');
    expect(panel).toContain('line.trim() === "## Knowledge"');
    expect(panel).not.toContain('knowledge.injectionPreview.section.overview');
  });

  it("shows a single estimated token line instead of the old summary strip", () => {
    const panel = read("src/components/knowledge/KnowledgeInjectionPreviewPanel.vue");

    expect(panel).toContain('import { estimateTextTokens } from "../../utils/tokenEstimate"');
    expect(panel).toContain('t("knowledge.injectionPreview.estimatedTokens")');
    expect(panel).not.toContain('class="injection-meta-strip"');
    expect(panel).not.toContain('knowledge.injectionPreview.subtitle');
  });

  it("builds the runtime knowledge prompt with the final section layout", () => {
    const runtime = read("src-tauri/src/agent/instance/mod.rs");
    const markdownEngine = read("src/composables/markdownEngine.ts");
    const markdownCodeLines = read("src/composables/markdownCodeLines.ts");

    expect(runtime).toContain('"## Knowledge\\n\\n{}"');
    expect(runtime).toContain('"### Structure"');
    expect(runtime).toContain('"```tree"');
    expect(runtime).toContain('"### Search"');
    expect(runtime).toContain('"### Maintenance"');
    expect(runtime).not.toContain("fn build_tools_section");
    expect(runtime).toContain('"### L2 Full Documents');
    expect(runtime).toContain('"## L3 Rules');
    expect(runtime).toContain('knowledge_rule::');
    expect(markdownEngine).toContain('normalizedLang === "tree"');
    expect(markdownEngine).toContain('renderHighlightedCodeLines(escapeMarkdownHtml(code), false)');
    expect(markdownCodeLines).toContain('code-line code-line-tree');
    expect(runtime).not.toContain("Knowledge-related guidance and runtime context are concentrated here.");
    expect(runtime).not.toContain('"### Index"');
  });
});
