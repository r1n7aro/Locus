<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { t } from "../../i18n";
import {
  getDefaultSkillPackageNamespace,
  knowledgeList,
  setDefaultSkillPackageNamespace,
} from "../../services/knowledge";
import { normalizeAppError } from "../../services/errors";
import { useProjectStore } from "../../stores/project";
import { useUiStore } from "../../stores/ui";
import { useNotificationStore } from "../../stores/notification";
import type {
  KnowledgeCatalogStats,
  KnowledgeDocumentSummary,
  KnowledgeDocumentType,
} from "../../types";
import { labelForInjectMode } from "../knowledge/knowledgeMetaLabels";
import WorkspaceRequiredState from "../WorkspaceRequiredState.vue";
import BaseButton from "../ui/BaseButton.vue";
import BaseSegmented from "../ui/BaseSegmented.vue";

const project = useProjectStore();
const uiStore = useUiStore();
const notificationStore = useNotificationStore();

const hasWorkspace = computed(() => !!project.workingDir.trim());
const loading = ref(false);
const error = ref("");
const documents = ref<KnowledgeDocumentSummary[]>([]);
const activeType = ref<KnowledgeDocumentType>("design");
const defaultSkillPackageNamespace = ref("");
const defaultSkillPackageNamespaceDraft = ref("");
const defaultSkillPackageNamespaceLoading = ref(false);
const defaultSkillPackageNamespaceSaving = ref(false);
const defaultSkillPackageNamespaceSaved = ref("");
const defaultSkillPackageNamespaceError = ref("");
let defaultSkillPackageNamespaceSaveTimer: ReturnType<typeof setTimeout> | null = null;

const skillPackageNamespaceChanged = computed(
  () => defaultSkillPackageNamespaceDraft.value.trim() !== defaultSkillPackageNamespace.value,
);

function emptyStats(): KnowledgeCatalogStats {
  return {
    total: 0,
    byType: {
      design: 0,
      memory: 0,
      skill: 0,
      reference: 0,
    },
    byStorageSource: {
      project: 0,
      app: 0,
    },
    commandEnabled: 0,
    aiMaintained: 0,
    fullInjectable: 0,
    summaryMissing: 0,
    external: 0,
  };
}

const stats = computed<KnowledgeCatalogStats>(() => {
  const next = emptyStats();
  for (const doc of documents.value) {
    next.total += 1;
    next.byType[doc.type] += 1;
    next.byStorageSource[doc.storageSource ?? "project"] += 1;
    if (doc.commandEnabled) next.commandEnabled += 1;
    if (doc.aiMaintained) next.aiMaintained += 1;
    if ((doc.type === "design" || doc.type === "memory") && doc.injectMode === "full") {
      next.fullInjectable += 1;
    }
    if (doc.summaryEnabled && !doc.hasSummary) next.summaryMissing += 1;
    if (doc.externalSource) next.external += 1;
  }
  return next;
});

const typeOptions = computed(() => [
  { value: "design", label: `${t("knowledge.type.design")} ${stats.value.byType.design}` },
  { value: "memory", label: `${t("knowledge.type.memory")} ${stats.value.byType.memory}` },
  { value: "skill", label: `${t("knowledge.type.skill")} ${stats.value.byType.skill}` },
  { value: "reference", label: `${t("knowledge.type.reference")} ${stats.value.byType.reference}` },
]);

const recentDocuments = computed(() =>
  documents.value
    .filter((doc) => doc.type === activeType.value)
    .sort((left, right) => {
      const updatedDelta = right.updatedAt - left.updatedAt;
      if (updatedDelta !== 0) return updatedDelta;
      return left.title.localeCompare(right.title);
    })
    .slice(0, 6),
);

async function loadDocuments() {
  if (!hasWorkspace.value) {
    documents.value = [];
    error.value = "";
    return;
  }
  loading.value = true;
  error.value = "";
  try {
    documents.value = await knowledgeList();
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : String(cause);
    documents.value = [];
  } finally {
    loading.value = false;
  }
}

async function loadDefaultSkillPackageNamespace() {
  defaultSkillPackageNamespaceLoading.value = true;
  defaultSkillPackageNamespaceError.value = "";
  try {
    const namespace = await getDefaultSkillPackageNamespace();
    defaultSkillPackageNamespace.value = namespace;
    defaultSkillPackageNamespaceDraft.value = namespace;
  } catch (cause) {
    const err = normalizeAppError(cause);
    defaultSkillPackageNamespaceError.value = err.message;
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "getDefaultSkillPackageNamespace",
    });
  } finally {
    defaultSkillPackageNamespaceLoading.value = false;
  }
}

async function saveDefaultSkillPackageNamespace() {
  if (defaultSkillPackageNamespaceSaving.value) return;
  defaultSkillPackageNamespaceSaving.value = true;
  defaultSkillPackageNamespaceError.value = "";
  try {
    const namespace = await setDefaultSkillPackageNamespace(
      defaultSkillPackageNamespaceDraft.value,
    );
    defaultSkillPackageNamespace.value = namespace;
    defaultSkillPackageNamespaceDraft.value = namespace;
    defaultSkillPackageNamespaceSaved.value = t("settings.knowledge.defaultSkillPackageNamespaceSaved");
    if (defaultSkillPackageNamespaceSaveTimer) {
      clearTimeout(defaultSkillPackageNamespaceSaveTimer);
    }
    defaultSkillPackageNamespaceSaveTimer = setTimeout(() => {
      defaultSkillPackageNamespaceSaved.value = "";
      defaultSkillPackageNamespaceSaveTimer = null;
    }, 2000);
  } catch (cause) {
    const err = normalizeAppError(cause);
    defaultSkillPackageNamespaceError.value = err.message;
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "setDefaultSkillPackageNamespace",
    });
  } finally {
    defaultSkillPackageNamespaceSaving.value = false;
  }
}

function openKnowledge(document?: KnowledgeDocumentSummary) {
  if (document) {
    uiStore.stageKnowledgeSelection({
      dashboard: document.type,
      path: `${document.type}/${document.path}`,
    });
  }
  uiStore.setTab("knowledge");
}

function labelForStoredScope(document: KnowledgeDocumentSummary): string {
  return document.storageSource === "app"
    ? t("knowledge.scope.user")
    : t("knowledge.scope.project");
}

function labelForInject(mode: KnowledgeDocumentSummary["injectMode"]): string {
  return labelForInjectMode(mode);
}

onMounted(() => {
  void loadDefaultSkillPackageNamespace();
  void loadDocuments();
});

onUnmounted(() => {
  if (defaultSkillPackageNamespaceSaveTimer) {
    clearTimeout(defaultSkillPackageNamespaceSaveTimer);
  }
});

watch(() => project.workingDir, () => {
  void loadDocuments();
});
</script>

<template>
  <div class="knowledge-settings">
    <div class="settings-header">
      <div class="settings-header-main">
        <div class="settings-title">{{ t("settings.tab.knowledge") }}</div>
        <div class="settings-subtitle">{{ t("settings.knowledge.unifiedDesc") }}</div>
      </div>
      <BaseButton v-if="hasWorkspace" type="button" @click="openKnowledge()">
        {{ t("settings.knowledge.openWorkspace") }}
      </BaseButton>
    </div>

    <div class="settings-section">
      <div class="section-label">{{ t("settings.knowledge.defaultSkillPackageNamespace") }}</div>
      <div class="package-namespace-row">
        <input
          v-model="defaultSkillPackageNamespaceDraft"
          class="package-namespace-input"
          :placeholder="t('settings.knowledge.defaultSkillPackageNamespacePlaceholder')"
          :disabled="defaultSkillPackageNamespaceLoading || defaultSkillPackageNamespaceSaving"
          @keydown.enter.prevent="saveDefaultSkillPackageNamespace"
        />
        <BaseButton
          type="button"
          :disabled="
            defaultSkillPackageNamespaceLoading ||
            defaultSkillPackageNamespaceSaving ||
            !skillPackageNamespaceChanged
          "
          @click="saveDefaultSkillPackageNamespace"
        >
          {{ t("common.save") }}
        </BaseButton>
      </div>
      <div class="section-hint">{{ t("settings.knowledge.defaultSkillPackageNamespaceHint") }}</div>
      <div v-if="defaultSkillPackageNamespaceSaved" class="settings-success">
        {{ defaultSkillPackageNamespaceSaved }}
      </div>
      <div v-if="defaultSkillPackageNamespaceError" class="settings-error">
        {{ defaultSkillPackageNamespaceError }}
      </div>
    </div>

    <template v-if="hasWorkspace">
      <div class="settings-section">
        <div class="section-label">{{ t("settings.knowledge.rootLabel") }}</div>
        <div class="root-path">Locus/knowledge/</div>
        <div class="section-hint">{{ t("settings.knowledge.rootHint") }}</div>
      </div>

      <div class="settings-grid">
        <section class="settings-card hero">
          <div class="card-label">{{ t("knowledge.overview.summary") }}</div>
          <div class="hero-value">
            <span class="hero-number">{{ stats.total }}</span>
            <span class="hero-unit">{{ t("knowledge.overview.documentsUnit") }}</span>
          </div>
          <div class="card-lines">
            <div class="card-line">
              <span>{{ t("settings.knowledge.commandDocs") }}</span>
              <span>{{ stats.commandEnabled }}</span>
            </div>
            <div class="card-line">
              <span>{{ t("settings.knowledge.aiDocs") }}</span>
              <span>{{ stats.aiMaintained }}</span>
            </div>
            <div class="card-line">
              <span>{{ t("settings.knowledge.fullDocs") }}</span>
              <span>{{ stats.fullInjectable }}</span>
            </div>
            <div class="card-line">
              <span>{{ t("settings.knowledge.externalDocs") }}</span>
              <span>{{ stats.external }}</span>
            </div>
            <div class="card-line">
              <span>{{ t("settings.knowledge.summaryMissing") }}</span>
              <span>{{ stats.summaryMissing }}</span>
            </div>
          </div>
        </section>

        <section class="settings-card">
          <div class="card-label">{{ t("settings.knowledge.typeBreakdown") }}</div>
          <div class="type-lines">
            <div class="type-line">
              <span>{{ t("knowledge.type.design") }}</span>
              <span>{{ stats.byType.design }}</span>
            </div>
            <div class="type-line">
              <span>{{ t("knowledge.type.memory") }}</span>
              <span>{{ stats.byType.memory }}</span>
            </div>
            <div class="type-line">
              <span>{{ t("knowledge.type.skill") }}</span>
              <span>{{ stats.byType.skill }}</span>
            </div>
            <div class="type-line">
              <span>{{ t("knowledge.type.reference") }}</span>
              <span>{{ stats.byType.reference }}</span>
            </div>
          </div>
          <div class="scope-caption">{{ t("settings.knowledge.scopeCaption") }}</div>
          <div class="type-lines compact">
            <div class="type-line">
              <span>{{ t("knowledge.scope.project") }}</span>
              <span>{{ stats.byStorageSource.project }}</span>
            </div>
            <div class="type-line">
              <span>{{ t("knowledge.scope.user") }}</span>
              <span>{{ stats.byStorageSource.app }}</span>
            </div>
          </div>
        </section>
      </div>

      <section class="settings-section recent-section">
        <div class="recent-header">
          <div class="section-label">{{ t("settings.knowledge.recentTitle") }}</div>
          <BaseSegmented
            size="sm"
            :model-value="activeType"
            :options="typeOptions"
            @update:model-value="activeType = $event as KnowledgeDocumentType"
          />
        </div>

        <div v-if="error" class="settings-error">{{ error }}</div>
        <div v-else-if="loading" class="settings-empty">{{ t("common.loading") }}</div>
        <div v-else-if="!recentDocuments.length" class="settings-empty">
          {{ t("settings.knowledge.emptyRecent") }}
        </div>
        <div v-else class="recent-list">
          <button
            v-for="document in recentDocuments"
            :key="document.id"
            type="button"
            class="recent-row"
            @click="openKnowledge(document)"
          >
            <span class="recent-main">
              <span class="recent-title">{{ document.title }}</span>
              <span class="recent-path">{{ `${document.type}/${document.path}` }}</span>
            </span>
            <span class="recent-meta">
              {{ labelForStoredScope(document) }} · {{ labelForInject(document.injectMode) }}
            </span>
          </button>
        </div>
      </section>
    </template>

    <WorkspaceRequiredState
      v-else
      :description="t('workspace.required.knowledgeDescription')"
    />
  </div>
</template>

<style scoped>
.knowledge-settings {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.settings-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

.settings-header-main {
  min-width: 0;
}

.settings-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-color);
}

.settings-subtitle {
  margin-top: 4px;
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-secondary);
  max-width: 760px;
}

.settings-section,
.settings-card {
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: color-mix(in srgb, var(--panel-bg) 76%, var(--bg-color));
}

.settings-section {
  padding: 12px 14px;
}

.settings-grid {
  display: grid;
  grid-template-columns: minmax(0, 1.2fr) minmax(0, 1fr);
  gap: 12px;
}

.settings-card {
  padding: 14px;
}

.section-label,
.card-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
}

.section-hint,
.scope-caption {
  margin-top: 6px;
  font-size: 11px;
  line-height: 1.5;
  color: var(--text-secondary);
}

.root-path {
  margin-top: 8px;
  font-size: 12px;
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
}

.package-namespace-row {
  display: flex;
  align-items: center;
  gap: 8px;
  max-width: 620px;
}

.package-namespace-input {
  flex: 1;
  min-width: 0;
  padding: 7px 10px;
  border-radius: 6px;
  border: 1px solid var(--border-color);
  background: var(--input-bg);
  color: var(--text-color);
  font-size: 13px;
  font-family: var(--font-mono-editor);
  outline: none;
  transition: border-color 0.15s ease, background 0.15s ease;
}

.package-namespace-input:focus {
  border-color: var(--accent-border);
  background: color-mix(in srgb, var(--input-bg) 88%, var(--accent-soft) 12%);
}

.package-namespace-input:disabled {
  opacity: 0.6;
}

.settings-success {
  margin-top: 8px;
  font-size: 11px;
  color: var(--status-good-fg);
}

.hero-value {
  display: flex;
  align-items: baseline;
  gap: 6px;
  margin-top: 10px;
}

.hero-number {
  font-size: 28px;
  line-height: 1;
  font-weight: 700;
  color: var(--text-color);
}

.hero-unit {
  font-size: 12px;
  color: var(--text-secondary);
}

.card-lines,
.type-lines {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-top: 12px;
}

.type-lines.compact {
  margin-top: 8px;
}

.card-line,
.type-line {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  font-size: 12px;
  color: var(--text-secondary);
}

.card-line span:last-child,
.type-line span:last-child {
  color: var(--text-color);
  font-weight: 600;
}

.recent-section {
  min-height: 240px;
}

.recent-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 12px;
}

.recent-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.recent-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  width: 100%;
  padding: 9px 10px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: transparent;
  color: var(--text-color);
  font: inherit;
  text-align: left;
  cursor: pointer;
}

.recent-row:hover {
  background: var(--hover-bg);
}

.recent-main {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.recent-title {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
}

.recent-path,
.recent-meta,
.settings-empty,
.settings-error {
  font-size: 11px;
  color: var(--text-secondary);
}

.recent-path {
  font-family: var(--font-mono-identifier);
}

.settings-error {
  color: var(--status-danger-fg);
}

@media (max-width: 900px) {
  .settings-grid {
    grid-template-columns: 1fr;
  }

  .settings-header,
  .recent-header,
  .recent-row {
    flex-direction: column;
    align-items: stretch;
  }

  .recent-meta {
    text-align: left;
  }
}
</style>
