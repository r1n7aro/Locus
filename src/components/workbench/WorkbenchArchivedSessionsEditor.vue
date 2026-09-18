<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { RefreshCw } from "lucide";
import { t } from "../../i18n";
import { normalizeAppError } from "../../services/errors";
import type { WorkspaceRef } from "../../services/project";
import { getArchivedCheckoutStorageBytes, listArchivedCheckoutSessions } from "../../services/session";
import { useNotificationStore } from "../../stores/notification";
import { useWorkspaceExplorerStore } from "../../stores/workspaceExplorer";
import type { SessionSummary } from "../../types";
import { formatWorktreeBytes } from "../../utils/worktreeDiskUsage";
import WorkspaceTree, { type WorkspaceTreeItem } from "../explorer/WorkspaceTree.vue";
import BaseButton from "../ui/BaseButton.vue";
import LucideIcon from "../icons/LucideIcon.vue";

const props = withDefaults(defineProps<{
  projectId: string;
  workspaceRef: WorkspaceRef | null;
  items: WorkspaceTreeItem[];
  active?: boolean;
  refreshKey?: number;
  toolbarTarget?: HTMLElement | null;
}>(), { active: true });
const emit = defineEmits<{
  sessionsChange: [sessions: SessionSummary[]];
  activate: [item: WorkspaceTreeItem, event: MouseEvent];
  contextmenu: [item: WorkspaceTreeItem, event: MouseEvent];
  dragPointerDown: [item: WorkspaceTreeItem, event: PointerEvent];
  sessionAction: [item: WorkspaceTreeItem];
  renameChange: [value: string];
  renameSubmit: [];
  renameCancel: [];
}>();
const notificationStore = useNotificationStore();
const explorerStore = useWorkspaceExplorerStore();
const count = ref(0);
const listLoading = ref(false);
const loadFailed = ref(false);
const storageBytes = ref<number | null>(null);
const storageLoading = ref(false);
const storageFailed = ref(false);
let listRequestId = 0;
let storageRequestId = 0;
let storageJob: Promise<void> = Promise.resolve();
const workspaceScopeKey = computed(() => JSON.stringify([props.projectId, props.workspaceRef]));
const activeSessionSignature = computed(() => (explorerStore.resources[props.projectId]?.sessions ?? [])
  .map((session) => session.id).join("|"));
const storageLabel = computed(() => storageLoading.value ? t("common.loading")
  : storageFailed.value ? "—" : storageBytes.value === null ? "—" : "≈ " + formatWorktreeBytes(storageBytes.value));
const visibleItems = computed<WorkspaceTreeItem[]>(() => props.items.length ? props.items : [{
  key: "archive-state", treeRow: { key: "archive-state", kind: "file", depth: 0, disabled: true,
    name: listLoading.value ? t("common.loading") : loadFailed.value
      ? t("development.archived.loadFailedShort") : t("development.archived.empty") },
}] );

function refreshStorage(workspaceRef: WorkspaceRef): Promise<void> {
  const requestId = ++storageRequestId;
  storageLoading.value = true;
  storageFailed.value = false;
  // Coalesce refreshes behind an in-flight scan; only the newest request runs.
  storageJob = storageJob.then(async () => {
    if (requestId !== storageRequestId) return;
    try {
      const bytes = await getArchivedCheckoutStorageBytes(workspaceRef);
      if (requestId === storageRequestId) storageBytes.value = bytes;
    } catch {
      if (requestId === storageRequestId) storageFailed.value = true;
    } finally {
      if (requestId === storageRequestId) storageLoading.value = false;
    }
  });
  return storageJob;
}

async function refreshArchived(): Promise<void> {
  const workspaceRef = props.workspaceRef;
  if (!workspaceRef || !props.active) return;
  const requestId = ++listRequestId;
  listLoading.value = true;
  // Storage is deliberately not awaited by the list or conversation path.
  void refreshStorage(workspaceRef);
  try {
    const sessions = await listArchivedCheckoutSessions(workspaceRef);
    if (requestId !== listRequestId) return;
    count.value = sessions.length;
    emit("sessionsChange", sessions);
    loadFailed.value = false;
  } catch (error) {
    if (requestId !== listRequestId) return;
    loadFailed.value = true;
    const normalized = normalizeAppError(error);
    notificationStore.addNotice("error", t("development.archived.loadFailed", normalized.message), {
      code: normalized.code, operation: "loadArchivedSessions",
    });
  } finally {
    if (requestId === listRequestId) listLoading.value = false;
  }
}

watch(workspaceScopeKey, () => {
  ++listRequestId;
  ++storageRequestId;
  count.value = 0;
  storageBytes.value = null;
  storageLoading.value = false;
  storageFailed.value = false;
  listLoading.value = false;
  loadFailed.value = false;
  emit("sessionsChange", []);
  void refreshArchived();
}, { immediate: true });
watch([() => props.active, () => props.refreshKey, activeSessionSignature], () => { void refreshArchived(); });
onUnmounted(() => { ++listRequestId; ++storageRequestId; });
</script>

<template>
  <Teleport v-if="toolbarTarget" :to="toolbarTarget">
    <span class="secondary-sidebar-count">{{ count }}</span>
    <span class="secondary-sidebar-count" role="status" :title="storageFailed ? t('development.archived.storageFailed') : t('development.archived.storageHint')">{{ storageLabel }}</span>
    <BaseButton class="secondary-sidebar-tool" :disabled="listLoading" :title="t('common.refresh')" :aria-label="t('common.refresh')" @click="refreshArchived">
      <LucideIcon :icon="RefreshCw" :size="13" />
    </BaseButton>
  </Teleport>
  <WorkspaceTree class="development-tree" :items="visibleItems" :row-height="30" :base-indent="12" :row-tab-index="0"
    @activate="(item, event) => emit('activate', item, event)"
    @contextmenu="(item, event) => item.treeRow?.session && emit('contextmenu', item, event)"
    @drag-pointer-down="(item, event) => emit('dragPointerDown', item, event)"
    @session-action="emit('sessionAction', $event)"
    @rename-change="emit('renameChange', $event)" @rename-submit="emit('renameSubmit')" @rename-cancel="emit('renameCancel')" />
</template>
