<script setup lang="ts">
import { t } from "../../i18n";
import type { ModelOption, ModelDefaults, AgentInfo } from "../../types";
import { isProviderVisible, visibleProviderOrder } from "../../config/providerVisibility";

interface ModelGroup {
  provider: string;
  label: string;
  models: ModelOption[];
}

const props = defineProps<{
  modelDefaults: ModelDefaults;
  allModels: ModelOption[];
  agents: AgentInfo[];
  subagents: AgentInfo[];
  modelSaveMsg: string;
}>();

const emit = defineEmits<{
  "update:modelDefaults": [defaults: ModelDefaults];
  save: [];
}>();

function providerLabel(provider: string): string {
  const labels: Record<string, string> = {
    openrouter: "OpenRouter",
    anthropic: t("model.provider.anthropic"),
    claude_code: t("model.provider.claude_code"),
    openai_codex: t("model.provider.openai"),
    custom: t("model.provider.custom"),
  };
  return labels[provider] || provider;
}

function groupedAllModels(): ModelGroup[] {
  const map = new Map<string, ModelOption[]>();
  for (const m of props.allModels) {
    const list = map.get(m.provider) || [];
    list.push(m);
    map.set(m.provider, list);
  }
  const groups: ModelGroup[] = [];
  for (const provider of visibleProviderOrder) {
    const models = map.get(provider);
    if (models && models.length > 0) {
      groups.push({ provider, label: providerLabel(provider), models });
    }
  }
  return groups;
}

function updateMainModel(value: string) {
  emit("update:modelDefaults", { ...props.modelDefaults, mainModel: value });
  emit("save");
}

function updatePlanModel(value: string) {
  emit("update:modelDefaults", { ...props.modelDefaults, planModel: value });
  emit("save");
}

function updateSubagentModel(agentId: string, value: string) {
  const subagentModels = { ...props.modelDefaults.subagentModels, [agentId]: value };
  emit("update:modelDefaults", { ...props.modelDefaults, subagentModels });
  emit("save");
}

const claudeCodeVisible = isProviderVisible("claude_code");

function updateClaudeCodeEnabled(value: boolean) {
  emit("update:modelDefaults", { ...props.modelDefaults, claudeCodeEnabled: value });
  emit("save");
}
</script>

<template>
  <div class="settings-section">
    <div class="section-label">{{ t("settings.models.title") }}</div>
    <p class="section-desc">{{ t("settings.models.desc") }}</p>

    <div class="model-default-card">
      <div class="model-default-header">
        <span class="model-default-label">{{ t("settings.models.main") }}</span>
        <span class="model-default-hint">{{ t("settings.models.mainHint") }}</span>
      </div>
      <select :value="modelDefaults.mainModel" class="model-select" @change="updateMainModel(($event.target as HTMLSelectElement).value)">
        <option value="">{{ t("settings.models.mainDefault") }}</option>
        <optgroup v-for="group in groupedAllModels()" :key="group.provider" :label="group.label">
          <option v-for="m in group.models" :key="m.id" :value="m.id">{{ m.name }}</option>
        </optgroup>
      </select>
    </div>

    <div class="model-default-card">
      <div class="model-default-header">
        <span class="model-default-label">{{ t("settings.models.plan") }}</span>
        <span class="model-default-hint">{{ t("settings.models.planHint") }}</span>
      </div>
      <select :value="modelDefaults.planModel" class="model-select" @change="updatePlanModel(($event.target as HTMLSelectElement).value)">
        <option value="">{{ t("settings.models.planDefault") }}</option>
        <optgroup v-for="group in groupedAllModels()" :key="group.provider" :label="group.label">
          <option v-for="m in group.models" :key="m.id" :value="m.id">{{ m.name }}</option>
        </optgroup>
      </select>
    </div>

    <div class="model-default-card compact" v-if="claudeCodeVisible">
      <div class="model-default-row">
        <div class="model-default-agent">
          <span class="model-default-label">{{ t("settings.models.claudeCodeEnable") }}</span>
          <span class="model-default-hint">{{ t("settings.models.claudeCodeEnableHint") }}</span>
        </div>
        <input
          type="checkbox"
          :checked="modelDefaults.claudeCodeEnabled === true"
          @change="updateClaudeCodeEnabled(($event.target as HTMLInputElement).checked)"
        />
      </div>
    </div>

    <div class="section-label" style="margin-top: 8px;">{{ t("settings.models.subagent") }}</div>
    <p class="section-desc">{{ t("settings.models.subagentDesc") }}</p>

    <div
      v-for="agent in subagents"
      :key="agent.id"
      class="model-default-card compact"
    >
      <div class="model-default-row">
        <div class="model-default-agent">
          <span class="model-default-label">{{ agent.name }}</span>
          <span class="model-default-hint">{{ agent.description }}</span>
        </div>
        <select
          :value="modelDefaults.subagentModels[agent.id] || ''"
          class="model-select inline"
          @change="updateSubagentModel(agent.id, ($event.target as HTMLSelectElement).value)"
        >
          <option value="">{{ t("settings.models.subagentDefault") }}</option>
          <optgroup v-for="group in groupedAllModels()" :key="group.provider" :label="group.label">
            <option v-for="m in group.models" :key="m.id" :value="m.id">{{ m.name }}</option>
          </optgroup>
        </select>
      </div>
    </div>
  </div>
</template>
