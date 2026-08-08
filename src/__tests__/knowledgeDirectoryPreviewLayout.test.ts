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

  it("uses the same continuous page structure as document config", () => {
    const preview = read("src/components/knowledge/KnowledgeDirectoryPreview.vue");

    expect(preview).not.toContain('t("common.save")');
    expect(preview).not.toContain('class="directory-meta-strip"');
    expect(preview).toContain('class="directory-preview-main"');
    expect(preview).toContain('class="directory-config-page"');
    expect(preview).toContain('class="directory-config-heading"');
    expect(preview).toContain('class="directory-config-title"');
    expect(preview).toContain('class="directory-properties"');
    expect(preview).toContain('class="directory-property-row"');
    expect(preview).toContain('class="directory-property-dropdown"');
    expect(preview).not.toContain('class="directory-primary-grid"');
    expect(preview).not.toContain('class="directory-option-row"');
    expect(preview).toMatch(/\.directory-config-page\s*\{[\s\S]*width:\s*min\(100%,\s*980px\);/);
    expect(preview).toMatch(/\.directory-property-row\s*\{[\s\S]*grid-template-columns:\s*140px minmax\(0,\s*1fr\);/);
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
    expect(preview).toContain(':content-key="`${directoryContentKey}:summary`"');
    expect(preview).toContain(':content-key="`${directoryContentKey}:maintenanceRules`"');
    expect(preview).toContain("defer-rendered-editor");
    expect(preview).toContain("auto-grow");
    expect(preview).toContain(':min-height="64"');
    expect(preview).toContain(':min-height="104"');
    expect(preview).toContain("const directoryContentKey = computed(() =>");
    expect(preview).toContain("import BaseSegmented from \"../ui/BaseSegmented.vue\"");
    expect(preview).toMatch(/\.directory-inline-field\s*:deep\(\.base-markdown-editor\)\s*\{[\s\S]*height:\s*auto;[\s\S]*border-left:\s*1px solid var\(--border-color\);/);
  });

  it("keeps the current folder visible until the next folder is ready", () => {
    const preview = read("src/components/knowledge/KnowledgeDirectoryPreview.vue");

    expect(preview).toContain('v-if="loading && !directory"');
  });

  it("keeps directory controls in compact property rows ahead of inline content", () => {
    const preview = read("src/components/knowledge/KnowledgeDirectoryPreview.vue");

    expect(preview).not.toContain('t("knowledge.directoryConfig.retrieval")');
    expect(preview).not.toContain('t("knowledge.directoryConfig.retrievalHint")');
    expect(preview).toMatch(/class="directory-properties"[\s\S]*knowledge\.directoryConfig\.injectMode[\s\S]*knowledge\.directoryConfig\.aiConfig[\s\S]*knowledge\.directoryConfig\.lexicalSearch[\s\S]*knowledge\.directoryConfig\.semanticSearch[\s\S]*knowledge\.directoryConfig\.explicitMaintenanceRules[\s\S]*knowledge\.directoryConfig\.allowMoveDirectories/);
    expect(preview).toMatch(/knowledge\.directoryConfig\.allowMoveDirectories[\s\S]*class="directory-inline-field directory-inline-summary"[\s\S]*class="directory-inline-field directory-inline-rules"/);
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
    expect(preview).toContain("const effectiveLabel = effectiveCapabilityLabel(kind, effectiveState);");
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
