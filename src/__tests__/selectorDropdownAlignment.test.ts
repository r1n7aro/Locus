import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("selector dropdown alignment", () => {
  it("anchors the model selector dropdown to the trigger's leading edge", () => {
    const source = read("src/components/ModelSelector.vue");

    expect(source).toContain(".model-trigger {");
    expect(source).toContain("min-height: 28px;");
    expect(source).toContain(".model-dropdown {");
    expect(source).toContain("left: 0;");
    expect(source).toContain("right: auto;");
    expect(source).toContain("transform-origin: bottom left;");
  });

  it("anchors the thinking selector dropdown to the trigger's leading edge", () => {
    const source = read("src/components/ThinkingSelector.vue");

    expect(source).toContain(".thinking-trigger {");
    expect(source).toContain("min-height: 28px;");
    expect(source).toContain(".thinking-dropdown {");
    expect(source).toContain("left: 0;");
    expect(source).toContain("right: auto;");
    expect(source).toContain("transform-origin: bottom left;");
  });
});
