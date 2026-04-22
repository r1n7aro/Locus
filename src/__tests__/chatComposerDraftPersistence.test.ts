import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("chat composer draft persistence", () => {
  it("stores drafts per session and restores the new-chat draft when switching back", () => {
    const chatView = read("src/components/ChatView.vue");

    expect(chatView).toContain('const NEW_CHAT_DRAFT_KEY = "__new_chat__";');
    expect(chatView).toContain("const composerDrafts = ref(new Map<string, string>());");
    expect(chatView).toContain("function draftSessionKey(sessionId: string | null)");
    expect(chatView).toContain("watch(inputText, (value) => {");
    expect(chatView).toContain("storeComposerDraft(props.activeSessionId, value);");
    expect(chatView).toContain("void restoreComposerDraft(nextSessionId ?? null);");
    expect(chatView).toContain("if (props.activeSessionId === null) {");
  });
});
