<script setup lang="ts">
import type { Text } from "@codemirror/state";
import { computed, onUnmounted, ref, watch } from "vue";
import type {
  KnowledgeDocument,
  KnowledgeDocumentEditOperation,
  KnowledgeDocumentPatch,
  KnowledgeEditMode,
  KnowledgeDocumentSection,
  KnowledgeSearchMatchSection,
  KnowledgeSearchSelectionContext,
  KnowledgeDocumentType,
  KnowledgeInjectMode,
  SkillUnityInstallStatus,
  SkillSurface,
} from "../../types";
import { skillSurfaceAllowsCommand } from "../../types";
import { t } from "../../i18n";
import { useNotificationStore } from "../../stores/notification";
import { useWorkspaceContextStore } from "../../stores/workspaceContext";
import { useSkills } from "../../composables/useSkills";
import {
  deriveSkillSurface,
  effectiveSkillInjectMode,
  findSkillCommandConflict,
  isValidSkillCommandTrigger,
  normalizeSkillCommandTrigger,
  skillActivationInactive,
  SKILL_COMMAND_NOTICE_OPERATION,
} from "../../composables/skillCommands";
import BaseDropdown from "../ui/BaseDropdown.vue";
import BaseButton from "../ui/BaseButton.vue";
import BaseMarkdownEditor from "../ui/BaseMarkdownEditor.vue";
import {
  markdownEditorTextHasContent,
  markdownEditorTextFromString,
  type MarkdownEditorDocumentChange,
} from "../ui/markdown-editor/markdownEditorDocumentChange";
import type { MarkdownReferenceToken } from "../ui/markdown-editor/markdownComplexTokens";
import BaseSwitch from "../ui/BaseSwitch.vue";
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
  getKnowledgeEditorDraftValue,
  normalizeKnowledgeEditorValue,
} from "./knowledgeEditorDrafts";
import {
  buildKnowledgeDocumentEditOperations,
  rebaseKnowledgeText,
  type KnowledgeTextConflict,
} from "./knowledgeCollaborativeEditing";
import {
  buildKnowledgeEditModePatch,
  defaultMaintenanceRulesForType,
  getKnowledgeEditMode,
  isKnowledgeEditModeLocked,
} from "./knowledgeEditMode";
import BaseSegmented from "../ui/BaseSegmented.vue";
import {
  useMarkdownEditorViewMode,
  type MarkdownEditorViewMode,
} from "../ui/markdownEditorViewMode";
import {
  KnowledgeEditorWorkspaceSessionStore,
  knowledgeDocumentEditorSessionKey,
  type KnowledgeDocumentEditorSession,
  type KnowledgeEditorDraftValues,
  type KnowledgeSectionConflicts,
} from "./knowledgeEditorWorkspaceSession";
import {
  buildKnowledgeDocumentWorkspaceDragPayload,
  startKnowledgeInternalDrag,
} from "./knowledgeWorkspaceDrag";
import { useInternalDragController } from "../../composables/useInternalDrag";
import type { WorkspaceRef } from "../../services/project";
import {
  claimWorkbenchReferencePointerEvent,
  workbenchReferenceFromElement,
  workbenchReferenceInternalDragSource,
  type WorkbenchReferenceDragData,
} from "../workbench/workbenchReferenceDrag";

const AUTO_SAVE_DELAY_MS = 700;
const MEMORY_PREVIEW_PATH_PREFIX = "unity-project-understanding";
const BUILTIN_MEMORY_PREVIEW_PATHS = new Set([
  "project-mistake-note.md",
  "user-preference.md",
]);
const notificationStore = useNotificationStore();
const workspaceContextStore = useWorkspaceContextStore();
const { skillItems, loadSkills } = useSkills();
const { markdownEditorViewMode, setMarkdownEditorViewMode } = useMarkdownEditorViewMode();
type InjectModeSelection = KnowledgeInjectMode | "inherit_parent";

const props = withDefaults(defineProps<{
  document: KnowledgeDocument | null;
  searchContext?: KnowledgeSearchSelectionContext | null;
  loading: boolean;
  saveLoading: boolean;
  embedded?: boolean;
  active?: boolean;
  workspaceRef?: WorkspaceRef | null;
  sessionStore?: KnowledgeEditorWorkspaceSessionStore | null;
  saveEdits?: (edits: KnowledgeDocumentEditOperation[]) => Promise<KnowledgeDocument | null>;
}>(), {
  active: true,
  workspaceRef: null,
  sessionStore: null,
});
const internalDrag = useInternalDragController();

const emit = defineEmits<{
  (e: "close"): void;
  (e: "delete"): void;
  (e: "saveSection", section: KnowledgeDocumentSection, value: string): void;
  (e: "updateMeta", patch: KnowledgeDocumentPatch): void;
  (e: "referenceOpen", reference: MarkdownReferenceToken): void;
}>();

const summaryDraft = ref("");
const rulesDraft = ref("");
const bodyDraft = ref("");
const sectionTextBuffers = new Map<KnowledgeDocumentSection, Text>();
const baseSectionTexts = new Map<KnowledgeDocumentSection, Text>();
const fileNameDraft = ref("");
const fileNameDirty = ref(false);
const dirtySections = ref<Set<KnowledgeDocumentSection>>(new Set());
const baseDrafts = ref(createKnowledgeEditorDraftValues(null));
const sectionConflicts = ref<Record<KnowledgeDocumentSection, KnowledgeTextConflict[]>>({
  summary: [],
  maintenanceRules: [],
  body: [],
});
const conflictResolutionDrafts = ref(createKnowledgeEditorDraftValues(null));
const saveBlockedSections = ref<Set<KnowledgeDocumentSection>>(new Set());
const autoSaveQueued = ref(false);
const autoSaveInFlight = ref(false);
const skillCommandDraft = ref("");
const skillArgumentHintDraft = ref("");
const skillUnityStatus = ref<SkillUnityInstallStatus | null>(null);
const skillUnityStatusLoading = ref(false);
const skillUnityActionPending = ref(false);
let autoSaveTimer: ReturnType<typeof setTimeout> | null = null;
let skillUnityStatusRequestId = 0;

const localEditorWorkspaceSessions = new KnowledgeEditorWorkspaceSessionStore();
const editorWorkspaceSessions = props.sessionStore ?? localEditorWorkspaceSessions;
const editorSessions = editorWorkspaceSessions.documents;
const markdownEditorSessions = editorWorkspaceSessions.markdownEditors;

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
  if (document.externalSource?.provider !== "package") return "";
  return document.externalSource.sourceId || document.path.split("/")[0] || "";
}

const isReadOnly = computed(() => !!props.document?.readOnly);
const isPackageDocument = computed(() => props.document?.externalSource?.provider === "package");
const documentPath = computed(() => props.document?.path?.trim() || "");
const isPackageRootDocument = computed(() => {
  if (!isPackageDocument.value) return false;
  const segments = documentPath.value.replace(/\\/g, "/").split("/").filter(Boolean);
  return segments.length === 2 && segments[1]?.toLowerCase() === "skill.md";
});
const packageDocumentConfigLocked = computed(() =>
  isPackageDocument.value && !isPackageRootDocument.value,
);
const documentMetaDisabled = computed(() => isReadOnly.value || packageDocumentConfigLocked.value);
const isEditModeLocked = computed(() =>
  isKnowledgeEditModeLocked(props.document) || packageDocumentConfigLocked.value,
);
const activeDocumentSessionKey = computed(() =>
  knowledgeDocumentEditorSessionKey(props.workspaceRef, props.document)
);
// CodeMirror state follows the same stable document identity as the draft
// session, so a path rename keeps selection and undo history.
const documentContentKey = computed(() => activeDocumentSessionKey.value);
const documentDisplayPath = computed(() => formatDocumentDisplayPath(props.document));
const documentTitle = computed(() => currentDocumentFileStem.value || t("knowledge.preview.untitled"));
const titleMeasureText = computed(() => fileNameDraft.value || " ");
const typeLabel = computed(() => labelForType(props.document?.type));
const scopeLabel = computed(() => labelForStoredScope(props.document));
const injectMode = computed(() => props.document?.effectiveInjectMode ?? "none");

function onDocumentDragPointerDown(event: PointerEvent): void {
  if (!props.document) return;
  startKnowledgeInternalDrag(internalDrag, event, {
    payload: buildKnowledgeDocumentWorkspaceDragPayload(props.document),
  });
}

function onEditorReferenceOpen(reference: MarkdownReferenceToken): void {
  emit("referenceOpen", reference);
}

function onEditorReferencePointerDown(payload: {
  reference: MarkdownReferenceToken;
  event: PointerEvent;
  element: HTMLElement;
}): void {
  const { event, element } = payload;
  const workspaceRef = props.workspaceRef;
  if (!workspaceRef || event.button !== 0 || event.isPrimary === false) return;
  const entry = workbenchReferenceFromElement(element);
  const checkout = workspaceContextStore.checkoutsById[workspaceRef.checkoutId];
  if (!entry || !checkout?.projectId || !checkout.root) return;

  const data: WorkbenchReferenceDragData = {
    version: 1,
    origin: {
      projectId: checkout.projectId,
      workspaceRef: { ...workspaceRef },
      workspaceRoot: checkout.root,
    },
    entries: [entry],
  };
  if (!internalDrag.start(event, workbenchReferenceInternalDragSource(data, element))) return;
  claimWorkbenchReferencePointerEvent(event);
}
// Skill documents show the effective auto-channel mode: a surface without the
// auto side reads as "none" regardless of the stored injectMode.
const displayInjectMode = computed<KnowledgeInjectMode>(() => (
  props.document?.type === "skill"
    ? effectiveSkillInjectMode(props.document?.skillSurface, props.document?.effectiveInjectMode)
    : injectMode.value
));
const injectModeSelection = computed<InjectModeSelection>(() => (
  props.document?.injectMode === "inherit" ? "inherit_parent" : displayInjectMode.value
));
const summaryEnabled = computed(() => !!props.document?.summary?.trim());
const showExtendedDocumentProperties = computed(() => (
  props.document?.type === "skill" || props.document?.type === "reference"
));
const editMode = computed<KnowledgeEditMode>(() => getKnowledgeEditMode(props.document));
const injectModeOptions = computed(() => [
  {
    value: "inherit_parent",
    label: t("knowledge.meta.inheritParent"),
    hint: t("knowledge.meta.inheritParentHint"),
  },
  {
    value: "none",
    label: labelForInjectMode("none", props.document?.type),
    hint: hintForInjectMode("none", props.document?.type),
  },
  {
    value: "path",
    label: labelForInjectMode("path"),
    hint: hintForInjectMode("path"),
  },
  {
    value: "excerpt",
    label: labelForInjectMode("excerpt"),
    hint: hintForInjectMode("excerpt"),
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
    value: "disabled",
    label: labelForKnowledgeEditMode("disabled"),
    hint: hintForKnowledgeEditMode("disabled"),
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
const effectiveEditMode = computed<"auto" | "proposal">(() => (
  props.document?.effectiveAiMaintained ? "auto" : "proposal"
));
const injectModeDropdownLabel = computed(() => {
  if (!props.document) return "";
  const effectiveLabel = labelForInjectMode(displayInjectMode.value, props.document.type);
  return props.document.injectMode === "inherit"
    ? labelForInheritedValue(effectiveLabel, props.document.injectModeSource)
    : effectiveLabel;
});
const editModeDropdownLabel = computed(() => {
  if (!props.document) return "";
  const effectiveLabel = labelForKnowledgeEditMode(effectiveEditMode.value);
  return props.document.aiEditMode === "inherit"
    ? labelForInheritedValue(effectiveLabel, props.document.aiConfigSource)
    : labelForKnowledgeEditMode(editMode.value);
});
const usesInheritedMaintenanceRules = computed(() => props.document?.aiEditMode === "inherit");
const rulesEditorDisabled = computed(() => documentMetaDisabled.value);
const rulesHint = computed(() => t("knowledge.preview.rulesHint"));
const rulesPropertyValue = computed(() => rulesDraft.value);
const rulesPropertyHasContent = computed(() => {
  void dirtySections.value;
  const buffered = sectionTextBuffers.get("maintenanceRules");
  return buffered
    ? markdownEditorTextHasContent(buffered)
    : !!rulesDraft.value.trim();
});

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
const fileDetailLabel = computed(() => (
  [fileSizeLabel.value, fileLengthLabel.value, `${estimatedTokensLabel.value} tokens`]
    .filter((value) => value && value !== "—" && value !== "— tokens")
    .join(" · ") || "—"
));
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

const hasUnsavedSectionChanges = computed(() => dirtySections.value.size > 0);
const currentDocumentFileStem = computed(() => extractDocumentFileStem(props.document?.path));
const hasUnsavedChanges = computed(() => hasUnsavedSectionChanges.value || fileNameDirty.value);
const conflictCount = computed(() =>
  Object.values(sectionConflicts.value).reduce((total, conflicts) => total + conflicts.length, 0)
  + saveBlockedSections.value.size,
);
const hasBlockingConflicts = computed(() => conflictCount.value > 0);

function isMarkdownEditorSessionPinned(section: KnowledgeDocumentSection): boolean {
  return dirtySections.value.has(section)
    || sectionConflicts.value[section].length > 0
    || saveBlockedSections.value.has(section);
}

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
  hasBlockingConflicts.value
  || (hasUnsavedChanges.value && !autoSaveQueued.value && !autoSaveInFlight.value),
);
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
const skillCommandChannelOn = computed(() => (
  isSkillDocument.value && skillSurfaceAllowsCommand(currentSkillSurface.value)
));
const skillAutoChannelOn = computed(() => (
  isSkillDocument.value
    && effectiveSkillInjectMode(props.document?.skillSurface, props.document?.effectiveInjectMode) !== "none"
));
const skillActivationWarningVisible = computed(() => (
  isSkillDocument.value
    && skillEnabled.value
    && skillActivationInactive({
      skillEnabled: props.document?.skillEnabled,
      skillSurface: props.document?.skillSurface,
      injectMode: props.document?.effectiveInjectMode,
    })
));
const currentSkillCommandTrigger = computed(() => {
  if (!isSkillDocument.value) return "";
  return normalizeSkillCommandTrigger(props.document?.commandTrigger ?? "", fallbackSkillName.value);
});
// Saving no longer disables the skill config controls: meta updates are
// optimistic and serialized upstream, so no disabled-dim flash per toggle.
const skillCommandInputDisabled = computed(() =>
  documentMetaDisabled.value || !skillCommandChannelOn.value,
);
const showSkillCommandFields = computed(() => skillCommandChannelOn.value);
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
const searchHighlightRe = computed<RegExp | null>(() => {
  if (!searchQueryTerms.value.length) return null;
  return new RegExp(`(${searchQueryTerms.value.map(escapeRegExp).join("|")})`, "gi");
});

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
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

watch(
  () => [props.document, activeDocumentSessionKey.value] as const,
  ([document, nextKey], previous) => {
    const previousKey = previous?.[1] ?? "";
    if (previousKey && previousKey !== nextKey) {
      captureEditorSession(previousKey);
    }
    if (!nextKey) {
      syncDrafts(true, null);
      fileNameDraft.value = "";
      fileNameDirty.value = false;
      return;
    }
    if (nextKey !== previousKey) {
      restoreEditorSession(nextKey, document);
      return;
    }
    syncDrafts(false, document);
  },
  { immediate: true },
);

watch(
  () => [activeDocumentSessionKey.value, props.document?.path ?? ""],
  (current, previous) => {
    if (!previous || current[0] !== previous[0]) return;
    const [, documentPathValue] = current;
    const currentStem = extractDocumentFileStem(documentPathValue);

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
);

watch(() => props.saveLoading, (loading, wasLoading) => {
  if (!loading && wasLoading && !autoSaveInFlight.value) {
    maybeScheduleAutoSave();
  }
});

watch(
  () => [props.document?.id ?? "", props.document?.modifiedAt ?? 0, props.document?.type ?? ""],
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

onUnmounted(() => {
  captureEditorSession(activeDocumentSessionKey.value);
  clearAutoSaveTimer();
  notificationStore.clearByOperation(SKILL_COMMAND_NOTICE_OPERATION);
});

function currentDraftValues() {
  return {
    summary: sectionValue("summary"),
    maintenanceRules: sectionValue("maintenanceRules"),
    body: sectionValue("body"),
  };
}

function cloneSectionConflicts(
  conflicts: KnowledgeSectionConflicts,
): KnowledgeSectionConflicts {
  return {
    summary: [...conflicts.summary],
    maintenanceRules: [...conflicts.maintenanceRules],
    body: [...conflicts.body],
  };
}

function captureEditorSession(key = activeDocumentSessionKey.value): void {
  if (!key) return;
  const previous = editorSessions.get(key);
  editorSessions.set(key, {
    drafts: { ...currentDraftValues() },
    baseDrafts: { ...baseDrafts.value },
    conflictResolutionDrafts: { ...conflictResolutionDrafts.value },
    dirtySections: new Set(dirtySections.value),
    sectionConflicts: cloneSectionConflicts(sectionConflicts.value),
    saveBlockedSections: new Set(saveBlockedSections.value),
    sectionTextBuffers: new Map(sectionTextBuffers),
    baseSectionTexts: new Map(baseSectionTexts),
    fileNameDraft: fileNameDraft.value,
    fileNameDirty: fileNameDirty.value,
    saveRevision: previous?.saveRevision ?? 0,
  });
}

function applyEditorSession(session: KnowledgeDocumentEditorSession): void {
  applyDraftValues({ ...session.drafts });
  setBaseDraftValues({ ...session.baseDrafts });
  sectionTextBuffers.clear();
  for (const [section, text] of session.sectionTextBuffers) {
    sectionTextBuffers.set(section, text);
  }
  baseSectionTexts.clear();
  for (const [section, text] of session.baseSectionTexts) {
    baseSectionTexts.set(section, text);
  }
  conflictResolutionDrafts.value = { ...session.conflictResolutionDrafts };
  dirtySections.value = new Set(session.dirtySections);
  sectionConflicts.value = cloneSectionConflicts(session.sectionConflicts);
  saveBlockedSections.value = new Set(session.saveBlockedSections);
  fileNameDraft.value = session.fileNameDraft;
  fileNameDirty.value = session.fileNameDirty;
}

function draftValuesEqual(
  left: KnowledgeEditorDraftValues,
  right: KnowledgeEditorDraftValues,
): boolean {
  return (["summary", "maintenanceRules", "body"] as const).every((section) =>
    normalizeKnowledgeEditorValue(getKnowledgeEditorDraftValue(left, section))
    === normalizeKnowledgeEditorValue(getKnowledgeEditorDraftValue(right, section)),
  );
}

function restoreEditorSession(key: string, document: KnowledgeDocument | null): void {
  clearAutoSaveTimer();
  autoSaveInFlight.value = false;
  const session = editorSessions.get(key);
  if (!session) {
    syncDrafts(true, document);
    fileNameDraft.value = extractDocumentFileStem(document?.path);
    fileNameDirty.value = false;
    captureEditorSession(key);
    return;
  }
  applyEditorSession(session);
  const currentStem = extractDocumentFileStem(document?.path);
  if (!fileNameDirty.value) {
    fileNameDraft.value = currentStem;
  } else if (
    normalizeDocumentFileStemValue(fileNameDraft.value)
    === normalizeDocumentFileStemValue(currentStem)
  ) {
    fileNameDraft.value = currentStem;
    fileNameDirty.value = false;
  }
  if (!draftValuesEqual(session.baseDrafts, createKnowledgeEditorDraftValues(document))) {
    syncDrafts(false, document);
  }
  captureEditorSession(key);
  maybeScheduleAutoSave();
}

function rebaseCachedEditorSession(
  session: KnowledgeDocumentEditorSession,
  sourceDocument: KnowledgeDocument,
  rebaseBaseDrafts: KnowledgeEditorDraftValues,
): KnowledgeDocumentEditorSession {
  const remoteDrafts = createKnowledgeEditorDraftValues(sourceDocument);
  const nextDrafts = { ...session.drafts };
  const nextConflictResolutions = { ...session.conflictResolutionDrafts };
  const nextDirtySections = new Set<KnowledgeDocumentSection>();
  const nextSectionTextBuffers = new Map<KnowledgeDocumentSection, Text>();
  const nextBaseSectionTexts = new Map<KnowledgeDocumentSection, Text>();
  const nextConflicts: KnowledgeSectionConflicts = {
    summary: [],
    maintenanceRules: [],
    body: [],
  };

  for (const section of ["summary", "maintenanceRules", "body"] as const) {
    const rebased = rebaseKnowledgeText(
      getKnowledgeEditorDraftValue(rebaseBaseDrafts, section),
      getKnowledgeEditorDraftValue(session.drafts, section),
      getKnowledgeEditorDraftValue(remoteDrafts, section),
    );
    setDraftSectionValue(nextDrafts, section, rebased.text);
    setDraftSectionValue(nextConflictResolutions, section, rebased.remotePreferredText);
    nextSectionTextBuffers.set(section, markdownEditorTextFromString(rebased.text));
    nextBaseSectionTexts.set(section, markdownEditorTextFromString(
      getKnowledgeEditorDraftValue(remoteDrafts, section),
    ));
    nextConflicts[section] = rebased.conflicts;
    if (
      normalizeKnowledgeEditorValue(rebased.text)
      !== normalizeKnowledgeEditorValue(getKnowledgeEditorDraftValue(remoteDrafts, section))
    ) {
      nextDirtySections.add(section);
    }
  }

  return {
    drafts: nextDrafts,
    baseDrafts: remoteDrafts,
    conflictResolutionDrafts: nextConflictResolutions,
    dirtySections: nextDirtySections,
    sectionConflicts: nextConflicts,
    saveBlockedSections: new Set(
      [...session.saveBlockedSections].filter((section) => nextDirtySections.has(section)),
    ),
    sectionTextBuffers: nextSectionTextBuffers,
    baseSectionTexts: nextBaseSectionTexts,
    fileNameDraft: session.fileNameDirty
      ? session.fileNameDraft
      : extractDocumentFileStem(sourceDocument.path),
    fileNameDirty: session.fileNameDirty,
    saveRevision: session.saveRevision,
  };
}

function applyDraftValues(nextDrafts: ReturnType<typeof createKnowledgeEditorDraftValues>) {
  sectionTextBuffers.clear();
  summaryDraft.value = nextDrafts.summary;
  rulesDraft.value = nextDrafts.maintenanceRules;
  bodyDraft.value = nextDrafts.body;
}

function setBaseDraftValues(nextDrafts: KnowledgeEditorDraftValues): void {
  baseDrafts.value = nextDrafts;
  for (const section of ["summary", "maintenanceRules", "body"] as const) {
    baseSectionTexts.set(
      section,
      markdownEditorTextFromString(getKnowledgeEditorDraftValue(nextDrafts, section)),
    );
  }
}

function setDraftSectionValue(
  drafts: ReturnType<typeof createKnowledgeEditorDraftValues>,
  section: KnowledgeDocumentSection,
  value: string,
) {
  if (section === "summary") drafts.summary = value;
  else if (section === "maintenanceRules") drafts.maintenanceRules = value;
  else drafts.body = value;
}

function resetSectionConflicts() {
  sectionConflicts.value = {
    summary: [],
    maintenanceRules: [],
    body: [],
  };
  saveBlockedSections.value = new Set();
}

function syncDrafts(
  force = false,
  sourceDocument = props.document,
  rebaseBaseDrafts?: ReturnType<typeof createKnowledgeEditorDraftValues>,
) {
  const remoteDrafts = createKnowledgeEditorDraftValues(sourceDocument);
  if (force) {
    setBaseDraftValues(remoteDrafts);
    conflictResolutionDrafts.value = remoteDrafts;
    applyDraftValues(remoteDrafts);
    dirtySections.value = new Set();
    resetSectionConflicts();
    autoSaveInFlight.value = false;
    clearAutoSaveTimer();
    return;
  }

  const currentDrafts = currentDraftValues();
  const nextDrafts = { ...currentDrafts };
  const nextBases = { ...baseDrafts.value };
  const nextConflictResolutions = { ...conflictResolutionDrafts.value };
  const nextDirtySections = new Set<KnowledgeDocumentSection>();
  const nextConflicts: Record<KnowledgeDocumentSection, KnowledgeTextConflict[]> = {
    summary: [],
    maintenanceRules: [],
    body: [],
  };

  for (const section of ["summary", "maintenanceRules", "body"] as const) {
    const base = getKnowledgeEditorDraftValue(rebaseBaseDrafts ?? baseDrafts.value, section);
    const local = getKnowledgeEditorDraftValue(currentDrafts, section);
    const remote = getKnowledgeEditorDraftValue(remoteDrafts, section);
    const rebased = rebaseKnowledgeText(base, local, remote);
    setDraftSectionValue(nextDrafts, section, rebased.text);
    setDraftSectionValue(nextBases, section, remote);
    setDraftSectionValue(
      nextConflictResolutions,
      section,
      rebased.remotePreferredText,
    );
    nextConflicts[section] = rebased.conflicts;
    if (
      normalizeKnowledgeEditorValue(rebased.text)
      !== normalizeKnowledgeEditorValue(remote)
    ) {
      nextDirtySections.add(section);
    }
  }

  setBaseDraftValues(nextBases);
  conflictResolutionDrafts.value = nextConflictResolutions;
  sectionConflicts.value = nextConflicts;
  saveBlockedSections.value = new Set(
    [...saveBlockedSections.value].filter((section) => nextDirtySections.has(section)),
  );
  applyDraftValues(nextDrafts);
  dirtySections.value = nextDirtySections;
  if (Object.values(nextConflicts).some((conflicts) => conflicts.length > 0)) {
    clearAutoSaveTimer();
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

function markDirty(section: KnowledgeDocumentSection, documentText?: Text) {
  const next = new Set(dirtySections.value);
  const matchesBase = documentText
    ? documentText.eq(
      baseSectionTexts.get(section)
        ?? markdownEditorTextFromString(getKnowledgeEditorDraftValue(baseDrafts.value, section)),
    )
    : normalizeKnowledgeEditorValue(sectionValue(section))
      === normalizeKnowledgeEditorValue(getKnowledgeEditorDraftValue(baseDrafts.value, section));
  if (matchesBase) {
    next.delete(section);
    sectionConflicts.value = {
      ...sectionConflicts.value,
      [section]: [],
    };
  } else {
    next.add(section);
  }
  dirtySections.value = next;
  const nextBlocked = new Set(saveBlockedSections.value);
  nextBlocked.delete(section);
  saveBlockedSections.value = nextBlocked;
  maybeScheduleAutoSave();
}

function maybeScheduleAutoSave() {
  clearAutoSaveTimer();
  if (
    !props.document
    || props.loading
    || props.saveLoading
    || autoSaveInFlight.value
    || isReadOnly.value
    || hasBlockingConflicts.value
    || !dirtySections.value.size
  ) {
    return;
  }
  if (!hasUnsavedSectionChanges.value) return;
  autoSaveQueued.value = true;
  autoSaveTimer = setTimeout(() => {
    autoSaveTimer = null;
    void flushPendingChanges("auto");
  }, AUTO_SAVE_DELAY_MS);
}

async function flushPendingChanges(mode: "auto" | "manual") {
  if (!props.document || props.saveLoading || autoSaveInFlight.value || isReadOnly.value) return;
  const requestDocumentKey = activeDocumentSessionKey.value;
  if (!requestDocumentKey) return;
  const shouldRenameDocument = mode === "manual" && fileNameDirty.value;
  if (!hasUnsavedSectionChanges.value && !shouldRenameDocument) return;

  if (hasBlockingConflicts.value) {
    clearAutoSaveTimer();
    return;
  }

  clearAutoSaveTimer();
  const sections = [...dirtySections.value];
  const submittedDrafts = currentDraftValues();
  const edits = sections.flatMap((section) =>
    buildKnowledgeDocumentEditOperations(
      section,
      getKnowledgeEditorDraftValue(baseDrafts.value, section),
      getKnowledgeEditorDraftValue(submittedDrafts, section),
    ),
  );
  if (edits.length) {
    autoSaveInFlight.value = true;
    if (props.saveEdits) {
      captureEditorSession(requestDocumentKey);
      const requestSession = editorSessions.get(requestDocumentKey);
      if (!requestSession) return;
      const requestRevision = requestSession.saveRevision + 1;
      editorSessions.set(requestDocumentKey, {
        ...requestSession,
        saveRevision: requestRevision,
      });
      const updated = await props.saveEdits(edits);
      const latestSession = editorSessions.get(requestDocumentKey);
      if (latestSession?.saveRevision === requestRevision) {
        if (updated) {
          if (activeDocumentSessionKey.value === requestDocumentKey) {
            syncDrafts(false, updated, submittedDrafts);
            captureEditorSession(requestDocumentKey);
          } else {
            editorSessions.set(
              requestDocumentKey,
              rebaseCachedEditorSession(latestSession, updated, submittedDrafts),
            );
          }
        } else if (activeDocumentSessionKey.value === requestDocumentKey) {
          saveBlockedSections.value = new Set([
            ...saveBlockedSections.value,
            ...sections,
          ]);
          captureEditorSession(requestDocumentKey);
        } else {
          editorSessions.set(requestDocumentKey, {
            ...latestSession,
            saveBlockedSections: new Set([
              ...latestSession.saveBlockedSections,
              ...sections,
            ]),
          });
        }
      }
      if (activeDocumentSessionKey.value === requestDocumentKey) {
        autoSaveInFlight.value = false;
      }
    } else {
      emitPendingSectionChanges();
      autoSaveInFlight.value = false;
    }
  }
  if (shouldRenameDocument && activeDocumentSessionKey.value === requestDocumentKey) {
    persistDocumentNameChange();
  }
  if (activeDocumentSessionKey.value === requestDocumentKey) {
    maybeScheduleAutoSave();
  }
}

function sectionValue(section: KnowledgeDocumentSection): string {
  const buffered = sectionTextBuffers.get(section);
  if (buffered) return buffered.toString();
  if (section === "summary") return summaryDraft.value;
  if (section === "maintenanceRules") return rulesDraft.value;
  return bodyDraft.value;
}

function onSectionInput(section: KnowledgeDocumentSection, value: string) {
  sectionTextBuffers.delete(section);
  if (section === "summary") summaryDraft.value = value;
  else if (section === "maintenanceRules") rulesDraft.value = value;
  else bodyDraft.value = value;
  markDirty(section);
}

function onSectionDocumentChange(
  section: KnowledgeDocumentSection,
  change: MarkdownEditorDocumentChange,
): void {
  sectionTextBuffers.set(section, change.doc);
  markDirty(section, change.doc);
}

function emitPendingSectionChanges() {
  const sections = [...dirtySections.value];
  for (const section of sections) {
    emit("saveSection", section, sectionValue(section));
  }
}

function acceptRemoteChanges() {
  const nextDrafts = currentDraftValues();
  const nextDirty = new Set(dirtySections.value);
  for (const section of ["summary", "maintenanceRules", "body"] as const) {
    if (!sectionConflicts.value[section].length && !saveBlockedSections.value.has(section)) continue;
    setDraftSectionValue(
      nextDrafts,
      section,
      sectionConflicts.value[section].length
        ? getKnowledgeEditorDraftValue(conflictResolutionDrafts.value, section)
        : getKnowledgeEditorDraftValue(baseDrafts.value, section),
    );
    if (
      normalizeKnowledgeEditorValue(getKnowledgeEditorDraftValue(nextDrafts, section))
      === normalizeKnowledgeEditorValue(getKnowledgeEditorDraftValue(baseDrafts.value, section))
    ) nextDirty.delete(section);
    else nextDirty.add(section);
  }
  applyDraftValues(nextDrafts);
  dirtySections.value = nextDirty;
  resetSectionConflicts();
  clearAutoSaveTimer();
}

function keepLocalChanges() {
  resetSectionConflicts();
  maybeScheduleAutoSave();
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
  if (
    !props.document
    || isReadOnly.value
    || isPackageDocument.value
    || props.saveLoading
    || !fileNameDirty.value
  ) return null;
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
    void flushPendingChanges("manual");
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

function onInjectModeChange(value: string) {
  if (value === "inherit_parent") {
    updateMeta({ injectMode: "inherit" });
    return;
  }
  const patch: KnowledgeDocumentPatch = {
    injectMode: value as KnowledgeInjectMode,
  };
  // For skills the inject mode drives the auto channel, so it also derives
  // the surface: L0/L1 turn the auto side on, none turns it off.
  if (
    props.document?.type === "skill" &&
    (value === "none" || value === "path" || value === "excerpt")
  ) {
    patch.skillSurface = deriveSkillSurface(
      skillCommandChannelOn.value,
      value !== "none",
    );
  }
  updateMeta(patch);
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
  const needsDefaultRules = nextMode === "auto" && !sectionValue("maintenanceRules").trim();
  if (needsDefaultRules) {
    const defaultRules = defaultMaintenanceRulesForType(props.document.type);
    if (defaultRules) {
      sectionTextBuffers.delete("maintenanceRules");
      rulesDraft.value = defaultRules;
      nextPatch.maintenanceRules = defaultRules;
    }
  }

  updateMeta(nextPatch);
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

function onSkillEnabledChange(value: boolean) {
  if (!props.document || props.document.type !== "skill" || isReadOnly.value) return;
  notificationStore.clearByOperation(SKILL_COMMAND_NOTICE_OPERATION);
  updateMeta({ skillEnabled: value });
}

function onSkillCommandChannelChange(value: boolean) {
  if (!props.document || props.document.type !== "skill" || isReadOnly.value) return;
  notificationStore.clearByOperation(SKILL_COMMAND_NOTICE_OPERATION);
  updateMeta({
    skillSurface: deriveSkillSurface(value, skillAutoChannelOn.value),
    commandTrigger: value
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

function persistSkillArgumentHint() {
  if (!props.document || props.document.type !== "skill" || isReadOnly.value) return;
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
  // Guard against stale responses: rapid package switches must not let a slow
  // earlier request overwrite the status of the currently shown package.
  const requestId = ++skillUnityStatusRequestId;
  const packageId = skillPackageId.value;
  const workspaceRef = workspaceContextStore.focusedWorkspaceRef;
  if (!packageId || !workspaceRef) {
    skillUnityStatus.value = null;
    return;
  }
  skillUnityStatusLoading.value = true;
  try {
    const status = await getSkillUnityInstallStatus(packageId, workspaceRef);
    if (requestId === skillUnityStatusRequestId) {
      skillUnityStatus.value = status;
    }
  } catch {
    if (requestId === skillUnityStatusRequestId) {
      skillUnityStatus.value = null;
    }
  } finally {
    if (requestId === skillUnityStatusRequestId) {
      skillUnityStatusLoading.value = false;
    }
  }
}

async function installSkillUnity() {
  const packageId = skillPackageId.value;
  const workspaceRef = workspaceContextStore.focusedWorkspaceRef;
  if (!packageId || !workspaceRef || skillUnityActionPending.value) return;
  skillUnityActionPending.value = true;
  try {
    skillUnityStatus.value = await installSkillUnityFiles(packageId, workspaceRef);
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
  const workspaceRef = workspaceContextStore.focusedWorkspaceRef;
  if (!packageId || !workspaceRef || skillUnityActionPending.value) return;
  skillUnityActionPending.value = true;
  try {
    skillUnityStatus.value = await removeSkillUnityFiles(packageId, workspaceRef);
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
    case "plan":
      return t("knowledge.type.plan");
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
  <div class="preview-panel">
    <div class="preview-shell">
      <div class="preview-main-column">
        <div v-if="!props.embedded" class="preview-header">
          <div
            class="preview-header-main"
            :class="{ 'drag-enabled': !!document }"
            @pointerdown="onDocumentDragPointerDown"
          >
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

        <div class="preview-main">
          <div v-if="loading && !document" class="preview-empty">{{ t("common.loading") }}</div>
          <div v-else-if="!document" class="preview-empty">{{ t("knowledge.empty.title") }}</div>
          <article v-else class="document-page">
            <header class="document-heading">
              <span
                v-if="!isReadOnly && !isPackageDocument"
                class="document-title-input-shell"
                :data-value="titleMeasureText"
              >
                <input
                  :value="fileNameDraft"
                  class="document-title-input"
                  type="text"
                  :disabled="saveLoading"
                  :placeholder="t('knowledge.preview.titlePlaceholder')"
                  :aria-label="t('knowledge.preview.titleLabel')"
                  @input="onFileNameInputEvent"
                  @blur="flushPendingChanges('manual')"
                  @keydown="onFileNameKeydown"
                />
              </span>
              <h1 v-else class="document-title">{{ documentTitle }}</h1>
            </header>

            <div v-if="hasBlockingConflicts" class="document-conflict" role="status">
              <span class="document-conflict-text">
                {{ t("knowledge.editor.conflict", conflictCount) }}
              </span>
              <span class="document-conflict-actions">
                <BaseButton size="sm" @click="acceptRemoteChanges">
                  {{ t("knowledge.editor.useLatest") }}
                </BaseButton>
                <BaseButton size="sm" @click="keepLocalChanges">
                  {{ t("knowledge.editor.keepLocal") }}
                </BaseButton>
              </span>
            </div>

            <section class="document-properties" :aria-label="t('knowledge.preview.properties')">
              <div class="document-properties-title">{{ t("knowledge.preview.properties") }}</div>

              <div v-if="document.type === 'skill' || document.type === 'reference'" class="document-property-row">
                <span class="document-property-label">{{ t("knowledge.meta.type") }}</span>
                <span class="document-property-value">{{ typeLabel }}</span>
              </div>
              <template v-if="showExtendedDocumentProperties">
                <div class="document-property-row">
                  <span class="document-property-label">{{ t("knowledge.meta.scope") }}</span>
                  <span class="document-property-value">{{ scopeLabel }}</span>
                </div>
                <div class="document-property-row">
                  <span class="document-property-label">{{ t("knowledge.meta.source") }}</span>
                  <span class="document-property-value document-property-value-wrap">{{ sourceSummary }}</span>
                </div>
              </template>
              <div class="document-property-row">
                <span class="document-property-label">{{ t("knowledge.meta.injectMode") }}</span>
                <BaseDropdown
                  class="document-property-dropdown meta-dropdown"
                  :model-value="injectModeSelection"
                  :selected-label="injectModeDropdownLabel"
                  :options="injectModeOptions"
                  teleport
                  :disabled="documentMetaDisabled"
                  :aria-label="t('knowledge.meta.injectMode')"
                  @update:model-value="onInjectModeChange"
                />
              </div>
              <div class="document-property-row">
                <span class="document-property-label">{{ t("knowledge.meta.editMode") }}</span>
                <BaseDropdown
                  class="document-property-dropdown meta-dropdown"
                  :model-value="editMode"
                  :selected-label="editModeDropdownLabel"
                  :options="editModeOptions"
                  teleport
                  :disabled="isEditModeLocked"
                  :aria-label="t('knowledge.meta.editMode')"
                  @update:model-value="onEditModeChange"
                />
              </div>

              <template v-if="document.type === 'skill'">
                <div class="document-property-row">
                  <span class="document-property-label">{{ t("knowledge.skill.enabledLabel") }}</span>
                  <BaseSwitch
                    :model-value="skillEnabled"
                    :disabled="documentMetaDisabled"
                    :aria-label="t('knowledge.skill.enabledLabel')"
                    @update:model-value="onSkillEnabledChange"
                  />
                </div>
                <div class="document-property-row">
                  <span class="document-property-label">{{ t("knowledge.skill.commandChannelLabel") }}</span>
                  <BaseSwitch
                    :model-value="skillCommandChannelOn"
                    :disabled="documentMetaDisabled"
                    :aria-label="t('knowledge.skill.commandChannelLabel')"
                    @update:model-value="onSkillCommandChannelChange"
                  />
                </div>
                <div v-if="showSkillCommandFields" class="document-property-row">
                  <span class="document-property-label">{{ t("knowledge.skill.commandTrigger") }}</span>
                  <input
                    v-model="skillCommandDraft"
                    class="document-property-input"
                    type="text"
                    :disabled="skillCommandInputDisabled"
                    :placeholder="t('knowledge.skill.commandTriggerPlaceholder')"
                    @blur="onSkillCommandBlur"
                    @keydown="onSkillCommandKeydown"
                  />
                </div>
                <div v-if="showSkillCommandFields" class="document-property-row">
                  <span class="document-property-label">{{ t("knowledge.skill.argumentHint") }}</span>
                  <input
                    v-model="skillArgumentHintDraft"
                    class="document-property-input"
                    type="text"
                    :disabled="documentMetaDisabled"
                    @blur="persistSkillArgumentHint"
                    @keydown="onSkillArgumentHintKeydown"
                  />
                </div>
                <div v-if="skillActivationWarningVisible" class="document-property-warning">
                  {{ t("knowledge.skill.activationWarning") }}
                </div>
                <template v-if="skillPackageId && (skillUnityStatusLoading || showSkillUnityStatus)">
                  <div class="document-property-row">
                    <span class="document-property-label">{{ t("knowledge.skill.unityStatus.label") }}</span>
                    <span class="document-property-value document-property-value-wrap">
                      {{ skillUnityStatusLoading ? t("knowledge.skill.unityStatus.loading") : skillUnityStatusLabel }}
                    </span>
                  </div>
                  <div v-if="skillUnityStatus?.installRoot" class="document-property-row">
                    <span class="document-property-label">{{ t("knowledge.skill.unityStatus.path") }}</span>
                    <span class="document-property-value document-property-value-wrap">{{ skillUnityStatus.installRoot }}</span>
                  </div>
                  <div class="document-property-row">
                    <span class="document-property-label"></span>
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
                </template>
              </template>

              <div v-if="documentFileMetadata" class="document-property-row">
                <span class="document-property-label">{{ t("knowledge.meta.fileSize") }}</span>
                <span class="document-property-value">{{ fileDetailLabel }}</span>
              </div>
              <template v-if="showExtendedDocumentProperties && documentFileMetadata">
                <div class="document-property-row">
                  <span class="document-property-label">{{ t("knowledge.meta.modifiedAt") }}</span>
                  <span class="document-property-value document-property-value-wrap">
                    {{ modifiedAtLabel }}<template v-if="showLastCommit"> · {{ lastCommitLabel }}</template>
                  </span>
                </div>
              </template>
            </section>

            <section
              v-if="summaryEnabled || !isReadOnly"
              class="document-inline-field document-inline-summary"
              :class="{ 'is-search-match': isSearchMatchSection('summary') }"
            >
              <div class="document-inline-label">{{ t("knowledge.preview.summary") }}</div>
              <div v-if="searchSnippetVisible('summary')" class="preview-search-hit">
                <div class="preview-search-hit-text">
                  <template v-for="(segment, index) in searchSnippetSegments('summary')" :key="`summary-${index}`">
                    <mark v-if="segment.hit" class="preview-search-hit-mark">{{ segment.text }}</mark>
                    <template v-else>{{ segment.text }}</template>
                  </template>
                </div>
              </div>
              <BaseMarkdownEditor
                :model-value="summaryDraft"
                :active="active"
                :session-cache="markdownEditorSessions"
                :session-pinned="isMarkdownEditorSessionPinned('summary')"
                :workspace-ref="workspaceRef"
                transaction-model
                :disabled="isReadOnly"
                :view-mode="editorViewMode"
                :content-key="`${documentContentKey}:summary`"
                auto-grow
                :min-height="64"
                :placeholder="t('knowledge.preview.summaryPlaceholder')"
                @update:model-value="onSectionInput('summary', $event)"
                @document-change="onSectionDocumentChange('summary', $event)"
                @shortcut-save="flushPendingChanges('manual')"
                @reference-open="onEditorReferenceOpen"
                @reference-pointer-down="onEditorReferencePointerDown"
              />
            </section>

            <section
              v-if="
                !usesInheritedMaintenanceRules &&
                (rulesPropertyHasContent || !rulesEditorDisabled)
              "
              class="document-inline-field document-inline-rules"
              :class="{
                'is-search-match': isSearchMatchSection('maintenanceRules'),
                'is-warning': document.effectiveAiMaintained && !rulesPropertyHasContent,
              }"
            >
              <div class="document-inline-label-row">
                <span class="document-inline-label">{{ t("knowledge.preview.rules") }}</span>
                <span class="document-inline-source">{{ rulesHint }}</span>
              </div>
              <div v-if="searchSnippetVisible('maintenanceRules')" class="preview-search-hit">
                <div class="preview-search-hit-text">
                  <template v-for="(segment, index) in searchSnippetSegments('maintenanceRules')" :key="`rules-${index}`">
                    <mark v-if="segment.hit" class="preview-search-hit-mark">{{ segment.text }}</mark>
                    <template v-else>{{ segment.text }}</template>
                  </template>
                </div>
              </div>
              <BaseMarkdownEditor
                :model-value="rulesPropertyValue"
                :active="active"
                :session-cache="markdownEditorSessions"
                :session-pinned="isMarkdownEditorSessionPinned('maintenanceRules')"
                :workspace-ref="workspaceRef"
                transaction-model
                :disabled="rulesEditorDisabled"
                :view-mode="editorViewMode"
                :content-key="`${documentContentKey}:maintenanceRules`"
                auto-grow
                :min-height="104"
                :placeholder="t('knowledge.preview.rulesPlaceholder')"
                @update:model-value="onSectionInput('maintenanceRules', $event)"
                @document-change="onSectionDocumentChange('maintenanceRules', $event)"
                @shortcut-save="flushPendingChanges('manual')"
                @reference-open="onEditorReferenceOpen"
                @reference-pointer-down="onEditorReferencePointerDown"
              />
            </section>

            <div
              v-if="
                !usesInheritedMaintenanceRules &&
                document.effectiveAiMaintained &&
                !rulesPropertyHasContent
              "
              class="document-property-warning"
            >
              {{ t("knowledge.meta.rulesRequiredHint") }}
            </div>

            <section class="document-body" :class="{ 'is-search-match': isSearchMatchSection('body'), 'is-loading': loading }">
                <div v-if="searchSnippetVisible('body')" class="preview-search-hit preview-search-hit-body">
                  <div class="preview-search-hit-text">
                    <template v-for="(segment, index) in searchSnippetSegments('body')" :key="`body-${index}`">
                      <mark v-if="segment.hit" class="preview-search-hit-mark">{{ segment.text }}</mark>
                      <template v-else>{{ segment.text }}</template>
                    </template>
                  </div>
                </div>
                <BaseMarkdownEditor
                  :model-value="bodyDraft"
                  :active="active"
                  :session-cache="markdownEditorSessions"
                  :session-pinned="isMarkdownEditorSessionPinned('body')"
                  :workspace-ref="workspaceRef"
                  transaction-model
                  :disabled="isReadOnly"
                  :view-mode="editorViewMode"
                  :content-path="documentPath"
                  :content-key="`${documentContentKey}:body`"
                  auto-grow
                  :min-height="360"
                  :placeholder="t('knowledge.preview.bodyPlaceholder')"
                  @update:model-value="onSectionInput('body', $event)"
                  @document-change="onSectionDocumentChange('body', $event)"
                  @shortcut-save="flushPendingChanges('manual')"
                  @reference-open="onEditorReferenceOpen"
                  @reference-pointer-down="onEditorReferencePointerDown"
                />
            </section>

            <div v-if="footerLabel" class="editor-footnote" :class="{ 'is-warning': footerWarning }">
              {{ footerLabel }}
            </div>
          </article>
        </div>
      </div>

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

.preview-header-main.drag-enabled {
  cursor: grab;
  touch-action: none;
}

.preview-header-main.drag-enabled:active {
  cursor: grabbing;
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
.preview-support-section.is-warning :deep(.cm-content) {
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

.preview-support-section-body :deep(.base-markdown-editor .cm-scroller) {
  height: 100%;
  min-height: 100%;
  box-sizing: border-box;
  overflow: auto;
  overscroll-behavior: contain;
  scrollbar-width: thin;
  scrollbar-color: color-mix(in srgb, var(--text-secondary) 40%, transparent) transparent;
}

.preview-support-section-body :deep(.base-markdown-editor .cm-scroller::-webkit-scrollbar) {
  width: 10px;
  height: 10px;
}

.preview-support-section-body :deep(.base-markdown-editor .cm-scroller::-webkit-scrollbar-track) {
  background: transparent;
}

.preview-support-section-body :deep(.base-markdown-editor .cm-scroller::-webkit-scrollbar-thumb) {
  border: 2px solid transparent;
  border-radius: 999px;
  background: color-mix(in srgb, var(--text-secondary) 34%, transparent);
  background-clip: padding-box;
}

.preview-support-section-body :deep(.base-markdown-editor .cm-scroller::-webkit-scrollbar-thumb:hover) {
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

.preview-body :deep(.base-markdown-editor .cm-scroller) {
  height: 100%;
  min-height: 100%;
  box-sizing: border-box;
  overflow: auto;
  overscroll-behavior: contain;
  scrollbar-width: thin;
  scrollbar-color: color-mix(in srgb, var(--text-secondary) 40%, transparent) transparent;
}

.preview-body :deep(.base-markdown-editor .cm-scroller::-webkit-scrollbar) {
  width: 10px;
  height: 10px;
}

.preview-body :deep(.base-markdown-editor .cm-scroller::-webkit-scrollbar-track) {
  background: transparent;
}

.preview-body :deep(.base-markdown-editor .cm-scroller::-webkit-scrollbar-thumb) {
  border: 2px solid transparent;
  border-radius: 999px;
  background: color-mix(in srgb, var(--text-secondary) 34%, transparent);
  background-clip: padding-box;
}

.preview-body :deep(.base-markdown-editor .cm-scroller::-webkit-scrollbar-thumb:hover) {
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

.skill-activation-warning {
  padding: 8px 10px;
  border: 1px solid color-mix(in srgb, var(--warning-color, #d9a03f) 42%, var(--border-color));
  border-radius: 6px;
  background: color-mix(in srgb, var(--warning-color, #d9a03f) 9%, transparent);
  color: var(--text-color);
  font-size: 11px;
  line-height: 1.55;
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

/* Continuous document workspace. Metadata and content share one scroll plane. */
.preview-main {
  display: block;
  overflow: auto;
  scrollbar-width: thin;
  scrollbar-color: color-mix(in srgb, var(--text-secondary) 34%, transparent) transparent;
}

.document-page {
  position: relative;
  width: min(100%, 980px);
  min-height: 100%;
  margin: 0 auto;
  padding: 32px 44px 72px;
  box-sizing: border-box;
}

.document-heading {
  margin: 0 0 22px;
}

.document-conflict {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  margin: -8px 0 20px;
  padding: 8px 10px;
  border-left: 2px solid var(--status-warn-border);
  background: color-mix(in srgb, var(--status-warn-bg) 56%, transparent);
  color: var(--status-warn-fg);
  font-size: 12px;
  line-height: 1.5;
}

.document-conflict-text {
  min-width: 0;
}

.document-conflict-actions {
  display: inline-flex;
  flex: none;
  align-items: center;
  gap: 4px;
}

.document-title,
.document-title-input {
  margin: 0;
  color: var(--text-color);
  font-size: 24px;
  font-weight: 650;
  line-height: 1.3;
  letter-spacing: -0.015em;
}

.document-title-input-shell {
  display: grid;
  width: 100%;
  min-width: 0;
}

.document-title-input-shell::after {
  content: attr(data-value) " ";
  grid-area: 1 / 1;
  visibility: hidden;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

.document-title-input {
  grid-area: 1 / 1;
  width: 100%;
  min-width: 0;
  padding: 2px 0 4px;
  border: none;
  border-bottom: 1px solid transparent;
  background: transparent;
  outline: none;
}

.document-title-input:hover {
  border-bottom-color: var(--border-color);
}

.document-title-input:focus {
  border-bottom-color: color-mix(in srgb, var(--accent-color) 48%, var(--border-color));
}

.document-title-input::placeholder {
  color: var(--text-secondary);
  opacity: 0.62;
}

.document-properties {
  display: flex;
  flex-direction: column;
  gap: 1px;
  margin-bottom: 24px;
}

.document-properties-title {
  margin-bottom: 7px;
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 600;
}

.document-property-row {
  display: grid;
  grid-template-columns: 112px minmax(0, 1fr);
  align-items: center;
  gap: 12px;
  min-height: 30px;
  padding: 1px 0;
}

.document-property-label {
  min-width: 0;
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.45;
}

.document-property-value {
  min-width: 0;
  overflow: hidden;
  color: var(--text-color);
  font-size: 13px;
  line-height: 1.5;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.document-property-value-wrap {
  overflow: visible;
  overflow-wrap: anywhere;
  text-overflow: clip;
  white-space: normal;
}

.document-property-dropdown {
  width: min(300px, 100%);
}

.document-property-dropdown :deep(.base-dropdown-trigger) {
  min-height: 28px;
  padding-inline: 8px;
  border-color: transparent;
  background: transparent;
}

.document-property-dropdown :deep(.base-dropdown-trigger:hover),
.document-property-dropdown.open :deep(.base-dropdown-trigger) {
  border-color: var(--border-color);
  background: var(--hover-bg);
}

.document-property-input {
  width: min(360px, 100%);
  height: 28px;
  padding: 0 8px;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--text-color);
  font: inherit;
  font-family: var(--font-mono-identifier);
  font-size: 12px;
  outline: none;
}

.document-property-input:hover,
.document-property-input:focus {
  border-color: var(--border-color);
  background: var(--hover-bg);
}

.document-property-warning {
  margin: 8px 0;
  padding: 8px 10px;
  border-left: 2px solid var(--status-warn-border);
  color: var(--status-warn-fg);
  background: color-mix(in srgb, var(--status-warn-bg) 48%, transparent);
  font-size: 11px;
  line-height: 1.55;
}

.document-inline-field {
  margin: 0 0 22px;
  padding: 0;
}

.document-inline-label-row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
}

.document-inline-label {
  display: block;
  margin-bottom: 7px;
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 600;
}

.document-inline-source {
  min-width: 0;
  overflow: hidden;
  color: var(--text-secondary);
  font-size: 11px;
  opacity: 0.68;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.document-inline-field :deep(.base-markdown-editor) {
  height: auto;
  border-left: 1px solid var(--border-color);
}

.document-inline-summary :deep(.base-markdown-editor) {
  min-height: 64px;
}

.document-inline-rules :deep(.base-markdown-editor) {
  min-height: 104px;
}

.document-inline-field :deep(.base-markdown-editor .cm-editor) {
  background: transparent;
}

.document-inline-field.is-search-match,
.document-body.is-search-match {
  box-shadow: inset 2px 0 0 color-mix(in srgb, var(--accent-color) 46%, transparent);
}

.document-inline-field.is-warning .document-inline-label {
  color: var(--status-warn-fg);
}

.document-body {
  min-height: 360px;
  padding-top: 20px;
  border-top: 1px solid var(--border-color);
}

.document-body :deep(.base-markdown-editor) {
  min-height: 360px;
  height: auto;
  padding-bottom: 32px;
}

.document-body :deep(.base-markdown-editor .cm-scroller) {
  height: auto;
  min-height: 360px;
  overflow: visible;
  overscroll-behavior: auto;
}

.document-page .preview-search-hit {
  margin: 6px 0 8px;
}

.document-page .editor-footnote {
  position: fixed;
  right: 16px;
  bottom: 10px;
}

@media (max-width: 860px) {
  .document-page {
    padding: 24px 24px 64px;
  }

  .document-property-row {
    grid-template-columns: 96px minmax(0, 1fr);
  }
}
</style>
