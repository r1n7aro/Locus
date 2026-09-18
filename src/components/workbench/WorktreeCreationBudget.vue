<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../i18n";
import type { WorktreeCreationPlan } from "../../services/worktrees";
import { formatWorktreeBytes } from "../../utils/worktreeDiskUsage";
import BaseButton from "../ui/BaseButton.vue";

const props = defineProps<{ plan: WorktreeCreationPlan; busy?: boolean }>();
const emit = defineEmits<{ confirm: []; cancel: [] }>();
const insufficientSpace = computed(() => props.plan.budget?.freeBytes != null
  && props.plan.budget.freeBytes < props.plan.budget.estimatedBytes);
</script>

<template>
  <section class="worktree-budget" :aria-label="t('worktrees.budget.title')">
    <strong>{{ t(plan.atCapacity ? 'worktrees.budget.atCapacity' : 'worktrees.budget.title') }}</strong>
    <p>{{ t(plan.atCapacity ? 'worktrees.budget.recycleFirst' : 'worktrees.budget.newProjectNeeded') }}</p>
    <template v-if="plan.budget">
      <dl>
        <div><dt>{{ t(plan.budget.cacheKnown ? 'worktrees.budget.estimate' : 'worktrees.budget.minimum') }}</dt><dd>{{ formatWorktreeBytes(plan.budget.estimatedBytes) }}</dd></div>
        <div v-if="plan.budget.freeBytes !== null"><dt>{{ t('worktrees.budget.free') }}</dt><dd>{{ formatWorktreeBytes(plan.budget.freeBytes) }}</dd></div>
      </dl>
      <p class="budget-note">{{ t(plan.budget.cacheKnown ? 'worktrees.budget.cacheIncluded' : 'worktrees.budget.cacheUnknown') }}</p>
      <span class="budget-path" :title="plan.budget.directory">{{ plan.budget.directory }}</span>
    </template>
    <p v-if="insufficientSpace" class="budget-error" role="alert">{{ t('worktrees.budget.insufficient') }}</p>
    <div class="budget-actions">
      <BaseButton v-if="!plan.atCapacity" :disabled="busy || insufficientSpace" @click="emit('confirm')">{{ t(busy ? 'worktrees.selector.creating' : 'worktrees.budget.confirm') }}</BaseButton>
      <BaseButton :disabled="busy" @click="emit('cancel')">{{ t('common.cancel') }}</BaseButton>
    </div>
  </section>
</template>

<style scoped>
.worktree-budget { display: flex; flex-direction: column; gap: 8px; padding: 10px 8px 6px; border-top: 1px solid var(--border-color); min-height: 0; overflow-y: auto; font-size: 12px; }
strong { font-size: 12px; color: var(--text-color); font-weight: 500; }
p { margin: 0; color: var(--text-secondary); line-height: 1.5; }
dl { margin: 0; display: flex; flex-direction: column; gap: 5px; }
dl > div { display: flex; justify-content: space-between; gap: 12px; }
dt { color: var(--text-secondary); }
dd { margin: 0; color: var(--text-color); font-variant-numeric: tabular-nums; }
.budget-path { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-family: var(--font-mono-identifier); font-size: 11px; color: var(--text-secondary); }
.budget-note { font-size: 11px; }
.budget-error { color: var(--status-danger-fg); }
.budget-actions { display: flex; gap: 6px; }
</style>
