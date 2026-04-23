import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("chat panel toggle layout", () => {
  it("renders the file changes toggle in the composer action row instead of the transcript footer", () => {
    const chatView = read("src/components/ChatView.vue");
    const transcript = read("src/components/chat/ChatTranscript.vue");

    expect(chatView).toContain("const hasPanelToggleRow = computed(() => chatChangesStore.currentFileCount > 0);");
    expect(chatView).toContain("<template #top-start>");
    expect(chatView).toMatch(/<RichChatInput[\s\S]*<template #top-start>[\s\S]*<ModelSelector[\s\S]*@select="emit\('selectModel', \$event\)"/);
    expect(chatView).toMatch(/<RichChatInput[\s\S]*<template #top-start>[\s\S]*<ThinkingSelector[\s\S]*@select="emit\('selectEffort', \$event\)"/);
    expect(chatView).toMatch(/<RichChatInput[\s\S]*<template #top-start>[\s\S]*<ThinkingSelector[\s\S]*@select="emit\('selectEffort', \$event\)"[\s\S]*class="perm-toggle-btn ui-select-none"/);
    expect(chatView).toContain("<template #top-end>");
    expect(chatView).toMatch(/<RichChatInput[\s\S]*<template #top-end>[\s\S]*v-if="!isViewingSubagent && hasPanelToggleRow"[\s\S]*class="changes-toggle-btn ui-select-none"/);
    expect(chatView).not.toMatch(/<ChatTranscript[\s\S]*<template #footer>[\s\S]*hasPanelToggleRow/);
    expect(chatView).not.toContain("<AgentSelector");
    expect(chatView).not.toContain("{{ t('todo.title') }}");
    expect(chatView).not.toContain("chatStore.visibleTodoCount");
    expect(transcript).toContain("const hasFooterSlot = computed(() => !!slots.footer);");
    expect(transcript).toContain("const showSessionFooter = computed(");
    expect(transcript).toContain(":class=\"`is-${variant}`\"");
    expect(transcript).toContain(".chat-transcript-footer.is-session {");
    expect(transcript).toContain("justify-content: flex-start;");
    expect(transcript).not.toContain(".chat-transcript-footer.is-session.has-divider {");
    expect(chatView).toContain(".changes-toggle-btn {");
    expect(chatView).toContain("min-height: 28px;");
    expect(chatView).toContain("padding: 0 10px;");
  });

  it("keeps the toggle mounted during streaming and leaves the footer focused on token usage", () => {
    const chatView = read("src/components/ChatView.vue");

    expect(chatView).toContain("v-if=\"!isViewingSubagent && hasPanelToggleRow\"");
    expect(chatView).toContain(":disabled=\"isStreaming\"");
    expect(chatView).toMatch(/<template #footer>[\s\S]*<div class="footer-spacer"><\/div>[\s\S]*<TokenUsageBar/);
    expect(chatView).not.toMatch(/<template #footer>[\s\S]*class="perm-toggle-btn ui-select-none"/);
    expect(chatView).not.toContain("v-if=\"!isStreaming && chatChangesStore.currentFileCount > 0\"");
  });
});
