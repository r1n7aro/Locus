import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8").replace(/\r\n/g, "\n");
}

describe("FeishuReferenceImportProgressWindow layout", () => {
  it("autosaves connection edits and drops the manual save and clear buttons", () => {
    const windowSource = read("src/components/FeishuReferenceImportProgressWindow.vue");

    expect(windowSource).toContain("const AUTO_SAVE_DELAY_MS = 700");
    expect(windowSource).toContain("const autoSaveQueued = ref(false)");
    expect(windowSource).toContain("const appSecretTouched = ref(false)");
    expect(windowSource).toContain("function scheduleAutoSave(");
    expect(windowSource).toContain("function markFormDirtyAndQueueSave(");
    expect(windowSource).toContain("function handleAppSecretInput()");
    expect(windowSource).toContain("void saveConfig();");
    expect(windowSource).toContain("clearAppSecret: shouldPersistAppSecret && !normalizedAppSecret");
    expect(windowSource).not.toContain('t("common.save")');
    expect(windowSource).not.toContain('t("knowledge.feishuReference.window.clearSecret")');
  });

  it("shows the test button only after oauth authorization is confirmed", () => {
    const windowSource = read("src/components/FeishuReferenceImportProgressWindow.vue");

    expect(windowSource).toContain("const showTestConnection = computed(");
    expect(windowSource).toContain('() => authMode.value !== "oauth" || oauthAuthorized.value');
    expect(windowSource).toContain('v-if="showTestConnection"');
    expect(windowSource).toContain("const oauthAuthorized = computed(");
    expect(windowSource).toContain("statusSnapshot.value?.authorized");
  });

  it("uses the unity-style titlebar close button", () => {
    const windowSource = read("src/components/FeishuReferenceImportProgressWindow.vue");

    expect(windowSource).toContain("const canCloseWindow = computed(");
    expect(windowSource).toContain("async function requestWindowClose()");
    expect(windowSource).toContain("!statusSnapshot.value?.running");
    expect(windowSource).toContain("!cancelAuthorizationPending.value");
    expect(windowSource).toContain('class="feishu-reference-window-titlebar-actions"');
    expect(windowSource).toContain('class="feishu-reference-window-close"');
    expect(windowSource).toContain(":aria-label=\"t('common.close')\"");
    expect(windowSource).toContain("@click=\"void requestWindowClose()\"");
  });

  it("shows the selected folder as the current import root", () => {
    const windowSource = read("src/components/FeishuReferenceImportProgressWindow.vue");
    const serviceSource = read("src/services/knowledge.ts");

    expect(windowSource).toContain("const selectedRootLabel = computed(() => {");
    expect(windowSource).toContain("function summarizeRootSelections(");
    expect(windowSource).toContain("const selectedRootTokenSet = computed(");
    expect(windowSource).toContain(
      "knowledgeTestFeishuReferenceConnection(\n      targetPath.value || undefined,",
    );
    expect(windowSource).toContain(
      'return t("knowledge.feishuReference.window.selectedRootCount", normalized.length);',
    );
    expect(windowSource).toContain('t("knowledge.feishuReference.window.spaceRoot"),');
    expect(serviceSource).toContain(
      'targetPath: targetPath ?? null,',
    );
  });

  it("rehydrates the saved app secret when the window reloads", () => {
    const windowSource = read("src/components/FeishuReferenceImportProgressWindow.vue");
    const typeSource = read("src/types.ts");

    expect(typeSource).toContain("appSecret?: string | null;");
    expect(windowSource).toContain('if (typeof status.appSecret === "string") {');
    expect(windowSource).toContain("appSecret.value = trimOrEmpty(status.appSecret);");
    expect(windowSource).toContain("} else if (!status.appSecretConfigured) {");
    expect(windowSource).toContain('appSecret.value = "";');
  });

  it("lets oauth authorization waiting be cancelled and retried", () => {
    const windowSource = read("src/components/FeishuReferenceImportProgressWindow.vue");

    expect(windowSource).toContain("knowledgeCancelFeishuReferenceOauthWait");
    expect(windowSource).toContain("const cancelAuthorizationPending = ref(false)");
    expect(windowSource).toContain("async function cancelAuthorizationWait(");
    expect(windowSource).toContain("const canCancelAuthorizationWait = computed(");
    expect(windowSource).toContain("await cancelAuthorizationWait({ syncUi: false, silent: true });");
    expect(windowSource).toContain("v-if=\"authMode === 'oauth' && waitingForAuthorization\"");
    expect(windowSource).toContain("knowledge.feishuReference.window.cancelAuthorization");
  });

  it("adds a visible window boundary for frameless child windows", () => {
    const windowSource = read("src/components/FeishuReferenceImportProgressWindow.vue");

    expect(windowSource).toContain("border: 1px solid var(--border-strong);");
    expect(windowSource).toContain("-webkit-app-region: no-drag;");
    expect(windowSource).toContain("background: color-mix(in srgb, var(--panel-bg) 94%, var(--bg-color) 6%);");
  });

  it("adds a visible resize handle for increasing the window height", () => {
    const windowSource = read("src/components/FeishuReferenceImportProgressWindow.vue");

    expect(windowSource).toContain("async function startWindowResize(");
    expect(windowSource).toContain("appWindow.startResizeDragging(direction)");
    expect(windowSource).toContain('class="feishu-reference-window-resize-handle"');
    expect(windowSource).toContain("@mousedown.prevent=\"void startWindowResize('SouthEast')\"");
    expect(windowSource).toContain("cursor: nwse-resize;");
    expect(windowSource).toContain("knowledge.feishuReference.window.resizeWindow");
  });

  it("updates the copy to reflect autosave and oauth-first testing", () => {
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(zh).toContain(
      '"knowledge.feishuReference.window.subtitle": "配置会自动保存。测试连接后选择知识空间与导入目录，再开始托管导入。"',
    );
    expect(zh).toContain(
      '"knowledge.feishuReference.window.connectionNote": "当前身份：{0}。配置会自动保存，应用身份可直接测试连接并导入，用户身份完成授权后可测试连接。"',
    );
    expect(zh).toContain(
      '"knowledge.feishuReference.window.cancelAuthorization": "停止等待授权"',
    );
    expect(zh).toContain(
      '"knowledge.feishuReference.window.resizeWindow": "调整窗口大小"',
    );
    expect(en).toContain(
      '"knowledge.feishuReference.window.subtitle": "Config saves automatically. Test the connection, choose a space and import folders, then start the managed import."',
    );
    expect(en).toContain(
      '"knowledge.feishuReference.window.connectionNote": "Current identity: {0}. Config saves automatically. App identity can test and import directly, while OAuth can test the connection after authorization completes."',
    );
    expect(en).toContain(
      '"knowledge.feishuReference.window.cancelAuthorization": "Stop Waiting"',
    );
    expect(en).toContain(
      '"knowledge.feishuReference.window.resizeWindow": "Resize Window"',
    );
  });
});
