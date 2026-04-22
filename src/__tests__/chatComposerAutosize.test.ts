import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("ChatComposer autosize", () => {
  it("resizes against the current textarea value and recomputes after external clears", () => {
    const source = read("src/components/chat/ChatComposer.vue");

    expect(source).toContain("function resizeTextarea(textarea: HTMLTextAreaElement | null = textareaRef.value) {");
    expect(source).toContain('textarea.style.height = "auto";');
    expect(source).toContain("resizeTextarea(target);");
    expect(source).toContain('flush: "post"');
  });
});
