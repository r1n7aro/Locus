import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("tool call block interactions", () => {
  it("allows collapsed tool blocks to expand from block clicks", () => {
    const source = read("src/components/ToolCallBlock.vue");

    expect(source).toContain("function expandFromBlockClick(event: MouseEvent)");
    expect(source).toContain("@click=\"expandFromBlockClick\"");
    expect(source).toContain("@click.stop=\"toggleExpanded\"");
    expect(source).toContain(".tool-call-block:not(.is-expanded)");
  });

  it("exposes open actions for completed View tool calls", () => {
    const source = read("src/components/ToolCallBlock.vue");

    expect(source).toContain("const showViewOpenButton = computed");
    expect(source).toContain("resolveViewToolOpenId");
    expect(source).toContain("async function openViewTool()");
    expect(source).toContain("await viewRun(workspaceRef, viewId)");
    expect(source).toContain("@click.stop=\"openViewTool\"");
  });

  it("keeps override tool blocks aligned with the base block click behavior", () => {
    for (const relPath of [
      "src/components/tool-block-overrides/UnityExecuteToolBlock.vue",
      "src/components/tool-block-overrides/UnityRunStatesToolBlock.vue",
    ]) {
      const source = read(relPath);

      expect(source).toContain("function expandFromBlockClick(event: MouseEvent)");
      expect(source).toContain("@click=\"expandFromBlockClick\"");
      expect(source).toContain("@click.stop=\"toggleExpanded\"");
      expect(source).toContain(".unity-tool-call-block:not(.is-expanded)");
    }
  });

  it("preserves user-expanded tool collections and blocks across stream handoff", () => {
    const transcript = read("src/components/chat/ChatTranscript.vue");
    const collection = read("src/components/ToolCallCollection.vue");
    const block = read("src/components/ToolCallBlock.vue");

    expect(collection).toContain("initialExpanded?: boolean;");
    expect(collection).toContain('(e: "userExpansionChange", expanded: boolean): void;');
    expect(block).toContain("initialExpanded?: boolean;");
    expect(block).toContain('(e: "userExpansionChange", expanded: boolean): void;');
    expect(transcript).toContain(':initial-expanded="rememberedCollectionExpanded(segment.toolCalls)"');
    expect(transcript).toContain(':initial-expanded="rememberedBlockExpanded(toolCall)"');
  });

  it("keeps tool rows at one fixed height when completion actions appear", () => {
    const block = read("src/components/ToolCallBlock.vue");
    const actionRule = block.match(/\.tool-call-action-button\s*\{([^}]+)\}/)?.[1] ?? "";

    expect(actionRule).toContain("height: 22px;");
    expect(actionRule).toContain("min-height: 22px;");
    expect(actionRule).not.toContain("min-height: 24px;");
  });

  it("renders the loaded Skill status as secondary inline metadata", () => {
    const block = read("src/components/ToolCallBlock.vue");
    const noteRule = block.match(/\.tool-call-inline-note\s*\{([^}]+)\}/)?.[1] ?? "";

    expect(block).toContain(':title="skillLoadedLabel"');
    expect(block).toContain('.tool-call-inline-note::before');
    expect(noteRule).toContain("var(--text-secondary)");
    expect(noteRule).not.toContain("var(--status-good-bg)");
    expect(noteRule).not.toContain("var(--status-good-border)");
  });

  it("uses proportional collapsed labels and monospace expanded tool names", () => {
    const toolBlockFiles = [
      ["src/components/ToolCallBlock.vue", ".tool-call-block.is-expanded .tool-call-name"],
      ["src/components/tool-block-overrides/UnityExecuteToolBlock.vue", ".unity-tool-call-block.is-expanded .tool-call-name"],
      ["src/components/tool-block-overrides/UnityRunStatesToolBlock.vue", ".unity-tool-call-block.is-expanded .tool-call-name"],
      ["src/components/tool-block-overrides/KnowledgeQueryToolBlock.vue", ".knowledge-query-tool-block.is-expanded .tool-call-name"],
      ["src/components/tool-block-overrides/ExitPlanModeToolBlock.vue", ".exit-plan-tool-block.is-expanded .tool-call-name"],
    ];

    for (const [relPath, expandedSelector] of toolBlockFiles) {
      const source = read(relPath);
      const nameRule = source.match(/\.tool-call-name\s*\{([^}]+)\}/)?.[1] ?? "";
      const summaryRule = source.match(/\.tool-call-summary\s*\{([^}]+)\}/)?.[1] ?? "";
      const normalizedSource = source.replaceAll("\r\n", "\n");

      expect(nameRule).toContain("font-weight: 400;");
      expect(nameRule).toContain("font-family: var(--font-ui-label);");
      expect(nameRule).toContain("font-size: 13px;");
      expect(nameRule).toContain("color: var(--text-secondary);");
      expect(summaryRule).toContain("font-family: var(--font-mono-identifier);");
      expect(summaryRule).toContain("color: var(--text-secondary);");
      expect(normalizedSource).toContain(
        `${expandedSelector} {\n  color: var(--text-color);\n  font-family: var(--font-mono-identifier);\n  font-size: 12px;\n  font-weight: 600;\n}`,
      );
    }

    const collection = read("src/components/ToolCallCollection.vue");
    const batchTitleRule = collection.match(/\.tool-call-batch-title\s*\{([^}]+)\}/)?.[1] ?? "";

    expect(batchTitleRule).toContain("font-weight: 400;");
    expect(batchTitleRule).toContain("font-family: var(--font-ui-label);");
    expect(batchTitleRule).toContain("font-size: 13px;");
    expect(batchTitleRule).toContain("color: var(--text-secondary);");
    expect(collection).toMatch(
      /\.tool-call-batch-summary\.open \.tool-call-batch-title\s*\{\s*color: var\(--text-color\);/,
    );
    expect(collection).toMatch(
      /\.tool-call-collection\.is-expanded :deep\(\.tool-call-name\),\s*\.tool-call-collection:not\(\.is-collapsible\) :deep\(\.tool-call-name\)\s*\{\s*color: var\(--text-color\);\s*font-family: var\(--font-mono-identifier\);\s*font-size: 12px;\s*font-weight: 600;/,
    );
  });
});
