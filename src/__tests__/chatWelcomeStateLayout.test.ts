import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("chat welcome state layout", () => {
  it("renders the empty welcome state from ChatView instead of the transcript slot", () => {
    const chatView = read("src/components/ChatView.vue");

    expect(chatView).toContain("const showWelcomeState = computed(");
    expect(chatView).toContain("hasRenderableTranscriptMessage");
    expect(chatView).toContain("<div class=\"chat-main\">");
    expect(chatView).toContain("<div v-if=\"showWelcomeState\" class=\"chat-empty-overlay\">");
    expect(chatView).toContain(".chat-main {");
    expect(chatView).toContain(".chat-empty-overlay {");
    expect(chatView).not.toContain("<template #empty>");
  });
});
