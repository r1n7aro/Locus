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
    expect(source).toContain("await viewRun(viewId)");
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
});
