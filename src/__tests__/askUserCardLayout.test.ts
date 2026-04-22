import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("AskUserCard layout", () => {
  it("keeps option text within the button width so long descriptions can wrap", () => {
    const card = read("src/components/chat/AskUserCard.vue");

    expect(card).toContain(".ask-option-btn");
    expect(card).toContain("min-width: 0;");
    expect(card).toContain(".ask-option-label,");
    expect(card).toContain(".ask-option-desc");
    expect(card).toContain("display: block;");
    expect(card).toContain("width: 100%;");
    expect(card).toContain("white-space: normal;");
    expect(card).toContain("overflow-wrap: anywhere;");
  });
});
