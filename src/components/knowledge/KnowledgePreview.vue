<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import type {
  KnowledgeDocument,
  KnowledgeDocumentPatch,
  KnowledgeEditMode,
  KnowledgeDocumentSection,
  KnowledgeSearchMatchSection,
  KnowledgeSearchSelectionContext,
  KnowledgeDocumentType,
  KnowledgeInjectMode,
  SkillManifest,
  SkillUnityInstallStatus,
  SkillSurface,
} from "../../types";
import { skillSurfaceAllowsCommand } from "../../types";
import { t } from "../../i18n";
import { useNotificationStore } from "../../stores/notification";
import { useSkills } from "../../composables/useSkills";
import {
  findSkillCommandConflict,
  isValidSkillCommandTrigger,
  normalizeSkillCommandTrigger,
  SKILL_COMMAND_NOTICE_OPERATION,
} from "../../composables/skillCommands";
import BaseDropdown from "../ui/BaseDropdown.vue";
import BaseMarkdownEditor from "../ui/BaseMarkdownEditor.vue";
import BaseSwitch from "../ui/BaseSwitch.vue";
import MarkdownRenderer from "../MarkdownRenderer.vue";
import KnowledgeChatPane from "./KnowledgeChatPane.vue";
import {
  getSkillUnityInstallStatus,
  installSkillUnityFiles,
  removeSkillUnityFiles,
} from "../../services/knowledge";
import {
  hintForInjectMode,
  hintForKnowledgeEditMode,
  labelForInheritedValue,
  labelForInjectMode,
  labelForKnowledgeEditMode,
} from "./knowledgeMetaLabels";
import {
  createKnowledgeEditorDraftValues,
  mergeKnowledgeEditorDraftValues,
  normalizeKnowledgeEditorValue,
} from "./knowledgeEditorDrafts";
import { getKnowledgeDocumentEditorSections } from "./knowledgeDocumentSections";
import {
  buildKnowledgeEditModePatch,
  defaultExplicitMaintenanceRulesForType,
  defaultMaintenanceRulesForType,
  getKnowledgeEditMode,
  isKnowledgeEditModeLocked,
} from "./knowledgeEditMode";
import { acquireSelectionLock } from "../../composables/useSelectionLock";
import {
  createAnimationFrameResizeObserver,
  type ResizeObserverHandle,
} from "../../composables/resizeObserver";
import BaseSegmented from "../ui/BaseSegmented.vue";
import {
  useMarkdownEditorViewMode,
  type MarkdownEditorViewMode,
} from "../ui/markdownEditorViewMode";

const AUTO_SAVE_DELAY_MS = 700;
const DEFAULT_SIDE_PANEL_WIDTH = 420;
const COLLAPSED_SIDE_PANEL_WIDTH = 42;
const MIN_SIDE_PANEL_WIDTH = 280;
const MAX_SIDE_PANEL_WIDTH = 720;
const MIN_MAIN_COLUMN_WIDTH = 320;
const SUPPORT_PANELS_STORAGE_KEY = "locus:knowledgePreviewSupportPanelsCollapsed";
const SUPPORT_STRIP_HEIGHT_STORAGE_KEY = "locus:knowledgePreviewSupportStripHeight";
const SUPPORT_SECTION_WIDTH_STORAGE_KEY = "locus:knowledgePreviewSupportSectionWidth";
const DEFAULT_SUPPORT_STRIP_HEIGHT = 182;
const MIN_SUPPORT_STRIP_HEIGHT = 112;
const MIN_BODY_HEIGHT = 180;
const DEFAULT_SUPPORT_SECTION_WIDTH = 360;
const MIN_SUPPORT_SECTION_WIDTH = 180;
const SUPPORT_LAYOUT_COMPACT_WIDTH = 680;
const MEMORY_PREVIEW_PATH_PREFIX = "unity-project-understanding";
const BUILTIN_MEMORY_PREVIEW_PATHS = new Set([
  "project-mistake-note.md",
  "user-preference.md",
]);
const notificationStore = useNotificationStore();
const { skillItems, loadSkills } = useSkills();
const { markdownEditorViewMode, setMarkdownEditorViewMode } = useMarkdownEditorViewMode();
type InjectModeSelection = KnowledgeInjectMode | "inherit_parent";

const props = defineProps<{
  document: KnowledgeDocument | null;
  searchContext?: KnowledgeSearchSelectionContext | null;
  loading: boolean;
  saveLoading: boolean;
}>();

const emit = defineEmits<{
  (e: "close"): void;
  (e: "delete"): void;
  (e: "saveSection", section: KnowledgeDocumentSection, value: string): void;
  (e: "updateMeta", patch: KnowledgeDocumentPatch): void;
}>();

const summaryDraft = ref("");
const rulesDraft = ref("");
const bodyDraft = ref("");
const fileNameDraft = ref("");
const fileNameDirty = ref(false);
const dirtySections = ref<Set<KnowledgeDocumentSection>>(new Set());
const autoSaveQueued = ref(false);
const autoSaveInFlight = ref(false);
const confirmDelete = ref(false);
const metaCollapsed = ref(false);
const isSideResizing = ref(false);
const isSupportHeightResizing = ref(false);
const isSupportWidthResizing = ref(false);
const sidePanelTab = ref<"meta" | "chat">("chat");
const sidePanelWidth = ref(DEFAULT_SIDE_PANEL_WIDTH);
const skillCommandDraft = ref("");
const skillArgumentHintDraft = ref("");
const skillUnityStatus = ref<SkillUnityInstallStatus | null>(null);
const skillUnityStatusLoading = ref(false);
const skillUnityActionPending = ref(false);
const supportPanelsCollapsedPreference = ref<boolean | null>(loadStoredSupportPanelsCollapsed());
const supportPanelsCollapsed = ref(supportPanelsCollapsedPreference.value ?? true);
const previewMainRef = ref<HTMLElement | null>(null);
const supportLayoutRef = ref<HTMLElement | null>(null);
const summaryRenderedSearchRef = ref<HTMLElement | null>(null);
const rulesRenderedSearchRef = ref<HTMLElement | null>(null);
const bodyRenderedSearchRef = ref<HTMLElement | null>(null);
const supportLayoutCompact = ref(false);
const supportStripHeight = ref(loadStoredPanelSize(
  SUPPORT_STRIP_HEIGHT_STORAGE_KEY,
  DEFAULT_SUPPORT_STRIP_HEIGHT,
));
const supportPrimaryWidth = ref(loadStoredPanelSize(
  SUPPORT_SECTION_WIDTH_STORAGE_KEY,
  DEFAULT_SUPPORT_SECTION_WIDTH,
));
let autoSaveTimer: ReturnType<typeof setTimeout> | null = null;
let sideResizing = false;
let supportHeightResizing = false;
let supportWidthResizing = false;
let sideResizeStartX = 0;
let sideResizeStartWidth = DEFAULT_SIDE_PANEL_WIDTH;
let supportHeightResizeStartY = 0;
let supportHeightResizeStartValue = DEFAULT_SUPPORT_STRIP_HEIGHT;
let supportWidthResizeStartX = 0;
let supportWidthResizeStartValue = DEFAULT_SUPPORT_SECTION_WIDTH;
let bodyCursorBeforeResize = "";
let releaseSelectionLock: (() => void) | null = null;
let layoutResizeObserver: ResizeObserverHandle | null = null;
let searchMatchScrollFrame = 0;

function formatDocumentDisplayPath(document: KnowledgeDocument | null | undefined): string {
  if (!document) return "";
  const path = document.path.trim().replace(/\\/g, "/").replace(/^\/+/, "");
  if (
    document.type === "memory"
    && !path.includes("/")
    && BUILTIN_MEMORY_PREVIEW_PATHS.has(path)
  ) {
    return `${MEMORY_PREVIEW_PATH_PREFIX}/${path}`;
  }
  return path;
}

function packageIdForSkillDocument(document: KnowledgeDocument | null | undefined): string {
  if (!document || document.type !== "skill") return "";
  if (document.storageSource !== "app") return "";
  if (document.externalSource?.provider !== "package") return "";
  return document.externalSource.sourceId || document.path.split("/")[0] || "";
}

function skillPackageManifestForDocument(document: KnowledgeDocument | null | undefined): SkillManifest | null {
  const packageId = packageIdForSkillDocument(document);
  if (!packageId) return null;
  return skillItems.value.find((item) =>
    item.source === "app"
    && item.kind === "package"
    && (item.packageId === packageId || item.dirName === packageId)
  ) ?? null;
}

function skillPackageL1Unavailable(): boolean {
  const manifest = skillPackageManifestForDocument(props.document);
  return manifest?.hasL1 === false;
}

const isReadOnly = computed(() => !!props.document?.readOnly);
const isEditModeLocked = computed(() => isKnowledgeEditModeLocked(props.document));
const documentPath = computed(() => props.document?.path?.trim() || "");
const documentDisplayPath = computed(() => formatDocumentDisplayPath(props.document));
const documentTitle = computed(() => currentDocumentFileStem.value || t("knowledge.preview.untitled"));
const titleMeasureText = computed(() => fileNameDraft.value || " ");
const typeLabel = computed(() => labelForType(props.document?.type));
const scopeLabel = computed(() => labelForStoredScope(props.document));
const injectMode = computed(() => props.document?.injectMode ?? "none");
const injectModeSelection = computed<InjectModeSelection>(() => (
  props.document?.inheritInjectMode ? "inherit_parent" : (props.document?.injectMode ?? "none")
));
const summaryEnabled = computed(() => !!props.document?.summaryEnabled);
const editMode = computed<KnowledgeEditMode>(() => getKnowledgeEditMode(props.document));
const explicitRulesEnabled = computed(() => !!props.document?.explicitMaintenanceRules);
const explicitRulesLocked = computed(() => editMode.value === "auto" || editMode.value === "inherit_parent");
const injectModeOptions = computed(() => [
  {
    value: "inherit_parent",
    label: t("knowledge.meta.inheritParent"),
    hint: t("knowledge.meta.inheritParentHint"),
  },
  {
    value: "none",
    label: labelForInjectMode("none"),
    hint: hintForInjectMode("none"),
  },
  {
    value: "path",
    label: labelForInjectMode("path"),
    hint: hintForInjectMode("path"),
  },
  {
    value: "excerpt",
    label: labelForInjectMode("excerpt"),
    hint: skillPackageL1Unavailable()
      ? t("knowledge.skill.l1Unavailable")
      : hintForInjectMode("excerpt"),
    disabled: skillPackageL1Unavailable(),
  },
  {
    value: "full",
    label: labelForInjectMode("full"),
    hint: hintForInjectMode("full"),
    disabled: props.document?.type === "skill" || props.document?.type === "reference",
  },
  {
    value: "rule",
    label: labelForInjectMode("rule"),
    hint: hintForInjectMode("rule"),
    disabled: props.document?.type === "skill" || props.document?.type === "reference",
  },
]);
const editModeOptions = computed(() => [
  {
    value: "inherit_parent",
    label: labelForKnowledgeEditMode("inherit_parent"),
    hint: hintForKnowledgeEditMode("inherit_parent"),
  },
  {
    value: "read_only",
    label: labelForKnowledgeEditMode("read_only"),
    hint: hintForKnowledgeEditMode("read_only"),
  },
  {
    value: "proposal",
    label: labelForKnowledgeEditMode("proposal"),
    hint: hintForKnowledgeEditMode("proposal"),
  },
  {
    value: "auto",
    label: labelForKnowledgeEditMode("auto"),
    hint: hintForKnowledgeEditMode("auto"),
  },
]);
const effectiveEditMode = computed<Exclude<KnowledgeEditMode, "inherit_parent">>(() => {
  if (props.document?.readOnly && !props.document?.inheritAiConfig) return "read_only";
  return props.document?.aiMaintained ? "auto" : "proposal";
});
const injectModeDropdownLabel = computed(() => {
  if (!props.document) return "";
  const effectiveLabel = labelForInjectMode(injectMode.value);
  return props.document.inheritInjectMode
    ? labelForInheritedValue(effectiveLabel, props.document.injectModeSource)
    : effectiveLabel;
});
const editModeDropdownLabel = computed(() => {
  if (!props.document) return "";
  const effectiveLabel = labelForKnowledgeEditMode(effectiveEditMode.value);
  return props.document.inheritAiConfig
    ? labelForInheritedValue(effectiveLabel, props.document.aiConfigSource)
    : labelForKnowledgeEditMode(editMode.value);
});
const rulesEditorDisabled = computed(() => isReadOnly.value || !!props.document?.inheritAiConfig);
const rulesHint = computed(() => (
  props.document?.inheritAiConfig
    ? t("knowledge.preview.rulesInheritedHint")
    : t("knowledge.preview.rulesHint")
));

const sourceSummary = computed(() => {
  const source = props.document?.externalSource;
  if (!source) {
    return props.document?.storageSource === "app"
      ? t("knowledge.meta.storageSourceApp")
      : t("knowledge.meta.storageSourceProject");
  }
  const locator = source.locator?.trim();
  return [labelForProvider(source.provider), locator].filter(Boolean).join(" · ");
});
const documentFileMetadata = computed(() => props.document?.fileMetadata ?? null);
const countFormatter = new Intl.NumberFormat();
const fileSizeLabel = computed(() => formatByteSize(documentFileMetadata.value?.byteSize));
const fileLengthLabel = computed(() =>
  formatDocumentLength(
    documentFileMetadata.value?.lineCount,
    documentFileMetadata.value?.charCount,
  ));
const estimatedTokensLabel = computed(() =>
  formatCount(documentFileMetadata.value?.estimatedTokens),
);
const modifiedAtLabel = computed(() =>
  formatDateTime(documentFileMetadata.value?.modifiedAt),
);
const lastCommitLabel = computed(() => {
  const author = documentFileMetadata.value?.lastCommitAuthor?.trim();
  const committedAt = formatDateTime(documentFileMetadata.value?.lastCommitAt);
  if (author && committedAt !== "—") return `${author} · ${committedAt}`;
  if (author) return author;
  if (committedAt !== "—") return committedAt;
  return "";
});
const showLastCommit = computed(() => !!lastCommitLabel.value);

const hasUnsavedSectionChanges = computed(() => {
  const doc = props.document;
  if (!doc) return false;
  return normalizeKnowledgeEditorValue(summaryDraft.value) !== normalizeKnowledgeEditorValue(doc.summary ?? "")
    || normalizeKnowledgeEditorValue(rulesDraft.value) !== normalizeKnowledgeEditorValue(doc.maintenanceRules ?? "")
    || normalizeKnowledgeEditorValue(bodyDraft.value) !== normalizeKnowledgeEditorValue(doc.body ?? "");
});
const currentDocumentFileStem = computed(() => extractDocumentFileStem(props.document?.path));
const hasUnsavedChanges = computed(() => hasUnsavedSectionChanges.value || fileNameDirty.value);

const statusLabel = computed(() => {
  if (!props.document) return "";
  if (props.saveLoading && !autoSaveInFlight.value) return t("knowledge.editor.saving");
  if (hasUnsavedChanges.value || autoSaveQueued.value || autoSaveInFlight.value) return t("knowledge.editor.unsaved");
  return t("knowledge.editor.saved");
});

const footerLabel = computed(() =>
  props.document ? `${statusLabel.value} · ${t("knowledge.editor.shortcut")}` : "",
);
const footerWarning = computed(() =>
  hasUnsavedChanges.value && !autoSaveQueued.value && !autoSaveInFlight.value,
);
const visibleSections = computed(() => getKnowledgeDocumentEditorSections(props.document));
const hasSupportPanels = computed(() => visibleSections.value.summary || visibleSections.value.maintenanceRules);
const hasTwoSupportSections = computed(() => visibleSections.value.summary && visibleSections.value.maintenanceRules);
const sidePanelOptions = computed(() => [
  { value: "meta", label: t("knowledge.side.meta") },
  { value: "chat", label: t("knowledge.side.chat") },
]);
const editorViewOptions = computed(() => [
  { value: "rendered", label: t("knowledge.editor.view.rendered") },
  { value: "native", label: t("knowledge.editor.view.native") },
]);
const editorViewMode = computed<MarkdownEditorViewMode>({
  get: () => markdownEditorViewMode.value,
  set: (value) => setMarkdownEditorViewMode(value),
});
const fallbackSkillName = computed(() => inferSkillName(props.document));
const isSkillDocument = computed(() => props.document?.type === "skill");
const skillEnabled = computed(() => (
  isSkillDocument.value ? props.document?.skillEnabled !== false : false
));
const currentSkillSurface = computed<SkillSurface | undefined>(() => (
  isSkillDocument.value ? (props.document?.skillSurface ?? "command") : undefined
));
const skillSurfaceValue = computed(() => (
  !isSkillDocument.value
    ? "disabled"
    : skillEnabled.value
      ? currentSkillSurface.value ?? "command"
      : "disabled"
));
const skillSurfaceOptions = computed(() => [
  {
    value: "disabled",
    label: t("knowledge.skill.surfaceDisabled"),
    hint: t("knowledge.skill.surfaceDisabledHint"),
  },
  {
    value: "command",
    label: t("knowledge.skill.surfaceCommand"),
    hint: t("knowledge.skill.surfaceCommandHint"),
  },
  {
    value: "auto",
    label: t("knowledge.skill.surfaceAuto"),
    hint: t("knowledge.skill.surfaceAutoHint"),
  },
  {
    value: "both",
    label: t("knowledge.skill.surfaceBoth"),
    hint: t("knowledge.skill.surfaceBothHint"),
  },
]);
const currentSkillCommandTrigger = computed(() => {
  if (!isSkillDocument.value) return "";
  return normalizeSkillCommandTrigger(props.document?.commandTrigger ?? "", fallbackSkillName.value);
});
const skillCommandInputDisabled = computed(() =>
  isReadOnly.value || props.saveLoading || !skillEnabled.value || !skillSurfaceAllowsCommand(currentSkillSurface.value),
);
const showSkillCommandFields = computed(() =>
  isSkillDocument.value && skillEnabled.value && skillSurfaceAllowsCommand(currentSkillSurface.value),
);
const skillPackageId = computed(() => {
  return packageIdForSkillDocument(props.document);
});
const showSkillUnityStatus = computed(() => Boolean(skillPackageId.value && skillUnityStatus.value?.hasUnity));
const skillUnityStatusLabel = computed(() => {
  const state = skillUnityStatus.value?.state ?? "";
  switch (state) {
    case "pluginMissing":
      return t("knowledge.skill.unityStatus.pluginMissing");
    case "notInstalled":
      return t("knowledge.skill.unityStatus.notInstalled");
    case "installed":
      return t("knowledge.skill.unityStatus.installed");
    case "partial":
      return t("knowledge.skill.unityStatus.partial");
    case "modified":
      return t("knowledge.skill.unityStatus.modified");
    case "sourceMissing":
      return t("knowledge.skill.unityStatus.sourceMissing");
    default:
      return t("knowledge.skill.unityStatus.notApplicable");
  }
});
const canInstallSkillUnityFiles = computed(() => {
  const state = skillUnityStatus.value?.state;
  return !!skillPackageId.value
    && !!skillUnityStatus.value?.hasUnity
    && state !== "pluginMissing"
    && state !== "sourceMissing"
    && state !== "installed";
});
const canRemoveSkillUnityFiles = computed(() => {
  const state = skillUnityStatus.value?.state;
  return !!skillPackageId.value
    && !!skillUnityStatus.value?.hasUnity
    && (state === "installed" || state === "modified" || state === "partial");
});
const sideRailStyle = computed(() => {
  if (metaCollapsed.value) {
    return {
      width: `${COLLAPSED_SIDE_PANEL_WIDTH}px`,
    };
  }
  return {
    width: `clamp(${MIN_SIDE_PANEL_WIDTH}px, ${sidePanelWidth.value}px, calc(100% - ${MIN_MAIN_COLUMN_WIDTH}px))`,
  };
});
const summaryPreviewText = computed(() => buildCollapsedPreview(summaryDraft.value, t("knowledge.preview.summaryPlaceholder")));
const rulesPreviewText = computed(() => buildCollapsedPreview(rulesDraft.value, t("knowledge.preview.rulesPlaceholder")));
const supportStripStyle = computed(() => {
  if (!hasSupportPanels.value || supportPanelsCollapsed.value) return undefined;
  return {
    height: `${supportStripHeight.value}px`,
  };
});
const supportLayoutStyle = computed(() => {
  if (!hasTwoSupportSections.value) return undefined;
  if (supportLayoutCompact.value) {
    return {
      gridTemplateRows: "minmax(0, 1fr) 8px minmax(0, 1fr)",
    };
  }
  return {
    gridTemplateColumns: `${supportPrimaryWidth.value}px 8px minmax(0, 1fr)`,
  };
});
const isPreviewResizing = computed(() =>
  isSideResizing.value || isSupportHeightResizing.value || isSupportWidthResizing.value,
);
const activeSearchContext = computed(() => {
  if (!props.document || !props.searchContext) return null;
  const result = props.searchContext.result;
  const matchesCurrentDocument = props.document.id === result.id
    || (
      props.document.type === result.type
      && props.document.path === result.path
    );
  return matchesCurrentDocument ? props.searchContext : null;
});
const searchMatchSection = computed<KnowledgeSearchMatchSection>(() => (
  activeSearchContext.value?.result.matchedSection ?? "body"
));
const searchQueryTerms = computed(() => {
  const raw = activeSearchContext.value?.query?.trim() ?? "";
  if (!raw) return [];
  return [...new Set(raw.split(/\s+/).filter(Boolean))].sort((left, right) => right.length - left.length);
});
const showSearchRenderedContent = computed(() =>
  !!activeSearchContext.value && editorViewMode.value === "rendered"
);
const searchHighlightRe = computed<RegExp | null>(() => {
  if (!searchQueryTerms.value.length) return null;
  return new RegExp(`(${searchQueryTerms.value.map(escapeRegExp).join("|")})`, "gi");
});

function loadStoredSupportPanelsCollapsed(): boolean | null {
  try {
    const raw = localStorage.getItem(SUPPORT_PANELS_STORAGE_KEY);
    if (raw === "true") return true;
    if (raw === "false") return false;
  } catch {
    // ignore persistence failures
  }
  return null;
}

function loadStoredPanelSize(storageKey: string, fallback: number): number {
  try {
    const raw = localStorage.getItem(storageKey);
    const parsed = raw ? Number(raw) : Number.NaN;
    if (Number.isFinite(parsed)) return parsed;
  } catch {
    // ignore persistence failures
  }
  return fallback;
}

function persistStoredPanelSize(storageKey: string, value: number) {
  try {
    localStorage.setItem(storageKey, String(Math.round(value)));
  } catch {
    // ignore persistence failures
  }
}

function persistSupportPanelsCollapsed(value: boolean) {
  try {
    localStorage.setItem(SUPPORT_PANELS_STORAGE_KEY, String(value));
  } catch {
    // ignore persistence failures
  }
}

function toggleSupportPanels() {
  const nextValue = !supportPanelsCollapsed.value;
  supportPanelsCollapsed.value = nextValue;
  supportPanelsCollapsedPreference.value = nextValue;
  persistSupportPanelsCollapsed(nextValue);
  if (!nextValue) refreshSupportLayoutMetrics();
}

function lockResizeInteraction(cursor: "col-resize" | "row-resize") {
  bodyCursorBeforeResize = document.body.style.cursor;
  document.body.style.cursor = cursor;
  releaseSelectionLock?.();
  releaseSelectionLock = acquireSelectionLock();
}

function unlockResizeInteraction() {
  document.body.style.cursor = bodyCursorBeforeResize;
  releaseSelectionLock?.();
  releaseSelectionLock = null;
}

function currentSupportLayoutWidth() {
  return supportLayoutRef.value?.getBoundingClientRect().width ?? 0;
}

function currentSupportStripMinHeight() {
  return hasTwoSupportSections.value && supportLayoutCompact.value
    ? MIN_SUPPORT_STRIP_HEIGHT * 2 + 8
    : MIN_SUPPORT_STRIP_HEIGHT;
}

function clampSupportStripHeight(next: number) {
  const minHeight = currentSupportStripMinHeight();
  const containerHeight = previewMainRef.value?.getBoundingClientRect().height ?? 0;
  if (!containerHeight) return Math.max(minHeight, next);
  const maxHeight = Math.max(minHeight, containerHeight - MIN_BODY_HEIGHT);
  return Math.min(maxHeight, Math.max(minHeight, next));
}

function clampSupportPrimaryWidth(next: number) {
  const layoutWidth = currentSupportLayoutWidth();
  if (!layoutWidth || supportLayoutCompact.value || !hasTwoSupportSections.value) {
    return Math.max(MIN_SUPPORT_SECTION_WIDTH, next);
  }
  const minWidth = Math.min(MIN_SUPPORT_SECTION_WIDTH, Math.max(140, Math.floor(layoutWidth * 0.25)));
  const maxWidth = Math.max(minWidth, layoutWidth - minWidth - 8);
  return Math.min(maxWidth, Math.max(minWidth, next));
}

function syncSupportLayoutMetrics() {
  const layoutWidth = currentSupportLayoutWidth();
  supportLayoutCompact.value = !!layoutWidth && layoutWidth <= SUPPORT_LAYOUT_COMPACT_WIDTH;
  supportStripHeight.value = clampSupportStripHeight(supportStripHeight.value);
  supportPrimaryWidth.value = clampSupportPrimaryWidth(supportPrimaryWidth.value);
}

function observeSupportLayout() {
  layoutResizeObserver?.disconnect();
  layoutResizeObserver = null;
  if (typeof ResizeObserver === "undefined") return;
  layoutResizeObserver = createAnimationFrameResizeObserver(() => {
    syncSupportLayoutMetrics();
  });
  if (!layoutResizeObserver) return;
  if (previewMainRef.value) layoutResizeObserver.observe(previewMainRef.value);
  if (supportLayoutRef.value) layoutResizeObserver.observe(supportLayoutRef.value);
}

function refreshSupportLayoutMetrics() {
  void nextTick(() => {
    observeSupportLayout();
    syncSupportLayoutMetrics();
  });
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function searchSectionLabel(section: KnowledgeSearchMatchSection): string {
  if (section === "summary") return t("knowledge.preview.summary");
  if (section === "maintenanceRules") return t("knowledge.preview.rules");
  return t("knowledge.preview.body");
}

function formatDateTime(value: number | null | undefined): string {
  if (!value) return "—";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "—";
  return date.toLocaleString(undefined, {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function formatByteSize(value: number | null | undefined): string {
  if (!value) return "0 B";
  if (value < 1024) return `${countFormatter.format(value)} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  if (value < 1024 * 1024 * 1024) return `${(value / (1024 * 1024)).toFixed(1)} MB`;
  return `${(value / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function formatCount(value: number | null | undefined): string {
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) return "—";
  return countFormatter.format(Math.round(value));
}

function formatDocumentLength(
  lineCount: number | null | undefined,
  charCount: number | null | undefined,
): string {
  const normalizedLineCount = typeof lineCount === "number" && Number.isFinite(lineCount)
    ? countFormatter.format(Math.round(lineCount))
    : "—";
  const normalizedCharCount = typeof charCount === "number" && Number.isFinite(charCount)
    ? countFormatter.format(Math.round(charCount))
    : "—";
  return t("knowledge.meta.lengthValue", normalizedLineCount, normalizedCharCount);
}

function searchSnippetTitle() {
  const kind = activeSearchContext.value?.result.matchKind ?? "semantic";
  if (kind === "lexical") return t("knowledge.preview.searchHitLexical");
  if (kind === "semantic") return t("knowledge.preview.searchHitSemantic");
  if (kind === "grep") return t("knowledge.preview.searchHitGrep");
  return t("knowledge.preview.searchHitHybrid");
}

function isSearchMatchSection(section: KnowledgeSearchMatchSection): boolean {
  return !!activeSearchContext.value && searchMatchSection.value === section;
}

function searchSnippetVisible(section: KnowledgeSearchMatchSection): boolean {
  return isSearchMatchSection(section) && !!activeSearchContext.value?.result.snippet.trim();
}

function searchSnippetSegments(section: KnowledgeSearchMatchSection) {
  if (!searchSnippetVisible(section)) return [];
  const text = activeSearchContext.value?.result.snippet ?? "";
  const re = searchHighlightRe.value;
  if (!re || !text) return [{ text, hit: false }];
  const result: Array<{ text: string; hit: boolean }> = [];
  let lastIndex = 0;
  re.lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = re.exec(text)) !== null) {
    if (match.index > lastIndex) {
      result.push({ text: text.slice(lastIndex, match.index), hit: false });
    }
    result.push({ text: match[0], hit: true });
    lastIndex = match.index + match[0].length;
    if (match[0].length === 0) re.lastIndex += 1;
  }
  if (lastIndex < text.length) {
    result.push({ text: text.slice(lastIndex), hit: false });
  }
  return result;
}

function normalizeSearchText(value: string): string {
  return value.replace(/\s+/g, " ").trim().toLowerCase();
}

function renderedSearchContainer(section: KnowledgeSearchMatchSection): HTMLElement | null {
  if (section === "summary") return summaryRenderedSearchRef.value;
  if (section === "maintenanceRules") return rulesRenderedSearchRef.value;
  return bodyRenderedSearchRef.value;
}

function clearTargetSearchMark() {
  for (const container of [
    summaryRenderedSearchRef.value,
    rulesRenderedSearchRef.value,
    bodyRenderedSearchRef.value,
  ]) {
    if (!container) continue;
    for (const node of container.querySelectorAll<HTMLElement>("mark.markdown-search-mark-target")) {
      node.classList.remove("markdown-search-mark-target");
    }
  }
}

function pickTargetSearchMark(
  container: HTMLElement,
  snippet: string,
): HTMLElement | null {
  const marks = [...container.querySelectorAll<HTMLElement>("mark.markdown-search-mark")];
  if (!marks.length) return null;
  const normalizedSnippet = normalizeSearchText(snippet);
  if (!normalizedSnippet) return marks[0] ?? null;

  let bestMark = marks[0] ?? null;
  let bestScore = -1;

  for (const mark of marks) {
    const contextText = normalizeSearchText(mark.parentElement?.textContent ?? "");
    const markText = normalizeSearchText(mark.textContent ?? "");
    let score = 0;
    if (markText && normalizedSnippet.includes(markText)) {
      score += markText.length * 2;
    }
    if (contextText && normalizedSnippet && contextText.includes(normalizedSnippet)) {
      score += normalizedSnippet.length;
    }
    if (score > bestScore) {
      bestScore = score;
      bestMark = mark;
    }
  }

  return bestMark;
}

function scrollSearchMatchIntoView(): boolean {
  const searchContext = activeSearchContext.value;
  if (!searchContext || !showSearchRenderedContent.value) return false;
  const container = renderedSearchContainer(searchMatchSection.value);
  if (!container) return false;

  clearTargetSearchMark();
  const target = pickTargetSearchMark(container, searchContext.result.snippet);
  if (!target) {
    container.scrollTop = 0;
    return false;
  }

  target.classList.add("markdown-search-mark-target");
  const containerRect = container.getBoundingClientRect();
  const targetRect = target.getBoundingClientRect();
  const offsetTop = container.scrollTop + (targetRect.top - containerRect.top);
  const padding = Math.max(18, Math.round(container.clientHeight * 0.18));
  const nextTop = Math.max(0, offsetTop - padding);
  container.scrollTo({
    top: nextTop,
    behavior: "smooth",
  });
  return true;
}

function cancelSearchMatchScroll() {
  if (!searchMatchScrollFrame || typeof window === "undefined" || typeof window.cancelAnimationFrame !== "function") {
    searchMatchScrollFrame = 0;
    return;
  }
  window.cancelAnimationFrame(searchMatchScrollFrame);
  searchMatchScrollFrame = 0;
}

function scheduleSearchMatchScroll() {
  cancelSearchMatchScroll();
  void nextTick(() => {
    if (scrollSearchMatchIntoView()) return;
    if (typeof window === "undefined" || typeof window.requestAnimationFrame !== "function") return;
    searchMatchScrollFrame = window.requestAnimationFrame(() => {
      searchMatchScrollFrame = 0;
      void nextTick(() => {
        scrollSearchMatchIntoView();
      });
    });
  });
}

watch(
  () => props.document,
  (document, previousDocument) => {
    const resetDrafts = (document?.id ?? "") !== (previousDocument?.id ?? "")
      || (document?.type ?? "") !== (previousDocument?.type ?? "");
    if ((document?.id ?? "") !== (previousDocument?.id ?? "")) {
      sidePanelTab.value = "chat";
    }
    if (resetDrafts) {
      if (supportPanelsCollapsedPreference.value == null) {
        resetSupportPanels(document);
      } else {
        supportPanelsCollapsed.value = supportPanelsCollapsedPreference.value;
      }
    }
    syncDrafts(resetDrafts);
    refreshSupportLayoutMetrics();
    confirmDelete.value = false;
  },
  { immediate: true },
);

watch(
  () => [props.document?.id ?? "", props.document?.path ?? ""],
  (current, previous = ["", ""]) => {
    const [documentId, documentPathValue] = current;
    const [previousId] = previous;
    const currentStem = extractDocumentFileStem(documentPathValue);
    if (documentId !== previousId) {
      fileNameDraft.value = currentStem;
      fileNameDirty.value = false;
      return;
    }

    const normalizedDocumentName = normalizeDocumentFileStemValue(currentStem);
    const normalizedDraftName = normalizeDocumentFileStemValue(fileNameDraft.value);
    if (fileNameDirty.value) {
      if (normalizedDraftName === normalizedDocumentName) {
        fileNameDraft.value = currentStem;
        fileNameDirty.value = false;
      }
      return;
    }
    fileNameDraft.value = currentStem;
  },
  { immediate: true },
);

watch(
  () => [
    props.document?.id ?? "",
    hasSupportPanels.value,
    hasTwoSupportSections.value,
    supportPanelsCollapsed.value,
  ],
  () => {
    refreshSupportLayoutMetrics();
  },
);

watch(
  () => [props.document?.id ?? "", searchMatchSection.value, !!activeSearchContext.value] as const,
  ([documentId, section, hasSearchContext]) => {
    if (!documentId || !hasSearchContext) return;
    if (section === "summary" || section === "maintenanceRules") {
      supportPanelsCollapsed.value = false;
      refreshSupportLayoutMetrics();
    }
  },
);

watch(
  () => [
    props.document?.id ?? "",
    props.loading,
    showSearchRenderedContent.value,
    searchMatchSection.value,
    activeSearchContext.value?.query ?? "",
    activeSearchContext.value?.result.snippet ?? "",
    supportPanelsCollapsed.value,
  ] as const,
  ([documentId, loading, showRenderedContent, section, query]) => {
    if (!documentId || loading || !showRenderedContent || !query) return;
    if ((section === "summary" || section === "maintenanceRules") && supportPanelsCollapsed.value) return;
    scheduleSearchMatchScroll();
  },
  { flush: "post" },
);

watch(() => props.saveLoading, (loading, wasLoading) => {
  if (!loading && wasLoading) {
    autoSaveInFlight.value = false;
    maybeScheduleAutoSave();
  }
});

watch(
  () => [props.document?.id ?? "", props.document?.updatedAt ?? 0, props.document?.type ?? ""],
  ([documentId, , documentType]) => {
    if (!documentId || documentType !== "skill") {
      notificationStore.clearByOperation(SKILL_COMMAND_NOTICE_OPERATION);
      return;
    }
    void loadSkills();
  },
  { immediate: true },
);

watch(currentSkillCommandTrigger, (value) => {
  skillCommandDraft.value = value;
}, { immediate: true });

watch(
  () => props.document?.argumentHint ?? "",
  (value) => {
    skillArgumentHintDraft.value = value ?? "";
  },
  { immediate: true },
);

watch(skillPackageId, () => {
  void refreshSkillUnityStatus();
}, { immediate: true });

onMounted(() => {
  observeSupportLayout();
  syncSupportLayoutMetrics();
  window.addEventListener("resize", syncSupportLayoutMetrics);
});

onUnmounted(() => {
  clearAutoSaveTimer();
  notificationStore.clearByOperation(SKILL_COMMAND_NOTICE_OPERATION);
  document.removeEventListener("mousemove", onSideResizeMove);
  document.removeEventListener("mouseup", onSideResizeEnd);
  document.removeEventListener("mousemove", onSupportHeightResizeMove);
  document.removeEventListener("mouseup", onSupportHeightResizeEnd);
  document.removeEventListener("mousemove", onSupportWidthResizeMove);
  document.removeEventListener("mouseup", onSupportWidthResizeEnd);
  window.removeEventListener("resize", syncSupportLayoutMetrics);
  layoutResizeObserver?.disconnect();
  layoutResizeObserver = null;
  cancelSearchMatchScroll();
  unlockResizeInteraction();
});

function currentDraftValues() {
  return {
    summary: summaryDraft.value,
    maintenanceRules: rulesDraft.value,
    body: bodyDraft.value,
  };
}

function applyDraftValues(nextDrafts: ReturnType<typeof createKnowledgeEditorDraftValues>) {
  summaryDraft.value = nextDrafts.summary;
  rulesDraft.value = nextDrafts.maintenanceRules;
  bodyDraft.value = nextDrafts.body;
}

function syncDrafts(force = false) {
  const { drafts, dirtySections: nextDirtySections } = mergeKnowledgeEditorDraftValues(
    props.document,
    currentDraftValues(),
    dirtySections.value,
    force,
  );
  applyDraftValues(drafts);
  dirtySections.value = nextDirtySections;
  if (force) {
    autoSaveInFlight.value = false;
    clearAutoSaveTimer();
    return;
  }
  if (!nextDirtySections.size) {
    clearAutoSaveTimer();
    if (!props.saveLoading) autoSaveInFlight.value = false;
  }
}

function clearAutoSaveTimer() {
  if (autoSaveTimer !== null) {
    clearTimeout(autoSaveTimer);
    autoSaveTimer = null;
  }
  autoSaveQueued.value = false;
}

function markDirty(section: KnowledgeDocumentSection) {
  const next = new Set(dirtySections.value);
  next.add(section);
  dirtySections.value = next;
  maybeScheduleAutoSave();
}

function maybeScheduleAutoSave() {
  clearAutoSaveTimer();
  if (!props.document || props.loading || props.saveLoading || isReadOnly.value || !dirtySections.value.size) {
    return;
  }
  if (!hasUnsavedSectionChanges.value) return;
  autoSaveQueued.value = true;
  autoSaveTimer = setTimeout(() => {
    autoSaveTimer = null;
    flushPendingChanges("auto");
  }, AUTO_SAVE_DELAY_MS);
}

function flushPendingChanges(mode: "auto" | "manual") {
  if (!props.document || props.saveLoading || isReadOnly.value) return;
  const shouldRenameDocument = mode === "manual" && fileNameDirty.value;
  if (!hasUnsavedSectionChanges.value && !shouldRenameDocument) return;

  clearAutoSaveTimer();
  autoSaveInFlight.value = mode === "auto";
  emitPendingSectionChanges();
  if (shouldRenameDocument) {
    persistDocumentNameChange();
  }
}

function sectionValue(section: KnowledgeDocumentSection): string {
  if (section === "summary") return summaryDraft.value;
  if (section === "maintenanceRules") return rulesDraft.value;
  return bodyDraft.value;
}

function onSectionInput(section: KnowledgeDocumentSection, value: string) {
  if (section === "summary") summaryDraft.value = value;
  else if (section === "maintenanceRules") rulesDraft.value = value;
  else bodyDraft.value = value;
  markDirty(section);
}

function emitPendingSectionChanges() {
  const sections = [...dirtySections.value];
  dirtySections.value = new Set();
  for (const section of sections) {
    emit("saveSection", section, sectionValue(section));
  }
}

function extractDocumentFileName(path?: string | null): string {
  const normalized = (path ?? "").trim().replace(/\\/g, "/");
  if (!normalized) return "";
  const segments = normalized.split("/").filter(Boolean);
  return segments[segments.length - 1] ?? "";
}

function extractDocumentFileStem(path?: string | null): string {
  const fileName = extractDocumentFileName(path);
  return fileName.replace(/\.[^.]+$/u, "");
}

function normalizeDocumentFileStemValue(value: string): string {
  return value.trim().replace(/\.md$/i, "");
}

function hasInvalidDocumentFileStem(value: string): boolean {
  return value.includes("/") || value.includes("\\") || value.includes("..");
}

function buildPendingDocumentNamePatch(): KnowledgeDocumentPatch | null {
  if (!props.document || isReadOnly.value || props.saveLoading || !fileNameDirty.value) return null;
  const nextStem = normalizeDocumentFileStemValue(fileNameDraft.value);
  const currentStem = normalizeDocumentFileStemValue(currentDocumentFileStem.value);
  if (!nextStem) {
    notificationStore.addNotice("error", t("knowledge.preview.titleRequired"), {
      operation: "knowledgeDocumentFileName",
      replaceOperation: true,
    });
    fileNameDraft.value = currentDocumentFileStem.value;
    fileNameDirty.value = false;
    return null;
  }
  if (hasInvalidDocumentFileStem(nextStem)) {
    notificationStore.addNotice("error", t("knowledge.preview.titleInvalid"), {
      operation: "knowledgeDocumentFileName",
      replaceOperation: true,
    });
    fileNameDraft.value = currentDocumentFileStem.value;
    fileNameDirty.value = false;
    return null;
  }
  notificationStore.clearByOperation("knowledgeDocumentFileName");
  if (nextStem === currentStem) {
    fileNameDraft.value = currentDocumentFileStem.value;
    fileNameDirty.value = false;
    return null;
  }
  const normalizedPath = documentPath.value.replace(/\\/g, "/").replace(/^\/+/, "");
  const segments = normalizedPath.split("/").filter(Boolean);
  const currentFileName = segments.pop() ?? "";
  const extensionMatch = currentFileName.match(/(\.[^.]+)$/);
  const nextFileName = `${nextStem}${extensionMatch?.[1] ?? ""}`;
  return {
    newPath: segments.length ? `${segments.join("/")}/${nextFileName}` : nextFileName,
  };
}

function persistDocumentNameChange() {
  const patch = buildPendingDocumentNamePatch();
  if (!patch) return;
  emit("updateMeta", patch);
}

function onFileNameInput(value: string) {
  fileNameDraft.value = value;
  fileNameDirty.value = normalizeDocumentFileStemValue(value) !== normalizeDocumentFileStemValue(currentDocumentFileStem.value);
}

function onFileNameInputEvent(event: Event) {
  onFileNameInput((event.target as HTMLInputElement | null)?.value ?? "");
}

function onFileNameKeydown(event: KeyboardEvent) {
  if (event.key === "Enter") {
    event.preventDefault();
    flushPendingChanges("manual");
    return;
  }

  if (event.key === "Escape") {
    fileNameDraft.value = currentDocumentFileStem.value;
    fileNameDirty.value = false;
    (event.target as HTMLInputElement | null)?.blur();
  }
}

function updateMeta(patch: KnowledgeDocumentPatch) {
  if (!props.document || isReadOnly.value) return;
  if (dirtySections.value.size) {
    clearAutoSaveTimer();
    emitPendingSectionChanges();
  }
  const renamePatch = buildPendingDocumentNamePatch();
  emit("updateMeta", renamePatch ? { ...patch, ...renamePatch } : patch);
}

function onSummaryEnabledChange(value: boolean) {
  updateMeta({ summaryEnabled: value });
}

function onInjectModeChange(value: string) {
  if (value === "inherit_parent") {
    updateMeta({ inheritInjectMode: true });
    return;
  }
  updateMeta({
    inheritInjectMode: false,
    injectMode: value as KnowledgeInjectMode,
  });
}

function onEditModeChange(value: string) {
  if (!props.document || isEditModeLocked.value) return;

  const nextMode = value as KnowledgeEditMode;
  const nextPatch: KnowledgeDocumentPatch = {
    ...buildKnowledgeEditModePatch(nextMode),
  };
  if (nextMode === "inherit_parent") {
    updateMeta(nextPatch);
    return;
  }
  const needsDefaultRules = nextMode === "auto" && !rulesDraft.value.trim();
  if (needsDefaultRules) {
    const defaultRules = defaultMaintenanceRulesForType(props.document.type);
    if (defaultRules) {
      rulesDraft.value = defaultRules;
      nextPatch.maintenanceRules = defaultRules;
    }
  }

  updateMeta(nextPatch);
}

function onExplicitRulesChange(value: boolean) {
  if (!props.document || isReadOnly.value) return;
  if (props.document.inheritAiConfig || (!value && explicitRulesLocked.value)) return;

  if (!value) {
    updateMeta({
      explicitMaintenanceRules: false,
    });
    return;
  }

  const nextPatch: KnowledgeDocumentPatch = {
    explicitMaintenanceRules: true,
  };
  if (!rulesDraft.value.trim()) {
    const defaultRules = defaultMaintenanceRulesForType(props.document.type)
      ?? (defaultExplicitMaintenanceRulesForType(props.document.type) ? "" : null);
    if (defaultRules !== null) {
      rulesDraft.value = defaultRules;
      nextPatch.maintenanceRules = defaultRules;
    }
  }
  updateMeta(nextPatch);
}

function onSideResizeStart(event: MouseEvent) {
  if (metaCollapsed.value) return;
  event.preventDefault();
  sideResizing = true;
  isSideResizing.value = true;
  sideResizeStartX = event.clientX;
  sideResizeStartWidth = sidePanelWidth.value;
  document.addEventListener("mousemove", onSideResizeMove);
  document.addEventListener("mouseup", onSideResizeEnd);
  lockResizeInteraction("col-resize");
}

function onSideResizeMove(event: MouseEvent) {
  if (!sideResizing) return;
  const delta = sideResizeStartX - event.clientX;
  sidePanelWidth.value = Math.min(
    MAX_SIDE_PANEL_WIDTH,
    Math.max(MIN_SIDE_PANEL_WIDTH, sideResizeStartWidth + delta),
  );
}

function onSideResizeEnd() {
  sideResizing = false;
  isSideResizing.value = false;
  document.removeEventListener("mousemove", onSideResizeMove);
  document.removeEventListener("mouseup", onSideResizeEnd);
  unlockResizeInteraction();
}

function onSupportHeightResizeStart(event: MouseEvent) {
  if (!hasSupportPanels.value || supportPanelsCollapsed.value) return;
  event.preventDefault();
  supportHeightResizing = true;
  isSupportHeightResizing.value = true;
  supportHeightResizeStartY = event.clientY;
  supportHeightResizeStartValue = supportStripHeight.value;
  document.addEventListener("mousemove", onSupportHeightResizeMove);
  document.addEventListener("mouseup", onSupportHeightResizeEnd);
  lockResizeInteraction("row-resize");
}

function onSupportHeightResizeMove(event: MouseEvent) {
  if (!supportHeightResizing) return;
  const delta = event.clientY - supportHeightResizeStartY;
  supportStripHeight.value = clampSupportStripHeight(supportHeightResizeStartValue + delta);
}

function onSupportHeightResizeEnd() {
  supportHeightResizing = false;
  isSupportHeightResizing.value = false;
  document.removeEventListener("mousemove", onSupportHeightResizeMove);
  document.removeEventListener("mouseup", onSupportHeightResizeEnd);
  persistStoredPanelSize(SUPPORT_STRIP_HEIGHT_STORAGE_KEY, supportStripHeight.value);
  unlockResizeInteraction();
}

function onSupportWidthResizeStart(event: MouseEvent) {
  if (!hasTwoSupportSections.value || supportLayoutCompact.value) return;
  event.preventDefault();
  supportWidthResizing = true;
  isSupportWidthResizing.value = true;
  supportWidthResizeStartX = event.clientX;
  supportWidthResizeStartValue = supportPrimaryWidth.value;
  document.addEventListener("mousemove", onSupportWidthResizeMove);
  document.addEventListener("mouseup", onSupportWidthResizeEnd);
  lockResizeInteraction("col-resize");
}

function onSupportWidthResizeMove(event: MouseEvent) {
  if (!supportWidthResizing) return;
  const delta = event.clientX - supportWidthResizeStartX;
  supportPrimaryWidth.value = clampSupportPrimaryWidth(supportWidthResizeStartValue + delta);
}

function onSupportWidthResizeEnd() {
  supportWidthResizing = false;
  isSupportWidthResizing.value = false;
  document.removeEventListener("mousemove", onSupportWidthResizeMove);
  document.removeEventListener("mouseup", onSupportWidthResizeEnd);
  persistStoredPanelSize(SUPPORT_SECTION_WIDTH_STORAGE_KEY, supportPrimaryWidth.value);
  unlockResizeInteraction();
}

function inferSkillName(document: KnowledgeDocument | null): string {
  const path = document?.path?.trim().replace(/\\/g, "/") ?? "";
  if (!path) return "";
  const segments = path.split("/").filter(Boolean);
  const fileName = segments[segments.length - 1] ?? "";
  if (fileName.toLowerCase() === "skill.md" && segments.length > 1) {
    return segments[segments.length - 2] ?? "";
  }
  return fileName.replace(/\.md$/i, "");
}

function showSkillCommandError(message: string) {
  notificationStore.addNotice("error", message, {
    operation: SKILL_COMMAND_NOTICE_OPERATION,
    replaceOperation: true,
    sticky: true,
  });
}

function onSkillSurfaceChange(value: string) {
  if (!props.document || props.document.type !== "skill" || isReadOnly.value) return;
  notificationStore.clearByOperation(SKILL_COMMAND_NOTICE_OPERATION);
  if (value === "disabled") {
    updateMeta({ skillEnabled: false });
    return;
  }

  const nextSurface = value as SkillSurface;
  updateMeta({
    skillEnabled: true,
    skillSurface: nextSurface,
    commandTrigger: skillSurfaceAllowsCommand(nextSurface)
      ? currentSkillCommandTrigger.value
      : props.document.commandTrigger ?? null,
  });
}

function persistSkillCommandTrigger() {
  if (!props.document || props.document.type !== "skill" || skillCommandInputDisabled.value) return;
  const normalizedTrigger = normalizeSkillCommandTrigger(skillCommandDraft.value, fallbackSkillName.value);
  if (!isValidSkillCommandTrigger(normalizedTrigger)) {
    showSkillCommandError(t("knowledge.skill.commandTriggerInvalid"));
    return;
  }

  const conflict = findSkillCommandConflict(normalizedTrigger, skillItems.value, {
    source: "project",
    dirName: fallbackSkillName.value,
  });
  if (conflict) {
    showSkillCommandError(
      conflict.type === "builtin"
        ? t("knowledge.skill.commandTriggerBuiltinConflict", conflict.command)
        : t("knowledge.skill.commandTriggerSkillConflict", conflict.command, conflict.skillName ?? ""),
    );
    return;
  }

  if (normalizedTrigger === currentSkillCommandTrigger.value) {
    notificationStore.clearByOperation(SKILL_COMMAND_NOTICE_OPERATION);
    skillCommandDraft.value = currentSkillCommandTrigger.value;
    return;
  }

  notificationStore.clearByOperation(SKILL_COMMAND_NOTICE_OPERATION);
  updateMeta({ commandTrigger: normalizedTrigger });
}

function onSkillCommandBlur() {
  persistSkillCommandTrigger();
}

function onSkillCommandKeydown(event: KeyboardEvent) {
  if (event.key === "Enter") {
    event.preventDefault();
    persistSkillCommandTrigger();
    return;
  }

  if (event.key === "Escape") {
    skillCommandDraft.value = currentSkillCommandTrigger.value;
    notificationStore.clearByOperation(SKILL_COMMAND_NOTICE_OPERATION);
    (event.target as HTMLInputElement | null)?.blur();
  }
}

function normalizeNullableInput(value: string): string | null {
  const normalized = value.trim();
  return normalized ? normalized : null;
}

function buildCollapsedPreview(value: string, fallback: string): string {
  const normalized = normalizeKnowledgeEditorValue(value).replace(/\s+/g, " ").trim();
  if (!normalized) return fallback;
  return normalized.length > 84 ? `${normalized.slice(0, 84).trimEnd()}…` : normalized;
}

function resetSupportPanels(document: KnowledgeDocument | null) {
  if (!document) {
    supportPanelsCollapsed.value = true;
    return;
  }
  const summaryVisible = getKnowledgeDocumentEditorSections(document).summary;
  const rulesVisible = getKnowledgeDocumentEditorSections(document).maintenanceRules;
  const summaryReady = !summaryVisible || !!normalizeKnowledgeEditorValue(document.summary ?? "");
  const rulesReady = !rulesVisible || !!normalizeKnowledgeEditorValue(document.maintenanceRules ?? "");
  supportPanelsCollapsed.value = summaryReady && rulesReady;
}

function persistSkillArgumentHint() {
  if (!props.document || props.document.type !== "skill" || isReadOnly.value || props.saveLoading) return;
  const nextValue = normalizeNullableInput(skillArgumentHintDraft.value);
  const currentValue = normalizeNullableInput(props.document.argumentHint ?? "");
  if (nextValue === currentValue) {
    skillArgumentHintDraft.value = props.document.argumentHint ?? "";
    return;
  }
  updateMeta({ argumentHint: nextValue });
}

function onSkillArgumentHintKeydown(event: KeyboardEvent) {
  if (event.key === "Enter") {
    event.preventDefault();
    persistSkillArgumentHint();
    return;
  }

  if (event.key === "Escape") {
    skillArgumentHintDraft.value = props.document?.argumentHint ?? "";
    (event.target as HTMLInputElement | null)?.blur();
  }
}

async function refreshSkillUnityStatus() {
  const packageId = skillPackageId.value;
  if (!packageId) {
    skillUnityStatus.value = null;
    return;
  }
  skillUnityStatusLoading.value = true;
  try {
    skillUnityStatus.value = await getSkillUnityInstallStatus(packageId);
  } catch {
    skillUnityStatus.value = null;
  } finally {
    skillUnityStatusLoading.value = false;
  }
}

async function installSkillUnity() {
  const packageId = skillPackageId.value;
  if (!packageId || skillUnityActionPending.value) return;
  skillUnityActionPending.value = true;
  try {
    skillUnityStatus.value = await installSkillUnityFiles(packageId);
    notificationStore.addNotice("success", t("knowledge.skill.unityInstallDone"), {
      operation: "skill_unity_install",
      replaceOperation: true,
    });
  } catch (cause) {
    notificationStore.addNotice("error", String(cause), {
      operation: "skill_unity_install",
      replaceOperation: true,
    });
  } finally {
    skillUnityActionPending.value = false;
  }
}

async function removeSkillUnity() {
  const packageId = skillPackageId.value;
  if (!packageId || skillUnityActionPending.value) return;
  skillUnityActionPending.value = true;
  try {
    skillUnityStatus.value = await removeSkillUnityFiles(packageId);
    notificationStore.addNotice("success", t("knowledge.skill.unityRemoveDone"), {
      operation: "skill_unity_remove",
      replaceOperation: true,
    });
  } catch (cause) {
    notificationStore.addNotice("error", String(cause), {
      operation: "skill_unity_remove",
      replaceOperation: true,
    });
  } finally {
    skillUnityActionPending.value = false;
  }
}

function labelForType(type?: KnowledgeDocumentType | null): string {
  switch (type) {
    case "design":
      return t("knowledge.type.design");
    case "memory":
      return t("knowledge.type.memory");
    case "skill":
      return t("knowledge.type.skill");
    case "reference":
      return t("knowledge.type.reference");
    default:
      return "—";
  }
}

function labelForStoredScope(document?: KnowledgeDocument | null): string {
  if (!document) return "—";
  return document.storageSource === "app"
    ? t("knowledge.scope.user")
    : t("knowledge.scope.project");
}

function labelForProvider(provider?: string | null): string {
  switch (provider) {
    case "local_folder":
      return t("knowledge.source.localFolder");
    case "feishu":
      return t("knowledge.source.feishu");
    case "url":
      return t("knowledge.source.url");
    case "package":
      return t("knowledge.source.package");
    case "unity":
      return t("knowledge.source.unity");
    default:
      return t("knowledge.source.custom");
  }
}

</script>

<template>
  <div class="preview-panel" :class="{ 'is-resizing': isPreviewResizing }">
    <div class="preview-shell">
      <div class="preview-main-column">
        <div class="preview-header">
          <div class="preview-header-main">
            <span
              v-if="document && !isReadOnly"
              class="preview-title-input-shell"
              :data-value="titleMeasureText"
            >
              <input
                :value="fileNameDraft"
                class="preview-title-input"
                type="text"
                :disabled="saveLoading"
                :placeholder="t('knowledge.preview.titlePlaceholder')"
                :aria-label="t('knowledge.preview.titleLabel')"
                @input="onFileNameInputEvent"
                @blur="flushPendingChanges('manual')"
                @keydown="onFileNameKeydown"
              />
            </span>
            <span v-else class="preview-title">{{ documentTitle }}</span>
            <span v-if="documentDisplayPath" class="preview-path">{{ documentDisplayPath }}</span>
          </div>
          <div class="preview-header-actions">
            <BaseSegmented
              v-if="document"
              v-model="editorViewMode"
              class="preview-view-segmented"
              size="sm"
              :options="editorViewOptions"
              :aria-label="t('knowledge.editor.viewMode')"
            />
            <span v-if="isReadOnly" class="preview-status-tag">{{ t("knowledge.meta.readOnly") }}</span>
          </div>
        </div>

        <div ref="previewMainRef" class="preview-main">
          <div v-if="loading && !document" class="preview-empty">{{ t("common.loading") }}</div>
          <div v-else-if="!document" class="preview-empty">{{ t("knowledge.empty.title") }}</div>
          <template v-else>
            <section
              v-if="hasSupportPanels"
              class="preview-support-strip"
              :class="{
                'is-warning': document.aiMaintained && (!document.explicitMaintenanceRules || !rulesDraft.trim()),
                'has-resize-divider': !supportPanelsCollapsed,
              }"
              :style="supportStripStyle"
            >
              <button
                type="button"
                class="preview-support-toggle"
                :aria-expanded="!supportPanelsCollapsed"
                @click="toggleSupportPanels"
              >
                <span class="preview-support-chevron" :class="{ open: !supportPanelsCollapsed }">▶</span>
              </button>

              <div
                ref="supportLayoutRef"
                class="preview-support-layout"
                :class="{
                  'has-two-sections': visibleSections.summary && visibleSections.maintenanceRules,
                  'is-compact': supportLayoutCompact,
                }"
                :style="supportLayoutStyle"
              >
                <div
                  v-if="visibleSections.summary"
                  class="preview-support-section preview-support-section-first"
                  :class="{ 'is-search-match': isSearchMatchSection('summary') }"
                >
                  <div class="preview-support-section-header">
                    <span class="preview-support-title">{{ t("knowledge.preview.summary") }}</span>
                    <span class="preview-support-text">
                      {{ supportPanelsCollapsed ? summaryPreviewText : t("knowledge.preview.summaryHint") }}
                    </span>
                  </div>
                  <div v-if="!supportPanelsCollapsed" class="preview-support-section-body" :class="{ 'is-loading': loading }">
                    <div v-if="searchSnippetVisible('summary')" class="preview-search-hit">
                      <div class="preview-search-hit-header">
                        <span class="preview-search-hit-label">{{ searchSnippetTitle() }}</span>
                        <span class="preview-search-hit-section">
                          {{ t("knowledge.preview.searchMatchedField") }} · {{ searchSectionLabel("summary") }}
                        </span>
                      </div>
                      <div class="preview-search-hit-text">
                        <template v-for="(segment, index) in searchSnippetSegments('summary')" :key="`summary-${index}`">
                          <mark v-if="segment.hit" class="preview-search-hit-mark">{{ segment.text }}</mark>
                          <template v-else>{{ segment.text }}</template>
                        </template>
                      </div>
                    </div>
                    <div
                      v-if="showSearchRenderedContent"
                      ref="summaryRenderedSearchRef"
                      class="preview-rendered-search"
                    >
                      <MarkdownRenderer
                        :content="summaryDraft"
                        :highlight-terms="searchQueryTerms"
                      />
                    </div>
                    <BaseMarkdownEditor
                      v-else
                      :model-value="summaryDraft"
                      :disabled="isReadOnly"
                      :view-mode="editorViewMode"
                      :placeholder="t('knowledge.preview.summaryPlaceholder')"
                      @update:model-value="onSectionInput('summary', $event)"
                      @shortcut-save="flushPendingChanges('manual')"
                    />
                  </div>
                </div>

                <div
                  v-if="visibleSections.summary && visibleSections.maintenanceRules"
                  class="preview-support-divider"
                  :class="{
                    'is-resizable': !supportLayoutCompact,
                    dragging: isSupportWidthResizing,
                  }"
                  aria-hidden="true"
                  @mousedown="onSupportWidthResizeStart"
                ></div>

                <div
                  v-if="visibleSections.maintenanceRules"
                  class="preview-support-section"
                  :class="{
                    'preview-support-section-first': !visibleSections.summary,
                    'is-warning': document.aiMaintained && (!document.explicitMaintenanceRules || !rulesDraft.trim()),
                    'is-search-match': isSearchMatchSection('maintenanceRules'),
                  }"
                >
                  <div class="preview-support-section-header">
                    <span class="preview-support-title">{{ t("knowledge.preview.rules") }}</span>
                    <span class="preview-support-text">
                      {{ supportPanelsCollapsed ? rulesPreviewText : rulesHint }}
                    </span>
                  </div>
                  <div v-if="!supportPanelsCollapsed" class="preview-support-section-body" :class="{ 'is-loading': loading }">
                    <div v-if="searchSnippetVisible('maintenanceRules')" class="preview-search-hit">
                      <div class="preview-search-hit-header">
                        <span class="preview-search-hit-label">{{ searchSnippetTitle() }}</span>
                        <span class="preview-search-hit-section">
                          {{ t("knowledge.preview.searchMatchedField") }} · {{ searchSectionLabel("maintenanceRules") }}
                        </span>
                      </div>
                      <div class="preview-search-hit-text">
                        <template
                          v-for="(segment, index) in searchSnippetSegments('maintenanceRules')"
                          :key="`rules-${index}`"
                        >
                          <mark v-if="segment.hit" class="preview-search-hit-mark">{{ segment.text }}</mark>
                          <template v-else>{{ segment.text }}</template>
                        </template>
                      </div>
                    </div>
                    <div
                      v-if="showSearchRenderedContent"
                      ref="rulesRenderedSearchRef"
                      class="preview-rendered-search"
                    >
                      <MarkdownRenderer
                        :content="rulesDraft"
                        :highlight-terms="searchQueryTerms"
                      />
                    </div>
                    <BaseMarkdownEditor
                      v-else
                      :model-value="rulesDraft"
                      :disabled="rulesEditorDisabled"
                      :view-mode="editorViewMode"
                      :placeholder="t('knowledge.preview.rulesPlaceholder')"
                      @update:model-value="onSectionInput('maintenanceRules', $event)"
                      @shortcut-save="flushPendingChanges('manual')"
                    />
                  </div>
                </div>
              </div>
            </section>

            <div
              v-if="hasSupportPanels && !supportPanelsCollapsed"
              class="preview-main-divider"
              :class="{ dragging: isSupportHeightResizing }"
              aria-hidden="true"
              @mousedown="onSupportHeightResizeStart"
            ></div>

            <section class="preview-pane preview-pane-body" :class="{ 'is-search-match': isSearchMatchSection('body') }">
              <div class="preview-pane-header">
                <span>{{ t("knowledge.preview.body") }}</span>
              </div>
              <div class="preview-body" :class="{ 'is-loading': loading }">
                <div v-if="searchSnippetVisible('body')" class="preview-search-hit preview-search-hit-body">
                  <div class="preview-search-hit-header">
                    <span class="preview-search-hit-label">{{ searchSnippetTitle() }}</span>
                    <span class="preview-search-hit-section">
                      {{ t("knowledge.preview.searchMatchedField") }} · {{ searchSectionLabel("body") }}
                    </span>
                  </div>
                  <div class="preview-search-hit-text">
                    <template v-for="(segment, index) in searchSnippetSegments('body')" :key="`body-${index}`">
                      <mark v-if="segment.hit" class="preview-search-hit-mark">{{ segment.text }}</mark>
                      <template v-else>{{ segment.text }}</template>
                    </template>
                  </div>
                </div>
                <div
                  v-if="showSearchRenderedContent"
                  ref="bodyRenderedSearchRef"
                  class="preview-rendered-search preview-rendered-search-body"
                >
                  <MarkdownRenderer
                    :content="bodyDraft"
                    :highlight-terms="searchQueryTerms"
                  />
                </div>
                <BaseMarkdownEditor
                  v-else
                  :model-value="bodyDraft"
                  :disabled="isReadOnly"
                  :view-mode="editorViewMode"
                  :placeholder="t('knowledge.preview.bodyPlaceholder')"
                  @update:model-value="onSectionInput('body', $event)"
                  @shortcut-save="flushPendingChanges('manual')"
                />
              </div>
            </section>

            <div v-if="footerLabel" class="editor-footnote" :class="{ 'is-warning': footerWarning }">
              {{ footerLabel }}
            </div>
          </template>
        </div>
      </div>

      <div
        v-if="document && !metaCollapsed"
        class="preview-side-resize-handle"
        @mousedown="onSideResizeStart"
      ></div>

      <aside
        v-if="document"
        class="preview-side-rail"
        :class="{ 'is-resizing': isSideResizing }"
        :style="sideRailStyle"
      >
        <div class="preview-side-rail-header">
          <div class="preview-side-tabs" role="tablist">
            <button
              type="button"
              class="preview-side-toggle preview-side-toggle-tab"
              :title="metaCollapsed ? t('knowledge.side.expand') : t('knowledge.side.collapse')"
              :aria-expanded="!metaCollapsed"
              @click="metaCollapsed = !metaCollapsed"
            >
              <span class="preview-side-toggle-chevron">{{ metaCollapsed ? "◀" : "▶" }}</span>
            </button>
            <button
              v-show="!metaCollapsed"
              type="button"
              class="preview-side-tab"
              :class="{ active: sidePanelTab === 'chat' }"
              role="tab"
              :aria-selected="sidePanelTab === 'chat'"
              @click="sidePanelTab = 'chat'"
            >
              {{ sidePanelOptions[1]?.label }}
            </button>
            <button
              v-show="!metaCollapsed"
              type="button"
              class="preview-side-tab"
              :class="{ active: sidePanelTab === 'meta' }"
              role="tab"
              :aria-selected="sidePanelTab === 'meta'"
              @click="sidePanelTab = 'meta'"
            >
              {{ sidePanelOptions[0]?.label }}
            </button>
          </div>
        </div>

        <div v-show="!metaCollapsed" class="preview-side-rail-body">
          <div v-show="sidePanelTab === 'meta'" class="preview-side-rail-panel preview-side-rail-panel-meta">
            <div class="meta-stack">
              <div class="meta-group">
                <div class="meta-row">
                  <span class="meta-label">{{ t("knowledge.meta.type") }}</span>
                  <span class="meta-value">{{ typeLabel }}</span>
                </div>
                <div class="meta-row">
                  <span class="meta-label">{{ t("knowledge.meta.scope") }}</span>
                  <span class="meta-value">{{ scopeLabel }}</span>
                </div>
                <div class="meta-row">
                  <span class="meta-label">{{ t("knowledge.meta.source") }}</span>
                  <span class="meta-value">{{ sourceSummary }}</span>
                </div>
              </div>
              <div class="meta-group">
                <div class="meta-row meta-row-control">
                  <span class="meta-label">{{ t("knowledge.meta.summaryEnabled") }}</span>
                  <div class="meta-control meta-control-switch">
                    <BaseSwitch
                      :model-value="summaryEnabled"
                      :disabled="isReadOnly"
                      :aria-label="t('knowledge.meta.summaryEnabled')"
                      @update:model-value="onSummaryEnabledChange"
                    />
                  </div>
                </div>
                <div class="meta-row meta-row-control">
                  <span class="meta-label">{{ t("knowledge.meta.explicitMaintenanceRules") }}</span>
                  <div
                    class="meta-control meta-control-switch"
                    :title="t('knowledge.meta.explicitMaintenanceRulesHint')"
                  >
                    <BaseSwitch
                      :model-value="explicitRulesEnabled"
                      :disabled="isReadOnly || saveLoading || explicitRulesLocked"
                      :aria-label="t('knowledge.meta.explicitMaintenanceRules')"
                      @update:model-value="onExplicitRulesChange"
                    />
                  </div>
                </div>
                <div class="meta-row meta-row-control meta-row-inject">
                  <span class="meta-label">{{ t("knowledge.meta.injectMode") }}</span>
                  <div class="meta-control">
                    <BaseDropdown
                      class="meta-dropdown"
                      :model-value="injectModeSelection"
                      :selected-label="injectModeDropdownLabel"
                      :options="injectModeOptions"
                      :disabled="isReadOnly || saveLoading"
                      :aria-label="t('knowledge.meta.injectMode')"
                      @update:model-value="onInjectModeChange"
                    />
                  </div>
                </div>
                <div v-if="document.type === 'skill'" class="meta-row meta-row-control">
                  <span class="meta-label">{{ t("knowledge.skill.surfaceLabel") }}</span>
                  <div class="meta-control">
                    <BaseDropdown
                      class="meta-dropdown"
                      :model-value="skillSurfaceValue"
                      :options="skillSurfaceOptions"
                      :disabled="isReadOnly || saveLoading"
                      :aria-label="t('knowledge.skill.surfaceLabel')"
                      @update:model-value="onSkillSurfaceChange"
                    />
                  </div>
                </div>
                <div v-if="showSkillCommandFields" class="meta-row meta-row-control">
                  <span class="meta-label">{{ t("knowledge.skill.commandTrigger") }}</span>
                  <div class="meta-control">
                    <input
                      v-model="skillCommandDraft"
                      class="meta-text-input"
                      type="text"
                      :disabled="skillCommandInputDisabled"
                      :placeholder="t('knowledge.skill.commandTriggerPlaceholder')"
                      @blur="onSkillCommandBlur"
                      @keydown="onSkillCommandKeydown"
                    />
                  </div>
                </div>
                <div v-if="showSkillCommandFields" class="meta-row meta-row-control">
                  <span class="meta-label">{{ t("knowledge.skill.argumentHint") }}</span>
                  <div class="meta-control">
                    <input
                      v-model="skillArgumentHintDraft"
                      class="meta-text-input"
                      type="text"
                      :disabled="isReadOnly || saveLoading"
                      @blur="persistSkillArgumentHint"
                      @keydown="onSkillArgumentHintKeydown"
                    />
                  </div>
                </div>
                <div
                  v-if="skillPackageId && (skillUnityStatusLoading || showSkillUnityStatus)"
                  class="meta-group skill-unity-group"
                >
                  <div class="meta-row">
                    <span class="meta-label">{{ t("knowledge.skill.unityStatus.label") }}</span>
                    <span class="meta-value meta-value-wrap">
                      {{
                        skillUnityStatusLoading
                          ? t("knowledge.skill.unityStatus.loading")
                          : skillUnityStatusLabel
                      }}
                    </span>
                  </div>
                  <div v-if="skillUnityStatus?.installRoot" class="meta-row">
                    <span class="meta-label">{{ t("knowledge.skill.unityStatus.path") }}</span>
                    <span class="meta-value meta-value-wrap">{{ skillUnityStatus.installRoot }}</span>
                  </div>
                  <div class="skill-unity-actions">
                    <button
                      type="button"
                      class="skill-unity-action"
                      :disabled="skillUnityStatusLoading || skillUnityActionPending || !canInstallSkillUnityFiles"
                      @click="installSkillUnity"
                    >
                      {{ t("knowledge.skill.unityStatus.install") }}
                    </button>
                    <button
                      type="button"
                      class="skill-unity-action danger"
                      :disabled="skillUnityStatusLoading || skillUnityActionPending || !canRemoveSkillUnityFiles"
                      @click="removeSkillUnity"
                    >
                      {{ t("knowledge.skill.unityStatus.remove") }}
                    </button>
                  </div>
                </div>
                <div class="meta-row meta-row-control">
                  <span class="meta-label">{{ t("knowledge.meta.editMode") }}</span>
                  <div class="meta-control">
                    <BaseDropdown
                      class="meta-dropdown"
                      :model-value="editMode"
                      :selected-label="editModeDropdownLabel"
                      :options="editModeOptions"
                      :disabled="isEditModeLocked || saveLoading"
                      :aria-label="t('knowledge.meta.editMode')"
                      @update:model-value="onEditModeChange"
                    />
                  </div>
                </div>
              </div>
              <div
                v-if="document.aiMaintained && (!document.explicitMaintenanceRules || !rulesDraft.trim())"
                class="meta-warning"
              >
                {{ t("knowledge.meta.rulesRequiredHint") }}
              </div>
              <div v-if="documentFileMetadata" class="meta-group meta-group-file">
                <div class="meta-row">
                  <span class="meta-label">{{ t("knowledge.meta.fileSize") }}</span>
                  <span class="meta-value">{{ fileSizeLabel }}</span>
                </div>
                <div class="meta-row">
                  <span class="meta-label">{{ t("knowledge.meta.length") }}</span>
                  <span class="meta-value">{{ fileLengthLabel }}</span>
                </div>
                <div class="meta-row">
                  <span class="meta-label">{{ t("knowledge.meta.estimatedTokens") }}</span>
                  <span class="meta-value">{{ estimatedTokensLabel }}</span>
                </div>
                <div class="meta-row">
                  <span class="meta-label">{{ t("knowledge.meta.modifiedAt") }}</span>
                  <span class="meta-value">{{ modifiedAtLabel }}</span>
                </div>
                <div v-if="showLastCommit" class="meta-row">
                  <span class="meta-label">{{ t("knowledge.meta.lastCommit") }}</span>
                  <span class="meta-value meta-value-wrap">{{ lastCommitLabel }}</span>
                </div>
              </div>
            </div>
          </div>

          <div v-show="sidePanelTab === 'chat'" class="preview-side-rail-panel preview-side-rail-panel-chat">
            <KnowledgeChatPane :document="document" />
          </div>
        </div>
      </aside>
    </div>
  </div>
</template>

<style scoped>
.preview-panel {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
  overflow: hidden;
  background: var(--panel-bg);
}

.preview-panel.is-resizing {
  user-select: none;
}

.preview-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 8px 16px;
  border-bottom: 1px solid var(--border-color);
  flex-shrink: 0;
}

.preview-header-main {
  min-width: 0;
  flex: 1;
  display: flex;
  align-items: center;
  gap: 8px;
}

.preview-header-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

.preview-view-segmented {
  flex-shrink: 0;
}

.preview-status-tag {
  display: inline-flex;
  align-items: center;
  min-height: 20px;
  padding: 0 8px;
  border-radius: var(--radius-badge);
  border: 1px solid color-mix(in srgb, var(--accent-border) 70%, var(--border-color) 30%);
  background: color-mix(in srgb, var(--accent-soft) 72%, var(--panel-bg) 28%);
  color: var(--accent-color);
  font-size: 11px;
  font-weight: 600;
  line-height: 1;
  flex-shrink: 0;
}

.preview-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-color);
  flex: 0 1 auto;
  max-width: min(100%, 420px);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.preview-title-input-shell {
  flex: 0 1 auto;
  min-width: 0;
  width: fit-content;
  max-width: min(100%, 420px);
  display: inline-grid;
  align-items: center;
}

.preview-title-input-shell::after {
  content: attr(data-value) " ";
  grid-area: 1 / 1;
  visibility: hidden;
  white-space: pre;
  height: 30px;
  padding: 0 10px;
  border: 1px solid transparent;
  font-size: 14px;
  font-weight: 600;
  line-height: 28px;
  box-sizing: border-box;
}

.preview-title-input {
  grid-area: 1 / 1;
  width: 100%;
  min-width: 0;
  height: 30px;
  padding: 0 10px;
  border-radius: 8px;
  border: 1px solid transparent;
  background: transparent;
  color: var(--text-color);
  font-size: 14px;
  font-weight: 600;
  outline: none;
  transition: border-color 0.15s ease, background 0.15s ease, box-shadow 0.15s ease;
}

.preview-title-input:hover {
  background: color-mix(in srgb, var(--hover-bg) 78%, transparent);
}

.preview-title-input:focus {
  border-color: color-mix(in srgb, var(--accent-color) 44%, var(--border-color));
  background: color-mix(in srgb, var(--panel-bg) 72%, var(--hover-bg) 28%);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent-color) 12%, transparent);
}

.preview-title-input::placeholder {
  color: var(--text-secondary);
  opacity: 0.72;
}

.preview-path {
  font-size: 11px;
  color: var(--text-secondary);
  opacity: 0.46;
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--font-mono-identifier);
}

.preview-shell {
  flex: 1;
  display: flex;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

.preview-main-column {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--panel-bg);
}

.preview-main {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  position: relative;
  flex-direction: column;
  overflow: hidden;
  background: var(--panel-bg);
}

.preview-pane {
  display: flex;
  flex-direction: column;
  min-width: 0;
  border-bottom: 1px solid var(--border-color);
}

.preview-support-strip {
  position: relative;
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
  min-height: 0;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--sidebar-bg) 42%, var(--panel-bg) 58%);
  overflow: hidden;
}

.preview-support-strip.has-resize-divider {
  border-bottom: none;
}

.preview-support-strip.is-warning {
  background: color-mix(in srgb, var(--status-warn-bg) 24%, var(--sidebar-bg) 32%, var(--panel-bg) 44%);
}

.preview-support-layout {
  flex: 1;
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  min-width: 0;
  min-height: 0;
  align-items: stretch;
}

.preview-support-layout.has-two-sections {
  grid-template-columns: minmax(0, 1fr) 8px minmax(0, 1fr);
}

.preview-support-layout.has-two-sections.is-compact {
  grid-template-columns: minmax(0, 1fr);
  grid-template-rows: minmax(0, 1fr) 8px minmax(0, 1fr);
}

.preview-support-toggle {
  position: absolute;
  top: 8px;
  left: 8px;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  padding: 0;
  border: none;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  box-shadow: none;
  transition: background 0.14s ease, color 0.14s ease;
  z-index: 1;
}

.preview-support-toggle:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.preview-support-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-secondary);
}

.preview-support-text {
  min-width: 0;
  font-size: 11px;
  line-height: 1.45;
  color: var(--text-secondary);
  opacity: 0.82;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.preview-support-chevron {
  flex-shrink: 0;
  font-size: 10px;
  line-height: 1;
  color: var(--text-secondary);
  transition: transform 0.14s ease, color 0.14s ease;
}

.preview-support-chevron.open {
  transform: rotate(90deg);
  color: var(--text-color);
}

.preview-support-section {
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
}

.preview-support-section.is-search-match {
  background: color-mix(in srgb, var(--accent-color) 4%, var(--panel-bg));
  box-shadow: inset 2px 0 0 color-mix(in srgb, var(--accent-color) 36%, transparent);
}

.preview-support-section-header {
  display: flex;
  flex-direction: column;
  justify-content: center;
  flex-shrink: 0;
  gap: 2px;
  padding: 8px 14px 10px;
  min-height: 46px;
}

.preview-support-section-first .preview-support-section-header {
  padding-left: 36px;
}

.preview-support-section.is-warning .preview-support-title,
.preview-support-section.is-warning :deep(.vditor) {
  color: var(--status-warn-fg);
}

.preview-support-section.is-search-match .preview-support-section-header {
  background: color-mix(in srgb, var(--accent-color) 8%, var(--sidebar-bg));
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent-color) 18%, transparent);
}

.preview-support-divider {
  position: relative;
  width: 8px;
  background: transparent;
  flex-shrink: 0;
}

.preview-support-divider::before {
  content: "";
  position: absolute;
  top: 0;
  bottom: 0;
  left: 50%;
  width: 1px;
  transform: translateX(-50%);
  background: color-mix(in srgb, var(--border-color) 88%, transparent);
  transition: background 0.15s ease;
}

.preview-support-divider.is-resizable {
  cursor: col-resize;
}

.preview-support-divider.is-resizable:hover::before,
.preview-support-divider.dragging::before {
  background: color-mix(in srgb, var(--accent-color) 38%, var(--border-color));
}

.preview-support-layout.is-compact .preview-support-divider {
  width: auto;
  height: 1px;
}

.preview-support-layout.is-compact .preview-support-divider::before {
  top: 50%;
  bottom: auto;
  left: 0;
  right: 0;
  width: auto;
  height: 1px;
  transform: translateY(-50%);
}

.preview-support-section-body {
  display: flex;
  flex-direction: column;
  flex: 1 1 auto;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 88%, transparent);
  background: var(--panel-bg);
  height: auto;
}

.preview-support-section-body.is-loading {
  opacity: 0.72;
}

.preview-support-section-body :deep(.base-markdown-editor) {
  flex: 1;
  min-height: 0;
  padding-bottom: 10px;
}

.preview-support-section-body :deep(.base-markdown-editor .vditor-ir pre.vditor-reset) {
  height: 100%;
  min-height: 100%;
  box-sizing: border-box;
  overflow: auto;
  overscroll-behavior: contain;
  scrollbar-width: thin;
  scrollbar-color: color-mix(in srgb, var(--text-secondary) 40%, transparent) transparent;
}

.preview-support-section-body :deep(.base-markdown-editor .base-markdown-editor-textarea) {
  height: 100%;
  min-height: 100%;
  box-sizing: border-box;
  overflow: auto;
  overscroll-behavior: contain;
  scrollbar-width: thin;
  scrollbar-color: color-mix(in srgb, var(--text-secondary) 40%, transparent) transparent;
}

.preview-support-section-body :deep(.base-markdown-editor .vditor-ir pre.vditor-reset::-webkit-scrollbar) {
  width: 10px;
  height: 10px;
}

.preview-support-section-body :deep(.base-markdown-editor .base-markdown-editor-textarea::-webkit-scrollbar) {
  width: 10px;
  height: 10px;
}

.preview-support-section-body :deep(.base-markdown-editor .vditor-ir pre.vditor-reset::-webkit-scrollbar-track) {
  background: transparent;
}

.preview-support-section-body :deep(.base-markdown-editor .base-markdown-editor-textarea::-webkit-scrollbar-track) {
  background: transparent;
}

.preview-support-section-body :deep(.base-markdown-editor .vditor-ir pre.vditor-reset::-webkit-scrollbar-thumb) {
  border: 2px solid transparent;
  border-radius: 999px;
  background: color-mix(in srgb, var(--text-secondary) 34%, transparent);
  background-clip: padding-box;
}

.preview-support-section-body :deep(.base-markdown-editor .base-markdown-editor-textarea::-webkit-scrollbar-thumb) {
  border: 2px solid transparent;
  border-radius: 999px;
  background: color-mix(in srgb, var(--text-secondary) 34%, transparent);
  background-clip: padding-box;
}

.preview-support-section-body :deep(.base-markdown-editor .vditor-ir pre.vditor-reset::-webkit-scrollbar-thumb:hover) {
  background: color-mix(in srgb, var(--text-secondary) 54%, transparent);
  background-clip: padding-box;
}

.preview-support-section-body :deep(.base-markdown-editor .base-markdown-editor-textarea::-webkit-scrollbar-thumb:hover) {
  background: color-mix(in srgb, var(--text-secondary) 54%, transparent);
  background-clip: padding-box;
}

.preview-main-divider {
  position: relative;
  height: 8px;
  flex-shrink: 0;
  background: transparent;
  cursor: row-resize;
}

.preview-main-divider::before {
  content: "";
  position: absolute;
  left: 0;
  right: 0;
  top: 50%;
  height: 1px;
  transform: translateY(-50%);
  background: color-mix(in srgb, var(--border-color) 88%, transparent);
  transition: background 0.15s ease;
}

.preview-main-divider:hover::before,
.preview-main-divider.dragging::before {
  background: color-mix(in srgb, var(--accent-color) 38%, var(--border-color));
}

.preview-pane-body {
  flex: 1 1 0;
  min-height: 0;
}

.preview-pane-body.is-search-match {
  background: color-mix(in srgb, var(--accent-color) 3%, var(--panel-bg));
  box-shadow: inset 2px 0 0 color-mix(in srgb, var(--accent-color) 36%, transparent);
}

.preview-pane-header {
  display: flex;
  align-items: center;
  padding: 10px 16px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--sidebar-bg) 84%, var(--panel-bg));
  font-size: 12px;
  font-weight: 600;
  color: var(--text-secondary);
}

.preview-pane-body.is-search-match .preview-pane-header {
  background: color-mix(in srgb, var(--accent-color) 8%, var(--sidebar-bg));
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent-color) 18%, transparent);
}

.preview-body {
  min-height: 160px;
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--panel-bg);
}

.preview-body.is-loading {
  opacity: 0.72;
}

.preview-pane-body .preview-body {
  min-height: 0;
}

.preview-body :deep(.base-markdown-editor) {
  flex: 1;
  min-height: 0;
  padding-bottom: 16px;
}

.preview-search-hit {
  margin: 10px 12px 0;
  padding: 8px 10px;
  border: 1px solid color-mix(in srgb, var(--accent-color) 22%, var(--border-color));
  border-radius: 8px;
  background: color-mix(in srgb, var(--accent-color) 8%, var(--hover-bg));
}

.preview-search-hit-body {
  margin: 12px 16px 0;
}

.preview-search-hit-header {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 10px;
  margin-bottom: 6px;
}

.preview-search-hit-label {
  font-size: 11px;
  font-weight: 600;
  color: var(--text-color);
}

.preview-search-hit-section {
  font-size: 11px;
  color: var(--text-secondary);
  white-space: nowrap;
}

.preview-search-hit-text {
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-secondary);
  white-space: pre-wrap;
  word-break: break-word;
}

.preview-search-hit-mark {
  padding: 0 2px;
  border-radius: 4px;
  background: color-mix(in srgb, var(--accent-color) 22%, var(--hover-bg));
  color: var(--text-color);
}

.preview-rendered-search {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 14px 16px 16px;
  overscroll-behavior: contain;
  scrollbar-width: thin;
  scrollbar-color: color-mix(in srgb, var(--text-secondary) 40%, transparent) transparent;
}

.preview-rendered-search-body {
  padding-bottom: 44px;
}

.preview-rendered-search :deep(.markdown-body) {
  min-height: 100%;
}

.preview-rendered-search::-webkit-scrollbar {
  width: 10px;
}

.preview-rendered-search::-webkit-scrollbar-track {
  background: transparent;
}

.preview-rendered-search::-webkit-scrollbar-thumb {
  border-radius: 999px;
  border: 2px solid transparent;
  background-clip: padding-box;
  background: color-mix(in srgb, var(--text-secondary) 32%, transparent);
}

.preview-rendered-search::-webkit-scrollbar-thumb:hover {
  background: color-mix(in srgb, var(--text-secondary) 46%, transparent);
}

.preview-body :deep(.base-markdown-editor .vditor-ir pre.vditor-reset) {
  height: 100%;
  min-height: 100%;
  box-sizing: border-box;
  overflow: auto;
  overscroll-behavior: contain;
  scrollbar-width: thin;
  scrollbar-color: color-mix(in srgb, var(--text-secondary) 40%, transparent) transparent;
}

.preview-body :deep(.base-markdown-editor .base-markdown-editor-textarea) {
  height: 100%;
  min-height: 100%;
  box-sizing: border-box;
  overflow: auto;
  overscroll-behavior: contain;
  scrollbar-width: thin;
  scrollbar-color: color-mix(in srgb, var(--text-secondary) 40%, transparent) transparent;
}

.preview-body :deep(.base-markdown-editor .vditor-ir pre.vditor-reset::-webkit-scrollbar) {
  width: 10px;
  height: 10px;
}

.preview-body :deep(.base-markdown-editor .base-markdown-editor-textarea::-webkit-scrollbar) {
  width: 10px;
  height: 10px;
}

.preview-body :deep(.base-markdown-editor .vditor-ir pre.vditor-reset::-webkit-scrollbar-track) {
  background: transparent;
}

.preview-body :deep(.base-markdown-editor .base-markdown-editor-textarea::-webkit-scrollbar-track) {
  background: transparent;
}

.preview-body :deep(.base-markdown-editor .vditor-ir pre.vditor-reset::-webkit-scrollbar-thumb) {
  border: 2px solid transparent;
  border-radius: 999px;
  background: color-mix(in srgb, var(--text-secondary) 34%, transparent);
  background-clip: padding-box;
}

.preview-body :deep(.base-markdown-editor .base-markdown-editor-textarea::-webkit-scrollbar-thumb) {
  border: 2px solid transparent;
  border-radius: 999px;
  background: color-mix(in srgb, var(--text-secondary) 34%, transparent);
  background-clip: padding-box;
}

.preview-body :deep(.base-markdown-editor .vditor-ir pre.vditor-reset::-webkit-scrollbar-thumb:hover) {
  background: color-mix(in srgb, var(--text-secondary) 54%, transparent);
  background-clip: padding-box;
}

.preview-body :deep(.base-markdown-editor .base-markdown-editor-textarea::-webkit-scrollbar-thumb:hover) {
  background: color-mix(in srgb, var(--text-secondary) 54%, transparent);
  background-clip: padding-box;
}

.preview-pane-body :deep(.base-markdown-editor) {
  padding-bottom: 44px;
}

.preview-empty {
  padding: 16px;
  font-size: 12px;
  color: var(--text-secondary);
}

.preview-side-resize-handle {
  width: 0;
  cursor: col-resize;
  background: transparent;
  flex-shrink: 0;
  position: relative;
  z-index: 4;
}

.preview-side-resize-handle::before {
  content: "";
  position: absolute;
  top: 0;
  bottom: 0;
  left: -3px;
  width: 6px;
}

.preview-side-resize-handle::after {
  content: "";
  position: absolute;
  top: 0;
  bottom: 0;
  left: -1px;
  width: 2px;
  background: transparent;
  transition: background 0.15s ease;
}

.preview-side-resize-handle:hover::after {
  background: color-mix(in srgb, var(--accent-color) 40%, transparent);
}

.preview-side-rail {
  flex-shrink: 0;
  min-width: 0;
  border-left: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--sidebar-bg) 86%, var(--panel-bg));
  display: flex;
  flex-direction: column;
  padding: 0;
  overflow: hidden;
  transition: width 0.16s ease;
}

.preview-side-rail:has(.meta-dropdown.open) {
  position: relative;
  z-index: 20;
  overflow: visible;
}

.preview-side-rail.is-resizing {
  transition: none;
}

.preview-side-rail-header {
  min-height: 38px;
  display: flex;
  align-items: stretch;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--sidebar-bg) 90%, var(--panel-bg));
  padding: 0;
}

.preview-side-tabs {
  width: 100%;
  min-width: 0;
  min-height: 38px;
  display: flex;
  align-items: stretch;
}

.preview-side-toggle {
  width: 30px;
  flex: 0 0 30px;
  border: none;
  background: transparent;
  color: inherit;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  box-shadow: none;
  padding: 0;
  transition: color 0.14s ease;
}

.preview-side-toggle:hover {
  color: inherit;
}

.preview-side-toggle-chevron {
  font-size: 10px;
  line-height: 1;
}

.preview-side-tab {
  flex: 1 1 0;
  min-width: 0;
  border: none;
  border-right: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
  background: transparent;
  color: var(--text-secondary);
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 0 12px;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  box-shadow: none;
  transition: background 0.14s ease, color 0.14s ease;
}

.preview-side-tabs .preview-side-tab:last-of-type {
  border-right: none;
}

.preview-side-tab:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.preview-side-tab.active {
  background: color-mix(in srgb, var(--accent-soft) 44%, var(--sidebar-bg) 56%);
  color: var(--text-color);
}

.preview-side-toggle-tab {
  border-right: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
  justify-content: center;
}

.preview-side-rail-body {
  flex: 1;
  min-height: 0;
  display: flex;
  overflow: hidden;
  width: 100%;
}

.preview-side-rail-panel {
  flex: 1;
  width: 100%;
  min-height: 0;
  overflow: auto;
}

.preview-side-rail:has(.meta-dropdown.open) .preview-side-rail-body,
.preview-side-rail:has(.meta-dropdown.open) .preview-side-rail-panel {
  overflow: visible;
}

.preview-side-rail-panel-meta {
  display: flex;
  flex-direction: column;
}

.preview-side-rail-panel-chat {
  overflow: hidden;
}

.meta-stack {
  flex: 1;
  min-height: 100%;
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 12px 14px 14px;
}

.meta-group {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.meta-group + .meta-group {
  padding-top: 12px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 86%, transparent);
}

.meta-group-file {
  margin-top: auto;
}

.meta-row {
  display: grid;
  grid-template-columns: 60px minmax(0, 1fr);
  gap: 12px;
  align-items: center;
}

.meta-label {
  font-size: 11px;
  color: var(--text-secondary);
  line-height: 1.45;
}

.meta-value {
  font-size: 12px;
  color: var(--text-color);
  line-height: 1.45;
  text-align: left;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.meta-value-wrap {
  white-space: normal;
  overflow: visible;
  text-overflow: clip;
  overflow-wrap: anywhere;
}

.skill-unity-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  padding-top: 2px;
}

.skill-unity-action {
  min-height: 28px;
  padding: 0 10px;
  border-radius: 6px;
  border: 1px solid var(--border-color);
  background: transparent;
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease, opacity 0.15s ease;
}

.skill-unity-action:hover:not(:disabled) {
  background: var(--hover-bg);
  border-color: var(--border-strong);
  color: var(--text-color);
}

.skill-unity-action.danger {
  color: var(--status-danger-fg);
  border-color: var(--status-danger-border);
}

.skill-unity-action.danger:hover:not(:disabled) {
  background: var(--status-danger-bg);
  border-color: var(--status-danger-fg);
}

.skill-unity-action:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.meta-row-control {
  align-items: center;
}

.meta-row-inject {
  align-items: center;
}

.meta-control {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
}

.meta-control-switch {
  align-items: flex-start;
}

.meta-dropdown {
  width: min(180px, 100%);
}

.meta-dropdown :deep(.base-dropdown-trigger) {
  min-width: 0;
  min-height: 30px;
}

.meta-text-input {
  flex: 1;
  width: 100%;
  min-width: 0;
  height: 30px;
  min-height: 30px;
  padding: 0 10px;
  box-sizing: border-box;
  border-radius: 6px;
  border: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 72%, var(--hover-bg) 28%);
  color: var(--text-color);
  font-size: 12px;
  line-height: 18px;
  font-family: var(--font-mono-identifier);
  outline: none;
  transition: border-color 0.15s ease, box-shadow 0.15s ease, color 0.15s ease;
}

.meta-text-input:hover:not(:disabled) {
  border-color: color-mix(in srgb, var(--text-secondary) 48%, var(--border-color));
}

.meta-text-input:focus {
  border-color: var(--accent-color);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent-color) 14%, transparent);
}

.meta-text-input:disabled {
  opacity: 0.52;
  cursor: not-allowed;
}

.meta-text-input::placeholder {
  color: var(--text-secondary);
  opacity: 0.72;
}

.meta-dropdown :deep(.base-dropdown-menu) {
  min-width: 260px;
}

.meta-warning {
  padding: 10px 12px;
  border: 1px solid var(--status-warn-border);
  border-radius: 8px;
  background: var(--status-warn-bg);
  color: var(--status-warn-fg);
  font-size: 11px;
  line-height: 1.5;
}

.editor-footnote {
  position: absolute;
  right: 16px;
  bottom: 10px;
  display: inline-flex;
  justify-content: flex-end;
  margin: 0;
  font-size: 11px;
  line-height: 1;
  color: var(--text-secondary);
  opacity: 0.62;
  pointer-events: none;
  user-select: none;
  text-align: right;
  white-space: nowrap;
  z-index: 1;
}

.editor-footnote.is-warning {
  color: var(--status-warn-fg, var(--text-color));
  opacity: 0.72;
}

@media (max-width: 860px) {
  .preview-support-layout,
  .preview-support-layout.has-two-sections,
  .preview-support-layout.is-compact,
  .preview-support-layout.has-two-sections.is-compact {
    grid-template-columns: minmax(0, 1fr);
  }

  .preview-support-layout.has-two-sections,
  .preview-support-layout.has-two-sections.is-compact {
    grid-template-rows: minmax(0, 1fr) 8px minmax(0, 1fr);
  }

  .preview-support-toggle {
    top: 8px;
    left: 8px;
  }

  .preview-support-section-header {
    min-height: 0;
  }

  .preview-support-divider {
    width: auto;
    height: 1px;
    cursor: default;
  }

  .preview-support-divider::before {
    top: 50%;
    bottom: auto;
    left: 0;
    right: 0;
    width: auto;
    height: 1px;
    transform: translateY(-50%);
  }

  .preview-support-section-body {
    min-height: 112px;
  }
}
</style>
