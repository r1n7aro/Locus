import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("chat responsive layout", () => {
  it("lets the shared chat view collapse sessions into a recent-session dropdown", () => {
    const chatView = read("src/components/ChatView.vue");
    const picker = read("src/components/chat/SessionCompactPicker.vue");

    expect(chatView).toContain('layoutMode?: ChatLayoutMode;');
    expect(chatView).toContain("layoutModeChange: [mode: ResolvedChatLayoutMode]");
    expect(chatView).toContain("AUTO_VERTICAL_MIN_CHAT_WIDTH");
    expect(chatView).toContain("<SessionCompactPicker");
    expect(chatView).toContain("!showInlineDiff && !isVerticalLayout");
    expect(chatView).toContain("'is-vertical-layout': isVerticalLayout");
    expect(chatView).toContain(".chat-view.is-vertical-layout :deep(.chat-transcript-message.is-session)");
    expect(picker).toContain("MAX_RECENT_SESSIONS = 12");
    expect(picker).toContain("recentSessions");
    expect(picker).toContain('class="session-compact-dropdown"');
    expect(picker).toContain('class="session-compact-option"');
  });

  it("keeps Unity and the native app on the same chat workspace contract", () => {
    const app = read("src/App.vue");
    const unityView = read("src/components/UnityEmbeddedSessionView.vue");
    const workspace = read("src/components/ChatWorkspaceView.vue");
    const sidebar = read("src/components/ChatSidebarPanel.vue");

    expect(app).toContain("loadChatWorkspaceView");
    expect(app).toContain("await registerListeners();");
    expect(unityView).toContain("<ChatWorkspaceView");
    expect(workspace).toContain("<ChatView");
    expect(workspace).toContain(":layout=\"isVerticalLayout ? 'bottom' : 'side'\"");
    expect(workspace).toContain("saveRawContext");
    expect(sidebar).toContain("layout?: \"side\" | \"bottom\"");
    expect(sidebar).toContain("document.body.style.cursor = props.layout === \"bottom\" ? \"row-resize\" : \"col-resize\"");
  });
});
