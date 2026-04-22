<script setup lang="ts">
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
import BaseSegmented from "../ui/BaseSegmented.vue";
import ReferenceExternalImportPanel from "./ReferenceExternalImportPanel.vue";
import {
  useMarkdownEditorViewMode,
  type MarkdownEditorViewMode,
} from "../ui/markdownEditorViewMode";

const props = defineProps<{
  directory: KnowledgeDirectoryConfigRecord | null;
  loading: boolean;
  saveLoading: boolean;
  pathExists?: ((path: string) => boolean) | null;
  ensureDirectory?: ((path: string) => Promise<boolean>) | null;
  selectDirectory?: ((path: string) => Promise<void>) | null;
  refreshKnowledge?: (() => Promise<void>) | null;
  deleteFeishuImport?: ((path: string) => Promise<void>) | null;
  deleteUnityImport?: ((path: string) => Promise<void>) | null;
}>();

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
  injectMode: "excerpt",
  inheritInjectMode: true,
  aiMaintained: false,
  inheritAiConfig: true,
  explicitMaintenanceRules: false,
  lexicalSearch: "inherit",
  vectorSearch: "inherit",
  inheritToChildren: true,
  allowCreateDocuments: true,
  allowCreateDirectories: true,
  allowMoveDocuments: true,
  allowMoveDirectories: true,
  maintenanceRules: "",
});
const autoSaveQueued = ref(false);
const autoSaveInFlight = ref(false);
let autoSaveTimer: ReturnType<typeof setTimeout> | null = null;

watch(
  () => props.directory,
  (directory) => {
    clearAutoSaveTimer();
    autoSaveInFlight.value = false;
    if (!directory) return;
    draft.value = {
      version: directory.version,
      summary: directory.summary ?? "",
      injectMode: directory.injectMode ?? "excerpt",
      inheritInjectMode: directory.inheritInjectMode ?? false,
      aiMaintained: !!directory.aiMaintained,
      inheritAiConfig: directory.inheritAiConfig ?? false,
      explicitMaintenanceRules: !!directory.explicitMaintenanceRules,
      lexicalSearch: directory.lexicalSearch ?? "inherit",
      vectorSearch: directory.vectorSearch ?? "inherit",
      inheritToChildren: directory.inheritToChildren !== false,
      allowCreateDocuments: directory.allowCreateDocuments !== false,
      allowCreateDirectories: directory.allowCreateDirectories !== false,
      allowMoveDocuments: directory.allowMoveDocuments !== false,
      allowMoveDirectories: directory.allowMoveDirectories !== false,
      maintenanceRules: directory.maintenanceRules ?? "",
    };
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
const interactionDisabled = computed(() => props.saveLoading || !!props.directory?.readOnly);
const editorViewOptions = computed(() => [
  { value: "rendered", label: t("knowledge.editor.view.rendered") },
  { value: "native", label: t("knowledge.editor.view.native") },
]);
const editorViewMode = computed<MarkdownEditorViewMode>({
  get: () => markdownEditorViewMode.value,
  set: (value) => setMarkdownEditorViewMode(value),
});

const hasRulesWarning = computed(
  () =>
    !draft.value.inheritAiConfig &&
    !!draft.value.aiMaintained &&
    (!draft.value.explicitMaintenanceRules ||
      !draft.value.maintenanceRules.trim()),
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
  draft.value.inheritInjectMode ? "inherit_parent" : (draft.value.injectMode ?? "excerpt")
));

const aiConfigValue = computed<DirectoryAiConfigMode>(() => {
  if (draft.value.inheritAiConfig) return "inherit_parent";
  return draft.value.aiMaintained ? "auto" : "manual";
});

const injectModeDropdownLabel = computed(() => {
  if (!props.directory) return "";
  const explicitLabel = labelForInjectMode(draft.value.injectMode ?? "excerpt");
  if (!draft.value.inheritInjectMode) return explicitLabel;
  if (props.directory.inheritInjectMode) {
    return labelForInheritedValue(
      labelForInjectMode(props.directory.injectMode ?? "excerpt"),
      props.directory.injectModeSource,
    );
  }
  return t("knowledge.meta.inheritParent");
});

const aiConfigDropdownLabel = computed(() => {
  if (!props.directory) return "";
  const explicitLabel = draft.value.aiMaintained
    ? t("knowledge.directoryConfig.aiConfig.auto")
    : t("knowledge.directoryConfig.aiConfig.manual");
  if (!draft.value.inheritAiConfig) return explicitLabel;
  if (props.directory.inheritAiConfig) {
    const effectiveLabel = props.directory.aiMaintained
      ? t("knowledge.directoryConfig.aiConfig.auto")
      : t("knowledge.directoryConfig.aiConfig.manual");
    return labelForInheritedValue(effectiveLabel, props.directory.aiConfigSource);
  }
  return t("knowledge.meta.inheritParent");
});

const rulesEditorDisabled = computed(() => props.saveLoading || draft.value.inheritAiConfig);

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
  return (
    JSON.stringify({
      version: directory.version,
      summary: directory.summary ?? "",
      injectMode: directory.injectMode ?? "excerpt",
      inheritInjectMode: directory.inheritInjectMode ?? false,
      aiMaintained: !!directory.aiMaintained,
      inheritAiConfig: directory.inheritAiConfig ?? false,
      explicitMaintenanceRules: !!directory.explicitMaintenanceRules,
      lexicalSearch: directory.lexicalSearch ?? "inherit",
      vectorSearch: directory.vectorSearch ?? "inherit",
      inheritToChildren: directory.inheritToChildren !== false,
      allowCreateDocuments: directory.allowCreateDocuments !== false,
      allowCreateDirectories: directory.allowCreateDirectories !== false,
      allowMoveDocuments: directory.allowMoveDocuments !== false,
      allowMoveDirectories: directory.allowMoveDirectories !== false,
      maintenanceRules: directory.maintenanceRules ?? "",
    }) !== JSON.stringify(draft.value)
  );
});

const pathLabel = computed(() =>
  props.directory ? `${props.directory.type}/${props.directory.path}` : "",
);
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
  if (!props.directory || props.loading || interactionDisabled.value || !isDirty.value) return;
  autoSaveQueued.value = true;
  autoSaveTimer = setTimeout(() => {
    autoSaveTimer = null;
    saveConfig("auto");
  }, AUTO_SAVE_DELAY_MS);
}

function saveConfig(mode: "auto" | "manual" = "manual") {
  if (!props.directory || interactionDisabled.value || !isDirty.value) return;
  clearAutoSaveTimer();
  autoSaveInFlight.value = mode === "auto";
  emit("save", props.directory.path, {
    ...draft.value,
    version: draft.value.version || 4,
  });
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
  draft.value = {
    ...draft.value,
    [key]: value,
  };
  maybeScheduleAutoSave();
}

function toggleExplicitMaintenanceRules(value: boolean) {
  if (draft.value.inheritAiConfig || (!value && draft.value.aiMaintained)) return;
  draft.value = {
    ...draft.value,
    explicitMaintenanceRules: value,
    maintenanceRules: value
      ? draft.value.maintenanceRules ||
        defaultMaintenanceRulesForType(props.directory?.type ?? "design") ||
        ""
      : "",
  };
  maybeScheduleAutoSave();
}

function onInjectModeChange(value: string) {
  if (value === "inherit_parent") {
    draft.value = {
      ...draft.value,
      inheritInjectMode: true,
    };
    maybeScheduleAutoSave();
    return;
  }
  draft.value = {
    ...draft.value,
    injectMode: value as KnowledgeInjectMode,
    inheritInjectMode: false,
  };
  maybeScheduleAutoSave();
}

function onAiConfigChange(value: string) {
  if (value === "inherit_parent") {
    draft.value = {
      ...draft.value,
      inheritAiConfig: true,
    };
    maybeScheduleAutoSave();
    return;
  }

  if (value === "auto") {
    draft.value = {
      ...draft.value,
      inheritAiConfig: false,
      aiMaintained: true,
      explicitMaintenanceRules: true,
      maintenanceRules:
        draft.value.maintenanceRules.trim()
          ? draft.value.maintenanceRules
          : defaultMaintenanceRulesForType(props.directory?.type ?? "design") || "",
    };
    maybeScheduleAutoSave();
    return;
  }

  draft.value = {
    ...draft.value,
    inheritAiConfig: false,
    aiMaintained: false,
  };
  maybeScheduleAutoSave();
}

function onFolderIndexRuleChange(
  key: "lexicalSearch" | "vectorSearch",
  value: string,
) {
  toggle(key, value as FolderIndexRuleSetting);
}

function onSummaryInput(event: Event) {
  const target = event.target as HTMLTextAreaElement | null;
  if (!target) return;
  draft.value = {
    ...draft.value,
    summary: target.value,
  };
  maybeScheduleAutoSave();
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

function effectiveCapabilitySourceLabel(state: EffectiveCapabilityState): string {
  switch (state.source) {
    case "self":
      return t("knowledge.folder.ruleExplicit");
    case "parent": {
      const sourceDir = state.sourceDir?.trim();
      return sourceDir
        ? `${t("knowledge.folder.ruleInherited")} / ${sourceDir}`
        : t("knowledge.folder.ruleInherited");
    }
    default:
      return t("knowledge.folder.ruleDefault");
  }
}

</script>

<template>
  <div class="directory-preview" @keydown.capture="handleKeydown">
    <div class="directory-preview-header">
      <div class="directory-preview-head">
        <div class="directory-preview-title">
          {{ t("knowledge.directoryConfig.title") }}
        </div>
        <div class="directory-preview-subtitle">{{ pathLabel }}</div>
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

    <div v-if="loading" class="directory-preview-empty">
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

      <div v-else class="directory-preview-scroll">
        <div class="directory-primary-grid">
          <section class="directory-card">
            <div class="directory-section-title">
              {{ t("knowledge.directoryConfig.injectMode") }}
            </div>
            <div class="directory-section-hint">
              {{ t("knowledge.directoryConfig.injectModeHint") }}
            </div>
            <div class="directory-inline-control">
              <BaseDropdown
                class="directory-dropdown"
                :model-value="injectModeValue"
                :selected-label="injectModeDropdownLabel"
                :options="injectModeOptions"
                :disabled="interactionDisabled"
                :aria-label="t('knowledge.directoryConfig.injectMode')"
                @update:model-value="onInjectModeChange"
              />
            </div>
          </section>

          <section class="directory-card">
            <div class="directory-section-title">
              {{ t("knowledge.directoryConfig.aiConfig") }}
            </div>
            <div class="directory-section-hint">
              {{ t("knowledge.directoryConfig.aiConfigHint") }}
            </div>
            <div class="directory-inline-control">
              <BaseDropdown
                class="directory-dropdown"
                :model-value="aiConfigValue"
                :selected-label="aiConfigDropdownLabel"
                :options="aiConfigOptions"
                :disabled="interactionDisabled"
                :aria-label="t('knowledge.directoryConfig.aiConfig')"
                @update:model-value="onAiConfigChange"
              />
            </div>
          </section>

          <section class="directory-card directory-card-plain directory-card-span">
            <div class="directory-search-grid">
              <div class="directory-search-rule">
                <div class="directory-rule-title">
                  {{ t("knowledge.directoryConfig.lexicalSearch") }}
                </div>
                <div class="directory-rule-hint">
                  {{ t("knowledge.directoryConfig.lexicalSearchHint") }}
                </div>
                <div class="directory-inline-control">
                  <BaseDropdown
                    class="directory-dropdown"
                    :model-value="draft.lexicalSearch"
                    :selected-label="
                      dropdownLabelForFolderIndexRule(
                        'lexical',
                        draft.lexicalSearch,
                        effectiveLexicalSearch,
                      )
                    "
                    :options="lexicalRuleOptions"
                    :disabled="interactionDisabled"
                    :aria-label="t('knowledge.directoryConfig.lexicalSearch')"
                    @update:model-value="
                      onFolderIndexRuleChange('lexicalSearch', $event)
                    "
                  />
                </div>
                <div class="directory-rule-status">
                  <span class="directory-rule-value">
                    {{ effectiveCapabilityLabel("lexical", effectiveLexicalSearch) }}
                  </span>
                  <span class="directory-rule-meta">
                    {{ effectiveCapabilitySourceLabel(effectiveLexicalSearch) }}
                  </span>
                </div>
              </div>

              <div class="directory-search-rule">
                <div class="directory-rule-title">
                  {{ t("knowledge.directoryConfig.semanticSearch") }}
                </div>
                <div class="directory-rule-hint">
                  {{ t("knowledge.directoryConfig.semanticSearchHint") }}
                </div>
                <div class="directory-inline-control">
                  <BaseDropdown
                    class="directory-dropdown"
                    :model-value="draft.vectorSearch"
                    :selected-label="
                      dropdownLabelForFolderIndexRule(
                        'semantic',
                        draft.vectorSearch,
                        effectiveVectorSearch,
                      )
                    "
                    :options="semanticRuleOptions"
                    :disabled="interactionDisabled"
                    :aria-label="t('knowledge.directoryConfig.semanticSearch')"
                    @update:model-value="
                      onFolderIndexRuleChange('vectorSearch', $event)
                    "
                  />
                </div>
                <div class="directory-rule-status">
                  <span class="directory-rule-value">
                    {{ effectiveCapabilityLabel("semantic", effectiveVectorSearch) }}
                  </span>
                  <span class="directory-rule-meta">
                    {{ effectiveCapabilitySourceLabel(effectiveVectorSearch) }}
                  </span>
                </div>
              </div>
            </div>
          </section>

          <section class="directory-card directory-card-span">
            <div class="directory-section-title">
              {{ t("knowledge.directoryConfig.capabilities") }}
            </div>
            <div class="directory-capability-grid">
              <label class="directory-option-row">
                <BaseCheckbox
                  :model-value="draft.explicitMaintenanceRules"
                  :disabled="interactionDisabled || draft.inheritAiConfig || draft.aiMaintained"
                  :aria-label="
                    t('knowledge.directoryConfig.explicitMaintenanceRules')
                  "
                  @update:model-value="toggleExplicitMaintenanceRules"
                />
                <span class="directory-option-text">
                  <span class="directory-option-title">{{
                    t("knowledge.directoryConfig.explicitMaintenanceRules")
                  }}</span>
                  <span class="directory-option-hint">{{
                    t("knowledge.directoryConfig.explicitMaintenanceRulesHint")
                  }}</span>
                </span>
              </label>

              <label class="directory-option-row">
                <BaseCheckbox
                  :model-value="draft.inheritToChildren"
                  :disabled="interactionDisabled"
                  :aria-label="t('knowledge.directoryConfig.inheritToChildren')"
                  @update:model-value="toggle('inheritToChildren', $event)"
                />
                <span class="directory-option-text">
                  <span class="directory-option-title">{{
                    t("knowledge.directoryConfig.inheritToChildren")
                  }}</span>
                  <span class="directory-option-hint">{{
                    t("knowledge.directoryConfig.inheritToChildrenHint")
                  }}</span>
                </span>
              </label>

              <label class="directory-option-row">
                <BaseCheckbox
                  :model-value="draft.allowCreateDocuments"
                  :disabled="interactionDisabled"
                  :aria-label="t('knowledge.directoryConfig.allowCreateDocuments')"
                  @update:model-value="toggle('allowCreateDocuments', $event)"
                />
                <span class="directory-option-text">
                  <span class="directory-option-title">{{
                    t("knowledge.directoryConfig.allowCreateDocuments")
                  }}</span>
                  <span class="directory-option-hint">{{
                    t("knowledge.directoryConfig.allowCreateDocumentsHint")
                  }}</span>
                </span>
              </label>

              <label class="directory-option-row">
                <BaseCheckbox
                  :model-value="draft.allowCreateDirectories"
                  :disabled="interactionDisabled"
                  :aria-label="
                    t('knowledge.directoryConfig.allowCreateDirectories')
                  "
                  @update:model-value="toggle('allowCreateDirectories', $event)"
                />
                <span class="directory-option-text">
                  <span class="directory-option-title">{{
                    t("knowledge.directoryConfig.allowCreateDirectories")
                  }}</span>
                  <span class="directory-option-hint">{{
                    t("knowledge.directoryConfig.allowCreateDirectoriesHint")
                  }}</span>
                </span>
              </label>

              <label class="directory-option-row">
                <BaseCheckbox
                  :model-value="draft.allowMoveDocuments"
                  :disabled="interactionDisabled"
                  :aria-label="t('knowledge.directoryConfig.allowMoveDocuments')"
                  @update:model-value="toggle('allowMoveDocuments', $event)"
                />
                <span class="directory-option-text">
                  <span class="directory-option-title">{{
                    t("knowledge.directoryConfig.allowMoveDocuments")
                  }}</span>
                  <span class="directory-option-hint">{{
                    t("knowledge.directoryConfig.allowMoveDocumentsHint")
                  }}</span>
                </span>
              </label>

              <label class="directory-option-row">
                <BaseCheckbox
                  :model-value="draft.allowMoveDirectories"
                  :disabled="interactionDisabled"
                  :aria-label="t('knowledge.directoryConfig.allowMoveDirectories')"
                  @update:model-value="toggle('allowMoveDirectories', $event)"
                />
                <span class="directory-option-text">
                  <span class="directory-option-title">{{
                    t("knowledge.directoryConfig.allowMoveDirectories")
                  }}</span>
                  <span class="directory-option-hint">{{
                    t("knowledge.directoryConfig.allowMoveDirectoriesHint")
                  }}</span>
                </span>
              </label>
            </div>
          </section>
        </div>

        <section class="directory-card">
          <div class="directory-section-title">
            {{ t("knowledge.directoryConfig.summary") }}
          </div>
          <textarea
            :value="draft.summary"
            class="directory-summary-input"
            :disabled="interactionDisabled"
            :placeholder="t('knowledge.directoryConfig.summaryPlaceholder')"
            @input="onSummaryInput"
          />
        </section>

        <div v-if="hasRulesWarning" class="directory-warning">
          {{ t("knowledge.directoryConfig.rulesRequiredHint") }}
        </div>

        <section
          v-if="draft.explicitMaintenanceRules"
          class="directory-card directory-card-rules"
        >
          <div class="directory-section-title">
            {{ t("knowledge.directoryConfig.maintenanceRules") }}
          </div>
          <div class="directory-section-hint">
            {{ t("knowledge.directoryConfig.maintenanceRulesHint") }}
          </div>
          <div class="directory-rules-editor">
            <BaseMarkdownEditor
              :model-value="draft.maintenanceRules"
              :disabled="rulesEditorDisabled || interactionDisabled"
              :view-mode="editorViewMode"
              :placeholder="
                t('knowledge.directoryConfig.maintenanceRulesPlaceholder')
              "
              @update:model-value="toggle('maintenanceRules', $event)"
              @shortcut-save="saveConfig('manual')"
            />
          </div>
        </section>
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
}

.directory-preview-head {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.directory-preview-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-color);
  line-height: 1.35;
}

.directory-preview-subtitle {
  font-size: 11px;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  opacity: 0.52;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
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
  background: color-mix(in srgb, var(--panel-bg) 90%, var(--bg-color) 10%);
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

.directory-primary-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
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

.directory-card-plain {
  border: none;
  background: transparent;
}

.directory-card-span {
  grid-column: 1 / -1;
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

.directory-summary-input {
  width: 100%;
  min-height: 92px;
  resize: vertical;
  box-sizing: border-box;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: color-mix(in srgb, var(--bg-color) 84%, var(--panel-bg) 16%);
  color: var(--text-color);
  padding: 10px 12px;
  font: inherit;
  line-height: 1.55;
}

.directory-summary-input:focus-visible {
  outline: none;
  border-color: color-mix(
    in srgb,
    var(--accent-color) 60%,
    var(--border-color) 40%
  );
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent-color) 14%, transparent);
}

.directory-inline-control {
  width: min(320px, 100%);
}

.directory-dropdown {
  width: 100%;
}

.directory-search-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
}

.directory-search-rule {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 10px 12px;
  border: 1px solid color-mix(in srgb, var(--border-color) 78%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--sidebar-bg) 22%, var(--panel-bg) 78%);
}

.directory-rule-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
  line-height: 1.4;
}

.directory-rule-hint {
  font-size: 11px;
  color: var(--text-secondary);
  line-height: 1.45;
}

.directory-rule-status {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  min-width: 0;
}

.directory-rule-value {
  font-size: 11px;
  color: var(--text-color);
  white-space: nowrap;
}

.directory-rule-meta {
  min-width: 0;
  font-size: 11px;
  color: var(--text-secondary);
  text-align: right;
  line-height: 1.45;
}

.directory-capability-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 8px 12px;
}

.directory-option-row {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  min-width: 0;
  padding: 8px 9px;
  border: 1px solid color-mix(in srgb, var(--border-color) 78%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--sidebar-bg) 34%, var(--panel-bg) 66%);
}

.directory-option-text {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.directory-option-title {
  font-size: 12px;
  color: var(--text-color);
  line-height: 1.4;
}

.directory-option-hint {
  font-size: 11px;
  color: var(--text-secondary);
  line-height: 1.45;
}

.directory-rules-editor {
  min-height: 280px;
  display: flex;
  overflow: hidden;
  border: 1px solid var(--border-color);
  border-radius: 8px;
}

.directory-rules-editor :deep(.base-markdown-editor) {
  flex: 1;
  min-height: 0;
  border: none;
}

.directory-rules-editor :deep(.base-markdown-editor .base-markdown-editor-textarea) {
  height: 100%;
  min-height: 100%;
  box-sizing: border-box;
}

.directory-warning {
  padding: 10px 12px;
  border: 1px solid var(--status-warn-border);
  border-radius: 8px;
  background: var(--status-warn-bg);
  color: var(--status-warn-fg);
  font-size: 11px;
  line-height: 1.5;
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

@media (max-width: 1120px) {
  .directory-search-grid,
  .directory-capability-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}

@media (max-width: 960px) {
  .directory-preview-header {
    flex-direction: column;
    align-items: stretch;
  }

  .directory-preview-actions {
    justify-content: flex-end;
  }

  .directory-primary-grid {
    grid-template-columns: minmax(0, 1fr);
  }

  .directory-search-grid,
  .directory-capability-grid {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
