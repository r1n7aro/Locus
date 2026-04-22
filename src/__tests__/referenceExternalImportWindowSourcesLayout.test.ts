import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("Reference external import source layouts", () => {
  it("uses a paged wizard flow for Feishu import", () => {
    const component = read("src/components/knowledge/externalImport/ReferenceExternalImportFeishuWindowFlow.vue");

    expect(component).toContain("const currentStep = ref(0)");
    expect(component).toContain("useCopyFeedback");
    expect(component).toContain('import FileTreeList from "../../explorer/FileTreeList.vue"');
    expect(component).toContain("model.steps");
    expect(component).toContain("() => props.model.isRunning");
    expect(component).toContain("if (currentStep.value === 0) return props.model.canContinueConnection;");
    expect(component).toContain("if (index > currentStep.value) return;");
    expect(component).toContain('class="reference-feishu-flow-steps"');
    expect(component).toContain('class="reference-feishu-flow-section"');
    expect(component).toContain('class="reference-feishu-flow-tree"');
    expect(component).toContain('class="reference-feishu-flow-scope"');
    expect(component).toContain('class="reference-feishu-flow-dropdown"');
    expect(component).toContain('class="reference-feishu-flow-tree-row-shell"');
    expect(component).toContain('class="reference-feishu-flow-tree-main"');
    expect(component).toContain(':row-height="30"');
    expect(component).toContain('class="reference-feishu-flow-callback"');
    expect(component).toContain('class="reference-feishu-flow-copy-indicator"');
    expect(component).toContain('void copyCallbackUrl(callbackUrl)');
    expect(component).toContain('t("common.clickToCopy")');
    expect(component).toContain('t("common.copied")');
    expect(component).toContain('v-if="model.showTest"');
    expect(component).toContain('v-if="model.canCancelAuthorization"');
    expect(component).toContain('v-else-if="model.showAuthorize"');
    expect(component).toContain(".reference-feishu-flow-dropdown :deep(.base-dropdown-menu)");
    expect(component).toContain("min-width: 100%");
    expect(component).toContain("justify-content: space-between;");
    expect(component).toContain(':disabled="index > currentStep || model.isRunning || model.waitingForAuthorization"');
    expect(component).not.toContain('class="reference-feishu-flow-summary"');
    expect(component).not.toContain("{{ model.summary }}");
    expect(component).not.toContain("BaseCheckbox");
    expect(component).not.toContain('class="reference-feishu-flow-grid compact"');
    expect(component).not.toContain('class="reference-feishu-flow-tree-meta"');
    expect(component).toContain('{{ t("onboarding.next") }}');
    expect(component).toContain('{{ t("common.back") }}');
  });

  it("keeps Unity import in a dedicated source pane", () => {
    const component = read("src/components/knowledge/externalImport/ReferenceExternalImportUnityWindowPane.vue");

    expect(component).toContain('class="reference-unity-pane"');
    expect(component).toContain('(e: "close"): void;');
    expect(component).toContain("@click=\"model.primaryClosesWindow ? emit('close') : emit('start')\"");
    expect(component).toContain('class="reference-unity-stage-list"');
    expect(component).toContain("grid-template-columns: repeat(2, minmax(0, 1fr));");
    expect(component).toContain("@media (max-width: 640px)");
    expect(component).toContain("grid-template-columns: minmax(0, 1fr);");
    expect(component).toContain('class="reference-unity-track"');
    expect(component).toContain('class="reference-unity-actions"');
    expect(component).toContain("ReferenceExternalImportUnityWindowModel");
  });
});
