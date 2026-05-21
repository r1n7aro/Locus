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
    expect(chatView).toContain(':unity-connection-status="unityConnectionStatus"');
    expect(chatView).toContain(':unity-recompiling="unityRecompileActive"');
    expect(chatView).toContain("workingDir?: string;");
    expect(chatView).toContain(':working-dir="workingDir"');
    expect(workspace).toContain(':unity-connection-status="projectStore.unityConnectionStatus"');
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
    expect(indicators).toContain('unityConnectionStatus?: UnityConnectionStatus | null;');
    expect(indicators).toContain('function unityPipeNameForWorkingDir(workingDir: string)');
    expect(indicators).toContain('return `\\\\\\\\.\\\\pipe\\\\locus_unity_${sanitized}`;');
    expect(indicators).toContain('label: t("chat.status.unity.pipe")');
    expect(indicators).toContain('label: t("chat.status.unity.workingDir")');
    expect(indicators).toContain('label: t("chat.status.unity.lastError")');
    expect(indicators).toContain('label: t("chat.status.unity.reconnectAttempts")');
    expect(indicators).toContain(':class="{ \'is-mono\': row.mono }"');
    expect(zh).toContain('"chat.status.unity.pipe": "管道"');
    expect(zh).toContain('"chat.status.unity.workingDir": "工作目录"');
    expect(zh).toContain('"chat.status.unity.lastError": "最后错误"');
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
    expect(indicators).toContain("unityRecompiling?: boolean;");
    expect(indicators).toContain('const unityCanLaunch = computed(() =>');
    expect(indicators).toContain('&& !props.unityConnected');
    expect(indicators).toContain('&& !props.unityPluginStatus');
    expect(indicators).toContain('&& !unityRecompileWaitingConnection.value');
    expect(indicators).toContain('const effectiveUnityLaunchState = computed<UnityLaunchState>(() =>');
    expect(indicators).toContain('if (effectiveUnityLaunchState.value === "starting") return t("chat.unity.launching");');
    expect(indicators).toContain('return t("chat.status.unity.waitingConnection");');
    expect(indicators).toContain(':variant="activeItem.actionVariant"');
    expect(indicators).toContain('actionVariant: props.unityPluginStatus ? "neutral" : "primary"');
    expect(projectStore).toContain('const unityLaunchState = ref<UnityLaunchState>("idle");');
    expect(projectStore).toContain('const unityLaunching = computed(() => unityLaunchState.value === "starting");');
    expect(projectStore).toContain("async function launchUnityProject()");
    expect(projectStore).toContain("await unityService.launchUnityProject();");
    expect(projectStore).toContain("await unityService.checkUnityConnectionStatus()");
    expect(projectStore).toContain('unityLaunchState.value = "starting";');
    expect(projectStore).toContain('unityLaunchState.value = "waitingConnection";');
    expect(projectStore).toContain("function handleUnityConnectionStatus(connected: boolean)");
    expect(projectStore).toContain("function handleUnityConnectionStatusDetail(status: UnityConnectionStatus)");
    expect(bootstrap).toContain("projectStore.handleUnityConnectionStatus(payload);");
    expect(bootstrap).toContain('"unity-connection-status-detail"');
    expect(unityService).toContain('return ipcInvoke<UnityLaunchResult>("launch_unity_project");');
    expect(zh).toContain('"chat.status.unity.launch": "启动"');
    expect(zh).toContain('"chat.status.unity.waitingConnection": "等待连接"');
    expect(zh).toContain('"chat.status.unity.launchTitle": "启动 Unity 项目"');
  });

  it("shows Unity recompile reconnect waits as the accent connection state", () => {
    const chatView = read("src/components/ChatView.vue");
    const indicators = read("src/components/chat/ChatStatusIndicators.vue");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(chatView).toContain("function hasRunningUnityRecompile(calls: ToolCallDisplay[] | undefined): boolean");
    expect(chatView).toContain('call.name === "unity_recompile" && call.status === "running"');
    expect(chatView).toContain("const unityRecompileActive = computed(() => hasRunningUnityRecompile(props.activeToolCalls));");
    expect(indicators).toContain("const unityRecompileWaitingConnection = computed(() =>");
    expect(indicators).toContain('if (unityRecompileWaitingConnection.value) return t("chat.unity.waitingRecompileConnection");');
    expect(indicators).toContain('unityRecompileWaitingConnection.value || effectiveUnityLaunchState.value !== "idle"');
    expect(zh).toContain('"chat.unity.waitingRecompileConnection": "Unity 重编译中，等待重连"');
    expect(en).toContain('"chat.unity.waitingRecompileConnection": "Unity recompiling, waiting for reconnect"');
  });

  it("shows staged asset reconcile progress without current file detail", () => {
    const indicators = read("src/components/chat/ChatStatusIndicators.vue");
    const assetStats = read("src/components/asset/AssetStatsView.vue");
    const types = read("src/types.ts");
    const rustTypes = read("src-tauri/src/asset_db/types.rs");
    const watcher = read("src-tauri/src/asset_db/watcher.rs");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");
    const scanEventBlock = types.match(/export type AssetDbScanEvent =[\s\S]*?\| \{ phase: "error"; error: AppErrorPayload \};/)?.[0] ?? "";

    expect(scanEventBlock).toContain('phase: "reconcile";');
    expect(scanEventBlock).toContain('stage?: "scanning" | "discovering" | "processing" | string | null;');
    expect(scanEventBlock).toContain("queued?: number | null;");
    expect(scanEventBlock).toContain("failed?: number | null;");
    expect(scanEventBlock).not.toContain("currentFile");

    expect(rustTypes).toContain("pub fn reconcile_started(verify_hashes: bool) -> Self");
    expect(rustTypes).toContain("stage: Option<String>");
    expect(rustTypes).toContain("queued: Option<u64>");
    expect(rustTypes).toContain("failed: Option<u64>");
    expect(rustTypes).not.toContain("current_file: Option<String>");
    expect(watcher).toContain('stage: "scanning"');
    expect(watcher).toContain('stage: "discovering"');
    expect(watcher).toContain('stage: "processing"');

    expect(indicators).toContain('case "reconcile": return reconcileScanLabel(p);');
    expect(indicators).toContain('label: t("chat.status.assetDb.stage")');
    expect(indicators).toContain('label: t("chat.status.assetDb.reconcileMode")');
    expect(indicators).toContain('label: t("chat.status.assetDb.queued")');
    expect(indicators).toContain('return value ? { label: t("chat.status.assetDb.progress"), value } : null;');
    expect(indicators).not.toContain("chat.status.assetDb.currentFile");

    expect(assetStats).toContain('stageLabel: reconcileStageLabel(phase.stage)');
    expect(assetStats).toContain("queued: isFiniteCount(phase.queued) ? phase.queued : null");
    expect(assetStats).toContain("failed: isFiniteCount(phase.failed) ? phase.failed : null");
    expect(assetStats).not.toContain("liveScanProgress.currentFile");

    expect(zh).toContain('"chat.assetDb.scanning.reconcile.scanning": "校验文件 {0}..."');
    expect(zh).toContain('"chat.status.assetDb.reconcileStage.processing": "同步变更"');
    expect(zh).not.toContain('"chat.status.assetDb.currentFile"');
    expect(en).toContain('"chat.assetDb.scanning.reconcile.processing": "Syncing changes {0}..."');
    expect(en).toContain('"chat.status.assetDb.reconcileStage.scanning": "Verifying files"');
    expect(en).not.toContain('"chat.status.assetDb.currentFile"');
  });
});
