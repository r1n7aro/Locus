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

    expect(command).toContain(String.raw`const CONTROL_PIPE_NAME_PREFIX: &str = r"\\.\pipe\locus_tauri_unity_embed_";`);
    expect(command).toContain('const EMBED_URL: &str = "/unity-embed?host=tauri-overlay";');
    expect(app).toContain("const UnityEmbeddedSessionView = defineAsyncComponent");
    expect(app).toContain("isUnityEmbedWindow");
    expect(app).toContain("<UnityEmbeddedSessionView");
    expect(app).toContain("await bootstrapCritical();");
    expect(app).toContain("await registerListeners();");
    expect(view).toContain("<ChatWorkspaceView");
    expect(view).toContain("layout-mode=\"auto\"");
    expect(view).not.toContain("default-session-panel-collapsed");
    expect(view).toContain('session-panel-storage-scope="unity"');
    expect(view).toContain("box-shadow: inset 0 1px 0 color-mix(in srgb, var(--border-color) 82%, var(--text-secondary) 18%);");
    expect(workspace).toContain("<ChatView");
    expect(workspace).toContain(':default-session-panel-collapsed="defaultSessionPanelCollapsed"');
    expect(workspace).toContain(':session-panel-storage-scope="sessionPanelStorageScope"');
    expect(workspace).toContain("defaultSessionPanelCollapsed: false");
    expect(workspace).toContain("<ThinkingPanel");
    expect(workspace).toContain("<ChatSidebarPanel");
    expect(workspace).toContain(':storage-scope="sessionPanelStorageScope"');
    expect(workspace).toContain("@layout-mode-change=\"handleLayoutModeChange\"");
    expect(view).not.toContain("useEmbeddedChatSession");
    expect(view).not.toContain("<EmbeddedChatPane");
  });

  it("boosts Unity overlay sync while the editor window is resizing", () => {
    const unityWindow = read("locus_unity/Editor/LocusEditorWindow.cs");

    expect(unityWindow).toContain("private const double ResizeSyncIntervalSeconds = 1d / 60d;");
    expect(unityWindow).toContain("private const double ResizeBoostDurationSeconds = 0.35d;");
    expect(unityWindow).toContain("resizeBoostActive ? ResizeSyncIntervalSeconds : SyncIntervalSeconds");
    expect(unityWindow).toContain("MarkResizeSyncBoost();");
  });
});
