<script setup lang="ts">
import { computed } from "vue";
import { t } from "../i18n";

const props = withDefaults(defineProps<{
  title?: string;
  description?: string;
  hint?: string;
  compact?: boolean;
}>(), {
  title: "",
  description: "",
  hint: "",
  compact: false,
});

const resolvedTitle = computed(() => props.title || t("workspace.required.title"));
const resolvedDescription = computed(() => props.description || t("workspace.required.description"));
const resolvedHint = computed(() => props.hint || t("workspace.required.hint"));
</script>

<template>
  <div class="workspace-required-state" :class="{ compact }">
    <div class="workspace-required-card">
      <div class="workspace-required-icon" aria-hidden="true">
        <svg viewBox="0 0 16 16" fill="none">
          <path
            d="M2 3.5A1.5 1.5 0 0 1 3.5 2h2.1c.4 0 .78.16 1.06.44l.9.9c.19.18.44.29.7.29h4.24A1.5 1.5 0 0 1 14 5.13v5.37A1.5 1.5 0 0 1 12.5 12h-9A1.5 1.5 0 0 1 2 10.5v-7Z"
            stroke="currentColor"
            stroke-width="1.2"
          />
          <path
            d="M2.75 6.25h10.5"
            stroke="currentColor"
            stroke-width="1.2"
            stroke-linecap="round"
          />
        </svg>
      </div>
      <div class="workspace-required-copy">
        <div class="workspace-required-title">{{ resolvedTitle }}</div>
        <div class="workspace-required-description">{{ resolvedDescription }}</div>
        <div class="workspace-required-hint">{{ resolvedHint }}</div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.workspace-required-state {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  box-sizing: border-box;
}

.workspace-required-state.compact {
  align-items: flex-start;
  justify-content: flex-start;
  padding: 0;
}

.workspace-required-card {
  width: min(560px, 100%);
  display: flex;
  align-items: flex-start;
  gap: 14px;
  padding: 18px 20px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: color-mix(in srgb, var(--sidebar-bg, var(--bg-color)) 88%, transparent);
  box-sizing: border-box;
}

.workspace-required-state.compact .workspace-required-card {
  width: 100%;
}

.workspace-required-icon {
  width: 34px;
  height: 34px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: var(--bg-color);
  color: var(--text-secondary);
}

.workspace-required-icon svg {
  width: 16px;
  height: 16px;
}

.workspace-required-copy {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.workspace-required-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.workspace-required-description {
  font-size: 12px;
  line-height: 1.5;
  color: var(--text-color);
}

.workspace-required-hint {
  font-size: 12px;
  line-height: 1.5;
  color: var(--text-secondary);
}
</style>
