<script setup lang="ts">
import { computed, ref, shallowRef, watch, type CSSProperties } from "vue";
import type { AgentInfo, EffortLevel, ModelOption } from "../types";
import { t } from "../i18n";
import { visibleProviderOrder } from "../config/providerVisibility";
import { formatModelOptionDisplayName } from "../utils/modelDisplay";
import { groupModelsForSelector, modelListEntryName, type ModelSelectorGroup } from "../utils/modelGrouping";
import { agentProjectTypesLabel } from "../utils/agentProjectTypes";
import { createAnimationFrameResizeObserver } from "../composables/resizeObserver";
import BaseSwitch from "./ui/BaseSwitch.vue";
import WorktreeSelectorPanel from "./chat/WorktreeSelectorPanel.vue";
import type { WorkspaceRef } from "../services/project";
import type { ManagedWorktree } from "../services/worktrees";

const props = defineProps<{
  models: ModelOption[];
  agents?: AgentInfo[];
  selectedAgentId?: string;
  agentLocked?: boolean;
  selectedId: string;
  effort: EffortLevel;
  efforts?: EffortLevel[];
  effortSupported?: boolean;
  multiAgentEnabled?: boolean;
  fastModeEnabled?: boolean;
  fastModeAvailable?: boolean;
  align?: "start" | "end";
  disabled?: boolean;
  workspaceRef?: WorkspaceRef | null;
  worktreeEnabled?: boolean;
  worktreeLocked?: boolean;
  selectWorktree?: (item: ManagedWorktree) => Promise<void>;
}>();

const emit = defineEmits<{
  selectAgent: [id: string];
  selectModel: [id: string];
  selectEffort: [level: EffortLevel];
  selectMultiAgent: [enabled: boolean];
  selectFastMode: [enabled: boolean];
  worktreeBusy: [busy: boolean];
}>();

interface LevelOption {
  value: EffortLevel;
  label: string;
  desc: string;
}

const open = ref(false);
const selectorRef = ref<HTMLElement | null>(null);
const dropdownRef = ref<HTMLElement | null>(null);
const dropdownTarget = shallowRef<HTMLElement | null>(null);
const dropdownStyle = ref<CSSProperties>({ visibility: "hidden" });
const worktreeBusy = ref(false);
const hasWorktreePanel = computed(() => !!props.worktreeEnabled && !!props.workspaceRef && !!props.selectWorktree);
function setWorktreeBusy(value: boolean) { worktreeBusy.value = value; emit("worktreeBusy", value); }
function close() { if (!worktreeBusy.value) open.value = false; }
function positionDropdown() {
  if (!open.value || !selectorRef.value || !dropdownRef.value) return;
  const ownerWindow = selectorRef.value.ownerDocument.defaultView;
  if (!ownerWindow) return;
  const bounds = selectorRef.value.getBoundingClientRect();
  // Inactive workbench tabs stay mounted with v-show. Hide their teleported
  // menu as well, without interrupting an in-flight worktree operation.
  if (!bounds.width || !bounds.height) {
    dropdownStyle.value = { visibility: "hidden" };
    return;
  }
  const width = dropdownRef.value.offsetWidth;
  const left = props.align === "start" ? bounds.left : bounds.right - width;
  dropdownStyle.value = {
    left: `${Math.max(12, Math.min(left, ownerWindow.innerWidth - width - 12))}px`,
    bottom: `${ownerWindow.innerHeight - bounds.top + 6}px`,
  };
}

watch(open, (isOpen, _previous, onCleanup) => {
  if (!isOpen) return;
  const ownerDocument = selectorRef.value?.ownerDocument;
  const ownerWindow = ownerDocument?.defaultView;
  if (!ownerDocument || !ownerWindow) return;
  positionDropdown();
  const observer = createAnimationFrameResizeObserver(positionDropdown);
  if (dropdownRef.value) observer?.observe(dropdownRef.value);
  // Follow split resizing and composer height changes, including tab visibility.
  for (let element = selectorRef.value; element; element = element.parentElement) {
    observer?.observe(element);
  }
  ownerDocument.addEventListener("click", onClickOutside);
  ownerWindow.addEventListener("resize", positionDropdown);
  ownerWindow.addEventListener("scroll", positionDropdown, true);
  onCleanup(() => {
    observer?.disconnect();
    ownerDocument.removeEventListener("click", onClickOutside);
    ownerWindow.removeEventListener("resize", positionDropdown);
    ownerWindow.removeEventListener("scroll", positionDropdown, true);
  });
}, { flush: "post" });
watch(() => props.align, positionDropdown, { flush: "post" });

const providerLabels = computed<Record<string, string>>(() => ({
  openrouter: "OpenRouter",
  anthropic: t("model.provider.anthropic"),
  claude_code: t("model.provider.claude_code"),
  openai_codex: t("model.provider.openai"),
  custom: t("model.provider.custom"),
  mock: t("model.provider.mock"),
}));

const providerShortLabels = computed<Record<string, string>>(() => ({
  openrouter: "OR",
  anthropic: t("model.provider.anthropic.short"),
  claude_code: t("model.provider.claude_code.short"),
  openai_codex: t("model.provider.openai.short"),
  custom: t("model.provider.custom"),
  mock: t("model.provider.mock.short"),
}));

const selectedModel = computed(() =>
  props.models.find((model) => model.id === props.selectedId) ?? null,
);
const hasAgentPanel = computed(() => (props.agents?.length ?? 0) > 0);
const selectedAgent = computed(() =>
  props.agents?.find((agent) => agent.id === props.selectedAgentId) ?? null,
);

const selectedDisplayName = computed(() => {
  const selected = selectedModel.value;
  if (!selected) return "Model";
  const displayName = optionDisplayName(selected);
  const duplicated = props.models.some((model) => model.id !== selected.id && optionDisplayName(model) === displayName);
  if (!duplicated) return displayName;
  if (selected.provider === "custom" && selected.customProviderName) {
    return `${selected.customProviderName} / ${displayName}`;
  }
  const prefix = providerShortLabels.value[selected.provider] || selected.provider;
  return `${prefix} / ${displayName}`;
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

const groupedModels = computed<ModelSelectorGroup[]>(() =>
  groupModelsForSelector(props.models, visibleProviderOrder, providerLabels.value),
);

const triggerTitle = computed(() => {
  const modelTitle = selectedModel.value?.id || t("model.select");
  const parts = [selectedAgent.value?.name, modelTitle].filter(Boolean);
  if (props.effortSupported && currentLevel.value) parts.push(currentLevel.value.desc);
  return parts.join(" / ");
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
  if (props.disabled || worktreeBusy.value) return;
  // Shared workbench windows keep this component in the opener's Vue runtime.
  // Pass the actual owner's body so Teleport does not query the opener's document.
  if (!open.value) dropdownTarget.value = selectorRef.value?.ownerDocument.body ?? null;
  open.value = !open.value;
}

function selectModel(id: string) {
  if (worktreeBusy.value || props.disabled) return;
  emit("selectModel", id);
  if (!hasAgentPanel.value || !props.effortSupported) open.value = false;
}

function selectAgent(id: string) {
  if (props.agentLocked || worktreeBusy.value || props.disabled) return;
  emit("selectAgent", id);
}

function modelDisplayName(model: ModelOption): string {
  return formatModelOptionDisplayName(model, props.fastModeEnabled === true);
}

function optionDisplayName(model: ModelOption): string {
  if (model.provider === "custom") return modelListEntryName(model);
  return modelDisplayName(model);
}

function selectEffort(level: EffortLevel) {
  if (worktreeBusy.value || props.disabled) return;
  emit("selectEffort", level);
  open.value = false;
}

function selectFastMode(enabled: boolean) {
  emit("selectFastMode", enabled);
}

function onClickOutside(event: MouseEvent) {
  const path = event.composedPath();
  if (selectorRef.value && !path.includes(selectorRef.value)
    && (!dropdownRef.value || !path.includes(dropdownRef.value))) {
    close();
  }
}
</script>

<template>
  <div class="model-effort-selector" ref="selectorRef" @keydown.esc.stop="close">
    <button
      class="model-effort-trigger ui-select-none"
      :class="{ open, disabled }"
      type="button"
      :title="triggerTitle"
      @click="toggle"
    >
      <span v-if="hasAgentPanel && selectedAgent" class="model-effort-agent">
        {{ selectedAgent.name }}
      </span>
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

    <Teleport v-if="dropdownTarget" :to="dropdownTarget">
      <Transition name="dropdown">
        <div
          v-if="open"
          ref="dropdownRef"
          class="model-effort-dropdown"
          :style="dropdownStyle"
          @keydown.esc.stop="close"
          :class="{
            'has-agent': hasAgentPanel,
            'has-effort': true,
            'has-worktree': hasWorktreePanel,
            'align-start': align === 'start',
          }"
        >
          <div v-if="hasAgentPanel" class="model-effort-agent-panel">
            <div class="model-effort-section-label">Agent</div>
            <button
              v-for="agent in agents"
              :key="agent.id"
              type="button"
              class="model-effort-option ui-select-none"
              :class="{ active: agent.id === selectedAgentId }"
              :disabled="disabled || worktreeBusy || agentLocked"
              :title="agent.description"
              @click="selectAgent(agent.id)"
            >
              <span class="model-effort-option-name">{{ agent.name }}</span>
              <span v-if="agentProjectTypesLabel(agent)" class="model-effort-option-meta">
                {{ agentProjectTypesLabel(agent) }}
              </span>
            </button>
          </div>

          <div class="model-effort-model-panel">
            <template v-if="groupedModels.length === 0">
              <div class="model-effort-empty">{{ t("model.noProvider") }}</div>
            </template>
            <template v-for="(group, groupIndex) in groupedModels" :key="group.key">
              <div v-if="groupIndex > 0" class="model-effort-divider"></div>
              <div class="model-effort-section-header">
                <div class="model-effort-section-label">{{ group.label }}</div>
                <div
                  v-if="group.provider === 'openai_codex'"
                  class="model-effort-fast-toggle"
                  :title="t('model.fastHint')"
                  @click.stop
                >
                  <span>{{ t("model.fast") }}</span>
                  <BaseSwitch
                    :model-value="fastModeEnabled === true"
                    :disabled="disabled || worktreeBusy || fastModeAvailable !== true"
                    :aria-label="t('model.fast')"
                    @update:model-value="selectFastMode"
                  />
                </div>
              </div>
              <button
                v-for="model in group.models"
                :key="model.id"
                type="button"
                class="model-effort-option ui-select-none"
                :class="{ active: model.id === selectedId }"
                :disabled="disabled || worktreeBusy"
                @click="selectModel(model.id)"
              >
                <span class="model-effort-option-name">{{ optionDisplayName(model) }}</span>
              </button>
            </template>
          </div>

          <div class="model-effort-effort-panel">
            <div class="model-effort-section-label">{{ t("thinking.selector.title") }}</div>
            <template v-if="effortSupported">
              <button
                v-for="level in levels"
                :key="level.value"
                type="button"
                class="model-effort-option ui-select-none"
                :class="{ active: level.value === effort }"
                :disabled="disabled || worktreeBusy"
                @click="selectEffort(level.value)"
              >
                <span class="model-effort-option-name">{{ level.label }}</span>
              </button>
            </template>
            <div class="model-effort-divider"></div>
            <button
              type="button"
              class="model-effort-option model-effort-multi-agent ui-select-none"
              :class="{ active: multiAgentEnabled }"
              :aria-pressed="multiAgentEnabled === true"
              :disabled="disabled || worktreeBusy"
              @click="emit('selectMultiAgent', !multiAgentEnabled)"
            >
              <span class="model-effort-option-name">Multi-Agent</span>
            </button>
          </div>
          <WorktreeSelectorPanel
            v-if="hasWorktreePanel && workspaceRef && selectWorktree"
            :workspace-ref="workspaceRef"
            :locked="worktreeLocked || disabled"
            :select-worktree="selectWorktree"
            @busy="setWorktreeBusy"
            @selected="open = false"
          />
        </div>
      </Transition>
    </Teleport>
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
  gap: 4px;
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

.model-effort-agent {
  flex: 0 1 auto;
  min-width: 0;
  max-width: 110px;
  overflow: hidden;
  color: inherit;
  font-size: inherit;
  font-weight: inherit;
  text-overflow: ellipsis;
}

.model-effort-agent::after {
  content: "|";
  margin-left: 4px;
  color: var(--text-secondary);
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
  position: fixed;
  min-width: 260px;
  max-width: min(420px, calc(100vw - 24px));
  max-height: min(420px, calc(100vh - 160px));
  overflow: hidden;
  padding: 4px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: var(--bg-color);
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.12);
  z-index: 2000;
  transform-origin: bottom right;
}

.model-effort-dropdown.align-start {
  transform-origin: bottom left;
}

.model-effort-dropdown.has-effort:not(.has-agent) {
  width: min(420px, calc(100vw - 24px));
  display: grid;
  grid-template-columns: minmax(0, 1fr) 120px;
}

.model-effort-dropdown.has-agent {
  width: min(560px, calc(100vw - 24px));
  max-width: calc(100vw - 24px);
  display: grid;
  grid-template-columns: 150px minmax(0, 1fr);
}

.model-effort-dropdown.has-agent.has-effort {
  width: min(660px, calc(100vw - 24px));
  grid-template-columns: 150px minmax(0, 1fr) 120px;
}

.model-effort-dropdown.has-worktree,
.model-effort-dropdown.has-effort.has-worktree:not(.has-agent) {
  width: min(740px, calc(100vw - 24px));
  max-width: calc(100vw - 24px);
  overflow-x: auto;
  grid-template-columns: minmax(160px, 1fr) 120px minmax(200px, 240px);
}

.model-effort-dropdown.has-agent.has-effort.has-worktree {
  width: min(900px, calc(100vw - 24px));
  grid-template-columns: minmax(100px, 150px) minmax(160px, 1fr) 120px minmax(200px, 240px);
}

:root[data-theme="dark"] .model-effort-dropdown {
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.4);
}

.model-effort-agent-panel,
.model-effort-model-panel,
.model-effort-effort-panel {
  min-width: 0;
  max-height: min(404px, calc(100vh - 176px));
  overflow-y: auto;
}

.model-effort-agent-panel {
  border-right: 1px solid var(--border-color);
  padding-right: 4px;
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

.model-effort-section-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding-right: 8px;
}

.model-effort-fast-toggle {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  color: var(--text-secondary);
  font-size: 11px;
  white-space: nowrap;
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

.model-effort-option:disabled {
  cursor: default;
  opacity: 0.55;
}

.model-effort-option:disabled:hover {
  background: transparent;
}

.model-effort-option.active {
  background: var(--active-bg);
}

.model-effort-multi-agent.active {
  background: color-mix(in srgb, var(--accent-color) 18%, var(--bg-color));
  box-shadow: inset 0 0 0 1px var(--accent-border);
}

.model-effort-multi-agent.active .model-effort-option-name {
  color: var(--accent-color);
  font-weight: 600;
}

.model-effort-option:focus-visible {
  outline: 1px solid var(--accent-color);
  outline-offset: -1px;
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

.model-effort-option-meta {
  flex-shrink: 0;
  color: var(--text-tertiary, var(--text-secondary));
  font-size: 10px;
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
