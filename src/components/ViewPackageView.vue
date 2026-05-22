<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { t } from "../i18n";
import { normalizeAppError } from "../services/errors";
import { getLocusRuntime, type RuntimeUnsubscribe } from "../services/locusRuntime";
import {
  viewList,
  viewRun,
  type ViewPackageSummary,
} from "../services/view";
import WorkspaceRequiredState from "./WorkspaceRequiredState.vue";
import LucideIcon from "./icons/LucideIcon.vue";
import { resolveLocusViewIcon } from "./icons/locusViewIcons";
import BaseButton from "./ui/BaseButton.vue";

const props = defineProps<{
  workingDir: string;
}>();

const views = ref<ViewPackageSummary[]>([]);
const selectedViewId = ref("");
const loading = ref(false);
const running = ref(false);
const error = ref("");
let unsubscribeViewReload: RuntimeUnsubscribe | null = null;

const hasWorkspace = computed(() => !!props.workingDir.trim());
const selectedView = computed(() =>
  views.value.find((view) => view.id === selectedViewId.value) ?? null,
);
const selectedViewPath = computed(() => selectedView.value?.packageRoot || "");
const selectedViewUpdatedAt = computed(() =>
  selectedView.value ? formatTimestamp(selectedView.value.updatedAt) : "",
);
const selectedViewCapabilityText = computed(() => {
  const caps = selectedView.value?.capabilities;
  if (!caps) return "";
  const enabled = [
    caps.unity ? "Unity" : "",
    caps.bindings ? "Bindings" : "",
    caps.writeBack ? "Write Back" : "",
  ].filter(Boolean);
  return enabled.length ? enabled.join(" / ") : t("view.metadata.capabilityNone");
});

function formatTimestamp(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return t("view.metadata.empty");
  return new Intl.DateTimeFormat(undefined, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(value));
}

async function loadViews() {
  if (!hasWorkspace.value) return;
  loading.value = true;
  error.value = "";
  try {
    views.value = await viewList();
    if (!views.value.some((view) => view.id === selectedViewId.value)) {
      selectedViewId.value = views.value[0]?.id ?? "";
    }
  } catch (loadError) {
    error.value = normalizeAppError(loadError).message;
  } finally {
    loading.value = false;
  }
}

async function openSelectedView() {
  if (!selectedViewId.value || running.value) return;
  running.value = true;
  error.value = "";
  try {
    await viewRun(selectedViewId.value);
  } catch (runError) {
    error.value = normalizeAppError(runError).message;
  } finally {
    running.value = false;
  }
}

watch(() => props.workingDir, () => {
  selectedViewId.value = "";
  if (hasWorkspace.value) void loadViews();
});

onMounted(async () => {
  unsubscribeViewReload = await getLocusRuntime().subscribe<ViewPackageSummary>(
    "view-package-reloaded",
    () => {
      void loadViews();
    },
  );
  if (hasWorkspace.value) await loadViews();
});

onUnmounted(() => {
  unsubscribeViewReload?.();
  unsubscribeViewReload = null;
});
</script>

<template>
  <div class="view-package-view">
    <WorkspaceRequiredState
      v-if="!hasWorkspace"
      :description="t('workspace.required.viewDescription')"
    />

    <template v-else>
      <div v-if="error" class="view-error" @click="error = ''">{{ error }}</div>

      <div class="view-layout">
        <aside class="view-sidebar">
          <div class="view-pane-header">
            <span>{{ t("view.list.title") }}</span>
          </div>

          <div class="view-list">
            <button
              v-for="view in views"
              :key="view.id"
              type="button"
              class="view-list-row"
              :class="{ active: selectedViewId === view.id }"
              @click="selectedViewId = view.id"
            >
              <span class="view-list-icon" aria-hidden="true">
                <LucideIcon :icon="resolveLocusViewIcon(view.icon)" :size="13" />
              </span>
              <span class="view-list-copy">
                <span class="view-list-name">{{ view.name }}</span>
                <span class="view-list-meta">{{ view.id }} · {{ view.template }}</span>
              </span>
            </button>
            <div v-if="!views.length && !loading" class="view-empty">
              {{ t("view.list.empty") }}
            </div>
            <div v-if="loading" class="view-empty">{{ t("common.loading") }}</div>
          </div>
        </aside>

        <section class="view-detail">
          <div class="view-detail-toolbar">
            <div class="view-detail-title">
              <span>{{ selectedView?.name || t("view.detail.emptyTitle") }}</span>
              <small v-if="selectedView">{{ selectedView.id }}</small>
            </div>
            <BaseButton :disabled="!selectedViewId || running" @click="openSelectedView">
              {{ running ? t("view.action.opening") : t("view.action.open") }}
            </BaseButton>
          </div>

          <div v-if="!selectedView" class="view-detail-state">{{ t("view.detail.empty") }}</div>
          <div v-else class="view-detail-body">
            <div class="view-section-header">{{ t("view.metadata.title") }}</div>
            <dl class="view-metadata-list">
              <div class="view-metadata-row">
                <dt>{{ t("view.metadata.name") }}</dt>
                <dd>{{ selectedView.name }}</dd>
              </div>
              <div class="view-metadata-row">
                <dt>{{ t("view.metadata.id") }}</dt>
                <dd class="mono">{{ selectedView.id }}</dd>
              </div>
              <div class="view-metadata-row">
                <dt>{{ t("view.metadata.template") }}</dt>
                <dd class="mono">{{ selectedView.template }}</dd>
              </div>
              <div class="view-metadata-row">
                <dt>{{ t("view.metadata.version") }}</dt>
                <dd class="mono">{{ selectedView.version }}</dd>
              </div>
              <div class="view-metadata-row">
                <dt>{{ t("view.metadata.capabilities") }}</dt>
                <dd>{{ selectedViewCapabilityText }}</dd>
              </div>
              <div class="view-metadata-row">
                <dt>{{ t("view.metadata.updatedAt") }}</dt>
                <dd>{{ selectedViewUpdatedAt }}</dd>
              </div>
              <div class="view-metadata-row">
                <dt>{{ t("view.metadata.location") }}</dt>
                <dd class="mono path">{{ selectedViewPath }}</dd>
              </div>
            </dl>
          </div>
        </section>
      </div>
    </template>
  </div>
</template>

<style scoped>
.view-package-view {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--bg-color);
}

.view-error {
  flex-shrink: 0;
  padding: 7px 12px;
  border-bottom: 1px solid var(--status-danger-border);
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
  font-size: 12px;
  cursor: pointer;
}

.view-layout {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  overflow: hidden;
}

.view-sidebar {
  width: 320px;
  min-width: 280px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  border-right: 1px solid var(--border-color);
  background: var(--sidebar-bg);
  overflow: hidden;
}

.view-pane-header {
  flex-shrink: 0;
  min-height: 38px;
  display: flex;
  align-items: center;
  justify-content: flex-start;
  gap: 8px;
  padding: 6px 10px 6px 12px;
  border-bottom: 1px solid var(--border-color);
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.view-list {
  flex: 1;
  min-height: 0;
  overflow: auto;
}

.view-list-row {
  width: 100%;
  min-height: 48px;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 7px 12px;
  border: none;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 62%, transparent);
  background: transparent;
  color: var(--text-color);
  font: inherit;
  text-align: left;
  cursor: pointer;
}

.view-list-icon {
  width: 16px;
  height: 16px;
  flex: 0 0 16px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: color-mix(in srgb, var(--accent-color) 64%, var(--text-secondary) 36%);
}

.view-list-copy {
  min-width: 0;
  flex: 1 1 auto;
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.view-list-row:hover {
  background: var(--hover-bg);
}

.view-list-row.active {
  background: var(--active-bg);
}

.view-list-name {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 13px;
  font-weight: 600;
}

.view-list-meta {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
}

.view-empty {
  padding: 12px;
  color: var(--text-secondary);
  font-size: 12px;
}

.view-detail {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--panel-bg);
}

.view-detail-toolbar {
  flex-shrink: 0;
  min-height: 46px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 7px 12px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 88%, var(--bg-color) 12%);
}

.view-detail-title {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.view-detail-title span {
  color: var(--text-color);
  font-size: 14px;
  font-weight: 650;
}

.view-detail-title small {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
}

.view-detail-state {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-secondary);
  font-size: 13px;
}

.view-detail-body {
  flex: 1;
  min-width: 0;
  min-height: 0;
  min-width: 0;
  display: flex;
  flex-direction: column;
  overflow: auto;
}

.view-section-header {
  flex-shrink: 0;
  min-height: 34px;
  display: flex;
  align-items: center;
  padding: 0 12px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--bg-color) 16%);
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 600;
}

.view-metadata-list {
  margin: 0;
  max-width: 780px;
  padding: 12px;
}

.view-metadata-row {
  min-height: 36px;
  display: grid;
  grid-template-columns: 150px minmax(0, 1fr);
  align-items: center;
  gap: 14px;
  padding: 8px 0;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 62%, transparent);
}

.view-metadata-row dt {
  color: var(--text-secondary);
  font-size: 12px;
}

.view-metadata-row dd {
  min-width: 0;
  margin: 0;
  color: var(--text-color);
  font-size: 13px;
}

.view-metadata-row dd.mono {
  font-family: var(--font-mono-identifier);
  font-size: 12px;
}

.view-metadata-row dd.path {
  overflow-wrap: anywhere;
}
</style>
