<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../i18n";
import type { ModelOption, ModelDefaults, AgentInfo, EffortLevel } from "../../types";
import { visibleProviderOrder } from "../../config/providerVisibility";
import { formatModelDisplayName, modelSupportsFastMode } from "../../utils/modelDisplay";
import { groupModelsForSelector, modelListEntryName } from "../../utils/modelGrouping";
import { isRemoteClaudeCode } from "../../services/claudeConnection";
import { makeDefaultSubagentModel, toggleSubagentModel, withSubagentModels } from "../../utils/subagentModels";
import BaseButton from "../ui/BaseButton.vue";
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
  claude_code: t(isRemoteClaudeCode.value ? "model.provider.claude_code.remote" : "model.provider.claude_code"),
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

/** Every agent the subagent tool can spawn gets a model override slot: top-level
 *  agents (default first) plus the subagent-only definitions. */
const spawnableAgents = computed<AgentInfo[]>(() => [...props.agents, ...props.subagents]);

const effortLevels: EffortLevel[] = ["none", "low", "medium", "high", "xhigh", "max"];
const effortLabels: Record<EffortLevel, string> = {
  none: "None",
  low: "Low",
  medium: "Medium",
  high: "High",
  xhigh: "XHigh",
  max: "Max",
};

/** Models the delegating agent may choose for this sub-agent, default first. */
function allowedModels(agentId: string): string[] {
  return props.modelDefaults.subagentModels[agentId] ?? [];
}

function knownAllowedModels(agentId: string): ModelOption[] {
  return allowedModels(agentId)
    .map((id) => props.allModels.find((model) => model.id === id))
    .filter((model): model is ModelOption => Boolean(model));
}

function subagentModelOptions(agentId: string): DropdownOption[] {
  const allowed = allowedModels(agentId);
  const options = optionsWithDefault(t("settings.models.subagentDefault")).map((option) =>
    allowed.length > 1 && option.value === allowed[0]
      ? { ...option, hint: t("settings.models.subagentModelDefaultHint") }
      : option);
  // Keep removed or unconfigured models visible so they can be deselected.
  const unavailable = allowed
    .filter((id) => !options.some((option) => option.value === id))
    .map((id) => ({ value: id, label: id, group: t("settings.models.subagentModelUnavailable") }));
  return [...options, ...unavailable];
}

function subagentModelSummary(agentId: string): string {
  const [first, ...others] = allowedModels(agentId);
  if (!first) return "";
  const label = selectedModelLabel(first);
  return others.length ? t("settings.models.subagentModelSummary", label, others.length) : label;
}

/** Effort and speed apply to whichever allowed model the agent picks, so only
 *  values every known allowed model supports are offered. */
function effortOptions(agentId: string): DropdownOption[] {
  const [first, ...others] = knownAllowedModels(agentId);
  const values = first
    ? (first.supportedEfforts ?? []).filter((level) =>
      others.every((model) => (model.supportedEfforts ?? []).includes(level)))
    : effortLevels;
  return [
    { value: "", label: t("settings.models.subagentEffortDefault") },
    ...values.map((value) => ({
      value,
      label: effortLabels[value],
      hint: t(`thinking.level.${value}`),
    })),
  ];
}

function selectedEffortLabel(agentId: string): string {
  const value = props.modelDefaults.subagentEfforts[agentId];
  return value ? effortLabels[value] : "";
}

function fastModeValue(agentId: string): string {
  const value = props.modelDefaults.subagentFastModes[agentId];
  if (value === true) return "fast";
  if (value === false) return "standard";
  return "";
}

function speedOptions(agentId: string): DropdownOption[] {
  const fastAvailable = knownAllowedModels(agentId).every(modelSupportsFastMode);
  return [
    { value: "", label: t("settings.models.subagentSpeedDefault") },
    { value: "standard", label: t("settings.models.subagentSpeedStandard") },
    { value: "fast", label: "Fast", disabled: !fastAvailable },
  ];
}

function updateMainModel(value: string) {
  emit("update:modelDefaults", { ...props.modelDefaults, mainModel: value });
  emit("save");
}

function updatePlanModel(value: string) {
  emit("update:modelDefaults", { ...props.modelDefaults, planModel: value });
  emit("save");
}

function saveSubagentModels(agentId: string, models: string[]) {
  const subagentModels = withSubagentModels(props.modelDefaults.subagentModels, agentId, models);
  emit("update:modelDefaults", { ...props.modelDefaults, subagentModels });
  emit("save");
}

/** The inherit option clears the list; any model toggles its membership. */
function updateSubagentModel(agentId: string, value: string) {
  saveSubagentModels(agentId, value ? toggleSubagentModel(allowedModels(agentId), value) : []);
}

function setDefaultSubagentModel(agentId: string, modelId: string) {
  saveSubagentModels(agentId, makeDefaultSubagentModel(allowedModels(agentId), modelId));
}

function updateSubagentEffort(agentId: string, value: string) {
  const subagentEfforts = { ...props.modelDefaults.subagentEfforts };
  if (value) subagentEfforts[agentId] = value as EffortLevel;
  else delete subagentEfforts[agentId];
  emit("update:modelDefaults", { ...props.modelDefaults, subagentEfforts });
  emit("save");
}

function updateSubagentSpeed(agentId: string, value: string) {
  const subagentFastModes = { ...props.modelDefaults.subagentFastModes };
  if (value === "fast") subagentFastModes[agentId] = true;
  else if (value === "standard") subagentFastModes[agentId] = false;
  else delete subagentFastModes[agentId];
  emit("update:modelDefaults", { ...props.modelDefaults, subagentFastModes });
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

    <div class="section-label" style="margin-top: 8px;">{{ t("settings.models.subagent") }}</div>
    <p class="section-desc">{{ t("settings.models.subagentDesc") }}</p>

    <div class="subagent-default-column-header" aria-hidden="true">
      <span>{{ t("settings.models.subagentModel") }}</span>
      <span>{{ t("settings.models.subagentEffort") }}</span>
      <span>{{ t("settings.models.subagentSpeed") }}</span>
    </div>

    <div
      v-for="agent in spawnableAgents"
      :key="agent.id"
      class="model-default-card compact"
    >
      <div class="model-default-row subagent-default-row">
        <div class="model-default-agent">
          <span class="model-default-label">{{ agent.name }}</span>
          <span class="model-default-hint">{{ agent.description }}</span>
        </div>
        <div class="subagent-default-controls">
          <BaseDropdown
            class="model-default-dropdown inline subagent-model-dropdown"
            :model-value="allowedModels(agent.id)[0] ?? ''"
            :selected-values="allowedModels(agent.id).length ? allowedModels(agent.id) : ['']"
            :options="subagentModelOptions(agent.id)"
            :selected-label="subagentModelSummary(agent.id)"
            :close-on-select="false"
            size="md"
            menu-align="end"
            teleport
            :aria-label="`${agent.name} ${t('settings.models.subagentModel')}`"
            @update:model-value="updateSubagentModel(agent.id, $event)"
          >
            <template v-if="allowedModels(agent.id).length > 1" #aside>
              <div class="subagent-model-order" role="group" :aria-label="t('settings.models.subagentModelOrder')">
                <div class="subagent-model-order-title">{{ t("settings.models.subagentModelOrder") }}</div>
                <div
                  v-for="(modelId, index) in allowedModels(agent.id)"
                  :key="modelId"
                  class="subagent-model-order-row"
                >
                  <span class="subagent-model-order-name" :title="modelId">{{ selectedModelLabel(modelId) }}</span>
                  <span v-if="index === 0" class="subagent-model-order-default">
                    {{ t("settings.models.subagentModelDefaultHint") }}
                  </span>
                  <BaseButton v-else @click="setDefaultSubagentModel(agent.id, modelId)">
                    {{ t("settings.models.subagentModelMakeDefault") }}
                  </BaseButton>
                </div>
              </div>
            </template>
          </BaseDropdown>
          <BaseDropdown
            class="subagent-effort-dropdown"
            :model-value="modelDefaults.subagentEfforts[agent.id] || ''"
            :options="effortOptions(agent.id)"
            :selected-label="selectedEffortLabel(agent.id)"
            size="md"
            menu-align="end"
            teleport
            :disabled="allowedModels(agent.id).length > 0 && effortOptions(agent.id).length === 1"
            :aria-label="`${agent.name} ${t('settings.models.subagentEffort')}`"
            @update:model-value="updateSubagentEffort(agent.id, $event)"
          />
          <BaseDropdown
            class="subagent-speed-dropdown"
            :model-value="fastModeValue(agent.id)"
            :options="speedOptions(agent.id)"
            size="md"
            menu-align="end"
            teleport
            :aria-label="`${agent.name} ${t('settings.models.subagentSpeed')}`"
            @update:model-value="updateSubagentSpeed(agent.id, $event)"
          />
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.subagent-default-column-header {
  display: grid;
  grid-template-columns: 220px 128px 112px;
  justify-content: end;
  gap: 8px;
  padding: 0 14px 5px;
  color: var(--text-secondary);
  font-size: 11px;
}

.subagent-default-controls {
  display: grid;
  grid-template-columns: 220px 128px 112px;
  flex-shrink: 0;
  gap: 8px;
}

.subagent-default-controls .model-default-dropdown.inline,
.subagent-effort-dropdown,
.subagent-speed-dropdown {
  width: 100%;
}

.subagent-model-order-title {
  padding: 4px 4px 8px;
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 500;
}

.subagent-model-order-row {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 30px;
  padding: 2px 4px;
  font-size: 12px;
}

.subagent-model-order-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.subagent-model-order-default {
  flex-shrink: 0;
  color: var(--accent-color);
  font-size: 11px;
}

@media (max-width: 980px) {
  .subagent-default-column-header {
    display: none;
  }

  .subagent-default-row {
    align-items: stretch;
    flex-direction: column;
  }

  .subagent-default-controls {
    grid-template-columns: minmax(0, 1fr) 128px 112px;
  }
}
</style>
