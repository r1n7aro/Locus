<script setup lang="ts">
import { t } from "../../i18n";
import BaseSwitch from "../ui/BaseSwitch.vue";

defineProps<{
  claudeCodeEnabled: boolean;
}>();

const emit = defineEmits<{
  "update:claudeCodeEnabled": [enabled: boolean];
}>();
</script>

<template>
  <div class="settings-section">
    <div class="section-label">{{ t("settings.experimental.title") }}</div>
    <p class="section-desc">{{ t("settings.experimental.desc") }}</p>

    <div class="feature-card">
      <div class="feature-row">
        <div class="feature-info">
          <span class="feature-name">{{ t("settings.models.claudeCodeEnable") }}</span>
          <span class="feature-desc">{{ t("settings.models.claudeCodeEnableHint") }}</span>
        </div>
        <BaseSwitch
          :model-value="claudeCodeEnabled"
          :aria-label="t('settings.models.claudeCodeEnable')"
          @update:model-value="emit('update:claudeCodeEnabled', $event)"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.feature-card {
  max-width: 760px;
  overflow: hidden;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--sidebar-bg) 16%);
}

.feature-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 12px 14px;
}

.feature-info {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 2px;
}

.feature-name {
  color: var(--text-color);
  font-size: 13px;
  font-weight: 600;
}

.feature-desc {
  color: var(--text-secondary);
  font-size: 11.5px;
  line-height: 1.45;
}
</style>
