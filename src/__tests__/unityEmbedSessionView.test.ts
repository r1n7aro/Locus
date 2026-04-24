import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("Unity embedded session view", () => {
  it("routes the Unity overlay to the shared chat workspace", () => {
    const app = read("src/App.vue");
    const view = read("src/components/UnityEmbeddedSessionView.vue");
    const workspace = read("src/components/ChatWorkspaceView.vue");
    const command = read("src-tauri/src/commands/unity_embed.rs");

    expect(command).toContain('const EMBED_URL: &str = "/unity-embed?host=tauri-overlay";');
    expect(app).toContain("const UnityEmbeddedSessionView = defineAsyncComponent");
    expect(app).toContain("isUnityEmbedWindow");
    expect(app).toContain("<UnityEmbeddedSessionView");
    expect(app).toContain("await bootstrapCritical();");
    expect(app).toContain("await registerListeners();");
    expect(view).toContain("<ChatWorkspaceView");
    expect(view).toContain("layout-mode=\"auto\"");
    expect(workspace).toContain("<ChatView");
    expect(workspace).toContain("<ThinkingPanel");
    expect(workspace).toContain("<ChatSidebarPanel");
    expect(workspace).toContain("@layout-mode-change=\"handleLayoutModeChange\"");
    expect(view).not.toContain("useEmbeddedChatSession");
    expect(view).not.toContain("<EmbeddedChatPane");
  });
});
