import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("selector dropdown alignment", () => {
  it("anchors the model selector dropdown to the trigger's trailing edge", () => {
    const source = read("src/components/ModelSelector.vue");

    expect(source).toContain(".model-dropdown {");
    expect(source).toContain("right: 0;");
    expect(source).toContain("left: auto;");
    expect(source).toContain("transform-origin: bottom right;");
  });

  it("anchors the thinking selector dropdown to the trigger's trailing edge", () => {
    const source = read("src/components/ThinkingSelector.vue");

    expect(source).toContain(".thinking-dropdown {");
    expect(source).toContain("right: 0;");
    expect(source).toContain("left: auto;");
    expect(source).toContain("transform-origin: bottom right;");
  });
});
