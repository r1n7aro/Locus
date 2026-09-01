import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("KnowledgePreview continuous document layout", () => {
  it("opens directly on the rendered document without a redundant header", () => {
    const preview = read("src/components/knowledge/KnowledgePreview.vue");

    expect(preview).not.toContain('class="preview-header"');
    expect(preview).not.toContain('class="preview-path"');
    expect(preview).not.toContain('class="preview-view-segmented"');
    expect(preview).toContain('const editorViewMode = "rendered" as const;');
    expect(preview).not.toContain('class="preview-pane-header"');
  });

  it("places the editable title inside the document page", () => {
    const preview = read("src/components/knowledge/KnowledgePreview.vue");

    expect(preview).toContain('class="document-page"');
    expect(preview).toContain('class="document-heading"');
    expect(preview).toContain('class="document-title-input-shell"');
    expect(preview).toContain('class="document-title-input"');
    expect(preview).toContain('@blur="flushPendingChanges(\'manual\')"');
    expect(preview).toContain("function buildPendingDocumentNamePatch()");
    expect(preview).toMatch(/\.document-title-input\s*\{[\s\S]*border-bottom:\s*1px solid transparent;/);
  });

  it("renders document metadata as compact property rows", () => {
    const preview = read("src/components/knowledge/KnowledgePreview.vue");

    expect(preview).toContain('class="document-properties"');
    expect(preview).toContain('class="document-property-row"');
    expect(preview).toContain('class="document-property-label"');
    expect(preview).toContain('class="document-property-dropdown meta-dropdown"');
    expect(preview).toContain("injectModeSelection");
    expect(preview).toContain("editModeDropdownLabel");
    expect(preview).toContain('teleport');
    expect(preview).toMatch(/\.document-property-row\s*\{[\s\S]*grid-template-columns:\s*112px minmax\(0, 1fr\);/);
  });

  it("keeps summary, maintenance rules, and body in one scroll plane", () => {
    const preview = read("src/components/knowledge/KnowledgePreview.vue");

    expect(preview).toContain('class="document-inline-field document-inline-summary"');
    expect(preview).toContain('class="document-inline-field document-inline-rules"');
    expect(preview).toContain('class="document-body"');
    expect(preview).toContain(':model-value="rulesPropertyValue"');
    expect(preview).toContain(':model-value="bodyDraft"');
    expect(preview).not.toContain('class="preview-support-strip"');
    expect(preview).not.toContain('class="preview-main-divider"');
    expect(preview).toMatch(/\.preview-main\s*\{[\s\S]*overflow:\s*auto;/);
    expect(preview.match(/\sauto-grow\s/g)).toHaveLength(3);
    expect(preview).not.toContain("defer-rendered-editor");
    expect(preview.match(/:content-key=/g)).toHaveLength(3);
    expect(preview.match(/:session-cache="markdownEditorSessions"/g)).toHaveLength(3);
    expect(preview.match(/:session-pinned=/g)).toHaveLength(3);
    expect(preview).toContain("function isMarkdownEditorSessionPinned(section: KnowledgeDocumentSection)");
    expect(preview).toContain("const documentContentKey = computed(() =>");
    expect(preview).toContain("const documentContentKey = computed(() => activeDocumentSessionKey.value)");
    expect(preview).toContain(':min-height="64"');
    expect(preview).toContain(':min-height="104"');
    expect(preview).toContain(':min-height="360"');
    expect(preview).toMatch(/\.document-body\s*:deep\(\.base-markdown-editor \.cm-scroller\)\s*\{[\s\S]*overflow:\s*visible;[\s\S]*overscroll-behavior:\s*auto;/);
    expect(preview).not.toMatch(/\.document-body\.is-loading\s*\{[\s\S]*opacity:/);
  });

  it("hides inherited maintenance-rule content and keeps explicit rules editable", () => {
    const preview = read("src/components/knowledge/KnowledgePreview.vue");

    expect(preview).toContain('const usesInheritedMaintenanceRules = computed(() => props.document?.aiEditMode === "inherit")');
    expect(preview).toContain("const rulesPropertyValue = computed(() => rulesDraft.value)");
    expect(preview).toContain("!usesInheritedMaintenanceRules &&");
    expect(preview).toContain(':disabled="rulesEditorDisabled"');
    expect(preview).not.toContain("knowledge.preview.rulesInheritedHint");
  });

  it("keeps storage details off ordinary documents while retaining file size and modification time", () => {
    const preview = read("src/components/knowledge/KnowledgePreview.vue");

    expect(preview).toContain("const showExtendedDocumentProperties = computed(() => (");
    expect(preview).toContain('props.document?.type === "skill" || props.document?.type === "reference"');
    expect(preview).toContain('<template v-if="showExtendedDocumentProperties">');
    expect(preview).toContain('<div v-if="documentFileMetadata" class="document-property-row">');
    expect(preview).toContain('documentFileMetadata.value?.modifiedAt ?? props.document?.modifiedAt');
    expect(preview).toContain('{{ t("knowledge.meta.modifiedAt") }}');
    expect(preview).not.toContain('showLastCommit');
  });

  it("keeps skill-only properties and Unity package actions in the same property list", () => {
    const preview = read("src/components/knowledge/KnowledgePreview.vue");

    expect(preview).toContain('<template v-if="document.type === \'skill\'">');
    expect(preview).toContain('t("knowledge.skill.commandTrigger")');
    expect(preview).toContain('class="document-property-input"');
    expect(preview).toContain('class="skill-unity-actions"');
    expect(preview).toContain('@click="installSkillUnity"');
    expect(preview).toContain('@click="removeSkillUnity"');
  });

  it("uses the full preview width without an embedded chat rail", () => {
    const preview = read("src/components/knowledge/KnowledgePreview.vue");

    expect(preview).toContain('<div class="preview-panel">');
    expect(preview).not.toContain("KnowledgeChatPane");
    expect(preview).not.toContain("preview-side-rail");
    expect(preview).not.toContain("preview-side-resize-handle");
    expect(preview).not.toContain("metaCollapsed");
  });

  it("keeps all three markdown editors mounted while showing search hits", () => {
    const preview = read("src/components/knowledge/KnowledgePreview.vue");

    expect(preview).not.toContain("showSearchRenderedContent");
    expect(preview).not.toContain("<MarkdownRenderer");
    expect(preview.match(/<BaseMarkdownEditor/g)).toHaveLength(3);
    expect(preview).toContain("isSearchMatchSection('summary')");
    expect(preview).toContain("isSearchMatchSection('maintenanceRules')");
    expect(preview).toContain("isSearchMatchSection('body')");
    expect(preview).toContain('class="preview-search-hit-mark"');
  });
});
