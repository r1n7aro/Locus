<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { t } from "../../i18n";
import { normalizeAppError } from "../../services/errors";
import { useWorkspaceExplorerStore } from "../../stores/workspaceExplorer";
import type { ProjectExplorerMountEntry } from "../../types/workbench";
import LucideIcon from "../icons/LucideIcon.vue";
import {
  unityAssetIconClassForPath,
  unityAssetIconNodeForPath,
  unityFolderIconClass,
  unityFolderIconNode,
} from "../icons/unityAssetIcons";

const props = defineProps<{
  projectId: string;
  nodeId: string;
  path: string;
  title: string;
  relativePath?: string | null;
}>();

const emit = defineEmits<{
  (event: "activate", entry: ProjectExplorerMountEntry): void;
}>();

const explorerStore = useWorkspaceExplorerStore();
const loading = ref(false);
const error = ref("");
let requestEpoch = 0;

function normalizeRelativePath(path?: string | null): string {
  return (path ?? "").trim().replace(/\\/g, "/").replace(/^\/+|\/+$/g, "");
}

const listing = computed(() => explorerStore.mountListing(props.projectId, props.nodeId));
const entries = computed(() => {
  const parent = normalizeRelativePath(props.relativePath);
  return (listing.value?.entries ?? []).filter((entry) => {
    const segments = normalizeRelativePath(entry.relativePath).split("/").filter(Boolean);
    const entryParent = segments.slice(0, -1).join("/");
    return entryParent === parent;
  });
});

async function loadDirectory(): Promise<void> {
  const epoch = ++requestEpoch;
  loading.value = true;
  error.value = "";
  try {
    await explorerStore.loadMount(props.projectId, props.nodeId);
  } catch (cause) {
    if (epoch === requestEpoch) error.value = normalizeAppError(cause).message;
  } finally {
    if (epoch === requestEpoch) loading.value = false;
  }
}

watch(
  () => [props.projectId, props.nodeId, props.relativePath] as const,
  loadDirectory,
  { immediate: true },
);
</script>

<template>
  <section class="workspace-directory-preview">
    <header class="workspace-directory-preview-header">
      <span>{{ title }}</span>
      <span :title="path">{{ path }}</span>
    </header>
    <div v-if="loading && !listing" class="workspace-directory-preview-state">
      {{ t("development.preview.loading") }}
    </div>
    <div v-else-if="error && !listing" class="workspace-directory-preview-state error">
      {{ error }}
    </div>
    <div v-else-if="entries.length" class="workspace-directory-preview-list">
      <button
        v-for="entry in entries"
        :key="entry.relativePath"
        type="button"
        class="workspace-directory-preview-row"
        :title="entry.absolutePath"
        @click="emit('activate', entry)"
      >
        <span
          class="workspace-directory-preview-icon"
          :class="entry.isDir
            ? unityFolderIconClass(false)
            : unityAssetIconClassForPath(entry.absolutePath, { isFolder: false })"
          aria-hidden="true"
        >
          <LucideIcon
            :icon="entry.isDir
              ? unityFolderIconNode(false)
              : unityAssetIconNodeForPath(entry.absolutePath, { isFolder: false })"
            :size="14"
          />
        </span>
        <span>{{ entry.name }}</span>
        <span>{{ entry.relativePath }}</span>
      </button>
    </div>
    <div v-else class="workspace-directory-preview-state">
      {{ t("development.emptyFolder") }}
    </div>
    <div v-if="listing?.truncated" class="workspace-directory-preview-footer">
      {{ t("development.directory.truncated") }}
    </div>
  </section>
</template>

<style scoped>
.workspace-directory-preview {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  background: var(--panel-bg);
}

.workspace-directory-preview-header {
  min-height: 38px;
  padding: 0 12px;
  display: flex;
  align-items: center;
  gap: 10px;
  border-bottom: 1px solid var(--border-color);
}

.workspace-directory-preview-header > span:first-child {
  flex-shrink: 0;
  font-size: 12px;
  font-weight: 600;
}

.workspace-directory-preview-header > span:last-child {
  min-width: 0;
  overflow: hidden;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.workspace-directory-preview-state {
  margin: auto;
  color: var(--text-secondary);
  font-size: 12px;
}

.workspace-directory-preview-state.error {
  color: var(--status-error-fg, var(--text-color));
}

.workspace-directory-preview-list {
  flex: 1;
  min-height: 0;
  overflow: auto;
}

.workspace-directory-preview-row {
  width: 100%;
  min-height: 30px;
  padding: 4px 12px;
  display: grid;
  grid-template-columns: 18px minmax(120px, 0.45fr) minmax(160px, 1fr);
  align-items: center;
  gap: 8px;
  border: 0;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 78%, transparent);
  background: transparent;
  color: var(--text-color);
  font: inherit;
  text-align: left;
  cursor: pointer;
}

.workspace-directory-preview-row:hover,
.workspace-directory-preview-row:focus-visible {
  background: var(--hover-bg);
}

.workspace-directory-preview-row:focus-visible {
  outline: 1px solid var(--accent-color);
  outline-offset: -1px;
}

.workspace-directory-preview-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

.workspace-directory-preview-row > span:nth-child(2),
.workspace-directory-preview-row > span:last-child {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--font-mono-identifier);
  font-size: 12px;
}

.workspace-directory-preview-row > span:last-child {
  color: var(--text-secondary);
  font-size: 11px;
}

.workspace-directory-preview-footer {
  min-height: 28px;
  padding: 6px 12px;
  border-top: 1px solid var(--border-color);
  color: var(--text-secondary);
  font-size: 11px;
}
</style>
