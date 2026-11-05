<script setup lang="ts">
import BaseSwitch from "../ui/BaseSwitch.vue";

defineProps<{
  label: string;
  description: string;
  modelValue: boolean;
  /** Absent until the current value has loaded. */
  ready: boolean;
  busy?: boolean;
}>();

const emit = defineEmits<{ "update:modelValue": [value: boolean] }>();
</script>

<template>
  <div class="service-switch-card" :aria-busy="!ready">
    <div class="service-switch-info">
      <span class="service-switch-name">{{ label }}</span>
      <span class="service-switch-desc">{{ description }}</span>
    </div>
    <BaseSwitch
      v-if="ready"
      :model-value="modelValue"
      :disabled="busy"
      :aria-label="label"
      @update:model-value="emit('update:modelValue', $event)"
    />
    <span v-else class="switch-placeholder" aria-hidden="true" />
  </div>
</template>

<style scoped>
.service-switch-card {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 20px;
  max-width: 760px;
  padding: 12px 16px;
  border: 1px solid var(--border-color);
  border-radius: 10px;
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--sidebar-bg) 16%);
}
.service-switch-info {
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}
.service-switch-name {
  color: var(--text-color);
  font-size: 13px;
  font-weight: 500;
}
.service-switch-desc {
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.45;
}
.switch-placeholder {
  flex-shrink: 0;
  width: 32px;
  height: 18px;
}
</style>
