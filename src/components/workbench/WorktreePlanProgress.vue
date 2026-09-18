<script setup lang="ts">
import { onUnmounted, ref } from "vue";
import { t } from "../../i18n";
import type { WorktreePlanProgress } from "../../services/worktrees";
import { formatWorktreeBytes } from "../../utils/worktreeDiskUsage";
import BaseButton from "../ui/BaseButton.vue";

defineProps<{ progress: WorktreePlanProgress; cancellable?: boolean }>();
const emit = defineEmits<{ cancel: [] }>();
const started = Date.now();
const seconds = ref(0);
const timer = setInterval(() => { seconds.value = Math.floor((Date.now() - started) / 1000); }, 1000);
onUnmounted(() => clearInterval(timer));
</script>

<template>
  <div class="worktree-progress" role="status" aria-live="polite">
    <div class="progress-heading"><span>{{ t(`worktrees.progress.${progress.phase}`) }}</span><span>{{ t('worktrees.progress.elapsed', seconds) }}</span></div>
    <progress :value="progress.totalFiles ? progress.files : undefined" :max="progress.totalFiles || 1" :aria-label="t(`worktrees.progress.${progress.phase}`)" />
    <span v-if="progress.files > 0" class="progress-detail">{{ t('worktrees.progress.files', progress.files.toLocaleString(), formatWorktreeBytes(progress.bytes)) }}</span>
    <BaseButton v-if="cancellable" @click="emit('cancel')">{{ t('common.cancel') }}</BaseButton>
  </div>
</template>

<style scoped>
.worktree-progress { display: flex; flex-direction: column; gap: 6px; padding: 10px 8px; border-top: 1px solid var(--border-color); color: var(--text-secondary); font-size: 11px; }
.progress-heading { display: flex; justify-content: space-between; gap: 8px; font-variant-numeric: tabular-nums; }
progress { width: 100%; height: 3px; accent-color: var(--accent-color); }
.progress-detail { font-variant-numeric: tabular-nums; }
.worktree-progress > :deep(button) { align-self: flex-start; }
</style>
