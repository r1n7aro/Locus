import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("KnowledgeDirectoryPreview layout", () => {
  it("uses proposal-based wording for non-automatic directory edits", () => {
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(zh).toContain('"knowledge.directoryConfig.aiConfig.manual": "提案后修改"');
    expect(zh).toContain('"knowledge.meta.editMode.proposal": "提案后修改"');
    expect(en).toContain('"knowledge.directoryConfig.aiConfig.manual": "Proposal-based"');
    expect(en).toContain('"knowledge.meta.editMode.proposal": "Proposal-based"');
  });

  it("drops the legacy save button and folder meta strip", () => {
    const preview = read("src/components/knowledge/KnowledgeDirectoryPreview.vue");

    expect(preview).not.toContain('t("common.save")');
    expect(preview).not.toContain('class="directory-meta-strip"');
    expect(preview).not.toContain("directory-meta-item");
    expect(preview).toContain('class="directory-preview-main"');
    expect(preview).toContain('class="directory-preview-scroll"');
  });

  it("autosaves directory edits and renders a lightweight save footnote", () => {
    const preview = read("src/components/knowledge/KnowledgeDirectoryPreview.vue");

    expect(preview).toContain("const AUTO_SAVE_DELAY_MS = 900");
    expect(preview).toContain("const autoSaveQueued = ref(false)");
    expect(preview).toContain("const autoSaveInFlight = ref(false)");
    expect(preview).toContain('saveConfig("auto")');
    expect(preview).toContain('class="directory-footnote"');
    expect(preview).toMatch(/\.directory-footnote\s*\{[\s\S]*position:\s*absolute;[\s\S]*bottom:\s*10px;/);
  });

  it("shares the markdown editor view mode with the document preview", () => {
    const preview = read("src/components/knowledge/KnowledgeDirectoryPreview.vue");

    expect(preview).toContain("useMarkdownEditorViewMode");
    expect(preview).toContain("const editorViewMode = computed<MarkdownEditorViewMode>({");
    expect(preview).toContain('class="directory-view-segmented"');
    expect(preview).toContain(":view-mode=\"editorViewMode\"");
    expect(preview).toContain("import BaseSegmented from \"../ui/BaseSegmented.vue\"");
    expect(preview).toMatch(/\.directory-rules-editor\s*:deep\(\.base-markdown-editor \.base-markdown-editor-textarea\)\s*\{[\s\S]*height:\s*100%;[\s\S]*box-sizing:\s*border-box;/);
  });

  it("keeps inject mode, search rules, and capabilities ahead of summary and rules", () => {
    const preview = read("src/components/knowledge/KnowledgeDirectoryPreview.vue");

    expect(preview).toContain('class="directory-primary-grid"');
    expect(preview).toContain(
      'class="directory-card directory-card-plain directory-card-span"',
    );
    expect(preview).not.toContain('t("knowledge.directoryConfig.retrieval")');
    expect(preview).not.toContain('t("knowledge.directoryConfig.retrievalHint")');
    expect(preview).toMatch(/class="directory-primary-grid"[\s\S]*knowledge\.directoryConfig\.injectMode[\s\S]*knowledge\.directoryConfig\.aiConfig[\s\S]*knowledge\.directoryConfig\.lexicalSearch[\s\S]*knowledge\.directoryConfig\.semanticSearch[\s\S]*knowledge\.directoryConfig\.capabilities/);
    expect(preview).toMatch(/knowledge\.directoryConfig\.capabilities[\s\S]*knowledge\.directoryConfig\.summary[\s\S]*knowledge\.directoryConfig\.maintenanceRules/);
    expect(preview).toMatch(
      /\.directory-card-plain\s*\{[\s\S]*border:\s*none;[\s\S]*background:\s*transparent;/,
    );
    expect(preview).toMatch(/\.directory-primary-grid\s*\{[\s\S]*grid-template-columns:\s*repeat\(2,\s*minmax\(0,\s*1fr\)\);/);
    expect(preview).toMatch(/\.directory-search-grid\s*\{[\s\S]*grid-template-columns:\s*repeat\(2,\s*minmax\(0,\s*1fr\)\);/);
    expect(preview).toMatch(/\.directory-capability-grid\s*\{[\s\S]*grid-template-columns:\s*repeat\(3,\s*minmax\(0,\s*1fr\)\);/);
  });

  it("shows the effective search rule label when a folder rule inherits", () => {
    const preview = read("src/components/knowledge/KnowledgeDirectoryPreview.vue");

    expect(preview).toContain("function dropdownLabelForFolderIndexRule(");
    expect(preview).toContain("const lexicalRuleOptions = computed(() => buildFolderIndexRuleOptions(\"lexical\"));");
    expect(preview).toContain("const semanticRuleOptions = computed(() => buildFolderIndexRuleOptions(\"semantic\"));");
    expect(preview).toContain("return labelForInheritedValue(");
    expect(preview).toContain('{ kind: "parent_directory", path: null }');
    expect(preview).toContain('{ kind: "type_default", path: null }');
    expect(preview).toContain(":options=\"lexicalRuleOptions\"");
    expect(preview).toContain(":options=\"semanticRuleOptions\"");
    expect(preview).toContain('effectiveCapabilityLabel("lexical", effectiveLexicalSearch)');
    expect(preview).toContain('effectiveCapabilityLabel("semantic", effectiveVectorSearch)');
    expect(preview).toContain("draft.lexicalSearch,");
    expect(preview).toContain("effectiveLexicalSearch,");
    expect(preview).toContain("draft.vectorSearch,");
    expect(preview).toContain("effectiveVectorSearch,");
  });

  it("adds a folder-scoped external import tab for reference directories", () => {
    const preview = read("src/components/knowledge/KnowledgeDirectoryPreview.vue");

    expect(preview).toContain('type DirectoryPanelTab = "config" | "external"');
    expect(preview).toContain('import ReferenceExternalImportPanel from "./ReferenceExternalImportPanel.vue"');
    expect(preview).toContain('t("knowledge.directoryConfig.panel.external")');
    expect(preview).toContain('t("knowledge.referenceFolder.external.hint")');
    expect(preview).toContain("<ReferenceExternalImportPanel");
    expect(preview).toContain(':fixed-target-path="directory.path"');
    expect(preview).toContain(':refresh-knowledge="refreshKnowledge ?? null"');
    expect(preview).toContain(':delete-feishu-import="deleteFeishuImport ?? null"');
    expect(preview).toContain(':delete-unity-import="deleteUnityImport ?? null"');
    expect(preview).not.toContain('emit("open-feishu-import", path)');
    expect(preview).not.toContain('emit("open-unity-import", path)');
  });
});
