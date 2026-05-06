<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import type { EffortLevel, ModelOption } from "../types";
import { t } from "../i18n";
import { visibleProviderOrder } from "../config/providerVisibility";

const props = defineProps<{
  models: ModelOption[];
  selectedId: string;
  effort: EffortLevel;
  efforts?: EffortLevel[];
  effortSupported?: boolean;
  align?: "start" | "end";
  disabled?: boolean;
}>();

const emit = defineEmits<{
  selectModel: [id: string];
  selectEffort: [level: EffortLevel];
}>();

interface LevelOption {
  value: EffortLevel;
  label: string;
  desc: string;
}

interface ProviderGroup {
  provider: string;
  label: string;
  models: ModelOption[];
}

const open = ref(false);
const selectorRef = ref<HTMLElement | null>(null);

const providerLabels = computed<Record<string, string>>(() => ({
  openrouter: "OpenRouter",
  anthropic: t("model.provider.anthropic"),
  anthropic_sdk: t("model.provider.anthropic_sdk"),
  openai_codex: t("model.provider.openai"),
  custom: t("model.provider.custom"),
}));

const providerShortLabels = computed<Record<string, string>>(() => ({
  openrouter: "OR",
  anthropic: t("model.provider.anthropic.short"),
  anthropic_sdk: t("model.provider.anthropic_sdk.short"),
  openai_codex: t("model.provider.openai.short"),
  custom: t("model.provider.custom"),
}));

const selectedModel = computed(() =>
  props.models.find((model) => model.id === props.selectedId) ?? null,
);

const selectedDisplayName = computed(() => {
  const selected = selectedModel.value;
  if (!selected) return "Model";
  const duplicated = props.models.some((model) => model.id !== selected.id && model.name === selected.name);
  if (!duplicated) return selected.name;
  const prefix = providerShortLabels.value[selected.provider] || selected.provider;
  return `${prefix} / ${selected.name}`;
});

const levels = computed<LevelOption[]>(() => {
  const defs: Record<EffortLevel, LevelOption> = {
    none: { value: "none", label: "None", desc: t("thinking.level.none") },
    low: { value: "low", label: "Low", desc: t("thinking.level.low") },
    medium: { value: "medium", label: "Med", desc: t("thinking.level.medium") },
    high: { value: "high", label: "High", desc: t("thinking.level.high") },
    xhigh: { value: "xhigh", label: "XHigh", desc: t("thinking.level.xhigh") },
    max: { value: "max", label: "Max", desc: t("thinking.level.max") },
  };
  const values: EffortLevel[] = props.efforts?.length
    ? props.efforts
    : ["none", "low", "medium", "high", "xhigh", "max"];
  return values.map((value) => defs[value]);
});

const currentLevel = computed(() =>
  levels.value.find((level) => level.value === props.effort) ?? levels.value[0],
);

const groupedModels = computed<ProviderGroup[]>(() => {
  const map = new Map<string, ModelOption[]>();
  for (const model of props.models) {
    const list = map.get(model.provider) || [];
    list.push(model);
    map.set(model.provider, list);
  }

  const groups: ProviderGroup[] = [];
  for (const provider of visibleProviderOrder) {
    const models = map.get(provider);
    if (models && models.length > 0) {
      groups.push({
        provider,
        label: providerLabels.value[provider] || provider,
        models,
      });
    }
  }
  return groups;
});

const triggerTitle = computed(() => {
  const modelTitle = selectedModel.value?.id || t("model.select");
  if (!props.effortSupported || !currentLevel.value) return modelTitle;
  return `${modelTitle} / ${currentLevel.value.desc}`;
});

function levelColor(level: EffortLevel) {
  switch (level) {
    case "low": return "var(--thinking-low, #38a169)";
    case "medium": return "var(--thinking-medium, #d69e2e)";
    case "high": return "var(--thinking-high, #dd6b20)";
    case "xhigh": return "var(--thinking-xhigh, #c05621)";
    case "max": return "var(--thinking-xhigh, #c05621)";
    default: return "var(--text-secondary)";
  }
}

function toggle() {
  if (props.disabled) return;
  open.value = !open.value;
}

function selectModel(id: string) {
  emit("selectModel", id);
  open.value = false;
}

function selectEffort(level: EffortLevel) {
  emit("selectEffort", level);
  open.value = false;
}

function onClickOutside(event: MouseEvent) {
  if (selectorRef.value && !selectorRef.value.contains(event.target as Node)) {
    open.value = false;
  }
}

onMounted(() => document.addEventListener("click", onClickOutside));
onUnmounted(() => document.removeEventListener("click", onClickOutside));
</script>

<template>
  <div class="model-effort-selector" ref="selectorRef">
    <button
      class="model-effort-trigger ui-select-none"
      :class="{ open, disabled }"
      type="button"
      :title="triggerTitle"
      @click="toggle"
    >
      <span class="model-effort-model">{{ selectedDisplayName }}</span>
      <span
        v-if="effortSupported && currentLevel"
        class="model-effort-level"
        :style="{ color: levelColor(effort) }"
      >
        {{ currentLevel.label }}
      </span>
      <span class="model-effort-chevron">&#9662;</span>
    </button>

    <Transition name="dropdown">
      <div
        v-if="open"
        class="model-effort-dropdown"
        :class="{ 'has-effort': effortSupported, 'align-start': align === 'start' }"
      >
        <div class="model-effort-model-panel">
          <template v-if="groupedModels.length === 0">
            <div class="model-effort-empty">{{ t("model.noProvider") }}</div>
          </template>
          <template v-for="(group, groupIndex) in groupedModels" :key="group.provider">
            <div v-if="groupIndex > 0" class="model-effort-divider"></div>
            <div class="model-effort-section-label">{{ group.label }}</div>
            <button
              v-for="model in group.models"
              :key="model.id"
              type="button"
              class="model-effort-option ui-select-none"
              :class="{ active: model.id === selectedId }"
              @click="selectModel(model.id)"
            >
              <span class="model-effort-option-name">{{ model.name }}</span>
            </button>
          </template>
        </div>

        <div v-if="effortSupported" class="model-effort-effort-panel">
          <div class="model-effort-section-label">{{ t("thinking.selector.title") }}</div>
          <button
            v-for="level in levels"
            :key="level.value"
            type="button"
            class="model-effort-option ui-select-none"
            :class="{ active: level.value === effort }"
            @click="selectEffort(level.value)"
          >
            <span class="model-effort-option-name">{{ level.label }}</span>
          </button>
        </div>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.model-effort-selector {
  position: relative;
  display: inline-flex;
  flex-shrink: 1;
  min-width: 0;
  margin-right: 4px;
}

.model-effort-trigger {
  display: flex;
  align-items: center;
  gap: 5px;
  min-width: 0;
  min-height: 28px;
  max-width: min(280px, 100%);
  padding: 4px 7px;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 12px;
  font-family: inherit;
  cursor: pointer;
  transition: color 0.15s ease, border-color 0.15s ease, background 0.15s ease;
  white-space: nowrap;
  box-shadow: none;
}

.model-effort-trigger:hover:not(.disabled) {
  color: var(--text-color);
  background: var(--hover-bg);
}

.model-effort-trigger.open {
  color: var(--text-color);
  background: var(--hover-bg);
}

.model-effort-trigger.disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.model-effort-model {
  flex: 1 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}

.model-effort-level {
  flex-shrink: 0;
  font-weight: 500;
}

.model-effort-chevron {
  flex-shrink: 0;
  font-size: 10px;
  transition: transform 0.15s ease;
}

.model-effort-trigger.open .model-effort-chevron {
  transform: rotate(180deg);
}

.model-effort-dropdown {
  position: absolute;
  right: 0;
  bottom: calc(100% + 6px);
  min-width: 260px;
  max-width: min(420px, calc(100vw - 24px));
  max-height: min(420px, calc(100vh - 160px));
  overflow: hidden;
  padding: 4px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: var(--bg-color);
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.12);
  z-index: 100;
  transform-origin: bottom right;
}

.model-effort-dropdown.align-start {
  left: 0;
  right: auto;
  transform-origin: bottom left;
}

.model-effort-dropdown.has-effort {
  width: min(420px, calc(100vw - 24px));
  display: grid;
  grid-template-columns: minmax(0, 1fr) 96px;
}

:root[data-theme="dark"] .model-effort-dropdown {
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.4);
}

.model-effort-model-panel,
.model-effort-effort-panel {
  min-width: 0;
  max-height: min(404px, calc(100vh - 176px));
  overflow-y: auto;
}

.model-effort-effort-panel {
  border-left: 1px solid var(--border-color);
  padding-left: 4px;
}

.model-effort-section-label {
  padding: 4px 12px 2px;
  font-size: 10px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--text-secondary);
  opacity: 0.7;
}

.model-effort-divider {
  height: 1px;
  margin: 4px 8px;
  background: var(--border-color);
}

.model-effort-empty {
  padding: 12px;
  font-size: 12px;
  color: var(--text-secondary);
  text-align: center;
}

.model-effort-option {
  width: 100%;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 12px;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: inherit;
  font-family: inherit;
  text-align: left;
  cursor: pointer;
  box-shadow: none;
  transition: background 0.12s ease;
}

.model-effort-option:hover {
  background: var(--hover-bg);
}

.model-effort-option.active {
  background: var(--active-bg);
}

.model-effort-option-name {
  flex: 1;
  min-width: 0;
  color: var(--text-color);
  font-size: 13px;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.dropdown-enter-active,
.dropdown-leave-active {
  transition: opacity 0.12s ease, transform 0.12s ease;
}

.dropdown-enter-from,
.dropdown-leave-to {
  opacity: 0;
  transform: translateY(4px);
}
</style>
