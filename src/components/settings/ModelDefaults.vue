<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../i18n";
import type { ModelOption, ModelDefaults, AgentInfo } from "../../types";
import { visibleProviderOrder, isProviderVisible } from "../../config/providerVisibility";
import { formatModelDisplayName } from "../../utils/modelDisplay";
import { groupModelsForSelector, modelListEntryName } from "../../utils/modelGrouping";
import BaseDropdown, { type DropdownOption } from "../ui/BaseDropdown.vue";

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

const providerLabels = computed<Record<string, string>>(() => ({
  openrouter: "OpenRouter",
  anthropic: t("model.provider.anthropic"),
  claude_code: t("model.provider.claude_code"),
  openai_codex: t("model.provider.openai"),
  custom: t("model.provider.custom"),
}));

function optionDisplayName(model: ModelOption): string {
  if (model.provider === "custom") return modelListEntryName(model);
  return formatModelDisplayName(model.name);
}

const modelOptions = computed<DropdownOption[]>(() =>
  groupModelsForSelector(props.allModels, visibleProviderOrder, providerLabels.value)
    .flatMap((group) => group.models.map((model) => ({
      value: model.id,
      label: optionDisplayName(model),
      group: group.label,
    }))),
);

function optionsWithDefault(defaultLabel: string): DropdownOption[] {
  return [{ value: "", label: defaultLabel }, ...modelOptions.value];
}

/** Keeps a stale model id readable instead of collapsing to a blank trigger. */
function selectedModelLabel(id: string): string {
  if (!id) return "";
  const model = props.allModels.find((item) => item.id === id);
  return model ? optionDisplayName(model) : id;
}

/** Every agent the task tool can spawn gets a model override slot: top-level
 *  agents (default first) plus the subagent-only definitions. */
const spawnableAgents = computed<AgentInfo[]>(() => [...props.agents, ...props.subagents]);

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
      <BaseDropdown
        class="model-default-dropdown"
        :model-value="modelDefaults.mainModel"
        :options="optionsWithDefault(t('settings.models.mainDefault'))"
        :selected-label="selectedModelLabel(modelDefaults.mainModel)"
        size="md"
        menu-align="start"
        teleport
        :aria-label="t('settings.models.main')"
        @update:model-value="updateMainModel"
      />
    </div>

    <div class="model-default-card">
      <div class="model-default-header">
        <span class="model-default-label">{{ t("settings.models.plan") }}</span>
        <span class="model-default-hint">{{ t("settings.models.planHint") }}</span>
      </div>
      <BaseDropdown
        class="model-default-dropdown"
        :model-value="modelDefaults.planModel"
        :options="optionsWithDefault(t('settings.models.planDefault'))"
        :selected-label="selectedModelLabel(modelDefaults.planModel)"
        size="md"
        menu-align="start"
        teleport
        :aria-label="t('settings.models.plan')"
        @update:model-value="updatePlanModel"
      />
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
      v-for="agent in spawnableAgents"
      :key="agent.id"
      class="model-default-card compact"
    >
      <div class="model-default-row">
        <div class="model-default-agent">
          <span class="model-default-label">{{ agent.name }}</span>
          <span class="model-default-hint">{{ agent.description }}</span>
        </div>
        <BaseDropdown
          class="model-default-dropdown inline"
          :model-value="modelDefaults.subagentModels[agent.id] || ''"
          :options="optionsWithDefault(t('settings.models.subagentDefault'))"
          :selected-label="selectedModelLabel(modelDefaults.subagentModels[agent.id] || '')"
          size="md"
          menu-align="end"
          teleport
          :aria-label="agent.name"
          @update:model-value="updateSubagentModel(agent.id, $event)"
        />
      </div>
    </div>
  </div>
</template>
