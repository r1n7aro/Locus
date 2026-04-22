<script setup lang="ts">
import { ref, computed, watch } from "vue";
import type { UnmergedFileEntry, MergeOperation, GitFileChange } from "../../types";
import { gitMergeAction, gitMergeApply } from "../../services/git";
import { useHideMeta, partitionMetaPaths } from "../../composables/useHideMeta";
import { normalizeAppError } from "../../services/errors";
import { t } from "../../i18n";
import { conflictCodeLabel } from "./mergeUi";
import {
  buildConflictResolutionKey,
  prunePendingConflictResolutionKeys,
} from "./conflictResolutionState";
import BaseButton from "../ui/BaseButton.vue";

const { hideMeta } = useHideMeta();

const props = defineProps<{
  unmergedFiles: UnmergedFileEntry[];
  stagedFiles: GitFileChange[];
  operation: MergeOperation | null;
  currentBranch: string;
  hasUnresolvedFiles: boolean;
  selectedConflictPath?: string | null;
}>();

const emit = defineEmits<{
  (e: "selectConflictFile", file: UnmergedFileEntry): void;
  (e: "actionDone"): void;
  (e: "fileResolved", file: UnmergedFileEntry): void;
}>();

const actionLoading = ref(false);
const actionError = ref<string | null>(null);
const showAbortConfirm = ref(false);
const pendingResolutionKeys = ref<Set<string>>(new Set());

const unmergedMetaPartition = computed(() =>
  partitionMetaPaths(props.unmergedFiles),
);
const orphanMetaPaths = computed(() => unmergedMetaPartition.value.orphanMetaPaths);
const orphanMetaCount = computed(() => orphanMetaPaths.value.size);
const filteredUnmerged = computed(() =>
  hideMeta.value
    ? props.unmergedFiles.filter((f) => !unmergedMetaPartition.value.hideableMetaPaths.has(f.path))
    : props.unmergedFiles,
);
const hiddenMetaCount = computed(() => {
  if (!hideMeta.value) return 0;
  return unmergedMetaPartition.value.hideableMetaPaths.size;
});
const sidebarBusy = computed(() => actionLoading.value || pendingResolutionKeys.value.size > 0);
const remainingCount = computed(() => props.unmergedFiles.length);
const operationBadge = computed(() => {
  if (!props.operation) return "";
  switch (props.operation.kind) {
    case "merge": return "MERGE";
    case "cherryPick": return "CHERRY-PICK";
    case "rebase": return "REBASE";
    case "revert": return "REVERT";
    default: return "CONFLICT";
  }
});
const isMetaOnlyState = computed(() => filteredUnmerged.value.length === 0 && props.hasUnresolvedFiles);
const isAllResolvedState = computed(() => filteredUnmerged.value.length === 0 && !props.hasUnresolvedFiles);

watch(() => props.unmergedFiles, (files) => {
  pendingResolutionKeys.value = prunePendingConflictResolutionKeys(
    pendingResolutionKeys.value,
    files,
  );
}, { immediate: true });

function addPendingResolution(file: UnmergedFileEntry): string {
  const key = buildConflictResolutionKey(file);
  const next = new Set(pendingResolutionKeys.value);
  next.add(key);
  pendingResolutionKeys.value = next;
  return key;
}

function removePendingResolution(key: string) {
  if (!pendingResolutionKeys.value.has(key)) return;
  const next = new Set(pendingResolutionKeys.value);
  next.delete(key);
  pendingResolutionKeys.value = next;
}

function isFileResolving(file: UnmergedFileEntry): boolean {
  return pendingResolutionKeys.value.has(buildConflictResolutionKey(file));
}

async function doAction(action: "continue" | "skip" | "abort") {
  if (!props.operation) return;
  actionLoading.value = true;
  actionError.value = null;
  try {
    await gitMergeAction(action, props.operation.kind);
    emit("actionDone");
  } catch (e) {
    actionError.value = normalizeAppError(e).message;
  } finally {
    actionLoading.value = false;
  }
}

async function resolveFile(file: UnmergedFileEntry, stage: "left" | "right" | "delete") {
  const resolutionKey = addPendingResolution(file);
  actionError.value = null;
  try {
    await gitMergeApply(file.path, { takeStage: { stage } });
    emit("fileResolved", file);
  } catch (e) {
    removePendingResolution(resolutionKey);
    actionError.value = normalizeAppError(e).message;
  }
}

function requestAbort() {
  showAbortConfirm.value = true;
}

function confirmAbort() {
  showAbortConfirm.value = false;
  void doAction("abort");
}

function cancelAbort() {
  showAbortConfirm.value = false;
}

function fileName(path: string): string {
  const parts = path.split("/");
  return parts[parts.length - 1];
}

function fileDir(path: string): string {
  const parts = path.split("/");
  if (parts.length <= 1) return "";
  return parts.slice(0, -1).join("/") + "/";
}

function conflictBadgeClass(conflictCode: string): string {
  if (conflictCode === "UU" || conflictCode === "AA") return "conflict-both";
  if (conflictCode.includes("D")) return "conflict-delete";
  return "conflict-mixed";
}
</script>

<template>
  <div class="files-panel merge-conflict-active">
    <div class="files-top-header">
      <div class="files-change-count">
        <span class="change-number">{{ remainingCount }}</span>
        <span class="change-label">{{ t("merge.queue.filesNeedingResolution") }}</span>
        <span v-if="hiddenMetaCount > 0" class="files-change-hidden">({{ t("collab.hiddenMetaInline", hiddenMetaCount) }})</span>
        <span class="merge-badge">{{ operationBadge }}</span>
      </div>
      <button
        class="hide-meta-btn"
        :class="{ active: hideMeta }"
        @click="hideMeta = !hideMeta"
        :aria-pressed="hideMeta"
        :title="t('common.hideMeta')"
      >.meta</button>
    </div>
    <div v-if="orphanMetaCount > 0" class="files-section-warning files-top-warning">
      {{ t("collab.orphanMetaWarning", orphanMetaCount) }}
    </div>

    <div class="files-scroll">
      <div v-if="isMetaOnlyState || isAllResolvedState" class="merge-queue-empty" :class="{ resolved: isAllResolvedState }">
        <div class="merge-queue-empty-icon" aria-hidden="true">
          <svg v-if="isAllResolvedState" viewBox="0 0 16 16" width="18" height="18" fill="currentColor">
            <path d="M13.78 4.22a.75.75 0 0 1 0 1.06l-6 6a.75.75 0 0 1-1.06 0l-2.5-2.5a.75.75 0 1 1 1.06-1.06l1.97 1.97 5.47-5.47a.75.75 0 0 1 1.06 0z"/>
          </svg>
          <svg v-else viewBox="0 0 16 16" width="18" height="18" fill="currentColor">
            <path d="M6.457 1.047c.659-1.234 2.427-1.234 3.086 0l6.082 11.378A1.75 1.75 0 0 1 14.082 15H1.918a1.75 1.75 0 0 1-1.543-2.575zM8 5a.75.75 0 0 0-.75.75v2.5a.75.75 0 0 0 1.5 0v-2.5A.75.75 0 0 0 8 5zm1 6a1 1 0 1 0-2 0 1 1 0 0 0 2 0z"/>
          </svg>
        </div>
        <div class="merge-queue-empty-title">
          {{ isMetaOnlyState ? t("merge.queue.hiddenMetaTitle") : t("merge.queue.readyTitle") }}
        </div>
        <div class="merge-queue-empty-desc">
          {{ isMetaOnlyState ? t("merge.queue.hiddenMetaOnly") : t("merge.queue.allResolvedBanner") }}
        </div>
        <div v-if="isMetaOnlyState" class="merge-queue-empty-actions">
          <BaseButton class="merge-empty-action" @click="hideMeta = false">
            {{ t("merge.queue.showMetaFiles") }}
          </BaseButton>
        </div>
      </div>

      <div v-else class="file-list merge-file-list">
        <div
          v-for="f in filteredUnmerged"
          :key="f.path"
          class="merge-file-row"
          :class="{ selected: f.path === selectedConflictPath, busy: isFileResolving(f) }"
        >
          <button
            class="file-item merge-file-main"
            type="button"
            :disabled="sidebarBusy"
            :title="f.path"
            @click="emit('selectConflictFile', f)"
          >
            <span class="file-status-icon status-conflict">
              <svg viewBox="0 0 16 16" width="14" height="14" fill="currentColor">
                <path d="M8 1.5a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13zM0 8a8 8 0 1 1 16 0A8 8 0 0 1 0 8zm9-3a1 1 0 1 1-2 0 1 1 0 0 1 2 0zm-.25 3a.75.75 0 0 0-1.5 0v3.5a.75.75 0 0 0 1.5 0V8z"/>
              </svg>
            </span>
            <span class="merge-file-copy">
              <span class="file-name ui-select-text">{{ fileName(f.path) }}</span>
              <span v-if="orphanMetaPaths.has(f.path)" class="orphan-meta-badge" :title="t('collab.orphanMetaHint')">{{ t("collab.orphanMetaTag") }}</span>
              <span v-if="fileDir(f.path)" class="merge-file-dir-inline ui-select-text">{{ fileDir(f.path) }}</span>
              <span
                class="conflict-code-badge"
                :class="conflictBadgeClass(f.conflictCode)"
                :title="conflictCodeLabel(f.conflictCode, f.semanticLabel)"
              >
                {{ conflictCodeLabel(f.conflictCode, f.semanticLabel) }}
              </span>
              <span v-if="f.lfs" class="lfs-badge">LFS</span>
            </span>
          </button>

          <div class="file-row-actions">
            <BaseButton
              class="merge-row-btn"
              :disabled="sidebarBusy"
              @click.stop="resolveFile(f, 'left')"
            >{{ t("merge.queue.keepCurrent") }}</BaseButton>
            <BaseButton
              class="merge-row-btn"
              :disabled="sidebarBusy"
              @click.stop="resolveFile(f, 'right')"
            >{{ t("merge.queue.keepStashed") }}</BaseButton>
            <BaseButton
              v-if="f.conflictCode.includes('D')"
              class="merge-row-btn"
              variant="danger"
              :disabled="sidebarBusy"
              @click.stop="resolveFile(f, 'delete')"
            >{{ t("merge.actions.delete") }}</BaseButton>
          </div>
        </div>
      </div>
    </div>

    <div v-if="actionError" class="merge-action-error">{{ actionError }}</div>

    <div v-if="operation" class="merge-actions">
      <BaseButton
        v-if="operation.canContinue"
        class="merge-footer-btn"
        variant="primary"
        size="md"
        :disabled="hasUnresolvedFiles || sidebarBusy"
        @click="doAction('continue')"
      >
        {{ actionLoading ? t("merge.actions.working") : t("merge.actions.continue") }}
      </BaseButton>
      <BaseButton
        v-if="operation.canSkip"
        class="merge-footer-btn"
        size="md"
        :disabled="sidebarBusy"
        @click="doAction('skip')"
      >{{ t("merge.actions.skip") }}</BaseButton>
      <BaseButton
        v-if="operation.canAbort"
        class="merge-footer-btn"
        variant="danger"
        size="md"
        :disabled="sidebarBusy"
        @click="requestAbort"
      >{{ t("merge.actions.abort") }}</BaseButton>
    </div>

    <Teleport to="body">
      <div v-if="showAbortConfirm" class="commit-modal-overlay" @click.self="cancelAbort">
        <div class="commit-modal" style="max-width: 380px">
          <div class="commit-modal-header">
            <span class="commit-modal-title">{{ t("merge.queue.abortTitle") }}</span>
            <button class="commit-modal-close" @click="cancelAbort">&times;</button>
          </div>
          <div class="commit-modal-body">
            <p class="merge-queue-confirm-text">{{ t("merge.queue.abortMessage") }}</p>
          </div>
          <div class="commit-modal-footer">
            <div class="commit-modal-actions">
              <BaseButton class="merge-queue-confirm-btn" size="md" @click="cancelAbort">
                {{ t("common.cancel") }}
              </BaseButton>
              <BaseButton class="merge-queue-confirm-btn" variant="danger" size="md" @click="confirmAbort">
                {{ t("merge.actions.abort") }}
              </BaseButton>
            </div>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>
