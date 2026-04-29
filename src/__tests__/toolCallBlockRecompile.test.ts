import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("unity recompile tool block", () => {
  it("keeps the foreground hint inside the highlighted tool block", () => {
    const source = read("src/components/ToolCallBlock.vue");
    const attentionClassIndex = source.indexOf("'is-recompile-attention': showRecompileHint");
    const hintIndex = source.indexOf("class=\"recompile-hint\"");

    expect(source).toContain("const showRecompileHint = computed");
    expect(attentionClassIndex).toBeGreaterThanOrEqual(0);
    expect(hintIndex).toBeGreaterThan(attentionClassIndex);
    expect(source).toMatch(/\.tool-call-block\.is-recompile-attention\s*\{[\s\S]*align-items:\s*stretch/);
    expect(source).toMatch(/\.tool-call-block\.is-recompile-attention\s*\{[\s\S]*border:\s*1px solid var\(--status-warn-border\)/);
    expect(source).toMatch(/\.tool-call-block\.is-recompile-attention\s*\{[\s\S]*border-radius:\s*4px/);
    expect(source).toMatch(/\.recompile-hint\s*\{[\s\S]*align-self:\s*stretch/);
    expect(source).not.toContain("background: #2a2520;");
    expect(source).not.toContain("border-left: 3px solid #e8a838;");
  });
});
