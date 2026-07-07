import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("Reference external import local source", () => {
  it("keeps local import in a dedicated source pane", () => {
    const component = read("src/components/knowledge/externalImport/ReferenceExternalImportLocalWindowPane.vue");

    expect(component).toContain('class="reference-local-pane"');
    expect(component).toContain("ReferenceExternalImportLocalWindowModel");
    expect(component).toContain('(e: "pick-file"): void;');
    expect(component).toContain('(e: "pick-folder"): void;');
    expect(component).toContain('(e: "update:mode", value: KnowledgeLocalSourceMode): void;');
    expect(component).toContain('(e: "update:ai-editable", value: boolean): void;');
    expect(component).toContain('(e: "sync"): void;');
    expect(component).toContain("@click=\"model.primaryClosesWindow ? emit('close') : emit('start')\"");
    expect(component).toContain('v-if="model.aiEditableVisible"');
    expect(component).toContain('v-if="model.canSync"');
    expect(component).toContain('v-if="model.sourceMissingText"');
    expect(component).toContain('class="reference-local-actions"');
  });

  it("registers local as a third source in the import panel", () => {
    const panel = read("src/components/knowledge/ReferenceExternalImportPanel.vue");

    expect(panel).toContain('export type ExternalImportSource = "feishu" | "unity" | "local";');
    expect(panel).toContain('if (sources.some((source) => source.provider === "local_folder")) return "local";');
    expect(panel).toContain('if (source.provider === "local_folder") providers.add("local");');
    expect(panel).toContain('value: "local",');
    expect(panel).toContain('t("knowledge.localReference.title")');
    expect(panel).toContain("ReferenceExternalImportLocalWindowPane");
    expect(panel).toContain("v-else-if=\"activeSource === 'local'\"");
    expect(panel).toContain("const localWindowModel = computed<ReferenceExternalImportLocalWindowModel>(() => ({");
    expect(panel).toContain("knowledgeImportLocalReferenceDocs({");
    expect(panel).toContain("knowledgeSyncLocalReferenceDocs(targetPath)");
    expect(panel).toContain("knowledgeDeleteLocalReferenceDocs(targetPath)");
    expect(panel).toContain("aiEditable: localMode.value === \"snapshot\" && localAiEditable.value,");
    expect(panel).toContain('t("knowledge.localReference.syncConfirmEditable")');
    expect(panel).toContain("!!localStatus.value?.running");
    expect(panel).toContain("clearLocalPollTimer();");
  });

  it("exposes local import services and window payloads", () => {
    const services = read("src/services/knowledge.ts");
    expect(services).toContain('"knowledge_preview_local_reference_import"');
    expect(services).toContain('"knowledge_import_local_reference_docs"');
    expect(services).toContain('"knowledge_get_local_reference_import_status"');
    expect(services).toContain('"knowledge_cancel_local_reference_import"');
    expect(services).toContain('"knowledge_sync_local_reference_docs"');
    expect(services).toContain('"knowledge_delete_local_reference_docs"');

    const windowService = read("src/services/referenceExternalImportWindow.ts");
    expect(windowService).toContain('export type ReferenceExternalImportSource = "feishu" | "unity" | "local";');
    expect(windowService).toContain('initialSource === "local"');

    const windowHost = read("src/components/ReferenceExternalImportWindow.vue");
    expect(windowHost).toContain("nextPayload.initialSource === \"unity\" || nextPayload.initialSource === \"local\"");
  });

  it("models the local mode and editable flags in shared types", () => {
    const types = read("src/types.ts");
    expect(types).toContain('export type KnowledgeLocalSourceMode = "live" | "snapshot";');
    expect(types).toContain("localMode?: KnowledgeLocalSourceMode | null;");
    expect(types).toContain("aiEditable?: boolean | null;");
    expect(types).toContain("syncedAt?: number | null;");
    expect(types).toContain("export interface LocalReferenceImportRequest {");
    expect(types).toContain("export interface LocalReferenceImportStatus {");
    expect(types).toContain("sourceMissing: boolean;");
  });

  it("keeps localized copy for the local source in both languages", () => {
    for (const language of ["src/language/zh.json", "src/language/en.json"]) {
      const messages = JSON.parse(read(language)) as Record<string, string>;
      expect(messages["knowledge.referenceFolder.external.sourceLocal"]).toBeTruthy();
      expect(messages["knowledge.localReference.title"]).toBeTruthy();
      expect(messages["knowledge.localReference.mode.live"]).toBeTruthy();
      expect(messages["knowledge.localReference.mode.snapshot"]).toBeTruthy();
      expect(messages["knowledge.localReference.editable.label"]).toBeTruthy();
      expect(messages["knowledge.localReference.syncConfirmEditable"]).toBeTruthy();
      expect(messages["knowledge.localReference.deleteConfirm"]).toBeTruthy();
      expect(messages["knowledge.localReference.sourceMissing"]).toBeTruthy();
      expect(messages["knowledge.localReference.previewFiles"]).toBeTruthy();
      expect(messages["knowledge.localReference.totalFiles"]).toBeTruthy();
      expect(messages["knowledge.localReference.searchableDocs"]).toBeTruthy();
    }
  });
});
