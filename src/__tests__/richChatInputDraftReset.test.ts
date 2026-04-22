import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("RichChatInput draft reset", () => {
  it("recomputes textarea height after clearing the draft", () => {
    const richInput = read("src/components/chat/RichChatInput.vue");

    expect(richInput).toContain("function resetDraft() {");
    expect(richInput).toContain('nextTick(() => {');
    expect(richInput).toContain("autoResizeTextarea();");
  });
});
