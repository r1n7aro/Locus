import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("chat status indicators", () => {
  it("moves Unity and asset database status onto the input backdrop", () => {
    const chatView = read("src/components/ChatView.vue");
    const sessionPanel = read("src/components/chat/SessionPanel.vue");
    const indicators = read("src/components/chat/ChatStatusIndicators.vue");
    const workspace = read("src/components/ChatWorkspaceView.vue");

    expect(chatView).toContain('import ChatStatusIndicators from "./chat/ChatStatusIndicators.vue"');
    expect(chatView).toMatch(/<div v-if="!inputControlsCollapsed" class="input-backdrop-status">[\s\S]*<ChatStatusIndicators/);
    expect(chatView).toMatch(/<template v-if="!inputControlsCollapsed" #footer-start>[\s\S]*<ModelEffortSelector[\s\S]*\/>\s*<TokenUsageBar/);
    expect(chatView).toContain('@start-scan="emit(\'startScan\')"');
    expect(chatView).toContain(':unity-plugin-status="unityPluginStatus"');
    expect(chatView).toContain(':unity-plugin-installing="unityPluginInstalling"');
    expect(chatView).toContain('@install-plugin="emit(\'installPlugin\')"');
    expect(chatView).toContain("workingDir?: string;");
    expect(chatView).toContain(':working-dir="workingDir"');
    expect(workspace).toContain(':working-dir="projectStore.workingDir"');
    expect(workspace).toContain(':unity-plugin-status="projectStore.pluginToast"');
    expect(workspace).toContain(':unity-plugin-installing="projectStore.pluginInstalling"');
    expect(workspace).toContain('@install-plugin="projectStore.installPlugin"');
    expect(sessionPanel).not.toContain("sp-unity-status");
    expect(sessionPanel).not.toContain("sp-scan-status");
    expect(indicators).toContain('id: "assetDb"');
    expect(indicators).toContain('id: "unity"');
  });

  it("shows Unity pipe and working directory in the Unity popover", () => {
    const indicators = read("src/components/chat/ChatStatusIndicators.vue");
    const zh = read("src/language/zh.json");

    expect(indicators).toContain('workingDir?: string;');
    expect(indicators).toContain('function unityPipeNameForWorkingDir(workingDir: string)');
    expect(indicators).toContain('return `\\\\\\\\.\\\\pipe\\\\locus_unity_${sanitized}`;');
    expect(indicators).toContain('label: t("chat.status.unity.pipe")');
    expect(indicators).toContain('label: t("chat.status.unity.workingDir")');
    expect(indicators).toContain(':class="{ \'is-mono\': row.mono }"');
    expect(zh).toContain('"chat.status.unity.pipe": "管道"');
    expect(zh).toContain('"chat.status.unity.workingDir": "工作目录"');
  });

  it("uses fixed icon triggers with top hover labels and click popovers", () => {
    const indicators = read("src/components/chat/ChatStatusIndicators.vue");

    expect(indicators).toContain('icon: "database"');
    expect(indicators).toContain('icon: "unity"');
    expect(indicators).toContain('class="chat-status-icon-btn ui-select-none"');
    expect(indicators).toContain('class="chat-status-icon-label"');
    expect(indicators).toContain("{{ item.inlineLabel }}");
    expect(indicators).toContain('bottom: calc(100% + 6px);');
    expect(indicators).toContain('left: 50%;');
    expect(indicators).toContain('transform: translate(-50%, 3px);');
    expect(indicators).toContain('color: currentColor;');
    expect(indicators).toContain('width: 24px;');
    expect(indicators).toContain(':aria-label="`${item.title}: ${item.summary}`"');
    expect(indicators).toContain('class="chat-status-popover"');
    expect(indicators).toContain('role="dialog"');
    expect(indicators).toContain("tone-danger");
    expect(indicators).toContain("var(--status-danger-fg)");
    expect(indicators).toContain('return props.isUnityProject ? "danger" : "muted";');
  });

  it("marks the Unity icon as actionable when the Unity plugin needs attention", () => {
    const indicators = read("src/components/chat/ChatStatusIndicators.vue");

    expect(indicators).toContain('unityPluginStatus?: UnityPluginNotice | null;');
    expect(indicators).toContain('if (props.unityPluginStatus === "outdated") return t("app.plugin.needUpdate");');
    expect(indicators).toContain('props.unityPluginStatus ? "danger" : props.unityConnected ? "success" : "danger"');
    expect(indicators).toContain('props.unityPluginStatus === "missing"');
    expect(indicators).toContain('emit("installPlugin");');
    expect(indicators).toContain('@click="runStatusAction(activeItem)"');
  });
});
