import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("KnowledgePreview layout", () => {
  it("does not keep legacy BaseButton actions in the preview shell", () => {
    const preview = read("src/components/knowledge/KnowledgePreview.vue");

    expect(preview).not.toContain("BaseButton");
  });

  it("keeps the document name editable in the preview header", () => {
    const preview = read("src/components/knowledge/KnowledgePreview.vue");

    expect(preview).toContain('const fileNameDraft = ref("")');
    expect(preview).toContain("const fileNameDirty = ref(false)");
    expect(preview).toContain('const titleMeasureText = computed(() => fileNameDraft.value || " ")');
    expect(preview).toContain('class="preview-title-input-shell"');
    expect(preview).toContain(':data-value="titleMeasureText"');
    expect(preview).toContain('class="preview-title-input"');
    expect(preview).toContain(`:placeholder="t('knowledge.preview.titlePlaceholder')"`);
    expect(preview).toContain(`:aria-label="t('knowledge.preview.titleLabel')"`);
    expect(preview).toContain('@blur="flushPendingChanges(\'manual\')"');
    expect(preview).toContain('@keydown="onFileNameKeydown"');
    expect(preview).toContain("function persistDocumentNameChange()");
    expect(preview).toContain("function buildPendingDocumentNamePatch()");
    expect(preview).toContain("newPath:");
    expect(preview).toContain('t("knowledge.preview.titleRequired")');
    expect(preview).toContain('t("knowledge.preview.titleInvalid")');
    expect(preview).toContain('class="preview-path"');
    expect(preview).toMatch(/\.preview-header-main\s*\{[\s\S]*display:\s*flex;[\s\S]*align-items:\s*center;[\s\S]*gap:\s*8px;/);
    expect(preview).toMatch(/\.preview-title-input-shell\s*\{[\s\S]*min-width:\s*0;[\s\S]*width:\s*fit-content;[\s\S]*max-width:\s*min\(100%,\s*420px\);[\s\S]*display:\s*inline-grid;/);
    expect(preview).toMatch(/\.preview-title-input-shell::after\s*\{[\s\S]*content:\s*attr\(data-value\)\s+" ";[\s\S]*grid-area:\s*1\s*\/\s*1;[\s\S]*visibility:\s*hidden;[\s\S]*white-space:\s*pre;/);
    expect(preview).toMatch(/\.preview-title-input\s*\{[\s\S]*height:\s*30px;[\s\S]*border:\s*1px solid transparent;[\s\S]*font-size:\s*14px;/);
    expect(preview).toMatch(/\.preview-title-input\s*\{[\s\S]*grid-area:\s*1\s*\/\s*1;[\s\S]*width:\s*100%;[\s\S]*min-width:\s*0;/);
    expect(preview).toMatch(/\.preview-path\s*\{[\s\S]*flex:\s*1 1 auto;[\s\S]*min-width:\s*0;[\s\S]*text-overflow:\s*ellipsis;[\s\S]*white-space:\s*nowrap;/);
    expect(preview).toMatch(/\.preview-title-input:focus\s*\{[\s\S]*border-color:[\s\S]*box-shadow:/);
  });

  it("shows builtin memory documents under the project understanding display path", () => {
    const preview = read("src/components/knowledge/KnowledgePreview.vue");

    expect(preview).toContain('const MEMORY_PREVIEW_PATH_PREFIX = "unity-project-understanding"');
    expect(preview).toContain('const BUILTIN_MEMORY_PREVIEW_PATHS = new Set([');
    expect(preview).toContain('"project-mistake-note.md"');
    expect(preview).toContain('"user-preference.md"');
    expect(preview).toContain("function formatDocumentDisplayPath(document: KnowledgeDocument | null | undefined): string {");
    expect(preview).toContain('document.type === "memory"');
    expect(preview).toContain("BUILTIN_MEMORY_PREVIEW_PATHS.has(path)");
    expect(preview).toContain('return `${MEMORY_PREVIEW_PATH_PREFIX}/${path}`;');
    expect(preview).toContain("const documentDisplayPath = computed(() => formatDocumentDisplayPath(props.document));");
    expect(preview).toContain('class="preview-path">{{ documentDisplayPath }}</span>');
  });

  it("keeps the body editor as the flexing pane", () => {
    const preview = read("src/components/knowledge/KnowledgePreview.vue");

    expect(preview).toContain('class="preview-pane preview-pane-body"');
    expect(preview).toContain('class="preview-view-segmented"');
    expect(preview).toContain("useMarkdownEditorViewMode");
    expect(preview).toContain("const editorViewMode = computed<MarkdownEditorViewMode>({");
    expect(preview).toContain(":view-mode=\"editorViewMode\"");
    expect(preview).toMatch(/\.preview-pane-body\s*\{[\s\S]*flex:\s*1 1 0;[\s\S]*min-height:\s*0;/);
    expect(preview).toMatch(/\.preview-pane-body\s+\.preview-body\s*\{[\s\S]*min-height:\s*0;/);
    expect(preview).toMatch(/\.preview-body\s*\{[\s\S]*display:\s*flex;[\s\S]*overflow:\s*hidden;/);
    expect(preview).toMatch(/\.preview-body\s*:deep\(\.base-markdown-editor \.vditor-ir pre\.vditor-reset\)\s*\{[\s\S]*height:\s*100%;[\s\S]*overflow:\s*auto;/);
    expect(preview).toMatch(/\.preview-body\s*:deep\(\.base-markdown-editor \.base-markdown-editor-textarea\)\s*\{[\s\S]*height:\s*100%;[\s\S]*overflow:\s*auto;/);
  });

  it("renders save state as an overlay instead of occupying editor height", () => {
    const preview = read("src/components/knowledge/KnowledgePreview.vue");

    expect(preview).toMatch(/\.editor-footnote\s*\{[\s\S]*position:\s*absolute;[\s\S]*bottom:\s*10px;/);
    expect(preview).toMatch(/\.preview-pane-body\s*:deep\(\.base-markdown-editor\)\s*\{[\s\S]*padding-bottom:\s*44px;/);
    expect(preview).toMatch(/\.preview-body\s*:deep\(\.base-markdown-editor\)\s*\{[\s\S]*flex:\s*1;[\s\S]*min-height:\s*0;/);
  });

  it("keeps metadata and embedded chat in the shared right side rail", () => {
    const preview = read("src/components/knowledge/KnowledgePreview.vue");

    expect(preview).toContain('const sidePanelTab = ref<"meta" | "chat">("chat")');
    expect(preview).toContain("const DEFAULT_SIDE_PANEL_WIDTH = 420");
    expect(preview).toContain("const COLLAPSED_SIDE_PANEL_WIDTH = 42");
    expect(preview).toContain("const MAX_SIDE_PANEL_WIDTH = 720");
    expect(preview).toContain("const MIN_MAIN_COLUMN_WIDTH = 320");
    expect(preview).toContain('const SIDE_RAIL_COLLAPSED_STORAGE_KEY = "locus:knowledgePreviewSideRailCollapsed"');
    expect(preview).toContain("const metaCollapsed = ref(loadStoredBoolean(SIDE_RAIL_COLLAPSED_STORAGE_KEY) ?? false)");
    expect(preview).toContain("const sidePanelWidth = ref(DEFAULT_SIDE_PANEL_WIDTH)");
    expect(preview).toContain("const isSideResizing = ref(false)");
    expect(preview).toContain("const sideRailStyle = computed(() => {");
    expect(preview).toContain('width: `clamp(${MIN_SIDE_PANEL_WIDTH}px, ${sidePanelWidth.value}px, calc(100% - ${MIN_MAIN_COLUMN_WIDTH}px))`');
    expect(preview).toContain("function loadStoredBoolean(storageKey: string): boolean | null");
    expect(preview).toContain("function persistStoredBoolean(storageKey: string, value: boolean)");
    expect(preview).toContain("function toggleSideRail()");
    expect(preview).toContain("persistStoredBoolean(SIDE_RAIL_COLLAPSED_STORAGE_KEY, nextValue)");
    expect(preview).toContain("function onSideResizeStart(event: MouseEvent)");
    expect(preview).toContain("event.preventDefault()");
    expect(preview).toContain('class="preview-side-rail"');
    expect(preview).toContain('class="preview-side-resize-handle"');
    expect(preview).toContain('class="preview-side-tabs"');
    expect(preview).toContain('class="preview-side-toggle preview-side-toggle-tab"');
    expect(preview).toContain('@click="toggleSideRail"');
    expect(preview).toContain("import BaseSegmented from \"../ui/BaseSegmented.vue\"");
    expect(preview).toContain("<KnowledgeChatPane :document=\"document\" />");
    expect(preview).toMatch(/\.preview-side-rail\s*\{[\s\S]*display:\s*flex;[\s\S]*flex-direction:\s*column;/);
    expect(preview).toMatch(/\.preview-side-rail\.is-resizing\s*\{[\s\S]*transition:\s*none;/);
    expect(preview).toMatch(/\.preview-main-column\s*\{[\s\S]*display:\s*flex;[\s\S]*flex-direction:\s*column;/);
    expect(preview).toMatch(/\.preview-side-resize-handle\s*\{[\s\S]*cursor:\s*col-resize;/);
    expect(preview).not.toContain("@media (max-width: 1180px)");
  });

  it("groups summary and maintenance rules into a compact collapsible strip", () => {
    const preview = read("src/components/knowledge/KnowledgePreview.vue");

    expect(preview).toContain('const SUPPORT_PANELS_STORAGE_KEY = "locus:knowledgePreviewSupportPanelsCollapsed"');
    expect(preview).toContain('const SUPPORT_STRIP_HEIGHT_STORAGE_KEY = "locus:knowledgePreviewSupportStripHeight"');
    expect(preview).toContain('const SUPPORT_SECTION_WIDTH_STORAGE_KEY = "locus:knowledgePreviewSupportSectionWidth"');
    expect(preview).toContain("const supportPanelsCollapsedPreference = ref<boolean | null>(loadStoredSupportPanelsCollapsed())");
    expect(preview).toContain("const supportPanelsCollapsed = ref(supportPanelsCollapsedPreference.value ?? true)");
    expect(preview).toContain("const hasSupportPanels = computed(() => visibleSections.value.summary || visibleSections.value.maintenanceRules)");
    expect(preview).toContain("const hasTwoSupportSections = computed(() => visibleSections.value.summary && visibleSections.value.maintenanceRules)");
    expect(preview).toContain("const supportStripHeight = ref(loadStoredPanelSize(");
    expect(preview).toContain("const supportPrimaryWidth = ref(loadStoredPanelSize(");
    expect(preview).toContain("function toggleSupportPanels()");
    expect(preview).toContain("function currentSupportStripMinHeight()");
    expect(preview).toContain("function onSupportHeightResizeStart(event: MouseEvent)");
    expect(preview).toContain("function onSupportWidthResizeStart(event: MouseEvent)");
    expect(preview).toContain('class="preview-support-strip"');
    expect(preview).toContain('class="preview-support-layout"');
    expect(preview).toContain('class="preview-support-toggle"');
    expect(preview).toContain('class="preview-support-divider"');
    expect(preview).toContain('class="preview-main-divider"');
    expect(preview).toContain('class="preview-support-section"');
    expect(preview).toContain('class="preview-support-section-header"');
    expect(preview).toContain('class="preview-support-section-body"');
    expect(preview).toContain('@click="toggleSupportPanels"');
    expect(preview).toMatch(/\.preview-support-strip\s*\{[\s\S]*position:\s*relative;/);
    expect(preview).toMatch(/\.preview-support-layout\s*\{[\s\S]*flex:\s*1;[\s\S]*display:\s*grid;[\s\S]*grid-template-columns:\s*minmax\(0,\s*1fr\);/);
    expect(preview).toMatch(/\.preview-support-layout\.has-two-sections\s*\{[\s\S]*grid-template-columns:\s*minmax\(0,\s*1fr\)\s+8px\s+minmax\(0,\s*1fr\);/);
    expect(preview).toMatch(/\.preview-support-layout\.has-two-sections\.is-compact\s*\{[\s\S]*grid-template-columns:\s*minmax\(0,\s*1fr\);[\s\S]*grid-template-rows:\s*minmax\(0,\s*1fr\)\s+8px\s+minmax\(0,\s*1fr\);/);
    expect(preview).toMatch(/\.preview-support-toggle\s*\{[\s\S]*position:\s*absolute;[\s\S]*width:\s*20px;[\s\S]*height:\s*20px;/);
    expect(preview).toMatch(/\.preview-support-section-header\s*\{[\s\S]*flex-shrink:\s*0;[\s\S]*min-height:\s*46px;/);
    expect(preview).toMatch(/\.preview-support-section-first\s+\.preview-support-section-header\s*\{[\s\S]*padding-left:\s*36px;/);
    expect(preview).toMatch(/\.preview-support-section-body\s*\{[\s\S]*display:\s*flex;[\s\S]*flex:\s*1 1 auto;[\s\S]*min-height:\s*0;[\s\S]*border-top:\s*1px solid[\s\S]*height:\s*auto;/);
    expect(preview).toMatch(/\.preview-support-divider\s*\{[\s\S]*position:\s*relative;[\s\S]*width:\s*8px;[\s\S]*background:\s*transparent;/);
    expect(preview).toMatch(/\.preview-main-divider\s*\{[\s\S]*height:\s*8px;[\s\S]*cursor:\s*row-resize;/);
    expect(preview).toMatch(/\.preview-support-section-body\s*:deep\(\.base-markdown-editor \.vditor-ir pre\.vditor-reset\)\s*\{[\s\S]*height:\s*100%;[\s\S]*overflow:\s*auto;/);
    expect(preview).toMatch(/\.preview-support-section-body\s*:deep\(\.base-markdown-editor \.base-markdown-editor-textarea\)\s*\{[\s\S]*height:\s*100%;[\s\S]*overflow:\s*auto;/);
    expect(preview).toMatch(/\.preview-support-chevron\.open\s*\{[\s\S]*transform:\s*rotate\(90deg\);/);
  });

  it("keeps maintenance rules as an inline metadata control", () => {
    const preview = read("src/components/knowledge/KnowledgePreview.vue");

    expect(preview).toContain('class="meta-control meta-control-switch"');
    expect(preview).toContain(`:title="t('knowledge.meta.explicitMaintenanceRulesHint')"`);
    expect(preview).not.toContain("BaseCheckbox");
  });

  it("keeps metadata dropdowns in place while raising the open side rail", () => {
    const preview = read("src/components/knowledge/KnowledgePreview.vue");
    const dropdown = read("src/components/ui/BaseDropdown.vue");

    expect(dropdown).toContain(':class="[`size-${size}`, { open }]"');
    expect(dropdown).not.toContain("<Teleport");
    expect(preview).not.toContain("teleport");
    expect(preview).toMatch(/\.preview-side-rail:has\(\.meta-dropdown\.open\)\s*\{[\s\S]*z-index:\s*20;[\s\S]*overflow:\s*visible;/);
    expect(preview).toMatch(/\.preview-side-rail:has\(\.meta-dropdown\.open\) \.preview-side-rail-body,[\s\S]*\.preview-side-rail:has\(\.meta-dropdown\.open\) \.preview-side-rail-panel\s*\{[\s\S]*overflow:\s*visible;/);
  });

  it("shows skill command fields only for command-capable skill surfaces", () => {
    const preview = read("src/components/knowledge/KnowledgePreview.vue");

    expect(preview).toContain("const showSkillCommandFields = computed(() =>");
    expect(preview).toContain("isSkillDocument.value && skillEnabled.value && skillSurfaceAllowsCommand(currentSkillSurface.value)");
    expect(preview).toContain('v-if="showSkillCommandFields" class="meta-row meta-row-control"');
    expect(preview).toContain('t("knowledge.skill.commandTrigger")');
    expect(preview).toContain('t("knowledge.skill.argumentHint")');
    expect(preview).not.toContain(`v-if="document.type === 'skill'" class="meta-row meta-row-control">
                  <span class="meta-label">{{ t("knowledge.skill.commandTrigger") }}</span>`);
  });

  it("keeps skill command text inputs readable in the metadata rail", () => {
    const preview = read("src/components/knowledge/KnowledgePreview.vue");

    expect(preview).toMatch(/\.meta-dropdown :deep\(\.base-dropdown-trigger\)\s*\{[\s\S]*min-height:\s*30px;/);
    expect(preview).toMatch(/\.meta-text-input\s*\{[\s\S]*width:\s*100%;[\s\S]*height:\s*30px;[\s\S]*min-height:\s*30px;[\s\S]*box-sizing:\s*border-box;[\s\S]*line-height:\s*18px;/);
  });

  it("renders file metadata below the document config block", () => {
    const preview = read("src/components/knowledge/KnowledgePreview.vue");

    expect(preview).toContain("const documentFileMetadata = computed(() => props.document?.fileMetadata ?? null);");
    expect(preview).toContain("function formatDocumentLength(");
    expect(preview).toContain('class="preview-side-rail-panel preview-side-rail-panel-meta"');
    expect(preview).toContain('class="meta-group meta-group-file"');
    expect(preview).toContain('t("knowledge.meta.fileSize")');
    expect(preview).toContain('t("knowledge.meta.length")');
    expect(preview).toContain('t("knowledge.meta.estimatedTokens")');
    expect(preview).toContain('t("knowledge.meta.modifiedAt")');
    expect(preview).toContain('t("knowledge.meta.lastCommit")');
    expect(preview).toContain('class="meta-value meta-value-wrap"');
    expect(preview).toMatch(/\.preview-side-rail-panel-meta\s*\{[\s\S]*display:\s*flex;[\s\S]*flex-direction:\s*column;/);
    expect(preview).toMatch(/\.meta-stack\s*\{[\s\S]*flex:\s*1;[\s\S]*min-height:\s*100%;[\s\S]*display:\s*flex;[\s\S]*flex-direction:\s*column;/);
    expect(preview).toMatch(/\.meta-group-file\s*\{[\s\S]*margin-top:\s*auto;/);
    expect(preview).toMatch(/\.meta-value-wrap\s*\{[\s\S]*white-space:\s*normal;[\s\S]*overflow-wrap:\s*anywhere;/);
  });

  it("labels app-backed documents separately from project documents", () => {
    const preview = read("src/components/knowledge/KnowledgePreview.vue");

    expect(preview).toContain('props.document?.storageSource === "app"');
    expect(preview).toContain('t("knowledge.meta.storageSourceApp")');
    expect(preview).toContain('t("knowledge.meta.storageSourceProject")');
  });

  it("renders subtle search-hit cues inside the matched preview section", () => {
    const preview = read("src/components/knowledge/KnowledgePreview.vue");

    expect(preview).toContain('import MarkdownRenderer from "../MarkdownRenderer.vue"');
    expect(preview).toContain('import SemanticCodeRenderer from "../ui/SemanticCodeRenderer.vue"');
    expect(preview).toContain('import { semanticCodeLanguageFromPath } from "../../composables/semanticCodeRendering"');
    expect(preview).toContain("searchContext?: KnowledgeSearchSelectionContext | null;");
    expect(preview).toContain("const activeSearchContext = computed(() => {");
    expect(preview).toContain("const matchesCurrentDocument = props.document.id === result.id");
    expect(preview).toContain("const showSearchRenderedContent = computed(() =>");
    expect(preview).toContain("const bodyCodeLanguage = computed(() => semanticCodeLanguageFromPath(documentPath.value))");
    expect(preview).toContain('const summaryRenderedSearchRef = ref<HTMLElement | null>(null)');
    expect(preview).toContain('const rulesRenderedSearchRef = ref<HTMLElement | null>(null)');
    expect(preview).toContain('const bodyRenderedSearchRef = ref<HTMLElement | null>(null)');
    expect(preview).toContain("function scrollSearchMatchIntoView(): boolean {");
    expect(preview).toContain("function clearTargetSearchMark() {");
    expect(preview).toContain('mark.markdown-search-mark-target');
    expect(preview).toContain('container.scrollTo({');
    expect(preview).toContain('behavior: "smooth"');
    expect(preview).toContain("function scheduleSearchMatchScroll()");
    expect(preview).toContain("function searchSnippetVisible(section: KnowledgeSearchMatchSection): boolean {");
    expect(preview).toContain('class="preview-search-hit"');
    expect(preview).toContain('class="preview-search-hit-mark"');
    expect(preview).toContain('class="preview-rendered-search"');
    expect(preview).toContain("<SemanticCodeRenderer");
    expect(preview).toContain('v-if="bodyCodeLanguage"');
    expect(preview).toContain(':content-path="documentPath"');
    expect(preview).toContain(':highlight-terms="searchQueryTerms"');
    expect(preview).toContain('ref="summaryRenderedSearchRef"');
    expect(preview).toContain('ref="rulesRenderedSearchRef"');
    expect(preview).toContain('ref="bodyRenderedSearchRef"');
    expect(preview).toContain("'is-search-match': isSearchMatchSection('maintenanceRules')");
    expect(preview).toContain(":class=\"{ 'is-search-match': isSearchMatchSection('body') }\"");
    expect(preview).toContain('supportPanelsCollapsed.value = false;');
    expect(preview).toMatch(/\.preview-search-hit\s*\{[\s\S]*border:\s*1px solid[\s\S]*background:/);
    expect(preview).toMatch(/\.preview-rendered-search\s*\{[\s\S]*overflow:\s*auto;[\s\S]*padding:\s*14px 16px 16px;/);
    expect(preview).toMatch(/\.preview-pane-body\.is-search-match\s+\.preview-pane-header\s*\{/);
  });
});
