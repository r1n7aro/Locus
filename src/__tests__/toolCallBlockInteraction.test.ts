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
});
