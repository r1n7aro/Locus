<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { Package, Trash2, Upload } from "lucide";
import { t } from "../i18n";
import { normalizeAppError } from "../services/errors";
import { useResizablePanel } from "../composables/useResizablePanel";
import {
  pluginInstallFromPath,
  pluginListInstalled,
  pluginUninstall,
  type InstalledPluginSummary,
  type PluginInstallScope,
} from "../services/plugin";
import { hasTauriWindowRuntime } from "../services/tauriRuntime";
import { useNotificationStore } from "../stores/notification";
import BaseButton from "./ui/BaseButton.vue";
import BaseDropdown, { type DropdownOption } from "./ui/BaseDropdown.vue";
import LucideIcon from "./icons/LucideIcon.vue";

const props = defineProps<{
  workingDir: string;
}>();

const notificationStore = useNotificationStore();
const pluginLayoutRef = ref<HTMLElement | null>(null);
const installedPlugins = ref<InstalledPluginSummary[]>([]);
const loading = ref(false);
const selectedInstallScope = ref<PluginInstallScope>("app");
const installingScope = ref<PluginInstallScope | "">("");
const uninstallKey = ref("");
const uninstallConfirmKey = ref("");
const loadError = ref("");
let unlistenPluginsChanged: UnlistenFn | null = null;
let unlistenKnowledgeChanged: UnlistenFn | null = null;
let unlistenViewTreeChanged: UnlistenFn | null = null;

const {
  size: installedPaneWidth,
  isDragging: resizingInstalledPane,
  onMouseDown: onInstalledPaneResizeMouseDown,
} = useResizablePanel(pluginLayoutRef, {
  storageKey: "locus:plugins:installed-pane-width",
  defaultSize: 420,
  minSize: 280,
  maxSize: (container) => Math.max(320, Math.min(720, container.clientWidth * 0.7)),
});

const hasWorkspace = computed(() => !!props.workingDir.trim());
const installedSorted = computed(() =>
  [...installedPlugins.value].sort((left, right) =>
    left.scope.localeCompare(right.scope)
    || left.id.localeCompare(right.id),
  ),
);
const installScopeOptions = computed<DropdownOption[]>(() => [
  {
    value: "app",
    label: t("plugin.scope.app"),
  },
  {
    value: "project",
    label: t("plugin.scope.project"),
    hint: hasWorkspace.value ? "" : t("plugin.install.projectDisabled"),
    disabled: !hasWorkspace.value,
  },
]);
const installButtonDisabled = computed(() =>
  !!installingScope.value || (selectedInstallScope.value === "project" && !hasWorkspace.value),
);
const installButtonTitle = computed(() =>
  selectedInstallScope.value === "project" && !hasWorkspace.value
    ? t("plugin.install.projectDisabled")
    : t("plugin.install.action"),
);

function errorMessage(error: unknown): string {
  return normalizeAppError(error).message;
}

async function refreshAll() {
  loading.value = true;
  loadError.value = "";
  try {
    installedPlugins.value = await pluginListInstalled();
  } catch (error) {
    const message = errorMessage(error);
    loadError.value = message;
    notificationStore.addNotice("error", message, { operation: "pluginRefresh" });
  } finally {
    loading.value = false;
  }
}

function pluginScopeLabel(scope: PluginInstallScope | string): string {
  return scope === "project" ? t("plugin.scope.project") : t("plugin.scope.app");
}

function pluginDependencyCount(plugin: InstalledPluginSummary): number {
  return plugin.dependencies?.project?.length ?? 0;
}

function pluginDependencyLabel(plugin: InstalledPluginSummary): string {
  const count = pluginDependencyCount(plugin);
  if (count > 0) return t("plugin.dependency.count", count);
  if (plugin.compatibility?.projectIndependent === false) {
    return t("plugin.dependency.projectDependent");
  }
  return t("plugin.dependency.independent");
}

function setInstallScope(scope: string) {
  if (scope !== "app" && scope !== "project") return;
  selectedInstallScope.value = scope;
}

async function installPlugin() {
  const scope = selectedInstallScope.value;
  if (scope === "project" && !hasWorkspace.value) return;
  const selected = await open({
    multiple: false,
    directory: false,
    filters: [{ name: "Locus Plugin", extensions: ["zip"] }],
  });
  if (typeof selected !== "string" || !selected) return;
  installingScope.value = scope;
  try {
    const plugin = await pluginInstallFromPath(selected, scope);
    notificationStore.addNotice(
      "success",
      t("plugin.notice.installed", plugin.name || plugin.id),
      { operation: "pluginInstall" },
    );
    await refreshAll();
  } catch (error) {
    notificationStore.addNotice("error", errorMessage(error), { operation: "pluginInstall" });
  } finally {
    installingScope.value = "";
  }
}

async function uninstallPlugin(plugin: InstalledPluginSummary) {
  const key = `${plugin.scope}:${plugin.id}`;
  if (uninstallConfirmKey.value !== key) {
    uninstallConfirmKey.value = key;
    return;
  }
  uninstallKey.value = key;
  try {
    await pluginUninstall(plugin.id, plugin.scope);
    notificationStore.addNotice(
      "success",
      t("plugin.notice.uninstalled", plugin.name || plugin.id),
      { operation: "pluginUninstall" },
    );
    await refreshAll();
  } catch (error) {
    notificationStore.addNotice("error", errorMessage(error), { operation: "pluginUninstall" });
  } finally {
    uninstallKey.value = "";
    uninstallConfirmKey.value = "";
  }
}

function formatComponentSummary(plugin: InstalledPluginSummary): string {
  return [
    plugin.agents.length ? t("plugin.component.agents", plugin.agents.length) : "",
    plugin.skills.length ? t("plugin.component.skills", plugin.skills.length) : "",
    plugin.views.length ? t("plugin.component.views", plugin.views.length) : "",
  ].filter(Boolean).join(" / ") || t("common.none");
}

watch(() => props.workingDir, () => {
  void refreshAll();
});

watch(hasWorkspace, (available) => {
  if (!available && selectedInstallScope.value === "project") {
    selectedInstallScope.value = "app";
  }
});

onMounted(async () => {
  await refreshAll();
  if (!hasTauriWindowRuntime()) return;
  try {
    unlistenPluginsChanged = await listen("plugins-changed", () => {
      void refreshAll();
    });
    unlistenKnowledgeChanged = await listen("knowledge-changed", () => {
      void refreshAll();
    });
    unlistenViewTreeChanged = await listen("view-tree-changed", () => {
      void refreshAll();
    });
  } catch (error) {
    console.warn("Failed to listen for plugin view refresh events:", error);
  }
});

onUnmounted(() => {
  unlistenPluginsChanged?.();
  unlistenKnowledgeChanged?.();
  unlistenViewTreeChanged?.();
});
</script>

<template>
  <div
    ref="pluginLayoutRef"
    class="plugin-view"
    :class="{ 'is-resizing-installed-pane': resizingInstalledPane }"
  >
    <aside class="plugin-pane plugin-installed-pane" :style="{ width: `${installedPaneWidth}px` }">
      <header class="plugin-pane-header">
        <div class="plugin-pane-title">
          <LucideIcon :icon="Package" :size="15" />
          <span>{{ t("plugin.installed.title") }}</span>
        </div>
        <div class="plugin-header-actions">
          <BaseDropdown
            class="plugin-install-scope"
            :model-value="selectedInstallScope"
            :options="installScopeOptions"
            :aria-label="t('plugin.install.scope')"
            menu-align="end"
            @update:model-value="setInstallScope"
          />
          <BaseButton
            :disabled="installButtonDisabled"
            :title="installButtonTitle"
            @click="installPlugin"
          >
            <LucideIcon :icon="Upload" :size="13" />
            {{ t("plugin.install.action") }}
          </BaseButton>
        </div>
      </header>

      <div v-if="loadError" class="plugin-error">{{ loadError }}</div>
      <div v-else-if="loading && installedSorted.length === 0" class="plugin-empty">
        {{ t("common.loading") }}
      </div>
      <div v-else-if="installedSorted.length === 0" class="plugin-empty">
        {{ t("plugin.installed.empty") }}
      </div>
      <div v-else class="plugin-list">
        <section
          v-for="plugin in installedSorted"
          :key="`${plugin.scope}:${plugin.id}`"
          class="plugin-list-item"
        >
          <div class="plugin-list-main">
            <div class="plugin-list-title">
              <span class="plugin-name">{{ plugin.name || plugin.id }}</span>
              <span class="plugin-version">{{ plugin.version || "0.0.0" }}</span>
            </div>
            <div class="plugin-list-id">{{ plugin.id }}</div>
            <div class="plugin-list-meta">
              <span>{{ pluginScopeLabel(plugin.scope) }}</span>
              <span>{{ formatComponentSummary(plugin) }}</span>
              <span>{{ pluginDependencyLabel(plugin) }}</span>
            </div>
          </div>
          <BaseButton
            variant="danger"
            :disabled="uninstallKey === `${plugin.scope}:${plugin.id}`"
            @click="uninstallPlugin(plugin)"
          >
            <LucideIcon :icon="Trash2" :size="13" />
            {{ uninstallConfirmKey === `${plugin.scope}:${plugin.id}` ? t("common.confirm") : t("common.delete") }}
          </BaseButton>
        </section>
      </div>
    </aside>

    <div
      class="plugin-resize-handle"
      role="separator"
      aria-orientation="vertical"
      @mousedown="onInstalledPaneResizeMouseDown"
    />

    <main class="plugin-pane plugin-store-pane">
      <header class="plugin-pane-header">
        <div class="plugin-pane-title">
          <LucideIcon :icon="Package" :size="15" />
          <span>{{ t("plugin.store.title") }}</span>
        </div>
      </header>
      <div class="plugin-store-body">
        <div class="plugin-empty">{{ t("plugin.store.placeholder") }}</div>
      </div>
    </main>
  </div>
</template>

<style scoped>
.plugin-view {
  width: 100%;
  height: 100%;
  min-width: 0;
  min-height: 0;
  display: flex;
  background: var(--bg-color);
  color: var(--text-color);
  overflow: hidden;
}

.plugin-pane {
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  border-right: 1px solid var(--border-color);
  background: var(--sidebar-bg);
}

.plugin-installed-pane {
  flex: 0 0 auto;
  border-right: none;
  background: var(--panel-bg);
}

.plugin-store-pane {
  flex: 1 1 0;
  border-right: none;
  background: var(--panel-bg);
}

.plugin-view.is-resizing-installed-pane {
  cursor: col-resize;
}

.plugin-resize-handle {
  position: relative;
  width: 5px;
  flex: 0 0 5px;
  cursor: col-resize;
  background: color-mix(in srgb, var(--border-color) 70%, transparent);
}

.plugin-resize-handle::before {
  content: "";
  position: absolute;
  inset: 0 2px;
  background: transparent;
  transition: background 0.15s ease;
}

.plugin-resize-handle:hover::before,
.plugin-view.is-resizing-installed-pane .plugin-resize-handle::before {
  background: var(--accent-color);
}

.plugin-pane-header {
  min-height: 44px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border-color);
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  box-sizing: border-box;
}

.plugin-pane-title {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  min-width: 0;
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.plugin-header-actions {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
  min-width: 0;
}

.plugin-install-scope {
  width: 112px;
  flex-shrink: 0;
}

.plugin-list {
  min-height: 0;
  overflow: auto;
  padding: 8px;
}

.plugin-store-body {
  min-height: 0;
  overflow: auto;
}

.plugin-list-item {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 10px;
  border: 1px solid transparent;
  border-radius: 6px;
}

.plugin-list-item + .plugin-list-item {
  margin-top: 4px;
}

.plugin-list-item:hover {
  background: var(--hover-bg);
  border-color: var(--border-color);
}

.plugin-list-main {
  min-width: 0;
  flex: 1;
}

.plugin-list-title {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.plugin-name {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 13px;
  font-weight: 600;
}

.plugin-version,
.plugin-list-id,
.plugin-list-meta {
  font-size: 11px;
  color: var(--text-secondary);
}

.plugin-version {
  flex-shrink: 0;
}

.plugin-list-id {
  margin-top: 3px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--font-mono-identifier);
}

.plugin-list-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 6px;
}

.plugin-empty,
.plugin-error {
  padding: 16px 12px;
  font-size: 12px;
  color: var(--text-secondary);
}

.plugin-empty.compact {
  padding: 10px;
}

.plugin-error {
  color: var(--status-danger-fg);
}

@media (max-width: 980px) {
  .plugin-view {
    flex-direction: column;
  }

  .plugin-installed-pane {
    width: 100% !important;
    min-height: 240px;
    flex: 0 0 auto;
  }

  .plugin-store-pane {
    flex: 1 1 0;
    border-top: 1px solid var(--border-color);
  }

  .plugin-resize-handle {
    display: none;
  }

  .plugin-pane-header {
    flex-wrap: wrap;
  }

  .plugin-header-actions {
    width: 100%;
    justify-content: flex-end;
  }
}
</style>
