
<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from "vue";
import type { ModelOption } from "../types";
import { t } from "../i18n";
import { visibleProviderOrder } from "../config/providerVisibility";

const props = defineProps<{
  models: ModelOption[];
  selectedId: string;
  disabled?: boolean;
}>();

const emit = defineEmits<{
  select: [id: string];
}>();

const open = ref(false);
const selectorRef = ref<HTMLElement | null>(null);

const selectedModel = () => props.models.find((m) => m.id === props.selectedId);

const selectedDisplayName = computed(() => {
  const sel = selectedModel();
  if (!sel) return "Model";
  const duplicated = props.models.some((m) => m.id !== sel.id && m.name === sel.name);
  if (!duplicated) return sel.name;
  const prefix = providerShortLabels.value[sel.provider] || sel.provider;
  return `${prefix} / ${sel.name}`;
});

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

interface ProviderGroup {
  provider: string;
  label: string;
  models: ModelOption[];
}

const groupedModels = computed<ProviderGroup[]>(() => {
  const map = new Map<string, ModelOption[]>();
  for (const m of props.models) {
    const list = map.get(m.provider) || [];
    list.push(m);
    map.set(m.provider, list);
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

function toggle() {
  if (props.disabled) return;
  open.value = !open.value;
}

function select(id: string) {
  emit("select", id);
  open.value = false;
}

function onClickOutside(e: MouseEvent) {
  if (selectorRef.value && !selectorRef.value.contains(e.target as Node)) {
    open.value = false;
  }
}

onMounted(() => document.addEventListener("click", onClickOutside));
onUnmounted(() => document.removeEventListener("click", onClickOutside));
</script>

<template>
  <div class="model-selector" ref="selectorRef">
    <button
      class="model-trigger"
      :class="{ open, disabled }"
      @click="toggle"
      :title="selectedModel()?.id || t('model.select')"
    >
      <span class="model-name">{{ selectedDisplayName }}</span>
      <span class="model-chevron">&#9662;</span>
    </button>

    <Transition name="dropdown">
      <div v-if="open" class="model-dropdown">
        <template v-if="groupedModels.length === 0">
          <div class="model-empty">{{ t("model.noProvider") }}</div>
        </template>
        <template v-for="(group, gi) in groupedModels" :key="group.provider">
          <div v-if="gi > 0" class="model-divider"></div>
          <div class="model-group-label">{{ group.label }}</div>
          <div
            v-for="model in group.models"
            :key="model.id"
            class="model-option"
            :class="{ active: model.id === selectedId }"
            @click="select(model.id)"
          >
            <div class="model-option-name">{{ model.name }}</div>
          </div>
        </template>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.model-selector {
  position: relative;
  display: inline-flex;
  flex-shrink: 0;
}

.model-trigger {
  display: flex;
  align-items: center;
  gap: 4px;
  min-height: 28px;
  padding: 4px 10px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 12px;
  font-family: inherit;
  cursor: pointer;
  transition: all 0.15s;
  white-space: nowrap;
  box-shadow: none;
}

.model-trigger:hover:not(.disabled) {
  color: var(--text-color);
  border-color: var(--text-secondary);
}

.model-trigger.open {
  color: var(--text-color);
  border-color: var(--accent-color);
}

.model-trigger.disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.model-chevron {
  font-size: 10px;
  transition: transform 0.15s;
}

.model-trigger.open .model-chevron {
  transform: rotate(180deg);
}

.model-dropdown {
  position: absolute;
  bottom: calc(100% + 6px);
  left: 0;
  right: auto;
  min-width: 220px;
  max-width: min(280px, calc(100vw - 24px));
  background: var(--bg-color);
  border: 1px solid var(--border-color);
  border-radius: 10px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.12);
  padding: 4px;
  z-index: 100;
  transform-origin: bottom left;
}

:root[data-theme="dark"] .model-dropdown {
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.4);
}

.model-group-label {
  padding: 4px 12px 2px;
  font-size: 10px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--text-secondary);
  opacity: 0.7;
}

.model-divider {
  height: 1px;
  background: var(--border-color);
  margin: 4px 8px;
}

.model-empty {
  padding: 12px;
  font-size: 12px;
  color: var(--text-secondary);
  text-align: center;
}

.model-option {
  padding: 6px 12px;
  border-radius: 8px;
  cursor: pointer;
  transition: background 0.12s;
}

.model-option:hover {
  background: var(--hover-bg);
}

.model-option.active {
  background: var(--active-bg);
}

.model-option-name {
  font-size: 13px;
  font-weight: 500;
  color: var(--text-color);
}

/* Dropdown transition */
.dropdown-enter-active,
.dropdown-leave-active {
  transition: opacity 0.12s, transform 0.12s;
}

.dropdown-enter-from,
.dropdown-leave-to {
  opacity: 0;
  transform: translateY(4px);
}
</style>
