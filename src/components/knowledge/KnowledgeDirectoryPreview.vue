<script setup lang="ts">
import type { Text } from "@codemirror/state";
import { computed, onUnmounted, ref, watch } from "vue";
import type {
  EffectiveCapabilityState,
  FolderIndexRuleSetting,
  KnowledgeDirectoryConfig,
  KnowledgeDirectoryConfigRecord,
  KnowledgeInjectMode,
} from "../../types";
import { t } from "../../i18n";
import { defaultMaintenanceRulesForType } from "./knowledgeEditMode";
import {
  hintForFolderSearchRule,
  labelForFolderSearchRule,
  labelForInheritedValue,
  labelForInjectMode,
  type KnowledgeSearchTagKind,
} from "./knowledgeMetaLabels";
import BaseButton from "../ui/BaseButton.vue";
import BaseCheckbox from "../ui/BaseCheckbox.vue";
import BaseDropdown from "../ui/BaseDropdown.vue";
import BaseMarkdownEditor from "../ui/BaseMarkdownEditor.vue";
import {
  markdownEditorTextHasContent,
  markdownEditorTextFromString,
  type MarkdownEditorDocumentChange,
} from "../ui/markdown-editor/markdownEditorDocumentChange";
import BaseSegmented from "../ui/BaseSegmented.vue";
import ReferenceExternalImportPanel from "./ReferenceExternalImportPanel.vue";
import {
  useMarkdownEditorViewMode,
  type MarkdownEditorViewMode,
} from "../ui/markdownEditorViewMode";
import type { WorkspaceRef } from "../../services/project";
import {
  KnowledgeEditorWorkspaceSessionStore,
  knowledgeDirectoryEditorSessionKey,
  type DirectoryMarkdownField,
  type KnowledgeDirectoryEditorSession,
} from "./knowledgeEditorWorkspaceSession";
import {
  buildKnowledgeFolderWorkspaceDragPayload,
  startKnowledgeInternalDrag,
} from "./knowledgeWorkspaceDrag";
import { useInternalDragController } from "../../composables/useInternalDrag";

const props = withDefaults(defineProps<{
  workspaceRef: WorkspaceRef;
  directory: KnowledgeDirectoryConfigRecord | null;
  loading: boolean;
  saveLoading: boolean;
  active?: boolean;
  sessionStore?: KnowledgeEditorWorkspaceSessionStore | null;
  pathExists?: ((path: string) => boolean) | null;
  ensureDirectory?: ((path: string) => Promise<boolean>) | null;
  selectDirectory?: ((path: string) => Promise<void>) | null;
  refreshKnowledge?: (() => Promise<void>) | null;
  deleteFeishuImport?: ((path: string) => Promise<void>) | null;
  deleteUnityImport?: ((path: string) => Promise<void>) | null;
}>(), {
  active: true,
  sessionStore: null,
});
const internalDrag = useInternalDragController();

const emit = defineEmits<{
  (e: "close"): void;
  (e: "save", path: string, config: KnowledgeDirectoryConfig): void;
}>();
type DirectoryInjectModeSelection = KnowledgeInjectMode | "inherit_parent";
type DirectoryAiConfigMode = "inherit_parent" | "manual" | "auto";
type DirectoryPanelTab = "config" | "external";
const AUTO_SAVE_DELAY_MS = 900;
const { markdownEditorViewMode, setMarkdownEditorViewMode } = useMarkdownEditorViewMode();
const activePanelTab = ref<DirectoryPanelTab>("config");

const draft = ref<KnowledgeDirectoryConfig>({
  version: 4,
  summary: "",
  injectMode: "inherit",
  effectiveInjectMode: "excerpt",
  aiMaintained: "inherit",
  effectiveAiMaintained: false,
  lexicalSearch: "inherit",
  vectorSearch: "inherit",
  inheritToChildren: true,
  allowCreateDocuments: true,
  allowCreateDirectories: true,
  allowMoveDocuments: true,
  allowMoveDirectories: true,
  maintenanceRules: null,
  effectiveMaintenanceRules: null,
});
const autoSaveQueued = ref(false);
const autoSaveInFlight = ref(false);
const markdownTextBuffers = new Map<DirectoryMarkdownField, Text>();
const markdownBaseTexts = new Map<DirectoryMarkdownField, Text>();
const markdownDirtyFields = ref<Set<DirectoryMarkdownField>>(new Set());
let autoSaveTimer: ReturnType<typeof setTimeout> | null = null;
let submittedDirectoryKey = "";
let submittedDraft: KnowledgeDirectoryConfig | null = null;
let activeDirectoryKey = "";
let activeDirectoryBaseDraft: KnowledgeDirectoryConfig = { ...draft.value };
const localEditorWorkspaceSessions = new KnowledgeEditorWorkspaceSessionStore();
const editorWorkspaceSessions = props.sessionStore ?? localEditorWorkspaceSessions;
const directorySessions = editorWorkspaceSessions.directories;
const markdownEditorSessions = editorWorkspaceSessions.markdownEditors;

function directoryKey(directory: KnowledgeDirectoryConfigRecord | null | undefined): string {
  return knowledgeDirectoryEditorSessionKey(props.workspaceRef, directory);
}

function configFromDirectory(directory: KnowledgeDirectoryConfigRecord): KnowledgeDirectoryConfig {
  return {
    version: directory.version,
    summary: directory.summary ?? "",
    injectMode: directory.injectMode,
    effectiveInjectMode: directory.effectiveInjectMode,
    aiMaintained: directory.aiMaintained,
    effectiveAiMaintained: directory.effectiveAiMaintained,
    lexicalSearch: directory.lexicalSearch ?? "inherit",
    vectorSearch: directory.vectorSearch ?? "inherit",
    inheritToChildren: directory.inheritToChildren !== false,
    allowCreateDocuments: directory.allowCreateDocuments !== false,
    allowCreateDirectories: directory.allowCreateDirectories !== false,
    allowMoveDocuments: directory.allowMoveDocuments !== false,
    allowMoveDirectories: directory.allowMoveDirectories !== false,
    maintenanceRules: directory.maintenanceRules,
    effectiveMaintenanceRules: directory.effectiveMaintenanceRules,
  };
}

function mergeDirectorySaveResponse(
  remote: KnowledgeDirectoryConfig,
  local: KnowledgeDirectoryConfig,
  submitted: KnowledgeDirectoryConfig,
): KnowledgeDirectoryConfig {
  const merged = { ...remote };
  for (const key of Object.keys(remote) as Array<keyof KnowledgeDirectoryConfig>) {
    if (JSON.stringify(local[key]) === JSON.stringify(submitted[key])) continue;
    (merged as Record<string, unknown>)[key] = local[key];
  }
  return merged;
}

function directoryConfigValuesEqual(left: unknown, right: unknown): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

function mergeUnsavedDirectoryDraft(
  remote: KnowledgeDirectoryConfig,
  local: KnowledgeDirectoryConfig,
  base: KnowledgeDirectoryConfig,
): KnowledgeDirectoryConfig {
  const merged = { ...remote };
  for (const key of Object.keys(remote) as Array<keyof KnowledgeDirectoryConfig>) {
    if (directoryConfigValuesEqual(local[key], base[key])) continue;
    (merged as Record<string, unknown>)[key] = local[key];
  }
  return merged;
}

function currentDirectorySession(): KnowledgeDirectoryEditorSession {
  const materialized = { ...materializedDirectoryDraft() };
  return {
    draft: materialized,
    baseDraft: { ...activeDirectoryBaseDraft },
    markdownTextBuffers: new Map(markdownTextBuffers),
    markdownBaseTexts: new Map(markdownBaseTexts),
    markdownDirtyFields: new Set(markdownDirtyFields.value),
    submittedDraft: submittedDirectoryKey === activeDirectoryKey && submittedDraft
      ? { ...submittedDraft }
      : null,
    dirty: markdownDirtyFields.value.size > 0
      || !directoryConfigValuesEqual(materialized, activeDirectoryBaseDraft),
  };
}

function captureDirectorySession(key = activeDirectoryKey): void {
  if (!key) return;
  directorySessions.set(key, currentDirectorySession());
}

function replaceDirectoryTextMap<K>(target: Map<K, Text>, source: Map<K, Text>): void {
  target.clear();
  for (const [key, text] of source) target.set(key, text);
}

function applyRemoteDirectory(
  remote: KnowledgeDirectoryConfig,
  session?: KnowledgeDirectoryEditorSession,
): void {
  if (!session) {
    draft.value = { ...remote };
    activeDirectoryBaseDraft = { ...remote };
    markdownTextBuffers.clear();
    markdownDirtyFields.value = new Set();
    setDirectoryMarkdownBase(remote);
    submittedDirectoryKey = "";
    submittedDraft = null;
    return;
  }

  const local = { ...session.draft };
  const merged = session.submittedDraft
    ? mergeDirectorySaveResponse(remote, local, session.submittedDraft)
    : mergeUnsavedDirectoryDraft(remote, local, session.baseDraft);
  draft.value = merged;
  activeDirectoryBaseDraft = { ...remote };
  replaceDirectoryTextMap(markdownTextBuffers, session.markdownTextBuffers);
  replaceDirectoryTextMap(markdownBaseTexts, session.markdownBaseTexts);
  for (const field of ["summary", "maintenanceRules"] as const) {
    if (directoryMarkdownValue(merged, field) === directoryMarkdownValue(remote, field)) {
      markdownTextBuffers.delete(field);
    }
  }
  setDirectoryMarkdownBase(remote);
  refreshMarkdownDirtyFields(remote);
  submittedDirectoryKey = "";
  submittedDraft = null;
}

watch(
  () => [props.directory, directoryKey(props.directory)] as const,
  ([directory, nextKey]) => {
    clearAutoSaveTimer();
    autoSaveInFlight.value = false;
    let session: KnowledgeDirectoryEditorSession | undefined;
    if (activeDirectoryKey && activeDirectoryKey === nextKey) {
      session = currentDirectorySession();
    } else {
      captureDirectorySession(activeDirectoryKey);
      session = nextKey ? directorySessions.get(nextKey) : undefined;
    }
    if (!directory) {
      activeDirectoryKey = "";
      markdownTextBuffers.clear();
      markdownBaseTexts.clear();
      markdownDirtyFields.value = new Set();
      submittedDirectoryKey = "";
      submittedDraft = null;
      return;
    }
    const remote = configFromDirectory(directory);
    activeDirectoryKey = nextKey;
    applyRemoteDirectory(remote, session);
    if (session?.dirty) {
      void Promise.resolve().then(() => {
        if (activeDirectoryKey === nextKey && isDirty.value) maybeScheduleAutoSave();
      });
    }
  },
  { immediate: true },
);

const showExternalImportTab = computed(() =>
  props.directory?.type === "reference" && !!props.directory?.path?.trim(),
);

watch(showExternalImportTab, (enabled) => {
  if (!enabled && activePanelTab.value === "external") {
    activePanelTab.value = "config";
  }
});

const statusLabel = computed(() => {
  if (!props.directory) return "";
  if (props.directory.readOnly) return t("knowledge.meta.readOnly");
  if (props.saveLoading) {
    return autoSaveInFlight.value
      ? t("knowledge.editor.autosaving")
      : t("knowledge.editor.saving");
  }
  if (autoSaveQueued.value) return t("knowledge.editor.autosavePending");
  if (!props.directory.exists && !isDirty.value) {
    return t("knowledge.directoryConfig.missing");
  }
  return isDirty.value ? t("knowledge.editor.unsaved") : t("knowledge.editor.saved");
});
const footerLabel = computed(() => {
  if (!props.directory) return "";
  if (props.directory.readOnly) return statusLabel.value;
  return `${statusLabel.value} · ${t("knowledge.editor.shortcut")}`;
});
const interactionDisabled = computed(() => !!props.directory?.readOnly);
const editorViewOptions = computed(() => [
  { value: "rendered", label: t("knowledge.editor.view.rendered") },
  { value: "native", label: t("knowledge.editor.view.native") },
]);
const editorViewMode = computed<MarkdownEditorViewMode>({
  get: () => markdownEditorViewMode.value,
  set: (value) => setMarkdownEditorViewMode(value),
});
const directoryContentKey = computed(() => directoryKey(props.directory));

const hasExplicitRulesContent = computed(() => {
  void markdownDirtyFields.value;
  const buffered = markdownTextBuffers.get("maintenanceRules");
  return buffered
    ? markdownEditorTextHasContent(buffered)
    : !!draft.value.maintenanceRules?.trim();
});
const hasRulesWarning = computed(
  () => draft.value.aiMaintained === true && !hasExplicitRulesContent.value,
);

const injectModeOptions = computed(() => [
  {
    value: "inherit_parent",
    label: t("knowledge.meta.inheritParent"),
    hint: t("knowledge.meta.inheritParentHint"),
  },
  {
    value: "none",
    label: labelForInjectMode("none"),
    hint: hintForDirectoryInjectMode("none"),
  },
  {
    value: "path",
    label: labelForInjectMode("path"),
    hint: hintForDirectoryInjectMode("path"),
  },
  {
    value: "excerpt",
    label: labelForInjectMode("excerpt"),
    hint: hintForDirectoryInjectMode("excerpt"),
  },
]);

const aiConfigOptions = computed(() => [
  {
    value: "inherit_parent",
    label: t("knowledge.meta.inheritParent"),
    hint: t("knowledge.meta.inheritParentHint"),
  },
  {
    value: "manual",
    label: t("knowledge.directoryConfig.aiConfig.manual"),
    hint: t("knowledge.directoryConfig.aiConfig.manualHint"),
  },
  {
    value: "auto",
    label: t("knowledge.directoryConfig.aiConfig.auto"),
    hint: t("knowledge.directoryConfig.aiConfig.autoHint"),
  },
]);

const lexicalRuleOptions = computed(() => buildFolderIndexRuleOptions("lexical"));
const semanticRuleOptions = computed(() => buildFolderIndexRuleOptions("semantic"));

const injectModeValue = computed<DirectoryInjectModeSelection>(() => (
  draft.value.injectMode === "inherit" ? "inherit_parent" : draft.value.injectMode
));

const aiConfigValue = computed<DirectoryAiConfigMode>(() => {
  if (draft.value.aiMaintained === "inherit") return "inherit_parent";
  return draft.value.aiMaintained ? "auto" : "manual";
});

const injectModeDropdownLabel = computed(() => {
  if (!props.directory) return "";
  const explicitLabel = labelForInjectMode(draft.value.effectiveInjectMode);
  if (draft.value.injectMode !== "inherit") return explicitLabel;
  if (props.directory.injectMode === "inherit") {
    return labelForInheritedValue(
      labelForInjectMode(props.directory.effectiveInjectMode),
      props.directory.injectModeSource,
    );
  }
  return t("knowledge.meta.inheritParent");
});

const aiConfigDropdownLabel = computed(() => {
  if (!props.directory) return "";
  const explicitLabel = draft.value.effectiveAiMaintained
    ? t("knowledge.directoryConfig.aiConfig.auto")
    : t("knowledge.directoryConfig.aiConfig.manual");
  if (draft.value.aiMaintained !== "inherit") return explicitLabel;
  if (props.directory.aiMaintained === "inherit") {
    const effectiveLabel = props.directory.effectiveAiMaintained
      ? t("knowledge.directoryConfig.aiConfig.auto")
      : t("knowledge.directoryConfig.aiConfig.manual");
    return labelForInheritedValue(effectiveLabel, props.directory.aiConfigSource);
  }
  return t("knowledge.meta.inheritParent");
});

const rulesEditorDisabled = computed(() => interactionDisabled.value || draft.value.aiMaintained === "inherit");

const effectiveLexicalSearch = computed<EffectiveCapabilityState>(() => (
  props.directory?.effectiveLexicalSearch ?? {
    enabled: true,
    source: "default",
  }
));

const effectiveVectorSearch = computed<EffectiveCapabilityState>(() => (
  props.directory?.effectiveVectorSearch ?? {
    enabled: true,
    source: "default",
  }
));

const isDirty = computed(() => {
  const directory = props.directory;
  if (!directory) return false;
  return markdownDirtyFields.value.size > 0 || (
    JSON.stringify({
      version: directory.version,
      summary: directory.summary ?? "",
      injectMode: directory.injectMode,
      effectiveInjectMode: directory.effectiveInjectMode,
      aiMaintained: directory.aiMaintained,
      effectiveAiMaintained: directory.effectiveAiMaintained,
      lexicalSearch: directory.lexicalSearch ?? "inherit",
      vectorSearch: directory.vectorSearch ?? "inherit",
      inheritToChildren: directory.inheritToChildren !== false,
      allowCreateDocuments: directory.allowCreateDocuments !== false,
      allowCreateDirectories: directory.allowCreateDirectories !== false,
      allowMoveDocuments: directory.allowMoveDocuments !== false,
      allowMoveDirectories: directory.allowMoveDirectories !== false,
      maintenanceRules: directory.maintenanceRules,
      effectiveMaintenanceRules: directory.effectiveMaintenanceRules,
    }) !== JSON.stringify(draft.value)
  );
});

const pathLabel = computed(() =>
  props.directory
    ? props.directory.path
      ? `${props.directory.type}/${props.directory.path}`
      : props.directory.type
    : "",
);
const directoryTitle = computed(() => {
  const path = props.directory?.path.trim();
  if (!path) return t("knowledge.directoryConfig.title");
  const segments = path.split("/").filter(Boolean);
  return segments[segments.length - 1] ?? path;
});
const directoryWorkspaceDragEnabled = computed(() => !!props.directory?.path.trim());

function onDirectoryDragPointerDown(event: PointerEvent): void {
  const payload = props.directory
    ? buildKnowledgeFolderWorkspaceDragPayload(props.directory)
    : null;
  if (!payload) return;
  startKnowledgeInternalDrag(internalDrag, event, { payload });
}
const panelTabOptions = computed(() => {
  const options = [
    {
      value: "config",
      label: t("knowledge.directoryConfig.panel.config"),
    },
  ];
  if (showExternalImportTab.value) {
    options.push({
      value: "external",
      label: t("knowledge.directoryConfig.panel.external"),
    });
  }
  return options;
});

watch(() => props.saveLoading, (loading, wasLoading) => {
  if (!loading && wasLoading) {
    autoSaveInFlight.value = false;
    if (isDirty.value) {
      maybeScheduleAutoSave();
      return;
    }
    clearAutoSaveTimer();
  }
});

onUnmounted(() => {
  captureDirectorySession();
  clearAutoSaveTimer();
});

function clearAutoSaveTimer() {
  if (autoSaveTimer !== null) {
    clearTimeout(autoSaveTimer);
    autoSaveTimer = null;
  }
  autoSaveQueued.value = false;
}

function maybeScheduleAutoSave() {
  clearAutoSaveTimer();
  if (
    !props.directory
    || props.loading
    || props.saveLoading
    || interactionDisabled.value
    || !isDirty.value
  ) return;
  autoSaveQueued.value = true;
  autoSaveTimer = setTimeout(() => {
    autoSaveTimer = null;
    saveConfig("auto");
  }, AUTO_SAVE_DELAY_MS);
}

function saveConfig(mode: "auto" | "manual" = "manual") {
  if (!props.directory || props.saveLoading || interactionDisabled.value || !isDirty.value) return;
  clearAutoSaveTimer();
  autoSaveInFlight.value = mode === "auto";
  draft.value = materializedDirectoryDraft();
  const nextDraft = {
    ...draft.value,
    version: draft.value.version || 4,
  };
  submittedDirectoryKey = directoryKey(props.directory);
  submittedDraft = { ...nextDraft };
  captureDirectorySession(submittedDirectoryKey);
  emit("save", props.directory.path, nextDraft);
}

function handleKeydown(event: KeyboardEvent) {
  const target = event.target as HTMLElement | null;
  if (target?.closest(".base-markdown-editor")) return;
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") {
    event.preventDefault();
    saveConfig("manual");
  }
}

function handleClose() {
  if (isDirty.value) saveConfig("manual");
  emit("close");
}

function toggle<K extends keyof KnowledgeDirectoryConfig>(
  key: K,
  value: KnowledgeDirectoryConfig[K],
) {
  if (key === "summary" || key === "maintenanceRules") {
    markdownTextBuffers.delete(key);
  }
  draft.value = {
    ...draft.value,
    [key]: value,
  };
  maybeScheduleAutoSave();
}

function toggleExplicitMaintenanceRules(value: boolean) {
  if (draft.value.aiMaintained === "inherit" || (!value && draft.value.aiMaintained === true)) return;
  const currentRules = bufferedDirectoryMarkdownValue("maintenanceRules");
  markdownTextBuffers.delete("maintenanceRules");
  draft.value = {
    ...draft.value,
    maintenanceRules: value
      ? currentRules ||
        defaultMaintenanceRulesForType(props.directory?.type ?? "design") ||
        ""
      : null,
  };
  maybeScheduleAutoSave();
}

function onInjectModeChange(value: string) {
  if (value === "inherit_parent") {
    draft.value = {
      ...draft.value,
      injectMode: "inherit",
    };
    maybeScheduleAutoSave();
    return;
  }
  draft.value = {
    ...draft.value,
    injectMode: value as KnowledgeInjectMode,
    effectiveInjectMode: value as KnowledgeInjectMode,
  };
  maybeScheduleAutoSave();
}

function onAiConfigChange(value: string) {
  draft.value = materializedDirectoryDraft();
  markdownTextBuffers.clear();
  if (value === "inherit_parent") {
    draft.value = {
      ...draft.value,
      aiMaintained: "inherit",
    };
    maybeScheduleAutoSave();
    return;
  }

  if (value === "auto") {
    draft.value = {
      ...draft.value,
      aiMaintained: true,
      effectiveAiMaintained: true,
      maintenanceRules:
        draft.value.maintenanceRules?.trim()
          ? draft.value.maintenanceRules
          : defaultMaintenanceRulesForType(props.directory?.type ?? "design") || "",
    };
    maybeScheduleAutoSave();
    return;
  }

  draft.value = {
    ...draft.value,
    aiMaintained: false,
    effectiveAiMaintained: false,
  };
  maybeScheduleAutoSave();
}

function directoryMarkdownValue(
  config: KnowledgeDirectoryConfig,
  field: DirectoryMarkdownField,
): string {
  return field === "summary" ? config.summary : config.maintenanceRules ?? "";
}

function bufferedDirectoryMarkdownValue(field: DirectoryMarkdownField): string {
  return markdownTextBuffers.get(field)?.toString() ?? directoryMarkdownValue(draft.value, field);
}

function materializedDirectoryDraft(): KnowledgeDirectoryConfig {
  if (!markdownTextBuffers.size) return draft.value;
  return {
    ...draft.value,
    summary: bufferedDirectoryMarkdownValue("summary"),
    maintenanceRules: draft.value.maintenanceRules === null
      && !markdownTextBuffers.has("maintenanceRules")
      ? null
      : bufferedDirectoryMarkdownValue("maintenanceRules"),
  };
}

function refreshMarkdownDirtyFields(remote: KnowledgeDirectoryConfig): void {
  const next = new Set<DirectoryMarkdownField>();
  for (const field of ["summary", "maintenanceRules"] as const) {
    const buffered = markdownTextBuffers.get(field);
    const matchesRemote = buffered
      ? buffered.eq(
        markdownBaseTexts.get(field)
          ?? markdownEditorTextFromString(directoryMarkdownValue(remote, field)),
      )
      : directoryMarkdownValue(draft.value, field) === directoryMarkdownValue(remote, field);
    if (!matchesRemote) next.add(field);
  }
  markdownDirtyFields.value = next;
}

function setDirectoryMarkdownBase(remote: KnowledgeDirectoryConfig): void {
  for (const field of ["summary", "maintenanceRules"] as const) {
    markdownBaseTexts.set(
      field,
      markdownEditorTextFromString(directoryMarkdownValue(remote, field)),
    );
  }
}

function onMarkdownModelUpdate(field: DirectoryMarkdownField, value: string): void {
  markdownTextBuffers.delete(field);
  draft.value = {
    ...draft.value,
    [field]: value,
  };
  const remote = props.directory ? configFromDirectory(props.directory) : null;
  if (remote) refreshMarkdownDirtyFields(remote);
  maybeScheduleAutoSave();
}

function onMarkdownDocumentChange(
  field: DirectoryMarkdownField,
  change: MarkdownEditorDocumentChange,
): void {
  markdownTextBuffers.set(field, change.doc);
  const next = new Set(markdownDirtyFields.value);
  const baseText = markdownBaseTexts.get(field)
    ?? markdownEditorTextFromString(
      props.directory
        ? directoryMarkdownValue(configFromDirectory(props.directory), field)
        : directoryMarkdownValue(draft.value, field),
    );
  if (change.doc.eq(baseText)) next.delete(field);
  else next.add(field);
  markdownDirtyFields.value = next;
  maybeScheduleAutoSave();
}

function onFolderIndexRuleChange(
  key: "lexicalSearch" | "vectorSearch",
  value: string,
) {
  toggle(key, value as FolderIndexRuleSetting);
}

function hintForDirectoryInjectMode(mode: KnowledgeInjectMode): string {
  switch (mode) {
    case "none":
      return t("knowledge.directoryConfig.inject.noneHint");
    case "path":
      return t("knowledge.directoryConfig.inject.pathHint");
    case "excerpt":
      return t("knowledge.directoryConfig.inject.excerptHint");
    default:
      return "";
  }
}

function buildFolderIndexRuleOptions(kind: KnowledgeSearchTagKind) {
  return [
    {
      value: "inherit",
      label: t("knowledge.folder.ruleInherit"),
      hint: hintForFolderSearchRule(kind, "inherit"),
    },
    {
      value: "enabled",
      label: labelForFolderSearchRule(kind, true),
      hint: hintForFolderSearchRule(kind, "enabled"),
    },
    {
      value: "disabled",
      label: labelForFolderSearchRule(kind, false),
      hint: hintForFolderSearchRule(kind, "disabled"),
    },
  ];
}

function labelForFolderIndexRule(
  kind: KnowledgeSearchTagKind,
  value: FolderIndexRuleSetting,
): string {
  switch (value) {
    case "enabled":
      return labelForFolderSearchRule(kind, true);
    case "disabled":
      return labelForFolderSearchRule(kind, false);
    default:
      return t("knowledge.folder.ruleInherit");
  }
}

function dropdownLabelForFolderIndexRule(
  kind: KnowledgeSearchTagKind,
  value: FolderIndexRuleSetting,
  effectiveState: EffectiveCapabilityState,
): string {
  if (value !== "inherit") return labelForFolderIndexRule(kind, value);

  const effectiveLabel = effectiveCapabilityLabel(kind, effectiveState);
  return labelForInheritedValue(
    effectiveLabel,
    effectiveState.source === "parent"
      ? { kind: "parent_directory", path: null }
      : { kind: "type_default", path: null },
  );
}

function effectiveCapabilityLabel(
  kind: KnowledgeSearchTagKind,
  state: EffectiveCapabilityState,
): string {
  return labelForFolderSearchRule(kind, state.enabled);
}

</script>

<template>
  <div class="directory-preview" @keydown.capture="handleKeydown">
    <div class="directory-preview-header">
      <div
        class="directory-preview-header-main"
        :class="{ 'drag-enabled': directoryWorkspaceDragEnabled }"
        @pointerdown="onDirectoryDragPointerDown"
      >
        <span class="directory-preview-path">{{ pathLabel }}</span>
      </div>
      <div class="directory-preview-actions">
        <BaseSegmented
          v-if="panelTabOptions.length > 1"
          v-model="activePanelTab"
          class="directory-panel-segmented"
          size="sm"
          :options="panelTabOptions"
          :aria-label="t('knowledge.directoryConfig.title')"
        />
        <BaseSegmented
          v-if="directory && activePanelTab === 'config'"
          v-model="editorViewMode"
          class="directory-view-segmented"
          size="sm"
          :options="editorViewOptions"
          :aria-label="t('knowledge.editor.viewMode')"
        />
        <BaseButton type="button" @click="handleClose">
          {{ t("common.close") }}
        </BaseButton>
      </div>
    </div>

    <div v-if="loading && !directory" class="directory-preview-empty">
      {{ t("common.loading") }}
    </div>
    <div v-else-if="!directory" class="directory-preview-empty">
      {{ t("knowledge.empty.title") }}
    </div>
    <div v-else class="directory-preview-main">
      <div
        v-if="showExternalImportTab && activePanelTab === 'external'"
        class="directory-preview-scroll"
      >
        <section class="directory-card">
          <div class="directory-section-title">
            {{ t("knowledge.directoryConfig.panel.external") }}
          </div>
          <div class="directory-section-hint">
            {{ t("knowledge.referenceFolder.external.hint") }}
          </div>
          <ReferenceExternalImportPanel
            :workspace-ref="workspaceRef"
            mode="directory"
            :directory="directory"
            :fixed-target-path="directory.path"
            :path-exists="pathExists ?? null"
            :ensure-directory="ensureDirectory ?? null"
            :select-directory="selectDirectory ?? null"
            :refresh-knowledge="refreshKnowledge ?? null"
            :delete-feishu-import="deleteFeishuImport ?? null"
            :delete-unity-import="deleteUnityImport ?? null"
          />
        </section>
      </div>

      <div v-else class="directory-preview-scroll directory-config-scroll">
        <article class="directory-config-page">
          <header class="directory-config-heading">
            <h1 class="directory-config-title">{{ directoryTitle }}</h1>
          </header>

          <section
            class="directory-properties"
            :aria-label="t('knowledge.preview.properties')"
          >
            <div class="directory-properties-title">
              {{ t("knowledge.preview.properties") }}
            </div>

            <div class="directory-property-row">
              <span class="directory-property-label">
                {{ t("knowledge.directoryConfig.injectMode") }}
              </span>
              <BaseDropdown
                class="directory-property-dropdown"
                :model-value="injectModeValue"
                :selected-label="injectModeDropdownLabel"
                :options="injectModeOptions"
                teleport
                :disabled="interactionDisabled"
                :aria-label="t('knowledge.directoryConfig.injectMode')"
                @update:model-value="onInjectModeChange"
              />
            </div>

            <div class="directory-property-row">
              <span class="directory-property-label">
                {{ t("knowledge.directoryConfig.aiConfig") }}
              </span>
              <BaseDropdown
                class="directory-property-dropdown"
                :model-value="aiConfigValue"
                :selected-label="aiConfigDropdownLabel"
                :options="aiConfigOptions"
                teleport
                :disabled="interactionDisabled"
                :aria-label="t('knowledge.directoryConfig.aiConfig')"
                @update:model-value="onAiConfigChange"
              />
            </div>

            <div class="directory-property-row">
              <span class="directory-property-label">
                {{ t("knowledge.directoryConfig.lexicalSearch") }}
              </span>
              <BaseDropdown
                class="directory-property-dropdown"
                :model-value="draft.lexicalSearch"
                :selected-label="
                  dropdownLabelForFolderIndexRule(
                    'lexical',
                    draft.lexicalSearch,
                    effectiveLexicalSearch,
                  )
                "
                :options="lexicalRuleOptions"
                teleport
                :disabled="interactionDisabled"
                :aria-label="t('knowledge.directoryConfig.lexicalSearch')"
                @update:model-value="
                  onFolderIndexRuleChange('lexicalSearch', $event)
                "
              />
            </div>

            <div class="directory-property-row">
              <span class="directory-property-label">
                {{ t("knowledge.directoryConfig.semanticSearch") }}
              </span>
              <BaseDropdown
                class="directory-property-dropdown"
                :model-value="draft.vectorSearch"
                :selected-label="
                  dropdownLabelForFolderIndexRule(
                    'semantic',
                    draft.vectorSearch,
                    effectiveVectorSearch,
                  )
                "
                :options="semanticRuleOptions"
                teleport
                :disabled="interactionDisabled"
                :aria-label="t('knowledge.directoryConfig.semanticSearch')"
                @update:model-value="
                  onFolderIndexRuleChange('vectorSearch', $event)
                "
              />
            </div>

            <div class="directory-property-row">
              <span class="directory-property-label">
                {{ t("knowledge.directoryConfig.explicitMaintenanceRules") }}
              </span>
              <BaseCheckbox
                :model-value="hasExplicitRulesContent"
                :disabled="
                  interactionDisabled ||
                  draft.aiMaintained === 'inherit' ||
                  draft.aiMaintained === true
                "
                :aria-label="t('knowledge.directoryConfig.explicitMaintenanceRules')"
                @update:model-value="toggleExplicitMaintenanceRules"
              />
            </div>

            <div class="directory-property-row">
              <span class="directory-property-label">
                {{ t("knowledge.directoryConfig.inheritToChildren") }}
              </span>
              <BaseCheckbox
                :model-value="draft.inheritToChildren"
                :disabled="interactionDisabled"
                :aria-label="t('knowledge.directoryConfig.inheritToChildren')"
                @update:model-value="toggle('inheritToChildren', $event)"
              />
            </div>

            <div class="directory-property-row">
              <span class="directory-property-label">
                {{ t("knowledge.directoryConfig.allowCreateDocuments") }}
              </span>
              <BaseCheckbox
                :model-value="draft.allowCreateDocuments"
                :disabled="interactionDisabled"
                :aria-label="t('knowledge.directoryConfig.allowCreateDocuments')"
                @update:model-value="toggle('allowCreateDocuments', $event)"
              />
            </div>

            <div class="directory-property-row">
              <span class="directory-property-label">
                {{ t("knowledge.directoryConfig.allowCreateDirectories") }}
              </span>
              <BaseCheckbox
                :model-value="draft.allowCreateDirectories"
                :disabled="interactionDisabled"
                :aria-label="t('knowledge.directoryConfig.allowCreateDirectories')"
                @update:model-value="toggle('allowCreateDirectories', $event)"
              />
            </div>

            <div class="directory-property-row">
              <span class="directory-property-label">
                {{ t("knowledge.directoryConfig.allowMoveDocuments") }}
              </span>
              <BaseCheckbox
                :model-value="draft.allowMoveDocuments"
                :disabled="interactionDisabled"
                :aria-label="t('knowledge.directoryConfig.allowMoveDocuments')"
                @update:model-value="toggle('allowMoveDocuments', $event)"
              />
            </div>

            <div class="directory-property-row">
              <span class="directory-property-label">
                {{ t("knowledge.directoryConfig.allowMoveDirectories") }}
              </span>
              <BaseCheckbox
                :model-value="draft.allowMoveDirectories"
                :disabled="interactionDisabled"
                :aria-label="t('knowledge.directoryConfig.allowMoveDirectories')"
                @update:model-value="toggle('allowMoveDirectories', $event)"
              />
            </div>
          </section>

          <section class="directory-inline-field directory-inline-summary">
            <div class="directory-inline-label">
              {{ t("knowledge.directoryConfig.summary") }}
            </div>
            <BaseMarkdownEditor
              :model-value="draft.summary"
              :active="active"
              :session-cache="markdownEditorSessions"
              :session-pinned="isDirty"
              :workspace-ref="workspaceRef"
              transaction-model
              :disabled="interactionDisabled"
              :view-mode="editorViewMode"
              :content-key="`${directoryContentKey}:summary`"
              auto-grow
              :min-height="64"
              :placeholder="t('knowledge.directoryConfig.summaryPlaceholder')"
              @update:model-value="onMarkdownModelUpdate('summary', $event)"
              @document-change="onMarkdownDocumentChange('summary', $event)"
              @shortcut-save="saveConfig('manual')"
            />
          </section>

          <div v-if="hasRulesWarning" class="directory-property-warning">
            {{ t("knowledge.directoryConfig.rulesRequiredHint") }}
          </div>

          <section
            v-if="draft.aiMaintained !== 'inherit' && hasExplicitRulesContent"
            class="directory-inline-field directory-inline-rules"
            :class="{ 'is-warning': hasRulesWarning }"
          >
            <div class="directory-inline-label">
              {{ t("knowledge.directoryConfig.maintenanceRules") }}
            </div>
            <BaseMarkdownEditor
              :model-value="draft.maintenanceRules ?? ''"
              :active="active"
              :session-cache="markdownEditorSessions"
              :session-pinned="isDirty"
              :workspace-ref="workspaceRef"
              transaction-model
              :disabled="rulesEditorDisabled || interactionDisabled"
              :view-mode="editorViewMode"
              :content-key="`${directoryContentKey}:maintenanceRules`"
              auto-grow
              :min-height="104"
              :placeholder="t('knowledge.directoryConfig.maintenanceRulesPlaceholder')"
              @update:model-value="onMarkdownModelUpdate('maintenanceRules', $event)"
              @document-change="onMarkdownDocumentChange('maintenanceRules', $event)"
              @shortcut-save="saveConfig('manual')"
            />
          </section>
        </article>
      </div>

      <div
        v-if="footerLabel"
        class="directory-footnote"
        :class="{ 'is-warning': isDirty || autoSaveQueued }"
      >
        {{ footerLabel }}
      </div>
    </div>
  </div>
</template>

<style scoped>
.directory-preview {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  background: var(--panel-bg);
}

.directory-preview-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 8px 16px;
  border-bottom: 1px solid var(--border-color);
  flex-shrink: 0;
}

.directory-preview-header-main {
  min-width: 0;
  flex: 1;
  display: flex;
  align-items: center;
}

.directory-preview-header-main.drag-enabled {
  cursor: grab;
  touch-action: none;
}

.directory-preview-header-main.drag-enabled:active {
  cursor: grabbing;
}

.directory-preview-path {
  min-width: 0;
  overflow: hidden;
  color: var(--text-secondary);
  font-size: 11px;
  font-family: var(--font-mono-identifier);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.directory-preview-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

.directory-view-segmented {
  flex-shrink: 0;
}

.directory-panel-segmented {
  flex-shrink: 0;
}

.directory-preview-empty {
  padding: 18px 16px;
  font-size: 12px;
  color: var(--text-secondary);
}

.directory-preview-main {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  position: relative;
  overflow: hidden;
  background: var(--panel-bg);
}

.directory-preview-scroll {
  flex: 1;
  min-height: 0;
  overflow: auto;
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 12px 16px 44px;
}

.directory-card {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 12px 14px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: color-mix(in srgb, var(--panel-bg) 78%, var(--bg-color));
}

.directory-section-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
}

.directory-section-hint {
  font-size: 11px;
  color: var(--text-secondary);
}

.directory-config-scroll {
  display: block;
  padding: 0;
}

.directory-config-page {
  position: relative;
  width: min(100%, 980px);
  min-height: 100%;
  margin: 0 auto;
  padding: 32px 44px 72px;
  box-sizing: border-box;
}

.directory-config-heading {
  margin: 0 0 22px;
}

.directory-config-title {
  margin: 0;
  color: var(--text-color);
  font-size: 24px;
  font-weight: 650;
  line-height: 1.3;
  letter-spacing: -0.015em;
}

.directory-properties {
  display: flex;
  flex-direction: column;
  gap: 1px;
  margin-bottom: 24px;
}

.directory-properties-title {
  margin-bottom: 7px;
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 600;
}

.directory-property-row {
  display: grid;
  grid-template-columns: 140px minmax(0, 1fr);
  align-items: center;
  gap: 12px;
  min-height: 30px;
  padding: 1px 0;
}

.directory-property-label {
  min-width: 0;
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.45;
}

.directory-property-dropdown {
  width: min(300px, 100%);
}

.directory-property-dropdown :deep(.base-dropdown-trigger) {
  min-height: 28px;
  padding-inline: 8px;
  border-color: transparent;
  background: transparent;
}

.directory-property-dropdown :deep(.base-dropdown-trigger:hover),
.directory-property-dropdown.open :deep(.base-dropdown-trigger) {
  border-color: var(--border-color);
  background: var(--hover-bg);
}

.directory-property-warning {
  margin: 8px 0 22px;
  padding: 8px 10px;
  border-left: 2px solid var(--status-warn-border);
  color: var(--status-warn-fg);
  background: color-mix(in srgb, var(--status-warn-bg) 48%, transparent);
  font-size: 11px;
  line-height: 1.55;
}

.directory-inline-field {
  margin: 0 0 22px;
  padding: 0;
}

.directory-inline-label {
  display: block;
  margin-bottom: 7px;
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 600;
}

.directory-inline-field :deep(.base-markdown-editor) {
  height: auto;
  border-left: 1px solid var(--border-color);
}

.directory-inline-summary :deep(.base-markdown-editor) {
  min-height: 64px;
}

.directory-inline-rules :deep(.base-markdown-editor) {
  min-height: 104px;
}

.directory-inline-field :deep(.base-markdown-editor .cm-editor) {
  background: transparent;
}

.directory-inline-field.is-warning .directory-inline-label {
  color: var(--status-warn-fg);
}

.directory-footnote {
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

.directory-footnote.is-warning {
  color: var(--status-warn-fg, var(--text-color));
  opacity: 0.74;
}

@media (max-width: 960px) {
  .directory-config-page {
    padding: 24px 24px 64px;
  }

  .directory-property-row {
    grid-template-columns: 120px minmax(0, 1fr);
  }
}
</style>
