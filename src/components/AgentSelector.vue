
<script setup lang="ts">
import { ref, onMounted, onUnmounted } from "vue";
import type { AgentInfo } from "../types";
import { t } from "../i18n";

const props = defineProps<{
  agents: AgentInfo[];
  selectedId: string;
  disabled?: boolean;
}>();

const emit = defineEmits<{
  select: [id: string];
}>();

const open = ref(false);
const selectorRef = ref<HTMLElement | null>(null);

const selectedAgent = () => props.agents.find((a) => a.id === props.selectedId);

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
  <div class="agent-selector" ref="selectorRef">
    <button
      class="agent-trigger"
      :class="{ open, disabled }"
      @click="toggle"
      :title="selectedAgent()?.description || t('agent.selector.select')"
    >
      <span class="agent-name">{{ selectedAgent()?.name || "Agent" }}</span>
      <span class="agent-chevron">&#9662;</span>
    </button>

    <Transition name="dropdown">
      <div v-if="open" class="agent-dropdown">
        <div
          v-for="agent in agents"
          :key="agent.id"
          class="agent-option"
          :class="{ active: agent.id === selectedId }"
          @click="select(agent.id)"
        >
          <div class="agent-option-name">
            {{ agent.name }}
            <span v-if="agent.isDefault" class="default-badge">{{ t("agent.selector.default") }}</span>
          </div>
          <div class="agent-option-desc">{{ agent.description }}</div>
        </div>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.agent-selector {
  position: relative;
}

.agent-trigger {
  display: flex;
  align-items: center;
  gap: 4px;
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

.agent-trigger:hover:not(.disabled) {
  color: var(--text-color);
  border-color: var(--text-secondary);
}

.agent-trigger.open {
  color: var(--text-color);
  border-color: var(--accent-color);
}

.agent-trigger.disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.agent-chevron {
  font-size: 10px;
  transition: transform 0.15s;
}

.agent-trigger.open .agent-chevron {
  transform: rotate(180deg);
}

.agent-dropdown {
  position: absolute;
  bottom: calc(100% + 6px);
  left: 0;
  min-width: 260px;
  background: var(--bg-color);
  border: 1px solid var(--border-color);
  border-radius: 10px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.12);
  padding: 4px;
  z-index: 100;
}

:root[data-theme="dark"] .agent-dropdown {
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.4);
}

.agent-option {
  padding: 8px 12px;
  border-radius: 8px;
  cursor: pointer;
  transition: background 0.12s;
}

.agent-option:hover {
  background: var(--hover-bg);
}

.agent-option.active {
  background: var(--active-bg);
}

.agent-option-name {
  font-size: 13px;
  font-weight: 500;
  color: var(--text-color);
  display: flex;
  align-items: center;
  gap: 6px;
}

.default-badge {
  font-size: 10px;
  font-weight: 500;
  color: var(--text-secondary);
  background: var(--hover-bg);
  padding: 1px 6px;
  border-radius: 4px;
}

.agent-option-desc {
  font-size: 11px;
  color: var(--text-secondary);
  margin-top: 2px;
  line-height: 1.4;
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
