import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  CHAT_WORKSPACE_CONTENT_MAX_WIDTH,
  resolveChatContentBalanceInset,
} from "../components/chat/chatSidebarBalance";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("centered chat sidebar stability", () => {
  it("balances the right sidebar only while the centered column still fits", () => {
    expect(CHAT_WORKSPACE_CONTENT_MAX_WIDTH).toBe(980);
    expect(resolveChatContentBalanceInset(1260, 280)).toBe(280);
    expect(resolveChatContentBalanceInset(1259, 280)).toBe(0);
    expect(resolveChatContentBalanceInset(1260, 0)).toBe(0);
    expect(resolveChatContentBalanceInset(Number.NaN, 280)).toBe(0);
  });

  it("tracks sidebar width and applies the balancing inset to ChatView", () => {
    const workspace = read("src/components/ChatWorkspaceView.vue");
    const chatView = read("src/components/ChatView.vue");

    expect(workspace).toContain("const assistantSidebarBalanceWidth = ref(0);");
    expect(workspace).toContain("function syncAssistantSidebarContentBalance");
    expect(workspace).toContain("resolveChatContentBalanceInset(");
    expect(workspace).toContain('workspaceRef.value.querySelector<HTMLElement>(".chat-view")');
    expect(workspace).toContain("connectAssistantSidebarResizeObserver(shell);");
    expect(workspace).toContain("assistantSidebarResizeObserver.observe(shell);");
    expect(workspace).toContain("assistantSidebarResizeObserver.observe(chatSurface);");
    expect(workspace).toContain("disconnectAssistantSidebarResizeObserver();");
    expect(workspace).toContain(':content-start-inset="assistantSidebarBalanceWidth"');
    expect(chatView).toContain("contentStartInset?: number;");
    expect(chatView).toContain("const chatContentStyle = computed");
    expect(chatView).toContain("paddingLeft: `${Math.max(0, props.contentStartInset ?? 0)}px`");
    expect(chatView).toContain(':style="chatContentStyle"');
  });
});
