import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("Knowledge external import entry", () => {
  it("routes the explorer and overview entry points into a dedicated source-first import window", () => {
    const view = read("src/components/KnowledgeView.vue");

    expect(view).toContain("../services/referenceExternalImportWindow");
    expect(view).toContain("openReferenceExternalImportWindow,");
    expect(view).toContain("type ReferenceExternalImportSource");
    expect(view).toContain("const hasUnityReferenceDocs = computed(");
    expect(view).toContain("function openExternalImportWindow(");
    expect(view).toContain("parentDir = \"\",");
    expect(view).toContain("initialSource: ReferenceExternalImportSource | null = null,");
    expect(view).toContain("const preferredSource =");
    expect(view).toContain("initialSource ??");
    expect(view).toContain('(!normalizedParent && !hasUnityReferenceDocs.value ? "unity" : null);');
    expect(view).toContain("void openReferenceExternalImportWindow({");
    expect(view).toContain('@request-external-import-folder="');
    expect(view).toContain("(parentDir) => void openExternalImportWindow(parentDir)");
    expect(view).toContain('@create-external-folder="');
    expect(view).toContain("(source) => void openExternalImportWindow('', source)");
    expect(view).toContain("async function ensureReferenceDirectory(path: string): Promise<boolean>");
    expect(view).toContain('await createFolder(segments.join("/"), name);');
    expect(view).not.toContain('import ReferenceExternalImportPanel from "./knowledge/ReferenceExternalImportPanel.vue"');
    expect(view).not.toContain("externalImportDialog");
    expect(view).not.toContain('class="knowledge-modal knowledge-modal-wide"');
    expect(view).not.toContain('knowledge.referenceFolder.external.folderName');
    expect(view).not.toContain("confirmExternalImportDialog");
  });
});
