import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("chat panel toggle layout", () => {
  it("anchors todo and changes toggles to the input area without reserving a full row", () => {
    const chatView = read("src/components/ChatView.vue");

    expect(chatView).toMatch(/<div v-if="!isViewingSubagent" class="input-area">[\s\S]*class="panel-toggle-row"[\s\S]*<RichChatInput/);
    expect(chatView).toContain(".input-area {");
    expect(chatView).toContain("position: relative;");
    expect(chatView).toContain(".panel-toggle-row {");
    expect(chatView).toContain("position: absolute;");
    expect(chatView).toContain("right: 48px;");
    expect(chatView).toContain("bottom: calc(100% + 8px);");
    expect(chatView).toContain("display: inline-flex;");
    expect(chatView).toContain("width: max-content;");
  });
});
