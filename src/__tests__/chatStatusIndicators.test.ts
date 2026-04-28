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
    expect(chatView).toContain('@launch-unity-project="emit(\'launchUnityProject\')"');
    expect(chatView).toContain(':unity-launching="unityLaunching"');
    expect(chatView).toContain(':unity-launch-state="unityLaunchState"');
    expect(chatView).toContain("workingDir?: string;");
    expect(chatView).toContain(':working-dir="workingDir"');
    expect(workspace).toContain(':working-dir="projectStore.workingDir"');
    expect(workspace).toContain(':unity-plugin-status="projectStore.pluginToast"');
    expect(workspace).toContain(':unity-plugin-installing="projectStore.pluginInstalling"');
    expect(workspace).toContain(':unity-launching="projectStore.unityLaunching"');
    expect(workspace).toContain(':unity-launch-state="projectStore.unityLaunchState"');
    expect(workspace).toContain('@install-plugin="projectStore.installPlugin"');
    expect(workspace).toContain('@launch-unity-project="projectStore.launchUnityProject"');
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
    expect(indicators).toContain('props.unityPluginStatus');
    expect(indicators).toContain('? "danger"');
    expect(indicators).toContain(': props.unityConnected');
    expect(indicators).toContain('props.unityPluginStatus === "missing"');
    expect(indicators).toContain('emit("installPlugin");');
    expect(indicators).toContain('emit("launchUnityProject");');
    expect(indicators).toContain('@click="runStatusAction(activeItem)"');
  });

  it("offers a launch action when the Unity plugin is ready and the editor is disconnected", () => {
    const indicators = read("src/components/chat/ChatStatusIndicators.vue");
    const projectStore = read("src/stores/project.ts");
    const unityService = read("src/services/unity.ts");
    const bootstrap = read("src/composables/useAppBootstrap.ts");
    const zh = read("src/language/zh.json");

    expect(indicators).toContain("unityLaunching?: boolean;");
    expect(indicators).toContain('type UnityLaunchState = "idle" | "starting" | "waitingConnection";');
    expect(indicators).toContain("unityLaunchState?: UnityLaunchState;");
    expect(indicators).toContain('const unityCanLaunch = computed(() =>');
    expect(indicators).toContain('!props.unityConnected && !props.unityPluginStatus');
    expect(indicators).toContain('const effectiveUnityLaunchState = computed<UnityLaunchState>(() =>');
    expect(indicators).toContain('if (effectiveUnityLaunchState.value === "starting") return t("chat.unity.launching");');
    expect(indicators).toContain('return t("chat.status.unity.waitingConnection");');
    expect(indicators).toContain(':variant="activeItem.actionVariant"');
    expect(indicators).toContain('actionVariant: props.unityPluginStatus ? "neutral" : "primary"');
    expect(projectStore).toContain('const unityLaunchState = ref<UnityLaunchState>("idle");');
    expect(projectStore).toContain('const unityLaunching = computed(() => unityLaunchState.value === "starting");');
    expect(projectStore).toContain("async function launchUnityProject()");
    expect(projectStore).toContain("await unityService.launchUnityProject();");
    expect(projectStore).toContain('unityLaunchState.value = "starting";');
    expect(projectStore).toContain('unityLaunchState.value = "waitingConnection";');
    expect(projectStore).toContain("function handleUnityConnectionStatus(connected: boolean)");
    expect(bootstrap).toContain("projectStore.handleUnityConnectionStatus(payload);");
    expect(unityService).toContain('return ipcInvoke<UnityLaunchResult>("launch_unity_project");');
    expect(zh).toContain('"chat.status.unity.launch": "启动"');
    expect(zh).toContain('"chat.status.unity.waitingConnection": "等待连接"');
    expect(zh).toContain('"chat.status.unity.launchTitle": "启动 Unity 项目"');
  });
});
