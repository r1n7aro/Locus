import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("ReferenceExternalImportPanel layout", () => {
  it("keeps source selection ahead of inline provider configuration", () => {
    const panel = read("src/components/knowledge/ReferenceExternalImportPanel.vue");

    expect(panel).toContain('export type ExternalImportSource = "feishu" | "unity"');
    expect(panel).toContain("const activeSource = ref<ExternalImportSource>(props.initialSource)");
    expect(panel).toContain("knowledgeSaveFeishuReferenceConfig");
    expect(panel).toContain("knowledgeImportFeishuReferenceDocs");
    expect(panel).toContain("knowledgeImportUnityReferenceDocs");
    expect(panel).toContain('t("knowledge.referenceFolder.external.dialogHint")');
    expect(panel).toContain('t("knowledge.referenceFolder.external.targetPending")');
    expect(panel).toContain('t("knowledge.referenceFolder.external.keepInBackground")');
    expect(panel).toContain("mode?: \"dialog\" | \"directory\" | \"window\"");
    expect(panel).toContain("const showPanelHeader = computed(() => props.mode !== \"window\")");
    expect(panel).toContain("const knownReferenceDirectories = ref<Set<string>>(new Set())");
    expect(panel).toContain('resolveStableExternalImportTargetPath({');
    expect(panel).toContain("function normalizeSingleRootSelection(");
    expect(panel).toContain("const feishuConnectionVerified = ref(false)");
    expect(panel).toContain("const feishuShowTestConnection = computed(() =>");
    expect(panel).toContain("const feishuOauthAuthorizedForCurrentConfig = computed(() => {");
    expect(panel).toContain("if (!feishuOauthAuthorized.value || feishuAppSecretTouched.value) return false;");
    expect(panel).toContain('feishuAuthMode.value !== "oauth" || feishuOauthAuthorizedForCurrentConfig.value');
    expect(panel).toContain("const feishuCanContinueConnectionStep = computed(() =>");
    expect(panel).toContain("const windowSourceOptions = computed(() => [");
    expect(panel).toContain('import ReferenceExternalImportFeishuWindowFlow from "./externalImport/ReferenceExternalImportFeishuWindowFlow.vue"');
    expect(panel).toContain('import ReferenceExternalImportUnityWindowPane from "./externalImport/ReferenceExternalImportUnityWindowPane.vue"');
    expect(panel).toContain('import { useNotificationStore } from "../../stores/notification"');
    expect(panel).toContain("const unityWindowModel = computed<ReferenceExternalImportUnityWindowModel>(() => ({");
    expect(panel).toContain("const feishuWindowModel = computed<ReferenceExternalImportFeishuWindowModel>(() => ({");
    expect(panel).toContain("const unityCloseAfterSuccess = ref(false)");
    expect(panel).toContain("const unityWindowPrimaryCloses = computed(() =>");
    expect(panel).toContain("const unityWindowPrimaryLabel = computed(() =>");
    expect(panel).toContain('class="reference-external-window-tabs"');
    expect(panel).toContain('class="reference-external-window-source-tabs"');
    expect(panel).toContain('class="reference-external-window-meta"');
    expect(panel).toContain("<ReferenceExternalImportUnityWindowPane");
    expect(panel).toContain("<ReferenceExternalImportFeishuWindowFlow");
    expect(panel).toContain("if (!props.pathExists || !props.ensureDirectory) {");
    expect(panel).toContain('class="reference-external-grid"');
    expect(panel).not.toContain("folderName");
  });

  it("surfaces the existing Unity binding and avoids duplicate folder imports", () => {
    const panel = read("src/components/knowledge/ReferenceExternalImportPanel.vue");

    expect(panel).toContain("knowledgeFindUnityReferenceDirectory");
    expect(panel).toContain("const unityExistingDirectory = ref<KnowledgeDirectoryConfigRecord | null>(null)");
    expect(panel).toContain("const unityMaterializedTargetPath = ref(\"\")");
    expect(panel).toContain("function isUnityManagedTargetPath(path: string | null | undefined): boolean {");
    expect(panel).toContain("const unityHasForeignBinding = computed(() =>");
    expect(panel).toContain("function applyUnityStatus(");
    expect(panel).toContain("if (unityImportSessionStarted.value && status?.state === \"ready\") {");
    expect(panel).toContain("primaryClosesWindow: unityWindowPrimaryCloses.value,");
    expect(panel).toContain('t("knowledge.referenceFolder.external.unityExistingConflict", referencePathLabel(unityExistingPath))');
    expect(panel).toContain("unityMaterializedTargetPath.value = targetPath;");
    expect(panel).toContain("const ready = isUnityManagedTargetPath(targetPath)");
    expect(panel).toContain('await focusDirectory(unityExistingPath.value, true);');
    expect(panel).toContain('@close="emit(\'close\')"');
    expect(panel).toContain("feishuSelectedRoots.value = [{");
    expect(panel).not.toContain("...feishuSelectedRoots.value");
    expect(panel).toContain("feishuConnectionVerified.value = true;");
    expect(panel).toContain('t("knowledge.feishuReference.window.authorizationSucceeded")');
    expect(panel).toContain('t("knowledge.feishuReference.window.connectionSucceeded")');
    expect(panel).toContain('operation: "feishuReferenceAuthorizationSuccess"');
    expect(panel).toContain('operation: "feishuReferenceConnectionSuccess"');
    expect(panel).toContain("showTest: feishuShowTestConnection.value,");
    expect(panel).toContain('showAuthorize: feishuAuthMode.value === "oauth" && !feishuWaitingForAuthorization.value,');
    expect(panel).toContain("canContinueConnection: feishuCanContinueConnectionStep.value,");
    expect(panel).toContain('v-else-if="feishuAuthMode === \'oauth\'"');
    expect(panel).not.toContain("feishuOauthAuthorizedInSession");
    expect(panel).toContain('t("knowledge.referenceFolder.external.openExistingUnity")');
    expect(panel).toContain('emit("runningChange", value);');
    expect(panel).toContain("if (createCause) throw createCause;");
  });

  it("keeps Feishu step-three progress empty before any document has been imported", () => {
    const panel = read("src/components/knowledge/ReferenceExternalImportPanel.vue");
    const feishuProgressFn = panel.match(
      /function feishuProgressRatioForStatus\(status: FeishuReferenceImportStatus \| null\): number \| null \{[\s\S]*?\n\}/,
    )?.[0] ?? "";

    expect(feishuProgressFn).toContain("function feishuProgressRatioForStatus(status: FeishuReferenceImportStatus | null): number | null {");
    expect(feishuProgressFn).toContain("if (typeof status.progress === \"number\") return clampProgress(status.progress);");
    expect(feishuProgressFn).toContain("if (status.totalDocs && status.processedDocs > 0) {");
    expect(feishuProgressFn).toContain("if (status.stage === \"ready\" && status.importedDocCount > 0) return 1;");
    expect(feishuProgressFn).not.toContain("if (status.stage === \"ready\" || status.state === \"ready\") return 1;");
  });
});
