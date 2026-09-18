<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import { t } from "../../i18n";
import type { AgentInfo } from "../../types";
import type { WorkspaceRef } from "../../services/project";
import { listAgents, listSubagentDefs, listWorkspaceAgents, listWorkspaceSubagentDefs } from "../../services/agent";
import BaseCheckbox from "../ui/BaseCheckbox.vue";
import BaseButton from "../ui/BaseButton.vue";

const props = defineProps<{
  modelValue?: string[];
  workspaceRef?: WorkspaceRef | null;
  disabled?: boolean;
}>();
const emit = defineEmits<{ "update:modelValue": [value: string[]] }>();
const agents = ref<AgentInfo[]>([]);
const loading = ref(false);
const failed = ref(false);
let requestId = 0;
const selected = computed(() => props.modelValue ?? ["unity"]);
// Preserve unavailable custom Agents when another selection is changed.
const options = computed(() => {
  const result = agents.value.map(({ id, name }) => ({ id, name }));
  for (const id of selected.value) {
    if (!result.some((agent) => agent.id === id)) result.push({ id, name: id });
  }
  return result;
});

async function loadAgents() {
  const current = ++requestId;
  agents.value = [];
  loading.value = true;
  failed.value = false;
  try {
    const scope = props.workspaceRef;
    const groups = await Promise.all(scope
      ? [listWorkspaceAgents(scope), listWorkspaceSubagentDefs(scope)]
      : [listAgents(), listSubagentDefs()]);
    if (current !== requestId) return;
    agents.value = [...new Map(groups.flat().map((agent) => [agent.id, agent])).values()];
  } catch {
    if (current === requestId) failed.value = true;
  } finally {
    if (current === requestId) loading.value = false;
  }
}

function toggle(id: string, enabled: boolean) {
  if (props.disabled || loading.value || failed.value) return;
  emit("update:modelValue", enabled
    ? [...new Set([...selected.value, id])]
    : selected.value.filter((agent) => agent !== id));
}

watch(() => JSON.stringify(props.workspaceRef ?? null), loadAgents, { immediate: true });
onUnmounted(() => { requestId += 1; });
</script>

<template>
  <div class="injection-agents" role="group" :aria-label="t('knowledge.meta.injectAgents')">
    <div class="injection-agents-title">{{ t("knowledge.meta.injectAgents") }}</div>
    <div v-if="loading" class="injection-agents-status">{{ t("common.loading") }}</div>
    <template v-else-if="failed">
      <div class="injection-agents-status">{{ t("knowledge.meta.injectAgentsLoadFailed") }}</div>
      <BaseButton @click="loadAgents">{{ t("knowledge.meta.injectAgentsRetry") }}</BaseButton>
    </template>
    <template v-else>
      <label v-for="agent in options" :key="agent.id" class="injection-agent-row" :title="agent.id">
        <BaseCheckbox
          :model-value="selected.includes(agent.id)"
          :disabled="disabled"
          :aria-label="agent.name"
          @update:model-value="toggle(agent.id, $event)"
        />
        <span>{{ agent.name }}</span>
      </label>
      <div v-if="!options.length" class="injection-agents-status">{{ t("knowledge.meta.injectAgentsEmpty") }}</div>
    </template>
  </div>
</template>

<style scoped>
.injection-agents-title {
  padding: 4px 4px 8px;
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 500;
}
.injection-agent-row {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 30px;
  padding: 2px 4px;
  border-radius: 4px;
  color: var(--text-color);
  font-size: 12px;
  cursor: pointer;
}
.injection-agent-row:hover { background: var(--hover-bg); }
.injection-agent-row span {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.injection-agent-row :deep(.base-checkbox) { flex-shrink: 0; }
.injection-agents-status {
  padding: 4px;
  color: var(--text-secondary);
  font-size: 11px;
  line-height: 1.5;
}
</style>
