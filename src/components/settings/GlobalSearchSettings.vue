<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../i18n";
import { detectShortcutPlatform } from "../../composables/useKeyboardShortcuts";
import { useGlobalSearchSettings } from "../../composables/useGlobalSearchSettings";
import BaseCheckbox from "../ui/BaseCheckbox.vue";
import BaseSwitch from "../ui/BaseSwitch.vue";
const { state, set } = useGlobalSearchSettings();
const shortcut = detectShortcutPlatform() === "mac" ? "Cmd+F" : "Ctrl+F";
const groups = computed(() => [
  { label: t("globalSearch.knowledge"), fields: ["knowledgeTitle", "knowledgeContent"] as const },
  { label: t("globalSearch.session"), fields: ["sessionTitle", "sessionContent"] as const },
]);
</script>

<template>
  <div class="settings-section">
    <div class="section-label">{{ t("settings.tab.globalSearch") }}</div>
    <p class="section-desc">{{ t("globalSearch.shortcutHint", shortcut) }}</p>
    <div class="toggle-row">
      <BaseSwitch :model-value="state.enabled" :aria-label="t('globalSearch.enabled')" @update:model-value="set('enabled', $event)" />
      <span>{{ t("globalSearch.enabled") }}</span>
    </div>
    <div v-for="group in groups" :key="group.label" class="search-scope">
      <div class="section-label">{{ group.label }}</div>
      <div v-for="field in group.fields" :key="field" class="toggle-row">
        <BaseCheckbox :model-value="state[field]" :disabled="!state.enabled" :aria-label="t(`globalSearch.${field}`)" @update:model-value="set(field, $event)" />
        <span :class="{ disabled: !state.enabled }">{{ t(field.endsWith('Title') ? 'globalSearch.title' : 'globalSearch.content') }}</span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.toggle-row { display: flex; align-items: center; gap: 10px; padding: 8px 0; font-size: 13px; }
.search-scope { margin-top: 24px; }
.disabled { color: var(--text-secondary); }
</style>
