
<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from "vue";
import type { EffortLevel } from "../types";
import { t } from "../i18n";

const props = defineProps<{
  effort: EffortLevel;
  efforts?: EffortLevel[];
  disabled?: boolean;
}>();

const emit = defineEmits<{
  select: [level: EffortLevel];
}>();

const open = ref(false);
const selectorRef = ref<HTMLElement | null>(null);

interface LevelOption {
  value: EffortLevel;
  label: string;
  desc: string;
}

const levels = computed<LevelOption[]>(() => {
  const defs: Record<EffortLevel, LevelOption> = {
    none: { value: "none", label: "None", desc: t("thinking.level.none") },
    low: { value: "low", label: "Low", desc: t("thinking.level.low") },
    medium: { value: "medium", label: "Med", desc: t("thinking.level.medium") },
    high: { value: "high", label: "High", desc: t("thinking.level.high") },
    xhigh: { value: "xhigh", label: "XHigh", desc: t("thinking.level.xhigh") },
  };
  const values: EffortLevel[] = props.efforts?.length
    ? props.efforts
    : ["none", "low", "medium", "high", "xhigh"];
  return values.map((value) => defs[value]);
});

const currentLevel = computed(() => levels.value.find((l) => l.value === props.effort) || levels.value[0]);

const levelColor = computed(() => {
  switch (props.effort) {
    case "low": return "var(--thinking-low, #38a169)";
    case "medium": return "var(--thinking-medium, #d69e2e)";
    case "high": return "var(--thinking-high, #dd6b20)";
    case "xhigh": return "var(--thinking-xhigh, #c05621)";
    default: return "var(--text-secondary)";
  }
});

function toggle() {
  if (props.disabled) return;
  open.value = !open.value;
}

function select(value: EffortLevel) {
  emit("select", value);
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
  <div class="thinking-selector" ref="selectorRef">
    <button
      class="thinking-trigger"
      :class="{ open, disabled }"
      @click="toggle"
      :title="t('thinking.selector.titleFull', currentLevel.label, currentLevel.desc)"
    >
      <svg class="thinking-icon" viewBox="0 0 16 16" fill="currentColor" width="12" height="12">
        <path d="M8 1C4.7 1 2 3.3 2 6.2c0 1.7.9 3.2 2.3 4.2.1.3.2.8.2 1.3 0 .5-.1 1-.2 1.3h7.4c-.1-.3-.2-.8-.2-1.3 0-.5.1-1 .2-1.3C13.1 9.4 14 7.9 14 6.2 14 3.3 11.3 1 8 1zm-2 14c0 .6.9 1 2 1s2-.4 2-1H6z"/>
      </svg>
      <span class="thinking-label" :style="{ color: levelColor }">{{ currentLevel.label }}</span>
      <span class="thinking-chevron">&#9662;</span>
    </button>

    <Transition name="dropdown">
      <div v-if="open" class="thinking-dropdown">
        <div class="thinking-header">{{ t("thinking.selector.title") }}</div>
        <div
          v-for="opt in levels"
          :key="opt.value"
          class="thinking-option"
          :class="{ active: opt.value === effort }"
          @click="select(opt.value)"
        >
          <div class="thinking-option-row">
            <span class="thinking-option-label">{{ opt.label }}</span>
            <span class="thinking-option-desc">{{ opt.desc }}</span>
          </div>
        </div>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.thinking-selector {
  position: relative;
  display: inline-flex;
  flex-shrink: 0;
}

.thinking-trigger {
  display: flex;
  align-items: center;
  gap: 4px;
  min-height: 28px;
  padding: 4px 8px;
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

.thinking-trigger:hover:not(.disabled) {
  color: var(--text-color);
  border-color: var(--text-secondary);
}

.thinking-trigger.open {
  color: var(--text-color);
  border-color: var(--accent-color);
}

.thinking-trigger.disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.thinking-icon {
  opacity: 0.7;
}

.thinking-label {
  font-weight: 500;
  transition: color 0.15s;
}

.thinking-chevron {
  font-size: 10px;
  transition: transform 0.15s;
}

.thinking-trigger.open .thinking-chevron {
  transform: rotate(180deg);
}

.thinking-dropdown {
  position: absolute;
  bottom: calc(100% + 6px);
  left: 0;
  right: auto;
  min-width: 180px;
  max-width: min(240px, calc(100vw - 24px));
  background: var(--bg-color);
  border: 1px solid var(--border-color);
  border-radius: 10px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.12);
  padding: 4px;
  z-index: 100;
  transform-origin: bottom left;
}

:root[data-theme="dark"] .thinking-dropdown {
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.4);
}

.thinking-header {
  padding: 4px 12px 2px;
  font-size: 10px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--text-secondary);
  opacity: 0.7;
}

.thinking-option {
  padding: 6px 12px;
  border-radius: 8px;
  cursor: pointer;
  transition: background 0.12s;
}

.thinking-option:hover {
  background: var(--hover-bg);
}

.thinking-option.active {
  background: var(--active-bg);
}

.thinking-option-row {
  display: flex;
  align-items: center;
  gap: 8px;
}

.thinking-option-label {
  font-size: 13px;
  font-weight: 500;
  color: var(--text-color);
  min-width: 40px;
}

.thinking-option-desc {
  font-size: 11px;
  color: var(--text-secondary);
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
