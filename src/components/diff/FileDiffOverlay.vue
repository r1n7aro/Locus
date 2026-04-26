<script setup lang="ts">
import { ref, watch, onMounted, onUnmounted } from "vue";
import FileDiffViewer from "./FileDiffViewer.vue";
import { useDiffOverlay, type DiffOverlayHeaderAction } from "../../composables/useDiffOverlay";
import { useProjectStore } from "../../stores/project";
import { selectUnityAsset, openFileExternal } from "../../services/unity";
import { refetchDiffByKey } from "../../services/diff";
import { canOpenInEditor } from "../../composables/useHideMeta";
import { t } from "../../i18n";

const projectStore = useProjectStore();

const overlay = useDiffOverlay();

const confirmAction = ref<DiffOverlayHeaderAction | null>(null);

// Clear stale confirm dialog when overlay closes or reopens
watch(() => overlay.visible.value, (visible) => {
  if (!visible) confirmAction.value = null;
});

function onKeydown(e: KeyboardEvent) {
  if (e.key === "Escape" && overlay.visible.value) {
    if (confirmAction.value) {
      confirmAction.value = null;
    } else {
      overlay.close();
    }
  }
}

onMounted(() => window.addEventListener("keydown", onKeydown));
onUnmounted(() => window.removeEventListener("keydown", onKeydown));

function handleHeaderAction(action: DiffOverlayHeaderAction) {
  if (action.confirmMessage) {
    confirmAction.value = action;
  } else {
    action.onClick();
  }
}

function executeConfirmedAction() {
  if (confirmAction.value) {
    confirmAction.value.onClick();
    confirmAction.value = null;
  }
}

async function onLfsPulled() {
  const payload = overlay.payload.value;
  if (!payload) return;
  try {
    const updated = await refetchDiffByKey(payload.key);
    if (updated) overlay.payload.value = updated;
  } catch (e) {
    console.error("[FileDiffOverlay] refetch after LFS pull failed:", e);
  }
}

</script>

<template>
  <Teleport to="body">
    <Transition name="overlay-fade">
      <div v-if="overlay.visible.value" class="diff-overlay-backdrop" @click.self="overlay.close()">
        <div class="diff-overlay-panel">
          <!-- Header -->
          <div class="diff-overlay-header">
            <div class="diff-overlay-title">
              <span
                class="diff-overlay-status"
                :class="'status-' + (overlay.payload.value?.status ?? '').toLowerCase()"
              >
                {{ overlay.payload.value?.status ?? "" }}
              </span>
              <span v-if="overlay.payload.value?.oldPath" class="diff-overlay-path">
                {{ overlay.payload.value.oldPath }} → {{ overlay.payload.value.filePath }}
              </span>
              <span v-else class="diff-overlay-path">
                {{ overlay.payload.value?.filePath ?? "" }}
              </span>
              <span v-if="overlay.payload.value?.semantic?.scriptClassName" class="diff-overlay-type-hint">
                {{ overlay.payload.value.semantic.scriptClassName }}
              </span>
              <span v-if="overlay.payload.value?.stats" class="diff-overlay-stats">
                <span class="stat-add">+{{ overlay.payload.value.stats.additions }}</span>
                <span class="stat-del">-{{ overlay.payload.value.stats.deletions }}</span>
              </span>
            </div>
            <div class="diff-overlay-actions">
              <button
                v-if="projectStore.unityConnected && overlay.payload.value"
                class="diff-overlay-btn"
                @click="selectUnityAsset(overlay.payload.value!.filePath)"
              >
                {{ t('common.selectInUnity') }}
              </button>
              <button
                v-if="overlay.payload.value && !overlay.payload.value.isBinary && canOpenInEditor(overlay.payload.value.filePath)"
                class="diff-overlay-btn"
                @click="openFileExternal(overlay.payload.value!.filePath)"
              >
                {{ t('common.openInEditor') }}
              </button>
              <button
                v-for="action in overlay.headerActions.value"
                :key="action.label"
                class="diff-overlay-btn"
                :class="{ 'btn-danger': action.danger }"
                @click="handleHeaderAction(action)"
              >
                {{ action.label }}
              </button>
              <button class="diff-overlay-btn close-btn" @click="overlay.close()">&#x2715;</button>
            </div>
          </div>
          <!-- Content -->
          <div class="diff-overlay-body">
            <FileDiffViewer
              v-if="overlay.payload.value"
              :payload="overlay.payload.value"
              @lfs-pulled="onLfsPulled"
            />
          </div>
          <!-- Confirm dialog for header actions -->
          <Transition name="overlay-fade">
            <div v-if="confirmAction" class="confirm-backdrop" @click.self="confirmAction = null">
              <div class="confirm-dialog">
                <p class="confirm-message">{{ confirmAction.confirmMessage }}</p>
                <div class="confirm-buttons">
                  <button class="diff-overlay-btn" @click="confirmAction = null">{{ t('chat.undo.cancel') }}</button>
                  <button class="diff-overlay-btn btn-danger" @click="executeConfirmedAction">{{ t('chat.undo.confirm') }}</button>
                </div>
              </div>
            </div>
          </Transition>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.diff-overlay-backdrop {
  position: fixed;
  inset: 0;
  z-index: 200;
  background: rgba(0, 0, 0, 0.5);
  display: flex;
  align-items: center;
  justify-content: center;
}
.diff-overlay-panel {
  width: 90vw;
  max-width: 1200px;
  height: 80vh;
  background: var(--sidebar-bg);
  border-radius: 8px;
  border: 1px solid var(--border-color);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.diff-overlay-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 16px;
  border-bottom: 1px solid var(--border-color);
  flex-shrink: 0;
}
.diff-overlay-title {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}
.diff-overlay-status {
  font-size: 11px;
  font-weight: 600;
  padding: 1px 6px;
  border-radius: 3px;
  text-transform: uppercase;
}
.status-m { background: rgba(214, 158, 46, 0.2); color: #d69e2e; }
.status-a { background: rgba(56, 161, 105, 0.2); color: #38a169; }
.status-d { background: rgba(229, 62, 62, 0.2); color: #e53e3e; }
.status-r { background: rgba(128, 90, 213, 0.2); color: #805ad5; }
.diff-overlay-path {
  font-family: var(--font-mono-identifier);
  font-size: 13px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.diff-overlay-type-hint {
  font-size: 11px;
  color: var(--text-secondary);
  opacity: 0.7;
  font-style: italic;
  white-space: nowrap;
  flex-shrink: 0;
}
.diff-overlay-stats {
  font-size: 12px;
  display: flex;
  gap: 6px;
}
.stat-add { color: #38a169; }
.stat-del { color: #e53e3e; }
.diff-overlay-actions {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}
.diff-overlay-btn {
  background: none;
  border: 1px solid var(--border-color);
  border-radius: 4px;
  padding: 4px 10px;
  color: var(--text-secondary);
  cursor: pointer;
  font-size: 12px;
}
.diff-overlay-btn:hover {
  color: var(--text-color);
  border-color: var(--text-secondary);
}
.close-btn {
  font-size: 14px;
  padding: 4px 8px;
}
.diff-overlay-body {
  flex: 1;
  overflow: auto;
}

.btn-danger {
  color: #e53e3e !important;
  border-color: #e53e3e !important;
}
.btn-danger:hover {
  background: rgba(229, 62, 62, 0.1);
}
.confirm-backdrop {
  position: absolute;
  inset: 0;
  background: rgba(0, 0, 0, 0.4);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 10;
}
.confirm-dialog {
  background: var(--sidebar-bg);
  border: 1px solid var(--border-color);
  border-radius: 8px;
  padding: 20px 24px;
  max-width: 400px;
}
.confirm-message {
  margin: 0 0 16px;
  font-size: 13px;
  line-height: 1.5;
  color: var(--text-color, #e0e0e0);
}
.confirm-buttons {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

/* Transition */
.overlay-fade-enter-active,
.overlay-fade-leave-active {
  transition: opacity 0.15s ease;
}
.overlay-fade-enter-from,
.overlay-fade-leave-to {
  opacity: 0;
}
</style>
